//! Desktop-produced thumbnails: a PDF's first page, a video's first frame,
//! HEIC/AVIF photos the `image` crate cannot decode.
//!
//! The formats both shells decode in process do not come here; this is the
//! fallback for what a decoder cannot open and the desktop can. Both shells use
//! it, and a `None` answer is normal: it means nothing installed claims the type,
//! and the view keeps its type icon or drawn glyph.
//!
//! Windows asks the shell's registered thumbnail handler through
//! `crate::fs::windows_image`. Linux reads the freedesktop Thumbnailing spec in
//! `linux`: the `.thumbnailer` files packages install, the `Exec=` line they
//! declare, and the shared cache under `$XDG_CACHE_HOME/thumbnails/` that every
//! viewer on the desktop reads and writes. macOS asks QuickLook through
//! `qlmanage`, which is the same generator Finder uses for PDF, video and the
//! image formats the `image` crate cannot decode.

use std::time::SystemTime;

/// One thumbnail a desktop produced, labelled with the type its bytes carry.
///
/// The label travels with the bytes because the producer decides the format: the
/// Windows shell hands back a bitmap this crate encodes as PNG, and a Linux
/// `.thumbnailer` is asked for a `.png` output name and checked against its
/// signature. A caller answers its protocol request with the pair verbatim
/// rather than assuming one.
pub struct Thumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Extensions with no in-process decoder that some desktop does render.
///
/// Kept equal to the Tauri shell's `DESKTOP_RENDERED_EXTENSIONS` so the two
/// shells ask for the same set: documents, video, and the image formats the
/// `image` crate cannot read. A `.jpg` is absent on purpose — decoding it in
/// process costs about 1.5 ms where spawning a `.thumbnailer` costs about 10 ms
/// before any decoding starts.
const DESKTOP_RENDERED_EXTENSIONS: &[&str] = &[
    "pdf", "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "heic", "heif", "avif", "jxl", "apng",
    "tga", "qoi", "exr", "dds", "pbm", "pgm", "ppm",
];

/// Whether this path's type is one a desktop may be able to thumbnail.
pub fn is_supported_path(path: &str) -> bool {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    DESKTOP_RENDERED_EXTENSIONS.contains(&extension.as_str())
}

/// A thumbnail of `path` at `size`, or `None` when this platform or this
/// machine has no producer for the type.
///
/// Callers choose what is worth asking for with [`is_supported_path`]: a listing
/// uses it to decide whether an entry gets an image slot at all. This function
/// deliberately does not repeat that filter — the views also come here for an
/// in-process format too large to decode (a 100 MP JPEG or TIFF), where the
/// desktop's own producer can still make one, and a type nothing claims is
/// declined quietly by every backend.
///
/// `modified` is the source's mtime, where the caller already has it. Linux's
/// shared cache is invalidated by comparing a cached file with its source, and a
/// producer that ran and produced nothing is remembered the same way; re-`stat`ing
/// here would be a second read of the same inode on the thread this pipeline
/// exists to keep free of blocking work. Windows answers from
/// `IShellItemImageFactory`'s own cache and ignores the value, so a caller that
/// cannot date the source passes `None`.
pub fn extract(path: &str, size: u32, modified: Option<SystemTime>) -> Option<Thumbnail> {
    backend::extract(path, size, modified)
}

/// Bounded bytes of a remote-backed image, for the in-process decoders to
/// thumbnail (G-5: Dolphin's "pull it when the user can see it").
///
/// Only the formats the view decodes itself are worth the wire: the
/// desktop-produced set (PDF, video, HEIC) would download megabytes the
/// caller cannot decode without a local file. `None` is a normal miss — the
/// row keeps its type glyph — and covers every reason: unsupported format,
/// unreachable backend, not a file, empty, or larger than `max_bytes`.
pub fn fetch_remote(path: &str, max_bytes: u64) -> Option<Vec<u8>> {
    use std::io::Read;

    if !super::preview::is_thumbnail_path(path) || max_bytes == 0 {
        return None;
    }
    let backend = super::vfs::resolve(path).ok()?;
    let stat = backend.stat(path).ok()?;
    if stat.kind != super::types::EntryKind::File || stat.size == 0 || stat.size > max_bytes {
        return None;
    }
    let reader = backend.open_read(path).ok()?;
    let mut bytes = Vec::new();
    reader.take(max_bytes).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

// Compiled under `cfg(test)` as well as on Linux, the same arrangement as
// `icons::linux`: the spec parsing and key arithmetic are where the quiet bugs
// live, and they can be tested on a host that cannot build a Linux app at all.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;

#[cfg(target_os = "linux")]
use linux as backend;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(windows)]
mod backend {
    //! The shell's registered thumbnail handler, through `windows_image`.
    //! `SIIGBF_THUMBNAILONLY` makes a file with no handler a miss rather than an
    //! icon scaled up to thumbnail size.

    use std::time::SystemTime;

    use super::Thumbnail;

    pub(super) fn extract(
        path: &str,
        size: u32,
        _modified: Option<SystemTime>,
    ) -> Option<Thumbnail> {
        crate::fs::windows_image::shell_image_png(
            path,
            size,
            crate::fs::windows_image::ShellImage::Thumbnail,
        )
        .map(|bytes| Thumbnail {
            mime: "image/png",
            bytes,
        })
    }
}

/// No producer is written for this platform. Saying so is the difference between
/// a view that falls back to its type glyph and one that fails to build.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod backend {
    use std::time::SystemTime;

    pub(super) fn extract(
        _path: &str,
        _size: u32,
        _modified: Option<SystemTime>,
    ) -> Option<super::Thumbnail> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_desktop_rendered_types() {
        assert!(is_supported_path("C:/tree/report.pdf"));
        assert!(is_supported_path("C:/tree/clip.MP4"));
        assert!(is_supported_path("C:/tree/photo.heic"));
        assert!(!is_supported_path("C:/tree/photo.jpg"));
        assert!(!is_supported_path("C:/tree/main.rs"));
    }

    #[test]
    fn fetch_remote_reads_a_non_local_image_and_rejects_the_rest() {
        use std::io::Write as _;

        let dir = std::env::temp_dir()
            .join(format!("dae-thumbnails-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A one-pixel PNG, small enough to spell out.
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];

        // The archive backend stands in for "remote": a real non-local
        // scheme whose `open_read` serves bounded bytes over the same trait.
        let zip_path = dir.join("shots.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("pixel.png", options).unwrap();
        zip.write_all(png).unwrap();
        zip.start_file("report.pdf", options).unwrap();
        zip.write_all(b"%PDF-1.4").unwrap();
        zip.start_file("huge.png", options).unwrap();
        zip.write_all(&vec![0u8; 512]).unwrap();
        zip.add_directory("folder", options).unwrap();
        zip.finish().unwrap();
        let base = format!("archive:{}!", zip_path.to_string_lossy());

        assert_eq!(
            fetch_remote(&format!("{base}/pixel.png"), 1024 * 1024).as_deref(),
            Some(png)
        );
        // A format the desktop produces but the view cannot decode over the
        // wire is not worth the download.
        assert_eq!(fetch_remote(&format!("{base}/report.pdf"), 1024 * 1024), None);
        // The byte cap answers before the first read.
        assert_eq!(fetch_remote(&format!("{base}/huge.png"), 64), None);
        // Directories and unknown members miss quietly.
        assert_eq!(fetch_remote(&format!("{base}/folder"), 1024 * 1024), None);
        assert_eq!(fetch_remote(&format!("{base}/missing.png"), 1024 * 1024), None);
    }
}
