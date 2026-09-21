use super::error::FileSystemError;
use image::{DynamicImage, GenericImageView};
use serde::Serialize;
use specta::Type;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
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
/// cheap `resize` step before the final smooth pass.
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

/// Extensions whose shell icon belongs to the individual file rather than its
/// type — executable/DLL icon resources, a shortcut's target, a `.url`'s site
/// icon. These stay path-keyed in `ICON_CACHE`. Every other extension resolves
/// to its registered handler's icon, identical across all files sharing it, so
/// one `(extension, size)` entry serves a whole folder and skips a COM/shell
/// roundtrip per file. Keep in sync with `NATIVE_ICON_EXTENSIONS` in
/// `src/features/explorer/native-icon.tsx`.
const FILE_SPECIFIC_ICON_EXTENSIONS: &[&str] = &["exe", "msi", "lnk", "url", "dll", "scr", "cpl"];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Extensions the `image` crate can decode on every supported platform.
fn is_image_extension(path: &str) -> bool {
    matches!(
        extension_of(Path::new(path)).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico"
    )
}

/// Extensions with any thumbnail strategy on some platform. The frontend
/// uses this to decide which entries get an image slot; the protocol
/// handler answers 404 when the current platform lacks a producer.
pub fn is_thumbnail_extension(path: &str) -> bool {
    let extension = extension_of(Path::new(path));
    is_image_extension(path)
        || extension == "svg"
        // Windows shell handlers render PDF first pages, video first frames,
        // and HEIC photos (with the HEIF extensions installed).
        || matches!(
            extension.as_str(),
            "pdf" | "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "heic" | "heif"
        )
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

/// Everyone waiting on one resource, and the lock that lets them arrive while
/// it renders.
struct InflightRender<T> {
    waiters: Mutex<Vec<T>>,
}

/// Renders in flight, keyed by the request's query string.
///
/// The query is the request's identity: the frontend embeds mtime and size in
/// the URL's version tag, so two requests carrying the same query are asking for
/// the same bytes. Deduplicating those is worth it because the duplicate arrives
/// exactly when serving it is most expensive — a fast scroll remounting a row
/// while its first fetch is still decoding. Keying on the raw query rather than
/// on the backend's mtime/size cache key is deliberate: that key needs a `stat`,
/// and this is the thread the whole module exists to keep free of blocking work.
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

/// Serves `fileicon://localhost/?path=...&size=...` with the operating
/// system's icon for the file (Windows shell icon extraction; other platforms
/// answer 404 so the frontend keeps its Phosphor fallback).
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
    let rendered = thumbnail_request_params(query).map(|(path, size)| match kind {
        RenderKind::Thumbnail => render_thumbnail(&path, size),
        RenderKind::FileIcon => render_file_icon(&path, size),
    });

    match rendered {
        Some(Ok(Some(rendered))) => tauri::http::Response::builder()
            .header("Content-Type", rendered.mime)
            // The URL embeds mtime + size, so a given URL is immutable.
            .header("Cache-Control", "public, max-age=86400, immutable")
            .body(Cow::Owned(rendered.bytes.clone()))
            .unwrap_or_else(|_| empty_response(500)),
        // Unsupported files yield 404 so the frontend can fall back to the
        // file icon; genuinely broken reads surface as 500.
        Some(Ok(None)) => empty_response(404),
        Some(Err(_)) => empty_response(500),
        None => empty_response(400),
    }
}

fn empty_response(status: u16) -> ProtocolResponse {
    tauri::http::Response::builder()
        .status(status)
        .body(Cow::Owned(Vec::new()))
        .expect("static protocol response is always valid")
}

/// Extracts `path` and `size` from the request query string.
fn thumbnail_request_params(query: &str) -> Option<(String, u16)> {
    let mut path: Option<String> = None;
    let mut size: Option<u16> = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "path" => path = Some(percent_decode(value)),
            "size" => size = value.parse().ok(),
            // `v` is a cache-busting version tag the backend can ignore.
            _ => {}
        }
    }

    path.map(|path| (path, size.unwrap_or(256)))
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
    let is_shell_source = !is_image_extension(path_string);
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
        // No shell thumbnail handler on this platform (or for this file):
        // 404 lets the frontend fall back to its type icon.
        let Some(png) = extract_shell_thumbnail_png(path_string, u32::from(size)) else {
            return Ok(None);
        };
        RenderedThumbnail {
            mime: "image/png",
            bytes: png,
        }
    } else {
        let Some(thumbnail) = decode_and_scale(path, size)? else {
            return Ok(None);
        };
        thumbnail
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

/// Renders the OS icon for `path_string` as PNG; `None` means the frontend
/// should keep its type-based Phosphor icon.
fn render_file_icon(
    path_string: &str,
    size: u16,
) -> Result<Option<Arc<RenderedThumbnail>>, FileSystemError> {
    let size = size.clamp(16, 256);
    let path = Path::new(path_string);

    let metadata = fs::metadata(path).map_err(FileSystemError::from)?;
    if !metadata.is_file() {
        return Ok(None);
    }

    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    let extension = extension_of(path);
    // A file with no extension has no type association to key on, and the
    // app-like set carries a per-file icon; both must stay path-keyed. Every
    // other extension resolves to one shared handler icon.
    let cache_key =
        if extension.is_empty() || FILE_SPECIFIC_ICON_EXTENSIONS.contains(&extension.as_str()) {
            format!(
                "icon|{}|{}|{}|{size}",
                path_string,
                modified_at.unwrap_or(0),
                metadata.len()
            )
        } else {
            format!("icon-ext|{extension}|{size}")
        };

    if let Some(cached) = lookup_cache(&ICON_CACHE, &cache_key) {
        return Ok(Some(cached));
    }

    let Some(bytes) = extract_file_icon_png(path_string, u32::from(size)) else {
        return Ok(None);
    };

    let icon = Arc::new(RenderedThumbnail {
        mime: "image/png",
        bytes,
    });
    store_cache(
        &ICON_CACHE,
        ICON_CACHE_MAX_ENTRIES,
        cache_key,
        Arc::clone(&icon),
    );
    Ok(Some(icon))
}

/// Windows shell icon extraction: `IShellItemImageFactory` resolves whatever
/// Explorer would show — the target icon for `.lnk`/`.url` shortcuts, the
/// embedded icon for executables, the registered handler icon otherwise.
///
/// `shell_commands.rs` reuses it for the icon paths an `IExplorerCommand`
/// reports.
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

// No shell thumbnail rendering on other platforms yet; the frontend falls
// back to its type-based Phosphor icons for PDF/video/HEIC.
#[cfg(not(windows))]
fn extract_shell_thumbnail_png(_path: &str, _size: u32) -> Option<Vec<u8>> {
    None
}

// No shell icon extraction on other platforms yet; the frontend falls back
// to its type-based Phosphor icons.
#[cfg(not(windows))]
fn extract_file_icon_png(_path: &str, _size: u32) -> Option<Vec<u8>> {
    None
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
        InflightRenders, RenderKind, THUMBNAIL_MAX_DECODED_PIXELS, render_response,
        within_thumbnail_pixel_cap,
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

fn decode_and_scale(path: &Path, size: u16) -> Result<Option<RenderedThumbnail>, FileSystemError> {
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
        return Ok(None);
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
        return Ok(None);
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
        Ok(Some(RenderedThumbnail {
            mime: "image/png",
            bytes: buffer,
        }))
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 82);
        DynamicImage::ImageRgb8(scaled.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        Ok(Some(RenderedThumbnail {
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
