//! Thumbnail and system-icon rendering for the Electron protocols.
//!
//! `electron/main.cjs` registers `thumbnail://` and `fileicon://` and forwards
//! each request to this module through the core process. The Tauri shell's
//! `file_system/preview.rs` is the behavior reference: the same extension
//! split, source caps, pixel cap, and desktop fallback, minus the Tauri
//! responder plumbing that has no counterpart here.

use std::fs;
use std::path::Path;

use dae_core::file_icons;

/// One rendered image and the type its bytes have.
pub struct Rendered {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Extensions the `image` crate decodes in process on every platform.
///
/// Everything outside this set is the desktop's to render, which keeps the two
/// categories honest: a name here means this process can draw the file, and a
/// name absent from it means the shell or a `.thumbnailer` gets the request
/// instead.
const IN_PROCESS_IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico",
];

/// Extensions with no in-process decoder, listed because some desktop has one:
/// documents (a PDF's first page), video (a first frame), and image formats the
/// `image` crate cannot read at all.
///
/// Membership is a possibility, not a promise: a platform with no producer
/// answers 404 and the row keeps its type glyph.
const DESKTOP_RENDERED_EXTENSIONS: &[&str] = &[
    "pdf", "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "heic", "heif", "avif", "jxl", "apng",
    "tga", "qoi", "exr", "dds", "pbm", "pgm", "ppm",
];

/// Source images above this size are skipped so browsing a folder of huge
/// archives-as-images cannot stall a render worker.
const THUMBNAIL_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// Hard cap on decoded thumbnail pixels; larger images route to the desktop's
/// producer, which scales them out of process instead of materialising the
/// full bitmap here.
const THUMBNAIL_MAX_DECODED_PIXELS: u64 = 40 * 1024 * 1024;
/// SVGs pass through to the webview as raw bytes (the browser rasterizes them).
const SVG_MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
/// Shell thumbnails are produced by an OS handler, so only pathological source
/// sizes are refused.
const SHELL_THUMBNAIL_MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Longest extension the shared type-icon route accepts. Real ones run to a
/// handful of characters; a longer "extension" is a name, not a type.
const MAX_ICON_EXTENSION_LEN: usize = 24;

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Whether a path has any thumbnail strategy on some platform. The frontend
/// uses the same list to decide which entries get an image slot.
pub fn is_thumbnail_extension(path: &str) -> bool {
    let extension = extension_of(Path::new(path));
    extension == "svg"
        || IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension.as_str())
        || DESKTOP_RENDERED_EXTENSIONS.contains(&extension.as_str())
}

fn within_thumbnail_pixel_cap(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) <= THUMBNAIL_MAX_DECODED_PIXELS
}

/// A thumbnail of `path` at `size` device pixels.
///
/// `Ok(None)` is a miss the frontend falls back from: an unsupported type, a
/// path that went away, or a type no installed producer can render. `Err` is a
/// genuinely broken read and surfaces as a 500.
pub fn render_thumbnail(path_string: &str, size: u32) -> Result<Option<Rendered>, String> {
    let path = Path::new(path_string);
    let extension = extension_of(path);
    if !is_thumbnail_extension(path_string) {
        return Ok(None);
    }

    // A file that disappeared between the listing and this request is a miss,
    // not a failed read: the frontend already has a glyph for it.
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(None),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let size = size.clamp(1, 1024);

    // SVG streams straight through as bytes — no decode, no resizing.
    if extension == "svg" {
        if metadata.len() > SVG_MAX_SOURCE_BYTES {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|error| format!("thumbnail read: {error}"))?;
        return Ok(Some(Rendered {
            mime: "image/svg+xml",
            bytes,
        }));
    }

    // Shell thumbnails cover PDFs, video frames, HEIC and the raster formats
    // too large to decode in process. They are comparatively expensive, so the
    // byte budget keeps a multi-gigabyte clip away from the shell handler.
    let is_shell_source = !IN_PROCESS_IMAGE_EXTENSIONS.contains(&extension.as_str());
    let source_cap = if is_shell_source {
        SHELL_THUMBNAIL_MAX_SOURCE_BYTES
    } else {
        THUMBNAIL_MAX_SOURCE_BYTES
    };
    if metadata.len() > source_cap {
        return Ok(None);
    }

    if is_shell_source {
        return desktop_thumbnail(path_string, size, &metadata);
    }

    match decode_and_scale(path, size)? {
        DecodeOutcome::Frame(rendered) => Ok(Some(rendered)),
        // Too many pixels to hold here, but not a file the desktop cannot
        // thumbnail. Every platform's producer scales as it decodes.
        DecodeOutcome::Oversized => desktop_thumbnail(path_string, size, &metadata),
    }
}

/// The operating system's icon for one entry.
pub fn render_file_icon(path: &str, is_dir: bool, size: u32) -> Option<Rendered> {
    file_icons::extract(path, size.clamp(16, 256), is_dir).map(rendered_icon)
}

/// The icon the desktop draws for a whole type, named by an extension.
pub fn render_type_icon(extension: &str, size: u32) -> Option<Rendered> {
    let extension = normalized_extension(extension)?;
    ensure_shell_apartment();
    file_icons::extract_type(&extension, size.clamp(16, 256)).map(rendered_icon)
}

/// The desktop icon theme's own icon for a name, on the platforms that key
/// icons on names.
pub fn render_named_icon(name: &str, size: u32) -> Option<Rendered> {
    file_icons::named(name, size.clamp(16, 256)).map(rendered_icon)
}

fn rendered_icon(icon: file_icons::FileIcon) -> Rendered {
    Rendered {
        mime: icon.mime,
        bytes: icon.bytes,
    }
}

/// The extension a shared-icon request names, normalised for a cache key and
/// checked so a URL cannot smuggle a path into the shell.
fn normalized_extension(value: &str) -> Option<String> {
    let extension = value.trim().to_ascii_lowercase();
    let names_a_type = !extension.is_empty()
        && extension.len() <= MAX_ICON_EXTENSION_LEN
        && extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');

    names_a_type.then_some(extension)
}

/// `SHDefExtractIconW` is called from a render worker that already owns an
/// apartment, but tests and the inline fallback can reach the type route from a
/// thread without one.
#[cfg(windows)]
fn ensure_shell_apartment() {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

#[cfg(not(windows))]
fn ensure_shell_apartment() {}

/// What an in-process decode produced, when it is asked to produce anything.
enum DecodeOutcome {
    Frame(Rendered),
    /// More pixels than [`THUMBNAIL_MAX_DECODED_PIXELS`], refused before the
    /// decode allocated anything.
    Oversized,
}

fn decode_and_scale(path: &Path, size: u32) -> Result<DecodeOutcome, String> {
    // Header-only read first, so an image over the cap is refused before its
    // pixels are allocated.
    let (header_width, header_height) = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| format!("thumbnail open: {error}"))?
        .into_dimensions()
        .map_err(|error| format!("thumbnail dimensions: {error}"))?;
    if !within_thumbnail_pixel_cap(header_width, header_height) {
        return Ok(DecodeOutcome::Oversized);
    }

    let mut reader = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| format!("thumbnail open: {error}"))?;

    // Bounds the allocation even if a decoder's body disagrees with its header.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(THUMBNAIL_MAX_DECODED_PIXELS * 4 * 2);
    reader.limits(limits);

    let decoded = reader
        .decode()
        .map_err(|error| format!("thumbnail decode: {error}"))?;

    let (width, height) = image::GenericImageView::dimensions(&decoded);
    if !within_thumbnail_pixel_cap(width, height) {
        return Ok(DecodeOutcome::Oversized);
    }

    // One fast nearest-neighbor step when the image is far larger than the
    // target keeps the final smooth pass cheap on huge photos.
    let target = size.max(1);
    let working = if width > target * 4 && height > target * 4 {
        decoded.resize_exact(target * 4, target * 4, image::imageops::FilterType::Nearest)
    } else {
        decoded
    };
    let scaled = working.thumbnail(target, target);

    // Photos decode to opaque pixels; keep PNG only where alpha matters so
    // typical JPEGs stay small.
    if scaled.color().has_alpha() {
        let mut bytes = Vec::new();
        scaled
            .to_rgba8()
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .map_err(|error| format!("thumbnail encode: {error}"))?;
        Ok(DecodeOutcome::Frame(Rendered {
            mime: "image/png",
            bytes,
        }))
    } else {
        let mut bytes = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82);
        image::DynamicImage::ImageRgb8(scaled.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|error| format!("thumbnail encode: {error}"))?;
        Ok(DecodeOutcome::Frame(Rendered {
            mime: "image/jpeg",
            bytes,
        }))
    }
}

/// The first-page / first-frame producer, one per platform.
///
/// macOS has no producer wired here; the frontend falls back to its glyph for
/// PDF, video and HEIC, exactly as the Tauri shell does today.
#[cfg(windows)]
fn desktop_thumbnail(
    path: &str,
    size: u32,
    _metadata: &fs::Metadata,
) -> Result<Option<Rendered>, String> {
    Ok(
        dae_core::file_system::preview::extract_shell_thumbnail_png(path, size).map(|bytes| {
            Rendered {
                mime: "image/png",
                bytes,
            }
        }),
    )
}

/// The freedesktop thumbnailers, run and cached by `dae_core::desktop_thumbnails`.
#[cfg(target_os = "linux")]
fn desktop_thumbnail(
    path: &str,
    size: u32,
    metadata: &fs::Metadata,
) -> Result<Option<Rendered>, String> {
    Ok(
        dae_core::desktop_thumbnails::extract(path, size, metadata.modified().ok()).map(
            |produced| Rendered {
                mime: produced.mime,
                bytes: produced.bytes,
            },
        ),
    )
}

#[cfg(not(any(windows, target_os = "linux")))]
fn desktop_thumbnail(
    _path: &str,
    _size: u32,
    _metadata: &fs::Metadata,
) -> Result<Option<Rendered>, String> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fixture file in a directory of its own, removed when the test ends.
    /// Per-fixture rather than per-run because the test binary runs these in
    /// parallel.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);

            let directory = std::env::temp_dir().join(format!(
                "dae-native-thumbnail-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).expect("create the fixture directory");

            Self(directory.join(name))
        }

        fn png(name: &str) -> Self {
            let fixture = Self::new(name);
            image::RgbaImage::new(4, 4)
                .save(fixture.path())
                .expect("write the fixture image");
            fixture
        }

        fn text(name: &str, contents: &str) -> Self {
            let fixture = Self::new(name);
            std::fs::write(fixture.path(), contents).expect("write the fixture file");
            fixture
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn path_string(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(directory) = self.0.parent() {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }

    /// Which files get an image slot at all, and that the two categories never
    /// overlap: a shared extension would spawn a desktop producer for a raster
    /// this process decodes in milliseconds.
    #[test]
    fn the_extension_sets_cover_the_right_files_and_never_overlap() {
        for extension in IN_PROCESS_IMAGE_EXTENSIONS {
            assert!(
                !DESKTOP_RENDERED_EXTENSIONS.contains(extension),
                "{extension} cannot be in both"
            );
            assert!(is_thumbnail_extension(&format!("photo.{extension}")));
        }

        for extension in DESKTOP_RENDERED_EXTENSIONS {
            assert!(
                is_thumbnail_extension(&format!("file.{extension}")),
                "{extension} should have a desktop producer"
            );
        }

        assert!(is_thumbnail_extension("vector.svg"));
        for name in ["notes.txt", "archive.zip", "Makefile", "noext"] {
            assert!(
                !is_thumbnail_extension(name),
                "{name} has no producer and must not claim an image slot"
            );
        }
    }

    #[test]
    fn caps_decoded_pixels_at_the_boundary() {
        let (wide, tall) = (4096u32, 10_240u32);
        assert_eq!(
            u64::from(wide) * u64::from(tall),
            THUMBNAIL_MAX_DECODED_PIXELS,
            "the fixture sits exactly on the cap"
        );
        assert!(within_thumbnail_pixel_cap(wide, tall));
        assert!(!within_thumbnail_pixel_cap(wide, tall + 1));
        assert!(!within_thumbnail_pixel_cap(0, 100));
        assert!(!within_thumbnail_pixel_cap(u32::MAX, u32::MAX));
    }

    #[test]
    fn serves_a_thumbnail_for_a_decodable_image() {
        let fixture = Fixture::png("thumb.png");
        let rendered = render_thumbnail(&fixture.path_string(), 32)
            .expect("the fixture decodes")
            .expect("a 4x4 png has a thumbnail");

        assert!(rendered.mime.starts_with("image/"));
        assert!(!rendered.bytes.is_empty());
    }

    /// The formats the old `image::open` call could not read. Each is decoded
    /// in process and re-encoded, so a missing cargo feature would be a miss
    /// here rather than a type icon in the grid.
    #[test]
    fn decodes_the_formats_the_old_pipeline_refused() {
        for (name, format) in [
            ("frame.bmp", image::ImageFormat::Bmp),
            ("frame.gif", image::ImageFormat::Gif),
            ("frame.tiff", image::ImageFormat::Tiff),
        ] {
            let fixture = Fixture::new(name);
            image::RgbaImage::new(4, 4)
                .save_with_format(fixture.path(), format)
                .expect("write the fixture image");

            let rendered = render_thumbnail(&fixture.path_string(), 32)
                .unwrap_or_else(|error| panic!("{name} failed to render: {error}"))
                .unwrap_or_else(|| panic!("{name} has a thumbnail"));
            assert!(!rendered.bytes.is_empty(), "{name} produced no bytes");
        }
    }

    #[test]
    fn an_unsupported_type_and_a_missing_file_are_misses_not_errors() {
        let text = Fixture::text("notes.txt", "not an image");
        assert!(
            render_thumbnail(&text.path_string(), 32)
                .expect("a miss is not an error")
                .is_none()
        );

        let gone = Fixture::png("gone.png");
        std::fs::remove_file(gone.path()).expect("remove the fixture");
        assert!(
            render_thumbnail(&gone.path_string(), 32)
                .expect("a path that went away is a miss")
                .is_none()
        );
    }

    /// The icon route end to end, through the same dae-core backends the
    /// protocol uses. Windows always has a shell icon for `notepad.exe` and a
    /// registered icon for the `.exe` type.
    #[cfg(windows)]
    #[test]
    fn extracts_a_shell_icon_and_a_type_icon() {
        assert!(
            render_file_icon("C:\\Windows\\System32\\notepad.exe", false, 64).is_some(),
            "notepad.exe exposes a shell icon"
        );
        assert!(
            render_type_icon("exe", 32).is_some(),
            "the .exe type is registered with the shell"
        );
    }

    /// The same extraction, on the pool thread the N-API surface uses. Kept
    /// apart from the test above because a pool worker has its own COM
    /// apartment and message pump, and that is the path a protocol request
    /// actually takes.
    #[cfg(windows)]
    #[test]
    fn extracts_a_shell_icon_on_a_render_worker() {
        let (sender, receiver) = std::sync::mpsc::channel();
        crate::render_pool::submit(Box::new(move || {
            let _ = sender.send(render_file_icon(
                "C:\\Windows\\System32\\notepad.exe",
                false,
                64,
            ));
        }));

        let rendered = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the render worker answers");
        assert!(rendered.is_some(), "the worker extracts the shell icon");
    }

    /// The value reaches the desktop's type registry as a name; anything that
    /// could read as a path is refused before it gets there.
    #[test]
    fn a_shared_icon_request_accepts_a_type_and_refuses_a_path() {
        assert_eq!(normalized_extension("rs").as_deref(), Some("rs"));
        assert_eq!(normalized_extension("PDF").as_deref(), Some("pdf"));
        assert_eq!(normalized_extension("msiexec").as_deref(), Some("msiexec"));
        assert_eq!(normalized_extension("tar.gz"), None);
        assert_eq!(normalized_extension(".."), None);
        assert_eq!(normalized_extension("..\\windows\\system32"), None);
        assert_eq!(normalized_extension(""), None);
        assert_eq!(
            normalized_extension(&"x".repeat(MAX_ICON_EXTENSION_LEN + 1)),
            None
        );
    }
}
