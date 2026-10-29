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
//! viewer on the desktop reads and writes. macOS has no producer in either shell
//! — the `image` crate covers the formats the current views show.

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

// Compiled under `cfg(test)` as well as on Linux, the same arrangement as
// `icons::linux`: the spec parsing and key arithmetic are where the quiet bugs
// live, and they can be tested on a host that cannot build a Linux app at all.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;

#[cfg(target_os = "linux")]
use linux as backend;

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
#[cfg(not(any(windows, target_os = "linux")))]
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
}
