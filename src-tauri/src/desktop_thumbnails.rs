//! Linux: the desktop's own thumbnailers, run for the formats the `image` crate
//! cannot decode — video first frames, PDF pages, HEIC and AVIF photos.
//!
//! Why a spec and not an API. Nothing on this platform answers "give me a
//! thumbnail of this file" the way `IShellItemImageFactory` does on Windows or
//! `NSWorkspace` does on macOS. What it has instead is the freedesktop
//! Thumbnailing spec: packages drop a `.thumbnailer` file into
//! `$XDG_DATA_DIRS/thumbnailers/` naming the MIME types they render and an
//! `Exec=` line with `%i`/`%o`/`%s` holes to fill, and the agreed output is
//! cached in `$XDG_CACHE_HOME/thumbnails/`. So this module is a reader of those
//! two things plus a process spawner — the same shape as [`crate::file_icons`]
//! and for the same reason: no GTK, no D-Bus, no display connection, nothing
//! that has to hop to the main thread. It runs on the render pool
//! ([`crate::file_system::preview`]), and a producer that needed a realized
//! display would undo the one thing that pool exists to avoid.
//!
//! Why writing into the shared cache is worth a directory outside the app's own
//! tree: every file manager and image viewer on the desktop reads and writes it,
//! so a clip another application already thumbnailed is served here without
//! spawning anything, a thumbnail made here is free for the next viewer, and
//! both survive this app's restarts — which the in-memory cache in `preview.rs`
//! does not.
//!
//! Key compatibility is best-effort. The spec fixes the layout and the name
//! (`MD5` of the file's URI, plus the basename above the smallest bucket) but
//! not the URI's escaping, and GLib's `g_filename_to_uri` is what the common
//! writers use, so [`file_uri_for`] follows it rather than the stricter encoding
//! the `url` crate would apply. Getting this wrong is benign: the entry lands
//! under a name nobody looks for, and the worst case is regenerating a thumbnail
//! that was already on disk. A key can produce a miss, never a wrong picture.
//!
//! Freshness is the spec's rule rather than an invention: a cached thumbnail is
//! valid while it is at least as new as its source. The key carries no mtime, so
//! that comparison is the only invalidation a shared thumbnail directory has.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::shell_commands::desktop_entry;
use crate::xdg::{cache_home, data_roots};

/// A thumbnail a producer rendered, labelled with the type its bytes have.
///
/// The mime travels with the bytes as it does for [`crate::file_icons::FileIcon`]
/// — the label belongs to whatever made them, and the caller answers a protocol
/// request with it verbatim. Here it is always PNG: a producer is asked for a
/// `.png` output name, and [`collect`] refuses anything else.
pub(crate) struct Produced {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Where every application's thumbnails are kept, under the cache home.
const THUMBNAIL_DIR: &str = "thumbnails";
/// The spec's directory recording that a file has already produced nothing.
const FAIL_DIR: &str = "fail";
/// The subdirectory `gnome-desktop` writes its failures into, read as well as
/// our own so a file Nautilus gave up on is not tried again here.
const GNOME_FAIL_DIR: &str = "gnome-thumbnail-factory";

/// How long a producer may run before it is killed.
///
/// Bounded because the callers are the three threads of the render pool: a
/// thumbnailer stuck on a damaged stream would otherwise hold one of them for as
/// long as the process lived, and three stuck decoders stop every thumbnail in
/// the app rather than just the broken one. Fifteen seconds is well past any
/// healthy first-frame decode, and past the point where showing the type icon is
/// the better answer.
const PRODUCER_TIMEOUT: Duration = Duration::from_secs(15);

/// How often a bounded wait looks for an exit: long enough that a healthy
/// producer costs a handful of polls, short enough that a hung one is reaped
/// promptly once the deadline passes.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How far into a clip the ffmpeg fallback seeks.
///
/// One second rather than the first frame: a clip that fades up from black — and
/// most consumer footage — opens on a black square, which is a worse thumbnail
/// than an icon. Past the end of a shorter file ffmpeg yields nothing, and
/// [`extract`] then retries from the start.
const FFMPEG_SEEK_SECONDS: u32 = 1;

/// The PNG signature every produced file is checked against.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The four size buckets the spec defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bucket {
    Normal,
    Large,
    XLarge,
    XXLarge,
}

impl Bucket {
    /// The bucket a request for `size` device pixels falls into.
    const fn for_size(size: u32) -> Self {
        match size {
            0..=128 => Self::Normal,
            129..=256 => Self::Large,
            257..=512 => Self::XLarge,
            _ => Self::XXLarge,
        }
    }

    /// The directory the spec gives this bucket, and the name every other writer
    /// on the desktop uses for it.
    const fn directory(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::XLarge => "x-large",
            Self::XXLarge => "xx-large",
        }
    }

    /// The edge length handed to a producer as `%s`.
    ///
    /// The bucket's own size rather than the request's, because `%s` is what the
    /// producer is told to *make*, and a one-off 200px render written into
    /// `normal/` would then be read back by the next viewer as the 128 the spec
    /// promises that directory holds.
    const fn edge(self) -> u32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
            Self::XLarge => 512,
            Self::XXLarge => 1024,
        }
    }
}

/// Where one file's thumbnail belongs, relative to the thumbnails root.
struct CacheEntry {
    bucket: Bucket,
    /// `<bucket>/<key>.png`, the path shared with every other viewer.
    thumbnail: PathBuf,
    /// `fail/<key>.png`, written when a producer tried and produced nothing.
    failure: PathBuf,
}

impl CacheEntry {
    /// Keyed by the file's URI, so a cached thumbnail is interchangeable with the
    /// one the rest of the desktop would have written for the same file.
    ///
    /// The smallest bucket keys on the digest alone; larger ones append the
    /// basename, because a 512px and a 1024px render of the *same* file must not
    /// share a path, and the digest alone would make them do exactly that.
    fn for_path(path: &str, size: u32) -> Self {
        let bucket = Bucket::for_size(size);
        let digest = md5_hex(&file_uri_for(path));

        let name = match bucket {
            Bucket::Normal => format!("{digest}.png"),
            _ => {
                let basename = Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                format!("{digest}-{basename}.png")
            }
        };

        Self {
            bucket,
            thumbnail: PathBuf::from(bucket.directory()).join(&name),
            failure: PathBuf::from(FAIL_DIR).join(&name),
        }
    }

    /// Where a failure for this file would be recorded: our own path and the one
    /// GNOME's factory writes.
    fn failure_paths(&self, root: &Path) -> Vec<PathBuf> {
        let name = self
            .failure
            .file_name()
            .expect("a cache entry always carries a name");

        vec![
            root.join(&self.failure),
            root.join(FAIL_DIR).join(GNOME_FAIL_DIR).join(name),
        ]
    }
}

/// Lowercase hex MD5 of `input`.
///
/// MD5 because the spec names it, not because it is a good choice for anything
/// else: this is a cache key over a path, and every other writer on the desktop
/// computes the same digest, which is what makes reading their thumbnails
/// possible at all.
fn md5_hex(input: &str) -> String {
    use md5::Digest as _;

    let mut digest = md5::Md5::new();
    digest.update(input.as_bytes());
    lowercase_hex(&digest.finalize())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX[usize::from(byte >> 4)] as char);
        hex.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    hex
}

/// The `file://` URI a path is known by, escaped the way GLib's
/// `g_filename_to_uri` escapes it.
///
/// The keep-list is what GLib passes through: the unreserved characters, the
/// sub-delims, `:` and `@`. `url::Url::from_file_path` — the obvious alternative
/// already in the tree — escapes `@`, `(` and `,` as well, so its encoding of a
/// path containing any of them hashes to a key no other thumbnailer computes, and
/// the shared cache would miss exactly the files whose names are not plain.
fn file_uri_for(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut uri = String::with_capacity(path.len() + 7);
    uri.push_str("file://");

    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b'/'
            | b':'
            | b'='
            | b'@' => {
                uri.push(*byte as char);
            }
            // `;`, `#`, `%`, `"`, `\`, a space and every non-ASCII byte are
            // escaped, the last as its UTF-8 octets — which is why this walks
            // bytes rather than characters.
            other => {
                uri.push('%');
                uri.push(HEX[usize::from(other >> 4)] as char);
                uri.push(HEX[usize::from(other & 0x0f)] as char);
            }
        }
    }

    uri
}

/// One `.thumbnailer` file, reduced to what running it needs.
struct Thumbnailer {
    /// The raw `Exec` line, placeholders still in it: the input, the output and
    /// the size change between requests, so expansion happens per request.
    exec: String,
    /// The `MimeType=` list, lowercased.
    mime_types: Vec<String>,
}

/// Whether the declared types cover `mime`.
///
/// Exact match or the bucket glob (`video/*`), which is the whole vocabulary a
/// `.thumbnailer` file uses. There is no "any file" spelling here as there is in
/// a KDE service menu, and no inheritance: a handler declaring `video/mp4` does
/// not claim `video/quicktime`. Walking the shared MIME database's parent chain
/// would mean parsing that database, and shipped entries name their types out in
/// full anyway.
fn covers(mime_types: &[String], mime: &str) -> bool {
    mime_types.iter().any(|declared| {
        declared == mime
            || match declared.split_once('/') {
                Some((category, "*")) => mime
                    .strip_prefix(category)
                    .is_some_and(|rest| rest.starts_with('/')),
                // Anything else — a declared type with no slash, or a specific
                // subtype that did not match above — claims nothing.
                _ => false,
            }
    })
}

/// The program a `.thumbnailer` names, resolved the way a shell would.
///
/// Both spellings are what packages ship today: `/usr/bin/glycin-thumbnailer`
/// absolute, `ffmpegthumbnailer` to be found on `PATH`.
fn resolve_program(word: &str) -> Option<PathBuf> {
    let candidate = Path::new(word);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    which(word)
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file() && is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // Only reachable when this file is compiled on a non-Unix host to be
        // type-checked, where the answer is never used.
        let _ = path;
        true
    }
}

/// Every `.thumbnailer` file the data roots declare, most specific root first so
/// a user's override wins over the packaged entry.
///
/// Cached for the life of the process. Installing a package adds a thumbnailer,
/// and the alternative is re-reading a directory of small INI files per request;
/// the same trade [`crate::file_icons::linux`] makes for the icon theme chain,
/// with the same consequence — a thumbnailer installed while this app runs is
/// picked up on the next launch.
fn thumbnailers() -> &'static Vec<Thumbnailer> {
    static CACHE: OnceLock<Vec<Thumbnailer>> = OnceLock::new();

    CACHE.get_or_init(read_thumbnailers)
}

fn read_thumbnailers() -> Vec<Thumbnailer> {
    let mut found: Vec<Thumbnailer> = Vec::new();
    let mut claimed: Vec<OsString> = Vec::new();

    for root in data_roots() {
        let Ok(entries) = std::fs::read_dir(root.join("thumbnailers")) else {
            continue;
        };

        // Sorted, because `read_dir` gives no order and two packages claiming the
        // same type in one directory would otherwise pick a winner per run.
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "thumbnailer")
            })
            .collect();
        paths.sort();

        for path in paths {
            let Some(name) = path.file_name().map(OsString::from) else {
                continue;
            };
            // A file of this name in an earlier root is the more specific one;
            // this root's copy is the override that did not happen.
            if claimed.contains(&name) {
                continue;
            }
            claimed.push(name);

            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            found.extend(parse_thumbnailer(&text));
        }
    }

    found
}

/// Parses one `.thumbnailer` file, or rejects it.
///
/// The format is the desktop-entry INI dialect — same groups, same `Key=value`
/// lines, same comment rules — so it is read with the parser `shell_commands`
/// already wrote for `.desktop` files rather than a second one that would have to
/// be kept in step with the first.
///
/// An entry whose handler is not on the machine is dropped here rather than at
/// request time. That is `TryExec=` in all but name, and it is why a machine
/// without ffmpeg answers a video with the type icon at once instead of failing to
/// spawn once per file per scroll. Shipped files spell the check inconsistently —
/// `TryExec=ffmpegthumbnailer` bare against `Exec=ffmpegthumbnailer …`, and some
/// declare no `TryExec=` at all — so the first word of `Exec=` is the answer that
/// is always there, and `TryExec=` is consulted only when it names something the
/// machine does not have.
fn parse_thumbnailer(text: &str) -> Option<Thumbnailer> {
    let entry = desktop_entry::parse_groups(text).remove("Thumbnailer Entry")?;

    let exec = entry.get("Exec")?.trim();
    if exec.is_empty() {
        return None;
    }

    // `MimeType=` is a semicolon-separated list with a required trailing
    // separator, and mixed case in the wild: `audio/AMR` next to `video/mp4`.
    let mime_types: Vec<String> = entry
        .get("MimeType")
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|mime| !mime.is_empty())
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_default();
    if mime_types.is_empty() {
        return None;
    }

    if entry
        .get("TryExec")
        .is_some_and(|try_exec| resolve_program(try_exec.trim()).is_none())
    {
        return None;
    }

    let words = desktop_entry::split_exec(exec);
    resolve_program(words.first()?)?;
    Some(Thumbnailer {
        exec: exec.to_owned(),
        mime_types,
    })
}

/// What renders one file.
enum Producer {
    /// A `.thumbnailer` entry, run by expanding its `Exec` line.
    Thumbnailer { exec: String },
    /// Nothing installed claims this type, so the frame is rendered directly.
    Ffmpeg { program: PathBuf },
}

/// The producer for a path's MIME type, or `None` when nothing installed can
/// render it.
fn producer_for(path: &str) -> Option<Producer> {
    let guessed = mime_guess::from_path(path).first()?;
    let mime = guessed.essence_str();

    if let Some(thumbnailer) = thumbnailers()
        .iter()
        .find(|entry| covers(&entry.mime_types, mime))
    {
        return Some(Producer::Thumbnailer {
            exec: thumbnailer.exec.clone(),
        });
    }

    // Video plus the image formats the `image` crate cannot decode. The rest are
    // documents, whose renderers are exactly what the `.thumbnailer` files above
    // declare: a type that reached here with no handler has no handler, and a
    // video decoder would not help it.
    let decodable = mime.starts_with("video/")
        || matches!(
            mime,
            "image/heic" | "image/heif" | "image/avif" | "image/jxl"
        );
    if decodable {
        return which("ffmpeg").map(|program| Producer::Ffmpeg { program });
    }

    None
}

/// The full argv for one request, program first.
///
/// `%i`, `%u` and `%f` all receive the input as a *URI*, which is what the spec
/// says and what the two families of handler in use expect: an ffmpeg-based one
/// hands the string to libavformat, which reads `file://` URLs, and glycin parses
/// one. `%o` receives a plain *path*, because its output is written by the
/// handler's own image encoder, which has no URL layer behind it.
///
/// `seek` belongs to the ffmpeg fallback alone; a `.thumbnailer` decides where to
/// start from in its own `Exec` line (`-c 0.10`, `-f`, whatever it ships).
fn build_command(
    producer: &Producer,
    input_uri: &str,
    output: &Path,
    size: u32,
    seek: u32,
) -> Vec<String> {
    match producer {
        Producer::Thumbnailer { exec } => expand_exec(exec, input_uri, output, size),
        Producer::Ffmpeg { program } => ffmpeg_command(program, input_uri, output, size, seek),
    }
}

/// Expands a `.thumbnailer` `Exec` line into argv.
///
/// Not a shell: the line is split with the desktop entry's own quoting rules and
/// the codes substituted, so a selected filename containing `;` or `$(…)` stays
/// the single argument its author wrote. Evaluating these lines through `/bin/sh`
/// would give a path a meaning no thumbnailer spec defines, and an `Exec=` is data
/// written by whoever packaged the handler.
fn expand_exec(exec: &str, input_uri: &str, output: &Path, size: u32) -> Vec<String> {
    let mut arguments = Vec::new();

    for (index, word) in desktop_entry::split_exec(exec).into_iter().enumerate() {
        // The program, which `Exec=` may spell as a bare name.
        let argument = match index {
            0 => resolve_program(&word).map_or(word, |path| path.display().to_string()),
            _ => substitute_codes(&word, input_uri, output, size),
        };
        if !argument.is_empty() {
            arguments.push(argument);
        }
    }

    arguments
}

/// Substitutes the thumbnailer codes inside one argument.
fn substitute_codes(word: &str, input_uri: &str, output: &Path, size: u32) -> String {
    let mut result = String::with_capacity(word.len());
    let mut characters = word.chars();

    while let Some(character) = characters.next() {
        if character != '%' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('%') => result.push('%'),
            Some('i') | Some('u') | Some('f') => result.push_str(input_uri),
            Some('o') => result.push_str(&output.display().to_string()),
            Some('s') => result.push_str(&size.to_string()),
            // An unknown code expands to nothing, as the desktop-entry grammar
            // requires of codes a thumbnailer line has no business carrying.
            Some(_) | None => {}
        }
    }

    result
}

/// One ffmpeg invocation: `seek` seconds in, one frame, scaled to fit.
fn ffmpeg_command(
    program: &Path,
    input_uri: &str,
    output: &Path,
    size: u32,
    seek: u32,
) -> Vec<String> {
    let mut arguments = vec![
        program.display().to_string(),
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        // No terminal is attached to a render worker, and ffmpeg reading stdin
        // from one would stop at an interactive prompt nobody is there to answer.
        "-nostdin".to_owned(),
        "-y".to_owned(),
    ];
    // Seeking *before* `-i` is the fast path: libavformat jumps to the nearest
    // keyframe instead of decoding from the start of a multi-gigabyte clip.
    if seek > 0 {
        arguments.extend(["-ss".to_owned(), seek.to_string()]);
    }
    arguments.extend([
        "-i".to_owned(),
        input_uri.to_owned(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-an".to_owned(),
        "-vf".to_owned(),
        // `decrease` rather than a bare `W:H`, which would stretch a non-square
        // frame into the wrong aspect.
        format!("scale=w={size}:h={size}:force_original_aspect_ratio=decrease"),
        output.display().to_string(),
    ]);
    arguments
}

/// One producer run, with the distinction [`extract`] needs in order to decide
/// whether the file itself is at fault.
#[derive(Debug)]
enum Rendered {
    /// The staged output and the bytes it holds.
    Frame { staged: PathBuf, bytes: Vec<u8> },
    /// Exited non-zero, could not be started, or wrote nothing usable: a property
    /// of this file, worth recording so a later pass does not retry it.
    Rejected,
    /// The deadline passed. Nothing about the file is known, and recording it
    /// would punish every later scroll for one busy moment.
    TimedOut,
}

/// A thumbnail of `path` at `size` device pixels, or `None` when there is nothing
/// to make one with.
///
/// `modified` is the source's mtime, which the caller has already paid for: it is
/// the freshness test for both a cached thumbnail and a recorded failure, and
/// asking again would be a second read of the same inode on the thread this whole
/// pipeline exists to keep free of blocking work.
///
/// `None` is the frontend's 404, and therefore its type icon. Three different
/// reasons answer it, and the caller has no use for the distinction: nothing on
/// this machine renders this type, the producer failed, or the producer wrote
/// something that is not a PNG.
pub(crate) fn extract(path: &str, size: u32, modified: Option<SystemTime>) -> Option<Produced> {
    let root = cache_home()?.join(THUMBNAIL_DIR);
    let entry = CacheEntry::for_path(path, size);

    // Up to date, so nothing is spawned and no decoder is paid for.
    if let Some(bytes) = read_fresh(&root.join(&entry.thumbnail), modified) {
        return Some(Produced {
            mime: "image/png",
            bytes,
        });
    }

    // Already tried since the file last changed.
    if entry
        .failure_paths(&root)
        .iter()
        .any(|failure| is_fresh(failure, modified))
    {
        return None;
    }

    let producer = producer_for(path)?;
    let input_uri = file_uri_for(path);
    let edge = entry.bucket.edge();
    let seek = match producer {
        Producer::Ffmpeg { .. } => FFMPEG_SEEK_SECONDS,
        Producer::Thumbnailer { .. } => 0,
    };

    let mut outcome = attempt(&producer, &input_uri, &root, &entry, edge, seek);

    // One second is past the end of a shorter clip, and ffmpeg then exits without
    // a frame. Its first frame is the only useful answer, so the same command runs
    // again from the start; a `.thumbnailer` gets one attempt, because its own line
    // already chose where to look.
    if matches!(outcome, Rendered::Rejected) && matches!(producer, Producer::Ffmpeg { .. }) {
        outcome = attempt(&producer, &input_uri, &root, &entry, edge, 0);
    }

    match outcome {
        Rendered::Frame { staged, bytes } => Some(publish(&root, &entry, staged, bytes)),
        Rendered::Rejected => {
            record_failure(&root, &entry);
            None
        }
        Rendered::TimedOut => None,
    }
}

/// Runs a producer once and reads what it wrote.
fn attempt(
    producer: &Producer,
    input_uri: &str,
    root: &Path,
    entry: &CacheEntry,
    edge: u32,
    seek: u32,
) -> Rendered {
    let Some(staged) = staged_output(root, entry) else {
        return Rendered::Rejected;
    };
    let argv = build_command(producer, input_uri, &staged, edge, seek);

    match run(&argv) {
        Outcome::Succeeded => match collect(&staged) {
            Some(bytes) => Rendered::Frame { staged, bytes },
            None => {
                let _ = std::fs::remove_file(&staged);
                Rendered::Rejected
            }
        },
        Outcome::Failed => {
            let _ = std::fs::remove_file(&staged);
            Rendered::Rejected
        }
        Outcome::TimedOut => {
            let _ = std::fs::remove_file(&staged);
            Rendered::TimedOut
        }
    }
}

/// How a producer's run ended.
enum Outcome {
    /// Exited zero.
    Succeeded,
    /// Non-zero exit, or a spawn that never happened.
    Failed,
    /// Killed at [`PRODUCER_TIMEOUT`].
    TimedOut,
}

fn run(argv: &[String]) -> Outcome {
    let Some((program, arguments)) = argv.split_first() else {
        return Outcome::Failed;
    };

    let Ok(mut child) = Command::new(program)
        .args(arguments)
        // All three handles closed: a producer's chatter is nobody's input, and a
        // child holding a pipe this end never reads can block on its own exit.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Outcome::Failed;
    };

    match wait_within(&mut child) {
        Some(status) if status.success() => Outcome::Succeeded,
        Some(_) => Outcome::Failed,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Outcome::TimedOut
        }
    }
}

/// Polls for an exit until [`PRODUCER_TIMEOUT`] passes.
///
/// Polled rather than waited on because this runs on a render-pool thread that
/// must not block indefinitely on a child that never exits.
fn wait_within(child: &mut Child) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + PRODUCER_TIMEOUT;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Reads a produced file: present, non-empty, and a PNG.
///
/// The signature check is the boundary. A handler is asked for a `.png` name and
/// picks its encoder from that extension, but the bytes come back from a program
/// somebody else wrote, and the protocol serves them labelled `image/png` — so a
/// handler that ignored the extension would be a broken image in the listing
/// rather than the miss it is here.
fn collect(staged: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(staged).ok()?;
    if bytes.is_empty() || !bytes.starts_with(PNG_SIGNATURE) {
        return None;
    }
    Some(bytes)
}

/// Moves a rendered thumbnail into the shared cache and hands back its bytes.
///
/// Renamed rather than copied, so an entry appears in the cache complete or not at
/// all — a viewer reading the same directory concurrently never sees half a PNG.
/// Failing to publish is not a failure to thumbnail: the cache directory may be
/// unwritable, and the bytes in hand are still the answer the request wants.
fn publish(root: &Path, entry: &CacheEntry, staged: PathBuf, bytes: Vec<u8>) -> Produced {
    if std::fs::rename(&staged, root.join(&entry.thumbnail)).is_err() {
        let _ = std::fs::remove_file(&staged);
    }

    Produced {
        mime: "image/png",
        bytes,
    }
}

/// The path a producer writes to.
///
/// Inside the destination bucket, so publishing is a rename on one filesystem
/// rather than a copy across the `/tmp`-is-tmpfs split, and named so nothing else
/// reads it as a cached thumbnail — a half-written file must not be the one the
/// next viewer loads.
fn staged_output(root: &Path, entry: &CacheEntry) -> Option<PathBuf> {
    let directory = root.join(&entry.thumbnail);
    let directory = directory.parent()?;
    std::fs::create_dir_all(directory).ok()?;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    Some(directory.join(format!(
        ".dae-thumbnail-{}-{}.png",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Whether `file` exists and is at least as new as its source.
fn is_fresh(file: &Path, modified: Option<SystemTime>) -> bool {
    let Ok(metadata) = std::fs::metadata(file) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    match (metadata.modified().ok(), modified) {
        (Some(cached), Some(source)) => cached >= source,
        // A source that cannot be dated cannot be shown to be newer than the
        // cached copy, which is the ordinary reading of the rule — and an exotic
        // filesystem is a reason to serve a possibly stale frame, not a reason to
        // spawn a decoder on every scroll.
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn read_fresh(file: &Path, modified: Option<SystemTime>) -> Option<Vec<u8>> {
    if !is_fresh(file, modified) {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

/// Records that this file produced nothing, so the next request does not try.
fn record_failure(root: &Path, entry: &CacheEntry) {
    let directory = root.join(FAIL_DIR);
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    // An empty file is the record; its mtime is the statement about when the
    // failure was last true.
    let Some(name) = entry.failure.file_name() else {
        return;
    };
    let _ = File::create(directory.join(name));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    /// A directory of this test's own, removed when it ends. Per-test rather than
    /// per-run because the binary runs these in parallel.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "dae-thumbnailer-test-{}-{}-{name}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).expect("create the scratch directory");
            Self(directory)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// A file stamped `modified`, which is how the freshness rule gets a
        /// source older than the thumbnail made from it.
        fn file(&self, name: &str, bytes: &[u8], modified: SystemTime) -> PathBuf {
            let path = self.path(name);
            let mut handle = File::create(&path).expect("write the scratch file");
            handle.write_all(bytes).expect("fill the scratch file");
            handle
                .set_modified(modified)
                .expect("stamp the scratch file");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sizes_map_to_their_bucket_and_its_edge() {
        assert_eq!(Bucket::for_size(44), Bucket::Normal);
        assert_eq!(Bucket::for_size(128), Bucket::Normal);
        // The boundary is the bucket's edge, not its middle: one pixel over
        // belongs to the next directory.
        assert_eq!(Bucket::for_size(129), Bucket::Large);
        assert_eq!(Bucket::for_size(256), Bucket::Large);
        assert_eq!(Bucket::for_size(257), Bucket::XLarge);
        assert_eq!(Bucket::for_size(512), Bucket::XLarge);
        assert_eq!(Bucket::for_size(513), Bucket::XXLarge);

        // The directory names every other writer on the desktop uses; inventing
        // `huge` here would put our output where nobody looks.
        assert_eq!(
            [
                Bucket::Normal.directory(),
                Bucket::Large.directory(),
                Bucket::XLarge.directory(),
                Bucket::XXLarge.directory()
            ],
            ["normal", "large", "x-large", "xx-large"]
        );
        assert_eq!(Bucket::Normal.edge(), 128);
        assert_eq!(Bucket::XXLarge.edge(), 1024);
    }

    /// The digest is asserted against `md5sum` on the URI GLib produces, not
    /// against this module's own helper, so a change to either half of the key is
    /// caught rather than restated.
    #[test]
    fn keys_a_normal_thumbnail_on_the_uri_alone() {
        let entry = CacheEntry::for_path("/tmp/x.mp4", 128);

        assert_eq!(
            entry.thumbnail,
            PathBuf::from("normal/df913687c5b473efcc4df2a445c2f347.png")
        );
        assert_eq!(
            entry.failure,
            PathBuf::from("fail/df913687c5b473efcc4df2a445c2f347.png")
        );
    }

    #[test]
    fn a_larger_bucket_carries_the_basename_too() {
        let entry = CacheEntry::for_path("/tmp/x.mp4", 512);

        // Two renderings of one file must not land on one path, and the basename
        // is the spec's answer rather than a digest of the size.
        assert_eq!(
            entry.thumbnail,
            PathBuf::from("x-large/df913687c5b473efcc4df2a445c2f347-x.mp4.png")
        );
        assert_ne!(
            CacheEntry::for_path("/tmp/x.mp4", 512).thumbnail,
            CacheEntry::for_path("/tmp/x.mp4", 1024).thumbnail
        );
    }

    /// What makes reading another application's thumbnail possible. The escapes
    /// are GLib's observed behaviour — measured against `g_filename_to_uri` over
    /// the whole printable-ASCII range, not guessed — and `url::Url::from_file_path`
    /// differs from it on `@`, `(` and `,`. A stricter key is a cache that silently
    /// never hits.
    #[test]
    fn escapes_a_path_the_way_glib_does() {
        assert_eq!(file_uri_for("/tmp/x.mp4"), "file:///tmp/x.mp4");
        assert_eq!(file_uri_for("/tmp/a b.mp4"), "file:///tmp/a%20b.mp4");
        assert_eq!(file_uri_for("/tmp/a%b.mp4"), "file:///tmp/a%25b.mp4");
        assert_eq!(file_uri_for("/tmp/a#b.mp4"), "file:///tmp/a%23b.mp4");
        assert_eq!(file_uri_for("/tmp/a;b.mp4"), "file:///tmp/a%3Bb.mp4");
        assert_eq!(file_uri_for("/tmp/中.mp4"), "file:///tmp/%E4%B8%AD.mp4");

        // Passed through, because GLib passes them through.
        for character in [
            '@', '(', ')', ',', ':', '=', '+', '*', '!', '~', '$', '&', '\'',
        ] {
            let path = format!("/tmp/a{character}b.mp4");
            assert!(
                file_uri_for(&path).contains(character),
                "{character} was escaped: {}",
                file_uri_for(&path)
            );
        }

        // Escaped, in the same sweep, with the hex in the uppercase GLib writes.
        for (character, code) in [
            ('"', "%22"),
            ('<', "%3C"),
            ('>', "%3E"),
            ('?', "%3F"),
            ('[', "%5B"),
            ('\\', "%5C"),
            (']', "%5D"),
            ('^', "%5E"),
            ('`', "%60"),
            ('{', "%7B"),
            ('|', "%7C"),
            ('}', "%7D"),
        ] {
            let path = format!("/tmp/a{character}b.mp4");
            assert_eq!(file_uri_for(&path), format!("file:///tmp/a{code}b.mp4"));
        }
    }

    #[test]
    fn covers_the_declared_type_and_its_bucket_glob() {
        let declared = vec![
            "video/mp4".to_owned(),
            "image/*".to_owned(),
            "no-slash".to_owned(),
        ];

        assert!(covers(&declared, "video/mp4"));
        assert!(covers(&declared, "image/png"));
        // No inheritance: a handler for one subtype does not claim its sibling.
        assert!(!covers(&declared, "video/quicktime"));
        assert!(!covers(&declared, "text/plain"));
        // A declared type with no slash is not a bucket glob.
        assert!(!covers(&declared, "no-slash-too"));
        assert!(!covers(&[], "video/mp4"));
    }

    /// The shape Arch's `ffmpegthumbnailer` package ships, kept verbatim.
    #[cfg(unix)]
    #[test]
    fn reads_a_declared_thumbnailer() {
        let parsed = parse_thumbnailer(
            "[Thumbnailer Entry]\n\
             TryExec=ffmpegthumbnailer\n\
             Exec=/bin/sh -i %i -o %o -s %s -f\n\
             MimeType=video/mp4;video/quicktime;\n",
        )
        .expect("an entry with an installed handler");

        assert_eq!(parsed.exec, "/bin/sh -i %i -o %o -s %s -f");
        // Lowercased on the way in, because shipped files write `audio/AMR`
        // beside `video/mp4` and the lookup mime is lowercase.
        assert_eq!(parsed.mime_types, ["video/mp4", "video/quicktime"]);
    }

    /// `TryExec=` is the spec's way of saying "this handler may not be here"; a
    /// machine without it must answer with an icon rather than a failed spawn per
    /// file, which is why the check runs when the list is built and not per
    /// request.
    #[test]
    fn rejects_a_thumbnailer_whose_handler_is_not_installed() {
        assert!(
            parse_thumbnailer(
                "[Thumbnailer Entry]\n\
                 Exec=dae-no-such-thumbnailer-zzz -i %i -o %o -s %s\n\
                 MimeType=video/mp4;\n"
            )
            .is_none()
        );

        assert!(
            parse_thumbnailer(
                "[Thumbnailer Entry]\n\
                 TryExec=dae-no-such-thumbnailer-zzz\n\
                 Exec=/bin/sh %i %o %s\n\
                 MimeType=video/mp4;\n"
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_an_entry_that_cannot_render_anything() {
        // No `MimeType=` means nothing matches it, and a `TryExec`/`Exec` pair
        // that resolves is not enough to make it useful.
        assert!(parse_thumbnailer("[Thumbnailer Entry]\nExec=/bin/sh %i %o\n").is_none());
        assert!(parse_thumbnailer("[Thumbnailer Entry]\nMimeType=;\nExec=/bin/sh %i\n").is_none());
        // The group name is the spec's; a `.desktop` handed to this parser is not
        // a thumbnailer.
        assert!(
            parse_thumbnailer("[Desktop Entry]\nType=Application\nExec=x\nMimeType=text/plain;\n")
                .is_none()
        );
    }

    /// The load-bearing one for safety: the input arrives as *one* argument no
    /// matter what it contains, because the line is never handed to a shell.
    #[test]
    fn expands_the_codes_without_letting_a_path_become_arguments() {
        let output = Path::new("/cache/normal/out.png");
        let arguments = expand_exec(
            "/bin/sh -i %i -o %o -s %s",
            "file:///tmp/a; rm -rf / ;x.mp4",
            output,
            128,
        );

        assert_eq!(
            arguments,
            vec![
                "/bin/sh",
                "-i",
                "file:///tmp/a; rm -rf / ;x.mp4",
                "-o",
                "/cache/normal/out.png",
                "-s",
                "128",
            ]
        );
    }

    #[test]
    fn knows_the_input_codes_glycin_and_ffmpeg_based_handlers_each_use() {
        let output = Path::new("/o.png");

        // `%u` is what glycin's shipped lines ask for; `%i` is the spec's.
        for code in ["%i", "%u", "%f"] {
            let line = format!("/bin/sh --input {code} --output %o");
            let arguments = expand_exec(&line, "file:///x.mp4", output, 128);
            assert_eq!(
                arguments[2], "file:///x.mp4",
                "{code} did not carry the input"
            );
        }

        // A code written into the middle of an argument still substitutes.
        assert_eq!(
            expand_exec("/bin/sh --size=%s", "file:///x.mp4", output, 512)[1],
            "--size=512"
        );
        // `%%` is a literal percent, and an unknown code expands to nothing —
        // dropping the word after it instead would shift every code the line
        // goes on to use.
        assert_eq!(
            expand_exec(
                "/bin/sh 100%% %z --size=%s --end",
                "file:///x.mp4",
                output,
                128
            ),
            ["/bin/sh", "100%", "--size=128", "--end"]
        );
    }

    #[test]
    fn seeks_past_the_black_opening_and_scales_without_stretching() {
        let arguments = ffmpeg_command(
            Path::new("/usr/bin/ffmpeg"),
            "file:///x.mp4",
            Path::new("/o.png"),
            256,
            1,
        );

        // `-ss` before `-i` is the fast keyframe seek; after it, ffmpeg decodes
        // the whole prefix to find the frame, which is the cost this avoids.
        let seek = arguments.iter().position(|a| a == "-ss").expect("a seek");
        let input = arguments.iter().position(|a| a == "-i").expect("an input");
        assert!(seek < input, "the seek must precede the input");
        assert!(
            arguments
                .iter()
                .any(|a| a == "scale=w=256:h=256:force_original_aspect_ratio=decrease"),
            "the frame has to keep its aspect: {arguments:?}"
        );

        // The retry for a clip shorter than the seek starts from frame one.
        let from_start = ffmpeg_command(
            Path::new("/usr/bin/ffmpeg"),
            "file:///x.mp4",
            Path::new("/o.png"),
            256,
            0,
        );
        assert!(!from_start.iter().any(|a| a == "-ss"));
    }

    #[test]
    fn a_producer_that_ignored_the_png_extension_is_a_miss() {
        let scratch = Scratch::new("collect");
        let written = scratch.file("out.png", b"\x89PNG\r\n\x1a\nrest", SystemTime::now());
        assert_eq!(collect(&written).expect("a png"), b"\x89PNG\r\n\x1a\nrest");

        // A JPEG, and an empty file, are both reads of a handler that did not do
        // what it was asked — which the protocol must not serve as a PNG.
        let jpeg = scratch.file("jpeg.png", b"\xff\xd8\xff\xe0not a png", SystemTime::now());
        assert!(collect(&jpeg).is_none());
        let empty = scratch.file("empty.png", b"", SystemTime::now());
        assert!(collect(&empty).is_none());
        assert!(collect(&scratch.path("absent.png")).is_none());
    }

    /// The freshness rule, which is the only invalidation a shared thumbnail
    /// directory has: the key carries no mtime.
    #[test]
    fn a_thumbnail_outlives_its_source_only_if_it_is_newer() {
        let scratch = Scratch::new("fresh");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);

        let thumb = scratch.file("thumb.png", b"pixels", new);
        assert!(
            is_fresh(&thumb, Some(old)),
            "the file predates the thumbnail"
        );
        assert!(
            !is_fresh(&thumb, Some(new + Duration::from_secs(1))),
            "the file was replaced"
        );
        // A source that cannot be dated cannot be shown to be newer, so the
        // cached frame stands.
        assert!(is_fresh(&thumb, None));
        assert!(!is_fresh(&scratch.path("absent.png"), Some(old)));

        assert_eq!(
            read_fresh(&thumb, Some(old)).expect("read").as_slice(),
            b"pixels"
        );
        assert!(read_fresh(&thumb, Some(new + Duration::from_secs(1))).is_none());
    }

    #[test]
    fn a_failure_is_recorded_under_the_spec_path() {
        let scratch = Scratch::new("fail");
        let entry = CacheEntry::for_path("/tmp/x.mp4", 512);
        record_failure(&scratch.path("thumbnails"), &entry);

        let recorded = scratch.path("thumbnails").join(&entry.failure);
        assert!(recorded.is_file(), "{recorded:?}");
        assert_eq!(recorded.metadata().expect("size").len(), 0);
        // And it is what a later request reads before spawning anything.
        assert!(entry.failure_paths(&scratch.path("thumbnails")).len() >= 2);
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_program_the_way_a_shell_would() {
        use std::ffi::OsStr;

        // Which directory holds `sh` is the host's business (`/bin` and `/usr/bin`
        // are the same tree on some, and the search order is theirs), so the test
        // pins the contract rather than the path.
        let found = which("sh").expect("`sh` is on PATH for any Unix session");
        assert_eq!(found.file_name(), Some(OsStr::new("sh")));
        assert!(found.is_file());
        assert_eq!(which("dae-no-such-program-zzz"), None);
        // An absolute path is taken as written, and a directory is not a program.
        assert!(resolve_program("sh").is_some());
        assert!(resolve_program("/tmp").is_none());
        assert!(resolve_program("dae-no-such-program-zzz").is_none());
    }

    /// The spawn half of the module, which nothing above touches: a producer that
    /// really runs, really writes where `%o` pointed, and really ends up at the
    /// path another application would read.
    ///
    /// Stubbed rather than ffmpeg or glycin because those are the host's, not the
    /// test's — the machine this runs on may have neither, and the point here is
    /// the argv, the staging directory and the rename, not the decoder.
    #[cfg(unix)]
    #[test]
    fn a_producer_frame_is_staged_then_renamed_into_the_cache() {
        let scratch = Scratch::new("pipeline");
        let script = scratch.path("producer.sh");
        // The octal escapes are printf's, and the bytes are a PNG header so
        // `collect` accepts the frame the way it accepts a real one.
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '\\211PNG\\r\\n\\032\\nstub' > \"$1\"\n",
        )
        .expect("write the stub producer");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make the stub producer runnable");

        let root = scratch.path("thumbnails");
        let entry = CacheEntry::for_path("/tmp/x.mp4", 256);
        let producer = Producer::Thumbnailer {
            exec: format!("{} %o", script.display()),
        };

        let (bytes, staged) = match attempt(&producer, "file:///tmp/x.mp4", &root, &entry, 256, 0) {
            Rendered::Frame { bytes, staged } => (bytes, staged),
            _ => panic!("the stub producer ran nothing"),
        };
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\nstub");
        // Staged inside the bucket it will be published into, and not under a name
        // a viewer would pick up.
        assert_eq!(
            staged.parent(),
            Some(root.join(&entry.thumbnail).parent().unwrap())
        );
        assert!(
            staged
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".dae-")
        );

        publish(&root, &entry, staged.clone(), bytes);
        let published = root.join(&entry.thumbnail);
        assert_eq!(
            std::fs::read(&published).expect("published"),
            b"\x89PNG\r\n\x1a\nstub"
        );
        assert!(!staged.exists(), "the staged file outlived the rename");

        // And the next request is a read, not a spawn.
        assert_eq!(
            read_fresh(&published, Some(SystemTime::UNIX_EPOCH)).expect("cache hit"),
            b"\x89PNG\r\n\x1a\nstub"
        );
    }

    /// A handler that exits non-zero is a miss the module records, not one it
    /// retries at every scroll.
    #[cfg(unix)]
    #[test]
    fn a_producer_that_fails_leaves_no_frame_and_records_the_miss() {
        let scratch = Scratch::new("failing");
        let script = scratch.path("producer.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 1\n").expect("write the stub producer");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make the stub producer runnable");

        let root = scratch.path("thumbnails");
        let entry = CacheEntry::for_path("/tmp/x.mp4", 128);
        let rendered = attempt(
            &Producer::Thumbnailer {
                exec: format!("{} %o", script.display()),
            },
            "file:///tmp/x.mp4",
            &root,
            &entry,
            128,
            0,
        );
        assert!(matches!(rendered, Rendered::Rejected), "got {rendered:?}");

        record_failure(&root, &entry);
        assert!(root.join(&entry.failure).is_file());
    }
}
