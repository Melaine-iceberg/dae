//! Desktop-produced thumbnails: a PDF's first page, a video's first frame,
//! HEIC/AVIF photos the `image` crate cannot decode.
//!
//! The formats both shells decode in process do not come here; this is the
//! fallback for what a decoder cannot open and the desktop can. Both shells use
//! it, and a `None` answer is normal: it means nothing installed claims the type,
//! and the view keeps its type icon or drawn glyph.
//!
//! Windows asks the shell's registered thumbnail handler through
//! [`crate::fs::windows_image`]. Linux's freedesktop `.thumbnailer` runner still
//! lives in the Tauri shell (`desktop_thumbnails`) because it also needs the XDG
//! desktop-entry parser, which has not moved yet; it arrives with the Qt shell's
//! Linux gate. macOS has no producer in either shell — the `image` crate covers
//! the formats the current views show.

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

/// PNG bytes for a desktop-produced thumbnail at `size`, or `None` when this
/// platform or this machine has no producer for the type.
pub fn extract(path: &str, size: u32) -> Option<Vec<u8>> {
    if !is_supported_path(path) {
        return None;
    }

    #[cfg(windows)]
    {
        return crate::fs::windows_image::shell_image_png(
            path,
            size,
            crate::fs::windows_image::ShellImage::Thumbnail,
        );
    }

    #[cfg(not(windows))]
    {
        let _ = (path, size);
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
