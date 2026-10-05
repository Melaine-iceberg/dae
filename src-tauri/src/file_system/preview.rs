use super::error::FileSystemError;
use crate::file_icons::FileIcon;
use image::{DynamicImage, GenericImageView};
use serde::Serialize;
use specta::Type;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Rendered thumbnail served as raw bytes over the custom `thumbnail://`
/// protocol. Raw bytes avoid the ~33% base64 inflation and JSON string
/// escaping that a `data:` URL over IPC would pay per image.
#[derive(Debug)]
pub struct RenderedThumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Source images above this size are skipped so browsing a folder of huge
/// archives-as-images cannot stall the UI.
const THUMBNAIL_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// Hard cap on decoded thumbnail pixels; larger images are downscaled in one
/// cheap `resize` step before the final smooth pass. An image past it is not
/// refused — it is handed to the desktop's own producer, which scales large
/// photos out of process instead of materialising them here.
const THUMBNAIL_MAX_DECODED_PIXELS: u64 = 40 * 1024 * 1024;

/// Whether an image of `width` × `height` is small enough to decode.
///
/// Split out of [`decode_and_scale`] so the boundary is testable without
/// materialising a 40 MP image, and so the rule has one definition rather than
/// the two spellings it used to have.
fn within_thumbnail_pixel_cap(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) <= THUMBNAIL_MAX_DECODED_PIXELS
}
const THUMBNAIL_CACHE_MAX_ENTRIES: usize = 256;

/// SVGs pass through to the webview as raw bytes (the browser rasterizes
/// them), capped so a hand-crafted multi-megabyte vector never floods IPC.
const SVG_MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
/// Shell thumbnails (PDF first page, video first frame, HEIC) are produced
/// by the OS handler; a 2GB cap keeps pathological files out.
const SHELL_THUMBNAIL_MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Capped in-memory cache keyed by path + mtime + size + target size.
/// Entries are `Arc`'d so lookups never clone image bytes; eviction drops
/// the oldest entry instead of clearing the map so scrolling back through a
/// large folder stays cache-hot.
struct RenderedCache {
    entries: HashMap<String, Arc<RenderedThumbnail>>,
    insertion_order: VecDeque<String>,
}

static THUMBNAIL_CACHE: Mutex<Option<RenderedCache>> = Mutex::new(None);
/// Icons extracted through the shell are comparatively expensive (COM +
/// handler invocation), so the icon cache keeps more entries than thumbnails.
static ICON_CACHE: Mutex<Option<RenderedCache>> = Mutex::new(None);
const ICON_CACHE_MAX_ENTRIES: usize = 512;

/// Icon keys already answered with a miss.
///
/// The counterpart of `ICON_CACHE`, and the reason an unknown extension is
/// cheap: a type the theme has nothing for is the ordinary case in a folder of
/// unfamiliar files, and without a remembered miss every request for it repeats
/// the whole freedesktop search — each theme in the chain, each data root, each
/// directory those themes declare, once per `.svg` and once per `.png`.
///
/// Cleared wholesale when it fills rather than evicting one key at a time:
/// recomputing a miss costs one more search, which is cheap, while evicting
/// rendered icons to make room for keys nobody asks about twice is not.
static ICON_MISS_KEYS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
const ICON_MISS_MAX_KEYS: usize = 1024;

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Extensions the `image` crate decodes on every supported platform.
///
/// Everything outside this set is the desktop's to render, which is what keeps
/// the two categories honest: a name in here means this process can draw the
/// file, and a name absent from it means nothing here can — whatever the shell,
/// `NSWorkspace` or a `.thumbnailer` file happens to have.
const IN_PROCESS_IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico",
];

/// Extensions with no in-process decoder, listed because some desktop *does*
/// have one: documents (a PDF's first page), video (a first frame), and the
/// image formats `image` cannot read at all — AVIF and JXL from modern phone and
/// screenshot pipelines, TGA and the portable bitmaps from older toolchains,
/// QOI/EXR/DDS from graphics work.
///
/// A `.jpg` is absent on purpose, and that absence is the performance argument:
/// decoding one in this process costs about 1.5 ms, where spawning a
/// `.thumbnailer` for the same file costs about 10 ms of process startup before
/// any decoding starts. Delegating a small photo would make the common case
/// seven times slower to save code, so the list is for the formats that would
/// otherwise be a type icon forever, not for the formats that already work.
///
/// Membership is a *possibility*, not a promise: [`crate::desktop_thumbnails`]
/// answers 404 when nothing installed claims the type, and so does macOS for
/// everything on this list.
const DESKTOP_RENDERED_EXTENSIONS: &[&str] = &[
    // Documents and video: the Windows shell handler, `.thumbnailer` files, or
    // the ffmpeg fallback, whichever the machine has.
    "pdf", "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv",
    // Still images this process cannot decode.
    "heic", "heif", "avif", "jxl", "apng", "tga", "qoi", "exr", "dds", "pbm", "pgm", "ppm",
];

fn is_in_process_image_extension(path: &str) -> bool {
    let extension = extension_of(Path::new(path));
    IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension.as_str())
}

/// Extensions with any thumbnail strategy on some platform. The frontend
/// uses this to decide which entries get an image slot; the protocol
/// handler answers 404 when the current platform lacks a producer.
pub fn is_thumbnail_extension(path: &str) -> bool {
    let extension = extension_of(Path::new(path));
    // SVG is neither: it streams through as bytes for the webview to
    // rasterize, so it is the one format with a producer on every platform and
    // no in-process decoder at all.
    extension == "svg"
        || IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension.as_str())
        || DESKTOP_RENDERED_EXTENSIONS.contains(&extension.as_str())
}

/// A protocol response whose body is borrowed-or-owned bytes.
///
/// The `Cow` is what lets one rendered resource answer several waiters: the
/// first owns the bytes, a duplicate copies them.
type ProtocolResponse = tauri::http::Response<Cow<'static, [u8]>>;

/// Renders allowed to run at once.
///
/// These used to run inline on the thread WebView2 raises
/// `WebResourceRequested` on — the UI thread — so a cold 24 MP thumbnail froze
/// the window for the length of a JPEG decode, and every request behind it
/// waited. They run here instead, which needs a bound of its own rather than the
/// async runtime's blocking pool: a decode holds up to
/// `THUMBNAIL_MAX_DECODED_PIXELS * 4` bytes of pixels, and scrolling a photo
/// folder puts dozens of requests in flight at once.
const RENDER_WORKERS: usize = 3;

/// Which producer a queued request wants.
#[derive(Clone, Copy)]
enum RenderKind {
    Thumbnail,
    FileIcon,
}

/// What a render request points at.
enum RenderSubject {
    /// One entry on disk, file or folder, drawn with whatever the OS has for
    /// *it*. `is_dir` and `version` travel in the URL because the listing that
    /// made the row already read both off the disk — see [`render_file_icon`].
    Entry {
        path: String,
        is_dir: bool,
        version: String,
    },
    /// A *type* rather than an entry: the icon every file sharing `extension` is
    /// registered to draw. One URL serves them all — see [`render_type_icon`].
    Type { extension: String },
    /// A freedesktop icon-theme name (or an absolute icon path, which the same
    /// search answers). Only `FileIcon` can render this.
    IconName(String),
}

/// One request the pool was handed: what to draw, and at what size.
struct RenderRequest {
    subject: RenderSubject,
    size: u16,
}

/// Everyone waiting on one resource, and the lock that lets them arrive while
/// it renders.
struct InflightRender<T> {
    waiters: Mutex<Vec<T>>,
}

/// Renders in flight, keyed by the request's query string.
///
/// The query is the request's identity: the frontend embeds mtime and size in
/// the URL's version tag, and a shared per-type URL leaves the entry out of it
/// altogether, so two requests carrying the same query are asking for the same
/// bytes. Deduplicating those is worth it because the duplicate arrives exactly
/// when serving it is most expensive — a fast scroll remounting a row while its
/// first fetch is still decoding. Keying on the raw query rather than on the
/// backend's own cache key is deliberate: reaching that key means parsing the
/// request and, for a thumbnail, reading the file's metadata — work this map
/// exists so nobody does twice.
///
/// The two halves of the protocol live on the type rather than on a free function
/// so they can be tested without a webview: the only thing a test cannot supply
/// is the waiter itself, and `UriSchemeResponder` is minted inside Tauri.
struct InflightRenders<T>(Mutex<Option<HashMap<String, Arc<InflightRender<T>>>>>);

/// The app's renders, waiting on [`tauri::UriSchemeResponder`]s.
static INFLIGHT: InflightRenders<tauri::UriSchemeResponder> = InflightRenders::new();

impl<T> InflightRenders<T> {
    const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Registers `waiter` for `query`. Returns `true` when this call is the one
    /// that has to run the render, `false` when it merely joined one already
    /// under way.
    ///
    /// Joining is a push onto the waiter list, never a wait: whichever thread
    /// finishes the render drains the list. Blocking here instead would let a few
    /// waiters occupy every pool thread, which is a deadlock.
    fn join(&self, query: &str, waiter: T) -> bool {
        let mut inflight = self
            .0
            .lock()
            .expect("the in-flight render map is never poisoned");
        let renders = inflight.get_or_insert_with(HashMap::new);

        if let Some(existing) = renders.get(query) {
            existing
                .waiters
                .lock()
                .expect("the render waiter list is never poisoned")
                .push(waiter);
            return false;
        }

        renders.insert(
            query.to_owned(),
            Arc::new(InflightRender {
                waiters: Mutex::new(vec![waiter]),
            }),
        );
        true
    }

    /// Removes `query` and returns everyone who was waiting on it.
    ///
    /// The entry leaves the map *before* the waiters are drained, and that
    /// ordering is the whole correctness argument. A request arriving after the
    /// removal starts a fresh render, which the cache answers; a request
    /// arriving before it joins a list this call is about to drain. Keeping the
    /// entry in place while responding would instead let a late arrival attach to
    /// a list nobody drains any more — a request that is never answered at all.
    fn drain(&self, query: &str) -> Vec<T> {
        let Some(inflight) = self
            .0
            .lock()
            .expect("the in-flight render map is never poisoned")
            .as_mut()
            .and_then(|renders| renders.remove(query))
        else {
            return Vec::new();
        };

        std::mem::take(
            &mut *inflight
                .waiters
                .lock()
                .expect("the render waiter list is never poisoned"),
        )
    }
}

/// One render, handed to the pool.
struct RenderJob {
    kind: RenderKind,
    query: String,
}

/// Serves `thumbnail://localhost/?path=...&size=...` with raw image bytes.
/// The webview fetches these in parallel through its HTTP stack and caches
/// them per URL, which replaces one base64 `invoke` payload per image.
///
/// Registered as an *asynchronous* scheme protocol (see `lib.rs`): the
/// synchronous form runs this handler inline in the WebView's request callback,
/// and image decoding has no business happening there. This only records the
/// request; the bytes are produced on the render pool.
pub fn handle_thumbnail_protocol(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    dispatch_render(RenderKind::Thumbnail, request, responder);
}

/// Serves `fileicon://localhost/?path=...&dir=...&v=...&size=...` with the operating
/// system's icon for the file or folder, on every platform — the extraction
/// itself is in [`crate::file_icons`]. `dir` and `v` are the entry's kind and its
/// listing-reported `mtime-size`, so nothing here re-reads the disk to name a
/// cache key.
///
/// `?ext=...&size=...` answers with the icon for a whole *type*, which every file
/// sharing the extension draws identically, and `?name=...&size=...` with the icon
/// theme's own icon for a freedesktop icon name — what a row describing an
/// application rather than a document needs.
///
/// Asynchronous for the same reason as [`handle_thumbnail_protocol`]: a shell
/// icon is a COM round-trip, and it used to happen on the UI thread.
pub fn handle_fileicon_protocol(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    dispatch_render(RenderKind::FileIcon, request, responder);
}

/// Queues one request, or attaches it to the render already running for the same
/// resource.
fn dispatch_render(
    kind: RenderKind,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let Some(query) = request.uri().query() else {
        // Malformed, and cheap to answer here: no render is involved, so there
        // is nothing to queue for.
        responder.respond(empty_response(400));
        return;
    };
    let query = query.to_owned();

    if !INFLIGHT.join(&query, responder) {
        // A render for this resource is already under way, and it will answer
        // this request along with the rest of its waiters.
        return;
    }

    let job = RenderJob {
        kind,
        query: query.clone(),
    };
    if render_queue().send(job).is_err() {
        // The pool is gone (the process is shutting down). Nothing will ever
        // drain this entry, so its waiters are answered here instead of never.
        for waiter in INFLIGHT.drain(&query) {
            waiter.respond(empty_response(500));
        }
    }
}

/// The one queue of render jobs, drained by [`RENDER_WORKERS`] threads.
///
/// A plain channel rather than a work-stealing pool: nothing here nests or
/// blocks, so the simplest thing that bounds concurrency is the right one. The
/// receiver is shared behind a mutex because `mpsc` has no multi-consumer
/// receiver; the guard is held only for the duration of the blocking `recv`, so
/// a busy worker never keeps a peer from picking up work.
fn render_queue() -> &'static Sender<RenderJob> {
    static QUEUE: OnceLock<Sender<RenderJob>> = OnceLock::new();

    QUEUE.get_or_init(|| {
        let (sender, receiver) = channel::<RenderJob>();
        let receiver = Arc::new(Mutex::new(receiver));

        for index in 0..RENDER_WORKERS {
            let receiver = Arc::clone(&receiver);
            let worker = thread::Builder::new()
                .name(format!("dae-render-{index}"))
                .spawn(move || render_loop(&receiver));

            if let Err(error) = worker {
                // Degraded, not fatal: the workers that did start still drain the
                // queue, so the worst case is a slower thumbnail, not a missing
                // one.
                log::warn!("Unable to start render worker {index}: {error}");
            }
        }

        sender
    })
}

/// Puts a render worker into a COM single-threaded apartment.
///
/// The shell paths reach `IShellItemImageFactory` and the registered thumbnail
/// providers behind it, and those are apartment-bound: a fresh thread has no
/// apartment to pin them to, so the call has to be made from one that does. Same
/// reasoning as `shell_commands::Host`, which documents it at length.
///
/// The apartment belongs to the thread, which lives as long as the process does,
/// so the matching `CoUninitialize` would never be reached anyway.
#[cfg(windows)]
fn enter_render_apartment() {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::core::HRESULT;

    // `S_FALSE` (a non-negative result) means the thread already had an
    // apartment, which is equally usable; only a hard failure is worth a word,
    // and even then the `image` decode path is unaffected.
    let result: HRESULT = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.0 < 0 {
        log::warn!(
            "Unable to initialize COM on a render worker: HRESULT {:#010x}",
            result.0
        );
    }
}

#[cfg(not(windows))]
fn enter_render_apartment() {}

/// Dispatches messages queued for a render worker's apartment.
///
/// A shell provider that calls back into an object living in our apartment — the
/// `IShellItem` behind `GetImage`, most obviously — arrives as a window message
/// and is only serviced when someone pumps. Pumping between jobs is what keeps
/// such a provider from waiting forever on a callback nobody will dispatch.
#[cfg(windows)]
fn pump_render_apartment() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    // SAFETY: `message` is a valid out-parameter, a zeroed filter range means
    // every message, and the queue belongs to this thread.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(not(windows))]
fn pump_render_apartment() {}

fn render_loop(receiver: &Mutex<Receiver<RenderJob>>) {
    // The apartment belongs to this thread, which runs for the process's
    // lifetime, so the matching `CoUninitialize` would never be reached anyway.
    // See `enter_render_apartment` for why the pool is apartment-threaded.
    enter_render_apartment();

    loop {
        // Scoped so the queue lock is not held while a render runs.
        let job = receiver
            .lock()
            .expect("the render queue receiver is never poisoned")
            .recv();

        let Ok(job) = job else {
            // Every sender is gone: the process is shutting down.
            return;
        };

        // A panicking render must not leak its entry in `INFLIGHT`, which would
        // leave everyone waiting on that query unanswered for good. Recovering
        // here turns a decoder that panics on a malformed file into one broken
        // thumbnail — the frontend falls back to its type icon — instead of a
        // request that never comes back. (Release builds abort on panic anyway;
        // this is what keeps the failure contained in dev and in tests.)
        let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_response(job.kind, &job.query)
        }))
        .unwrap_or_else(|_| empty_response(500));

        for waiter in INFLIGHT.drain(&job.query) {
            waiter.respond(response.clone());
        }

        // Between jobs is the only place a provider's callback into our
        // apartment can be dispatched — see `pump_render_apartment`.
        pump_render_apartment();
    }
}

/// Produces the response for one request. Runs on a render worker.
fn render_response(kind: RenderKind, query: &str) -> ProtocolResponse {
    let Some(request) = render_request_params(query) else {
        return empty_response(400);
    };

    let rendered = match (kind, request.subject) {
        (RenderKind::Thumbnail, RenderSubject::Entry { path, .. }) => {
            render_thumbnail(&path, request.size)
        }
        (RenderKind::FileIcon, RenderSubject::Entry { path, is_dir, version }) => {
            Ok(render_file_icon(&path, is_dir, &version, request.size))
        }
        (RenderKind::FileIcon, RenderSubject::Type { extension }) => {
            Ok(render_type_icon(&extension, request.size))
        }
        (RenderKind::FileIcon, RenderSubject::IconName(name)) => {
            Ok(render_named_icon(&name, request.size))
        }
        // A thumbnail is made from a file's bytes, so a type or a theme name is
        // no resource for it at all.
        (RenderKind::Thumbnail, _) => return empty_response(400),
    };

    match rendered {
        Ok(Some(rendered)) => tauri::http::Response::builder()
            .header("Content-Type", rendered.mime)
            // Every URL is versioned by what it is an icon *of* — an entry's
            // mtime and size, or a type and a size — so a given URL never
            // changes. See [`render_file_icon`] and [`render_type_icon`].
            .header("Cache-Control", "public, max-age=86400, immutable")
            .body(Cow::Owned(rendered.bytes.clone()))
            .unwrap_or_else(|_| empty_response(500)),
        // Unsupported files yield 404 so the frontend can fall back to the
        // file icon; genuinely broken reads surface as 500.
        Ok(None) => empty_response(404),
        Err(_) => empty_response(500),
    }
}

fn empty_response(status: u16) -> ProtocolResponse {
    tauri::http::Response::builder()
        .status(status)
        .body(Cow::Owned(Vec::new()))
        .expect("static protocol response is always valid")
}

/// Extracts what a render request names and the size it wants.
///
/// A request names one of three things: an entry on disk (the listing and the
/// preview pane), a *type* whose icon every entry of that type shares, or a
/// freedesktop icon-theme name — which is how a row that describes an
/// application rather than a document (the "Open With" picker) gets the icon the
/// desktop would draw for it.
///
/// An entry's request also carries what the listing already knew about it: `dir`
/// is the entry's kind, `v` its mtime and size. A thumbnail still reads its own
/// metadata, because it has to know the file's length before it knows what to
/// decode; an icon only needed the two values to name its cache key, and the
/// frontend has them.
fn render_request_params(query: &str) -> Option<RenderRequest> {
    let mut path: Option<String> = None;
    let mut extension: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version = String::new();
    let mut is_dir = false;
    let mut size: Option<u16> = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "path" => path = Some(percent_decode(value)),
            "ext" => extension = icon_extension(value),
            "name" => name = Some(percent_decode(value)),
            "dir" => is_dir = value == "1",
            "size" => size = value.parse().ok(),
            "v" => version = value.to_owned(),
            _ => {}
        }
    }

    // Most specific first: an entry outranks a type, and a type outranks a theme
    // name. Nothing sends two at once, and the order is here so a hand-built URL
    // cannot ask for both.
    let subject = match (path, extension, name) {
        (Some(path), _, _) => RenderSubject::Entry {
            path,
            is_dir,
            version,
        },
        (None, Some(extension), _) => RenderSubject::Type { extension },
        (None, None, Some(name)) => RenderSubject::IconName(name),
        (None, None, None) => return None,
    };

    Some(RenderRequest {
        subject,
        size: size.unwrap_or(256),
    })
}

/// Longest extension the shared-icon route accepts. Real ones run to a handful
/// of characters; a longer "extension" is a name, not a type.
const MAX_ICON_EXTENSION_LEN: usize = 24;

/// The extension a shared-icon request names, normalised for a cache key.
///
/// The value arrives through a URL and reaches the desktop as a *type* rather
/// than as a file, so anything that could read as a path — a separator, a dot, a
/// drive letter — is refused here rather than trusted by three platform backends
/// downstream. A refused extension makes the request a 400, and the row keeps the
/// glyph the frontend draws.
fn icon_extension(query_value: &str) -> Option<String> {
    let extension = percent_decode(query_value).to_ascii_lowercase();
    let names_a_type = !extension.is_empty()
        && extension.len() <= MAX_ICON_EXTENSION_LEN
        && extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');

    names_a_type.then_some(extension)
}

/// Percent-decoder for `encodeURIComponent`-encoded query values.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    output.push((high << 4) | low);
                    index += 3;
                } else {
                    output.push(b'%');
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn render_thumbnail(
    path_string: &str,
    size: u16,
) -> Result<Option<Arc<RenderedThumbnail>>, FileSystemError> {
    let path = Path::new(path_string);
    let extension = extension_of(path);
    if !is_thumbnail_extension(path_string) {
        return Ok(None);
    }

    let metadata = fs::metadata(path).map_err(FileSystemError::from)?;
    if !metadata.is_file() {
        return Ok(None);
    }

    // SVGs stream straight through as bytes — no decode, no cache entry.
    if extension == "svg" {
        if metadata.len() > SVG_MAX_SOURCE_BYTES {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(FileSystemError::from)?;
        return Ok(Some(Arc::new(RenderedThumbnail {
            mime: "image/svg+xml",
            bytes,
        })));
    }

    // Shell thumbnails cover formats Explorer itself thumbs (PDF pages,
    // video frames, HEIC); they are comparatively expensive, so the cache
    // matters more than for the cheap `image` crate path.
    //
    // They also cover the raster formats that are too large to decode in
    // process — see [`DecodeOutcome::Oversized`]. The source cap does not move
    // with that routing: a photo over the pixel cap is still a photo, and the
    // byte budget that keeps browsing a folder of huge archives-as-images from
    // stalling the UI is worth more than the rare 100 MP file it excludes.
    let is_shell_source = !is_in_process_image_extension(path_string);
    let source_cap = if is_shell_source {
        SHELL_THUMBNAIL_MAX_SOURCE_BYTES
    } else {
        THUMBNAIL_MAX_SOURCE_BYTES
    };
    if metadata.len() > source_cap {
        return Ok(None);
    }

    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    let cache_key = format!(
        "{}|{}|{}|{size}",
        path_string,
        modified_at.unwrap_or(0),
        metadata.len()
    );

    if let Some(cached) = lookup_cache(&THUMBNAIL_CACHE, &cache_key) {
        return Ok(Some(cached));
    }

    let thumbnail = if is_shell_source {
        // No producer for this file on this platform: the 404 that follows lets
        // the frontend fall back to its type icon.
        let Some(produced) = produce_shell_thumbnail(path_string, size, &metadata)? else {
            return Ok(None);
        };
        produced
    } else {
        match decode_and_scale(path, size)? {
            DecodeOutcome::Frame(thumbnail) => thumbnail,
            DecodeOutcome::Oversized => {
                // Too many pixels to hold in this process, but not a file the
                // desktop cannot make a thumbnail of: every platform's own
                // producer scales the image as it decodes, out of process and
                // in bounded memory. Refusing it here — which is what this used
                // to do — meant a 75 MP photo silently got a type icon while
                // every other viewer on the desktop showed it a picture.
                let Some(produced) = produce_shell_thumbnail(path_string, size, &metadata)? else {
                    return Ok(None);
                };
                produced
            }
        }
    };

    let thumbnail = Arc::new(thumbnail);
    store_cache(
        &THUMBNAIL_CACHE,
        THUMBNAIL_CACHE_MAX_ENTRIES,
        cache_key,
        Arc::clone(&thumbnail),
    );
    Ok(Some(thumbnail))
}

/// The OS icon for one entry, file or folder; `None` means the frontend keeps
/// its own type artwork.
///
/// `is_dir` and `version` arrive with the request instead of being read from the
/// disk. The listing that made this row stat'd the file to report its kind, its
/// mtime and its size, and those three are everything this function used to go
/// and fetch again: a `stat` per icon request, on the one pool whose entire reason
/// for existing is to keep blocking reads off the UI thread, for a value the
/// caller already held. `version` is the listing's `mtime-size`, so a replaced
/// file lands on a new cache key exactly as it lands on a new URL.
fn render_file_icon(
    path_string: &str,
    is_dir: bool,
    version: &str,
    size: u16,
) -> Option<Arc<RenderedThumbnail>> {
    let size = size.clamp(16, 256);
    let cache_key = format!("icon|{path_string}|{version}|{size}");

    cached_icon(cache_key, || {
        crate::file_icons::extract(path_string, u32::from(size), is_dir)
    })
}

/// The icon the desktop draws for a whole *type*, which every file sharing
/// `extension` gets identically.
///
/// Keyed and fetched by extension rather than by path, and that is the whole
/// performance argument: a column of 800 `.rs` files asks for one URL, the
/// webview's own cache answers 799 of them, and the shell is asked once. Going
/// through a *sample* file of the type would have meant naming one — and then
/// resolving it, which is the per-request work this route exists to remove.
fn render_type_icon(extension: &str, size: u16) -> Option<Arc<RenderedThumbnail>> {
    let size = size.clamp(16, 256);
    let cache_key = format!("icon-ext|{extension}|{size}");

    cached_icon(cache_key, || {
        crate::file_icons::extract_type(extension, u32::from(size))
    })
}

/// The cached icon for `cache_key`, drawing it through `produce` on a miss.
///
/// Both answers are remembered, and the miss is the one that matters for the
/// shared keys: an extension no theme or shell class claims is the ordinary case
/// in a folder of unfamiliar files, and an unremembered miss repeats the entire
/// search behind it — every theme in the inheritance chain, every XDG data root,
/// every directory each of those declares, for `.svg` and `.png` alike — for each
/// request that arrives. A per-entry key carries the listing's mtime and size, so
/// an entry that changes gets a new key and asks again.
fn cached_icon(
    cache_key: String,
    produce: impl FnOnce() -> Option<FileIcon>,
) -> Option<Arc<RenderedThumbnail>> {
    if let Some(cached) = lookup_cache(&ICON_CACHE, &cache_key) {
        return Some(cached);
    }
    if is_remembered_icon_miss(&cache_key) {
        return None;
    }

    let Some(icon) = produce() else {
        remember_icon_miss(cache_key);
        return None;
    };

    let rendered = Arc::new(RenderedThumbnail {
        mime: icon.mime,
        bytes: icon.bytes,
    });
    store_cache(
        &ICON_CACHE,
        ICON_CACHE_MAX_ENTRIES,
        cache_key,
        Arc::clone(&rendered),
    );
    Some(rendered)
}

fn is_remembered_icon_miss(cache_key: &str) -> bool {
    ICON_MISS_KEYS
        .lock()
        .ok()
        .is_some_and(|misses| misses.as_ref().is_some_and(|misses| misses.contains(cache_key)))
}

fn remember_icon_miss(cache_key: String) {
    if let Ok(mut guard) = ICON_MISS_KEYS.lock() {
        let misses = guard.get_or_insert_with(HashSet::new);
        if misses.len() >= ICON_MISS_MAX_KEYS {
            misses.clear();
        }
        misses.insert(cache_key);
    }
}

/// The theme's icon for an application name, for a row that describes a program
/// rather than a document.
///
/// Keyed by name + size alone. There is no `mtime` to version by — the name is
/// not a path, and the theme that answers it is fixed for the life of the
/// process — so a name that resolves once keeps resolving the same way, which is
/// what the immutable cache header on the response promises.
fn render_named_icon(name: &str, size: u16) -> Option<Arc<RenderedThumbnail>> {
    let size = size.clamp(16, 256);
    let cache_key = format!("icon-name|{name}|{size}");

    cached_icon(cache_key, || {
        crate::file_icons::named(name, u32::from(size))
    })
}

/// Windows shell icon extraction: `IShellItemImageFactory` resolves whatever
/// Explorer would show — the target icon for `.lnk`/`.url` shortcuts, the
/// embedded icon for executables, the registered handler icon otherwise.
///
/// Two callers: [`crate::file_icons`] on the render pool, and `shell_commands.rs`
/// for the icon paths an `IExplorerCommand` reports. They share the apartment
/// and the cache here for exactly that reason.
#[cfg(windows)]
pub(crate) fn extract_file_icon_png(path: &str, size: u32) -> Option<Vec<u8>> {
    use windows::Win32::UI::Shell::{SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT};
    // ICONONLY keeps document thumbnails out — views pair these icons
    // with their own image thumbnails.
    shell_image_png(path, size, SIIGBF_ICONONLY | SIIGBF_RESIZETOFIT)
}

/// First-page / first-frame thumbnails from the registered shell thumbnail
/// handler (Edge for PDFs, the WMP/Media handler for videos, the HEIF
/// extensions for `.heic`). Files without a handler yield `None`, which the
/// frontend turns into a type-icon fallback.
#[cfg(windows)]
fn extract_shell_thumbnail_png(path: &str, size: u32) -> Option<Vec<u8>> {
    use windows::Win32::UI::Shell::SIIGBF_RESIZETOFIT;
    shell_image_png(path, size, SIIGBF_RESIZETOFIT)
}

/// Shared `IShellItemImageFactory` pipeline: resolves the shell image for
/// `path` at `size` and rasterizes it to PNG.
#[cfg(windows)]
fn shell_image_png(
    path: &str,
    size: u32,
    flags: windows::Win32::UI::Shell::SIIGBF,
) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::Graphics::Gdi::HGDIOBJ;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName};
    use windows::core::HSTRING;

    unsafe {
        // Protocol-handler threads may never have initialized COM; the shell
        // item APIs require an apartment.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let bitmap = factory
            .GetImage(
                SIZE {
                    cx: size as i32,
                    cy: size as i32,
                },
                flags,
            )
            .ok()?;

        let result = bitmap_to_png(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        result
    }
}

/// Copies a shell-produced HBITMAP into top-down RGBA pixels and encodes it
/// as PNG. Shared with `shell_commands.rs`, which reaches a bitmap through
/// `SHDefExtractIconW` when a command names an icon resource rather than a file.
#[cfg(windows)]
pub(crate) fn bitmap_to_png(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW,
        HGDIOBJ, ReleaseDC,
    };

    unsafe {
        let mut info = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut BITMAP as *mut core::ffi::c_void),
        ) == 0
        {
            return None;
        }
        let width = info.bmWidth as usize;
        let height = info.bmHeight as usize;
        if width == 0 || height == 0 {
            return None;
        }

        // Top-down 32bpp read so scanlines arrive in image order.
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: info.bmWidth,
                biHeight: -(info.bmHeight),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; width * height * 4];
        // GetDIBits wants a real DC for format negotiation; the screen DC
        // is process-wide and costs nothing here.
        let dc = GetDC(None);
        let copied = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, dc);
        if copied == 0 {
            return None;
        }

        // Shell bitmaps are BGRA; swap channels for the `image` crate.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }

        let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels)?;
        let mut buffer = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut buffer),
                image::ImageFormat::Png,
            )
            .ok()?;
        Some(buffer)
    }
}

/// The first-page / first-frame producers that are not the `image` crate, one
/// per platform, behind the seam [`render_thumbnail`] asks its question through.
///
/// The answer carries its own mime rather than being labelled `image/png` here,
/// because what it takes to make one of these is whichever producer the desktop
/// declares: Windows' shell hands back a bitmap this file encodes, and on Linux
/// [`crate::desktop_thumbnails`] runs the `.thumbnailer` files somebody else
/// shipped, which are only checked for being PNG.
///
/// `metadata` is the caller's, already read for the size cap and the cache key:
/// Linux needs the source's mtime to decide whether a shared-cache thumbnail is
/// still current, and re-`stat`ing it here would be a second read of the same
/// inode on the thread this module exists to keep free of blocking work.
#[cfg(windows)]
fn produce_shell_thumbnail(
    path: &str,
    size: u16,
    _metadata: &fs::Metadata,
) -> Result<Option<RenderedThumbnail>, FileSystemError> {
    Ok(
        extract_shell_thumbnail_png(path, u32::from(size)).map(|bytes| RenderedThumbnail {
            mime: "image/png",
            bytes,
        }),
    )
}

/// The freedesktop thumbnailers, run and cached by [`crate::desktop_thumbnails`].
#[cfg(target_os = "linux")]
fn produce_shell_thumbnail(
    path: &str,
    size: u16,
    metadata: &fs::Metadata,
) -> Result<Option<RenderedThumbnail>, FileSystemError> {
    Ok(
        crate::desktop_thumbnails::extract(path, u32::from(size), metadata.modified().ok()).map(
            |produced| RenderedThumbnail {
                mime: produced.mime,
                bytes: produced.bytes,
            },
        ),
    )
}

// macOS has no in-process renderer wired up yet: `NSWorkspace` would be the
// right call, and until it is made the frontend falls back to its type glyph for
// PDF, video and HEIC.
#[cfg(not(any(windows, target_os = "linux")))]
fn produce_shell_thumbnail(
    _path: &str,
    _size: u16,
    _metadata: &fs::Metadata,
) -> Result<Option<RenderedThumbnail>, FileSystemError> {
    Ok(None)
}

#[cfg(all(test, windows))]
mod tests {
    use super::extract_file_icon_png;

    #[test]
    fn extracts_png_icon_for_executable() {
        let bytes = extract_file_icon_png("C:\\Windows\\System32\\notepad.exe", 64)
            .expect("notepad.exe exposes a shell icon");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(bytes.len() > 100);
    }

    #[test]
    fn missing_path_yields_none() {
        assert!(extract_file_icon_png("C:\\does-not-exist.lnk", 64).is_none());
    }
}

/// Tests for the parts of the render pool that do not need a webview.
///
/// The waiter type is what a test cannot supply — `UriSchemeResponder` is minted
/// inside Tauri — so the protocol is exercised over `&str`, which is the reason
/// [`InflightRenders`] is generic in the first place.
#[cfg(test)]
mod render_tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{
        DecodeOutcome, FileIcon, ICON_CACHE, InflightRenders, MAX_ICON_EXTENSION_LEN, RenderKind,
        RenderSubject, THUMBNAIL_MAX_DECODED_PIXELS, cached_icon, decode_and_scale, icon_extension,
        is_remembered_icon_miss, is_thumbnail_extension, lookup_cache, render_request_params,
        render_response, within_thumbnail_pixel_cap,
    };

    /// The deduplication itself: a second request for a resource already being
    /// rendered waits for that render instead of starting another.
    #[test]
    fn a_second_request_for_the_same_query_joins_the_render() {
        let renders = InflightRenders::new();

        assert!(
            renders.join("path=a&size=32", "first"),
            "the first request is the one that renders"
        );
        assert!(
            !renders.join("path=a&size=32", "second"),
            "the second joins it rather than rendering again"
        );

        let mut drained = renders.drain("path=a&size=32");
        drained.sort_unstable();
        assert_eq!(drained, vec!["first", "second"]);
    }

    #[test]
    fn different_queries_render_independently() {
        let renders = InflightRenders::new();

        assert!(renders.join("path=a&size=32", "a"));
        assert!(renders.join("path=b&size=32", "b"));

        assert_eq!(renders.drain("path=a&size=32"), vec!["a"]);
        assert_eq!(renders.drain("path=b&size=32"), vec!["b"]);
    }

    #[test]
    fn drains_every_waiter_exactly_once() {
        let renders = InflightRenders::new();

        renders.join("path=a&size=32", "first");
        for waiter in ["second", "third", "fourth", "fifth"] {
            assert!(!renders.join("path=a&size=32", waiter));
        }

        assert_eq!(
            renders.drain("path=a&size=32").len(),
            5,
            "every waiter is answered"
        );
        assert!(
            renders.drain("path=a&size=32").is_empty(),
            "and none of them twice"
        );
    }

    /// The property the drain ordering exists to guarantee. Removing the entry
    /// before the waiters are handed out is what makes this true; leaving it in
    /// place would attach a late request to a list nobody drains, and that
    /// request would never be answered at all.
    #[test]
    fn a_request_arriving_after_the_drain_renders_for_itself() {
        let renders = InflightRenders::new();

        assert!(renders.join("path=a&size=32", "first"));
        assert_eq!(renders.drain("path=a&size=32"), vec!["first"]);

        assert!(
            renders.join("path=a&size=32", "late"),
            "a late request must render rather than wait on a drained list"
        );
        assert_eq!(renders.drain("path=a&size=32"), vec!["late"]);
    }

    #[test]
    fn drains_nothing_for_a_query_that_never_rendered() {
        let renders: InflightRenders<&str> = InflightRenders::new();
        assert!(renders.drain("path=never").is_empty());
    }

    /// The cap is checked against the header before any pixels are allocated, so
    /// its boundary is the difference between "no thumbnail" and a decode that
    /// allocates `cap * 4` bytes.
    #[test]
    fn caps_decoded_pixels_at_the_boundary() {
        let (wide, tall) = (4096u32, 10_240u32);
        assert_eq!(
            u64::from(wide) * u64::from(tall),
            THUMBNAIL_MAX_DECODED_PIXELS,
            "the fixture sits exactly on the cap"
        );

        assert!(
            within_thumbnail_pixel_cap(wide, tall),
            "the cap itself is allowed"
        );
        assert!(
            !within_thumbnail_pixel_cap(wide, tall + 1),
            "one row past it is not"
        );
        assert!(
            !within_thumbnail_pixel_cap(tall + 1, wide),
            "in either axis"
        );
    }

    #[test]
    fn refuses_degenerate_dimensions() {
        assert!(!within_thumbnail_pixel_cap(0, 100));
        assert!(!within_thumbnail_pixel_cap(100, 0));
        assert!(!within_thumbnail_pixel_cap(0, 0));
        assert!(within_thumbnail_pixel_cap(1, 1));
        // The product must not wrap: both axes at `u32::MAX` square to less than
        // `u64::MAX`, so this has to read as over the cap rather than as small.
        assert!(!within_thumbnail_pixel_cap(u32::MAX, u32::MAX));
    }

    /// Which files get an image slot at all, and — the part that was wrong —
    /// that the two categories do not overlap.
    ///
    /// An extension in both lists would make [`render_thumbnail`] pay the
    /// desktop's process spawn for a JPEG it can decode in 1.5 ms, and one in
    /// neither leaves a file with a producer on the machine rendering as a type
    /// icon. Both are silent, so both are asserted here rather than noticed.
    #[test]
    fn the_two_extension_sets_cover_the_right_files_and_never_overlap() {
        for extension in [
            "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico",
        ] {
            assert!(
                super::IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension),
                "{extension} should decode in process"
            );
        }

        // Formats `image` cannot read, which is the whole reason the delegated
        // list grew: AVIF is what a phone camera writes by default now.
        for extension in [
            "pdf", "mp4", "mov", "mkv", "webm", "avi", "wmv", "heic", "heif", "avif", "jxl",
            "apng", "tga", "qoi", "exr", "dds", "pbm", "pgm", "ppm",
        ] {
            assert!(
                super::DESKTOP_RENDERED_EXTENSIONS.contains(&extension),
                "{extension} should be the desktop's to render"
            );
            assert!(
                !super::IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension),
                "{extension} cannot be in both: that would spawn a process to decode a raster"
            );
        }

        for name in [
            "photo.jpg",
            "photo.JPG",
            "anim.gif",
            "icon.ico",
            "vector.svg",
            "clip.mp4",
            "scan.avif",
            "lossless.jxl",
            "frame.exr",
            "mesh.qoi",
            "texture.dds",
        ] {
            assert!(
                is_thumbnail_extension(name),
                "{name} should have a producer"
            );
        }

        for name in [
            "notes.txt",
            "archive.zip",
            "binary.bin",
            "Makefile",
            "noext",
        ] {
            assert!(
                !is_thumbnail_extension(name),
                "{name} has no producer anywhere and must not claim an image slot"
            );
        }
    }

    /// A photo past the pixel cap is a routing decision, not a miss.
    ///
    /// It used to be a miss, which is what made a 75 MP camera JPEG render as a
    /// type icon while every other viewer on the desktop showed a picture:
    /// `Option` could not tell the caller "this is too big for me, ask the
    /// desktop" apart from "there is no thumbnail of this file", so the one
    /// case with an obvious answer was the one case the caller threw away.
    #[test]
    fn an_image_past_the_pixel_cap_is_oversized_rather_than_a_miss() {
        // 10667 × 7111 — the dimensions of a real 75 MP photo, and past the
        // 40 MP cap by nearly a factor of two.
        let fixture = Fixture::header_only_jpeg("oversized.jpg", 10_667, 7_111);

        assert!(
            matches!(
                decode_and_scale(fixture.path(), 128),
                Ok(DecodeOutcome::Oversized)
            ),
            "an over-cap image must reach the caller's desktop-producer branch"
        );

        // The cap is about pixels, not bytes: a 75 MP JPEG is routinely only a
        // few megabytes, so the source-size budget must not have refused it
        // first and turned this into a 404 again.
        assert_eq!(
            10_667u64 * 7_111 / (1024 * 1024),
            72,
            "the fixture is meant to be a large but ordinary photograph"
        );
        assert!(within_thumbnail_pixel_cap(8_192, 4_096));
    }

    /// A fixture image in a directory of its own, removed when the test ends.
    ///
    /// Per-fixture rather than per-run: the test binary runs these in parallel,
    /// and a shared directory would have one case deleting another's file.
    struct Fixture(PathBuf);

    impl Fixture {
        /// Claims a directory of this fixture's own, and the path `name` inside
        /// it.
        fn new(name: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);

            let directory = std::env::temp_dir().join(format!(
                "dae-thumbnail-response-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).expect("create the fixture directory");

            Self(directory.join(name))
        }

        /// A small RGBA PNG. `name` stays alphanumeric so the resulting path
        /// needs none of the percent-encoding `buildThumbnailUrl` applies — the
        /// query parser splits on `&` and `=`, which such a name cannot contain.
        fn png(name: &str) -> Self {
            let fixture = Self::new(name);
            image::RgbaImage::new(4, 4)
                .save(fixture.path())
                .expect("write the fixture image");
            fixture
        }

        /// Arbitrary bytes, for a case that must not be decodable image data at
        /// all — `ImageBuffer::save` refuses to write a PNG under a `.txt` name.
        fn text(name: &str, contents: &str) -> Self {
            let fixture = Self::new(name);
            std::fs::write(fixture.path(), contents).expect("write the fixture file");
            fixture
        }

        /// A JPEG whose frame header declares the given size, with no scan data
        /// after it — everything a decoder's dimension probe reads, and nothing
        /// it would have to decode.
        ///
        /// A header is what makes the fixture cheap; the pixels are what makes
        /// it impossible. Every assertion about the pixel cap would otherwise
        /// have to allocate the very buffer the cap exists to avoid.
        fn header_only_jpeg(name: &str, width: u16, height: u16) -> Self {
            let fixture = Self::new(name);

            let mut bytes = vec![0xff, 0xd8];
            // `SOF0`: segment length, sample precision, the two dimensions, and
            // a three-component baseline frame header.
            bytes.extend_from_slice(&[0xff, 0xc0]);
            bytes.extend_from_slice(&17u16.to_be_bytes());
            bytes.push(8);
            bytes.extend_from_slice(&height.to_be_bytes());
            bytes.extend_from_slice(&width.to_be_bytes());
            bytes.extend_from_slice(&[3, 1, 0x11, 0x00, 2, 0x11, 1, 3, 0x11, 1]);
            // `SOS`, which is where a JPEG header probe stops reading. Without
            // it the decoder reports a premature end rather than dimensions.
            bytes.extend_from_slice(&[0xff, 0xda]);
            bytes.extend_from_slice(&12u16.to_be_bytes());
            bytes.push(3);
            bytes.extend_from_slice(&[1, 0x00, 2, 0x11, 3, 0x11]);
            bytes.extend_from_slice(&[0, 63, 0]);
            // A single entropy-coded byte is not a valid scan, and never has to
            // be: this fixture is only ever asked for its size.
            bytes.push(0x00);
            bytes.extend_from_slice(&[0xff, 0xd9]);

            std::fs::write(fixture.path(), bytes).expect("write the fixture header");
            fixture
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }

        /// The query string a webview would send for this fixture.
        fn query(&self, size: u16) -> String {
            format!("path={}&size={size}", self.0.to_string_lossy())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(directory) = self.0.parent() {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }

    /// The whole request path, not just its pieces: query parsing, rendering,
    /// status mapping and headers. `render_response` needs no webview, so the
    /// contract the frontend's `<img>` and its `onError` fallback depend on is
    /// testable directly.
    #[test]
    fn serves_a_thumbnail_for_a_decodable_image() {
        let fixture = Fixture::png("thumb.png");
        let response = render_response(RenderKind::Thumbnail, &fixture.query(32));

        assert_eq!(response.status(), 200);

        let content_type = response.headers()["content-type"]
            .to_str()
            .expect("a content type is printable ASCII");
        assert!(
            content_type.starts_with("image/"),
            "expected an image content type, got {content_type}"
        );
        assert!(
            !response.body().is_empty(),
            "the body carries the encoded image, not just the metadata"
        );
        assert_eq!(
            response.headers()["cache-control"],
            "public, max-age=86400, immutable",
            "the URL embeds mtime and size, so the response has to be cacheable"
        );
    }

    /// The two failure modes are deliberately different, and the frontend reads
    /// them differently: an unsupported type keeps the type icon, anything else
    /// is a real error.
    #[test]
    fn distinguishes_an_unsupported_type_from_a_failed_read() {
        let unsupported = Fixture::text("notes.txt", "not an image");
        assert_eq!(
            render_response(RenderKind::Thumbnail, &unsupported.query(32)).status(),
            404,
            "a type with no producer is a miss the frontend falls back from"
        );

        let missing = Fixture::png("gone.png");
        std::fs::remove_file(missing.path()).expect("remove the fixture image");
        assert_eq!(
            render_response(RenderKind::Thumbnail, &missing.query(32)).status(),
            500,
            "a thumbnailable type that cannot be read is an error, not a miss"
        );
    }

    #[test]
    fn answers_400_for_a_query_that_names_no_resource() {
        assert_eq!(
            render_response(RenderKind::Thumbnail, "pathonly").status(),
            400
        );
        assert_eq!(render_response(RenderKind::Thumbnail, "").status(), 400);
        assert_eq!(render_response(RenderKind::FileIcon, "").status(), 400);
    }

    /// A request that names an icon theme entry rather than a file is the "Open
    /// With" picker's.
    ///
    /// Asserted as a miss rather than a hit: whether `org.kde.dolphin` has an
    /// icon is the machine's business, whereas what the frontend's `onError`
    /// fallback depends on is that an unanswerable name reads as a 404 — the same
    /// answer an unresolvable file gives — and not as a failed request.
    #[test]
    fn answers_a_named_icon_with_a_miss_not_an_error() {
        assert_eq!(
            render_response(RenderKind::FileIcon, "name=dae-no-such-icon-zzz&size=32").status(),
            404
        );
        // A thumbnail is made from a file's bytes, so a theme name is no
        // resource for it at all.
        assert_eq!(
            render_response(RenderKind::Thumbnail, "name=dae-no-such-icon-zzz&size=32").status(),
            400
        );
        assert!(
            is_remembered_icon_miss("icon-name|dae-no-such-icon-zzz|32"),
            "and the theme search behind a miss is not run again for that name"
        );
    }

    /// The two ways a row asks for an icon, told apart by the URL alone.
    ///
    /// A type is asked for by type — which is what makes every `.rs` in a column
    /// one request — and an entry that cannot share its answer is asked for by
    /// path, bringing its kind and its version with it so the icon route never has
    /// to read the file's metadata to name its cache key.
    #[test]
    fn a_type_asks_for_itself_and_an_entry_asks_for_its_path() {
        let Some(request) = render_request_params("ext=RS&size=44") else {
            panic!("a type request is a well-formed request");
        };
        let RenderSubject::Type { extension } = request.subject else {
            panic!("an `ext` request must reach the type route");
        };
        assert_eq!(extension, "rs", "one key per type, whatever it is spelled");
        assert_eq!(request.size, 44);

        let Some(request) = render_request_params("path=/tmp/report.pdf&dir=1&size=44&v=1700-2048")
        else {
            panic!("an entry request is a well-formed request");
        };
        let RenderSubject::Entry {
            path,
            is_dir,
            version,
        } = request.subject
        else {
            panic!("a `path` request must reach the entry route");
        };
        assert_eq!(path, "/tmp/report.pdf");
        assert!(is_dir, "`dir=1` is how the listing's kind arrives");
        assert_eq!(version, "1700-2048");

        // An entry outranks a type: a URL naming both is asking about a file.
        let Some(request) = render_request_params("path=/tmp/a.rs&ext=rs&size=32") else {
            panic!("a path and a type is still a path");
        };
        assert!(
            matches!(request.subject, RenderSubject::Entry { .. }),
            "the entry route answers it"
        );
    }

    /// What the filter is for: the value reaches the desktop's *type* registry as
    /// a name, and a name that could read as a path would stop being a type
    /// question. Refusing it is what makes a stray URL a 400 rather than a lookup
    /// of something the request had no business naming.
    #[test]
    fn a_shared_icon_request_accepts_a_type_and_refuses_a_path() {
        assert_eq!(icon_extension("rs").as_deref(), Some("rs"));
        assert_eq!(icon_extension("msiexec").as_deref(), Some("msiexec"));
        assert_eq!(icon_extension("PDF").as_deref(), Some("pdf"));
        assert_eq!(icon_extension("tar.gz"), None, "a dot means a name");
        assert_eq!(icon_extension(".."), None);
        assert_eq!(icon_extension("%2Fetc%2Fpasswd"), None, "an encoded separator");
        assert_eq!(icon_extension("x%20y"), None, "and an encoded space");
        assert_eq!(icon_extension(""), None);
        assert_eq!(
            icon_extension(&"x".repeat(MAX_ICON_EXTENSION_LEN + 1)),
            None,
            "a long enough 'extension' is not one"
        );
    }

    /// The half of the sharing that the webview's cache cannot do: an answer that
    /// does not exist is not cacheable, so the second request for a type still
    /// arrives here. It must not search again.
    #[test]
    fn a_remembered_miss_never_reaches_the_producer() {
        let key = "icon-ext|dae-miss-test-zzz|32".to_owned();
        let mut searches = 0;

        assert!(
            cached_icon(key.clone(), || {
                searches += 1;
                None
            })
            .is_none()
        );
        assert!(
            cached_icon(key, || {
                searches += 1;
                None
            })
            .is_none()
        );

        assert_eq!(searches, 1, "only the first request ran the search");
    }

    /// The mirrored case, and the one a scrolling column lives on: the type's icon
    /// is rendered once however many rows ask for it.
    #[test]
    fn a_cached_icon_is_drawn_once_for_every_row_of_its_type() {
        let key = "icon-ext|dae-hit-test-zzz|32".to_owned();
        let mut draws = 0;

        for row in 0..3u8 {
            let drawn = cached_icon(key.clone(), || {
                draws += 1;
                Some(FileIcon {
                    mime: "image/png",
                    bytes: vec![row],
                })
            });
            assert!(drawn.is_some(), "row {row} gets an icon");
        }

        assert_eq!(draws, 1, "the second and third rows take the cache");
        assert_eq!(
            lookup_cache(&ICON_CACHE, &key).expect("the type is cached").bytes,
            vec![0],
            "and what they take is the one rendering"
        );
    }

    /// An entry whose file went away between the listing and the icon request is
    /// not a failed read: the icon route never opened the file, so there is no
    /// read to fail, and the answer is the one its type carries.
    #[test]
    fn an_icon_for_a_missing_entry_is_not_an_error() {
        let gone = Fixture::text("gone.rs", "removed below");
        let query = format!("{}&v=1700-2", gone.query(32));
        std::fs::remove_file(gone.path()).expect("remove the fixture file");

        let status = render_response(RenderKind::FileIcon, &query).status();
        assert_ne!(
            status, 500,
            "a missing file is a miss the row falls back from, not a broken read"
        );
    }
}

fn lookup_cache(
    cache: &'static Mutex<Option<RenderedCache>>,
    cache_key: &str,
) -> Option<Arc<RenderedThumbnail>> {
    cache.lock().ok()?.as_ref()?.entries.get(cache_key).cloned()
}

fn store_cache(
    cache: &'static Mutex<Option<RenderedCache>>,
    max_entries: usize,
    cache_key: String,
    thumbnail: Arc<RenderedThumbnail>,
) {
    if let Ok(mut guard) = cache.lock() {
        let cache = guard.get_or_insert_with(|| RenderedCache {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
        });

        // Refresh the recency marker when the key already exists.
        if cache.entries.contains_key(&cache_key)
            && let Some(position) = cache
                .insertion_order
                .iter()
                .position(|key| key == &cache_key)
        {
            cache.insertion_order.remove(position);
        }

        while cache.entries.len() >= max_entries {
            let Some(oldest) = cache.insertion_order.pop_front() else {
                break;
            };
            cache.entries.remove(&oldest);
        }

        cache.insertion_order.push_back(cache_key.clone());
        cache.entries.insert(cache_key, thumbnail);
    }
}

/// What an in-process decode produced, when it is asked to produce anything.
///
/// The distinction is the whole point of the type: a file over the pixel cap is
/// not a failure, it is a file for [`produce_shell_thumbnail`], which scales
/// large images the way the desktop does — out of process, in bounded memory.
/// Collapsing this back into `Option` is what made those files lose their
/// thumbnails silently.
enum DecodeOutcome {
    /// The scaled bitmap, ready to serve.
    Frame(RenderedThumbnail),
    /// More pixels than [`THUMBNAIL_MAX_DECODED_PIXELS`], refused before the
    /// decode allocated anything.
    Oversized,
}

fn decode_and_scale(path: &Path, size: u16) -> Result<DecodeOutcome, FileSystemError> {
    // Header-only read first, so an image over the cap is refused *before* its
    // pixels are allocated. Checking the dimensions after `decode` — which is
    // what this used to do — meant a 100 MP file allocated ~400 MB during the
    // decode and was then discarded, with the caller waiting the whole time.
    let (header_width, header_height) = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| FileSystemError::Io(error.to_string()))?
        .into_dimensions()
        .map_err(|error| FileSystemError::Io(error.to_string()))?;
    if !within_thumbnail_pixel_cap(header_width, header_height) {
        return Ok(DecodeOutcome::Oversized);
    }

    // Reopening costs one `open` on a file the header read just put in the page
    // cache, and it is what lets the limits below be set before the decoder is
    // constructed.
    let mut reader = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    // Belt and braces on the check above: the allocation is bounded here instead
    // of by the allocator, so a decoder whose header disagreed with its body is
    // refused rather than reserving whatever it likes. The `image` default is
    // 512 MiB, several times what a capped decode needs.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(THUMBNAIL_MAX_DECODED_PIXELS * 4 * 2);
    reader.limits(limits);

    let decoded = reader
        .decode()
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    // A second guard rather than a redundant one: the header is a claim, and a
    // decoder whose body disagrees with it would otherwise sail past the check
    // above and allocate whatever it liked.
    let (width, height) = decoded.dimensions();
    if !within_thumbnail_pixel_cap(width, height) {
        return Ok(DecodeOutcome::Oversized);
    }

    // One fast nearest-neighbor step when the image is far larger than the
    // target keeps the final `thumbnail` pass cheap on huge photos.
    let target = u32::from(size.max(1));
    let working = if width > target * 4 && height > target * 4 {
        decoded.resize_exact(target * 4, target * 4, image::imageops::FilterType::Nearest)
    } else {
        decoded
    };
    let scaled = working.thumbnail(target, target);

    // Photos decode to opaque pixels; keep PNG only where alpha matters so
    // typical JPEGs stay small.
    if scaled.color().has_alpha() {
        let mut buffer = Vec::new();
        scaled
            .to_rgba8()
            .write_to(
                &mut std::io::Cursor::new(&mut buffer),
                image::ImageFormat::Png,
            )
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        Ok(DecodeOutcome::Frame(RenderedThumbnail {
            mime: "image/png",
            bytes: buffer,
        }))
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 82);
        DynamicImage::ImageRgb8(scaled.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        Ok(DecodeOutcome::Frame(RenderedThumbnail {
            mime: "image/jpeg",
            bytes: buffer,
        }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn read_text_preview(
    path: String,
    max_bytes: u32,
) -> Result<TextPreview, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let metadata = fs::metadata(&path).map_err(FileSystemError::from)?;
        if !metadata.is_file() {
            return Err(FileSystemError::InvalidInput(path));
        }

        let limit = max_bytes.clamp(1, 256 * 1024) as usize;
        let file_size = metadata.len() as usize;
        let truncated = file_size > limit;
        let bytes_to_read = file_size.min(limit);

        let mut buffer = vec![0u8; bytes_to_read];
        let mut file = fs::File::open(&path).map_err(FileSystemError::from)?;
        use std::io::Read;
        file.read_exact(&mut buffer)
            .map_err(FileSystemError::from)?;

        // Lossy decoding keeps multi-byte characters cut at the boundary from
        // failing the whole preview.
        Ok::<_, FileSystemError>(TextPreview {
            content: String::from_utf8_lossy(&buffer).into_owned(),
            truncated,
        })
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}
