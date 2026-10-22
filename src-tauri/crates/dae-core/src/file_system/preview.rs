//! Windows shell images: the icons and thumbnails Explorer itself draws.
//!
//! `IShellItemImageFactory` resolves a path to whatever the shell would show —
//! the target icon for a `.lnk`/`.url` shortcut, the embedded icon for an
//! executable, the registered thumbnail handler for a PDF or a video. Two flags
//! separate the two questions: `SIIGBF_ICONONLY` asks for the icon, and
//! `SIIGBF_RESIZETOFIT` without it asks for the thumbnail.
//!
//! The GDI copy at the bottom is shared with `file_icons`, which reaches a
//! bitmap through `SHDefExtractIconW` when a type names an icon resource rather
//! than a file.

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_ICONONLY,
    SIIGBF_RESIZETOFIT,
};
use windows::core::HSTRING;

/// The shell icon for `path` at `size` pixels, as PNG bytes.
///
/// `SIIGBF_ICONONLY` keeps document thumbnails out: views pair these icons with
/// their own image thumbnails.
pub fn extract_file_icon_png(path: &str, size: u32) -> Option<Vec<u8>> {
    shell_image_png(path, size, SIIGBF_ICONONLY | SIIGBF_RESIZETOFIT)
}

/// The shell thumbnail for `path` at `size` pixels, as PNG bytes.
///
/// First page, first frame or first camera image, whichever handler the machine
/// registered. A type with no handler yields `None`, which callers turn into a
/// 404 so the row falls back to its type glyph.
pub fn extract_shell_thumbnail_png(path: &str, size: u32) -> Option<Vec<u8>> {
    shell_image_png(path, size, SIIGBF_RESIZETOFIT)
}

/// Shared `IShellItemImageFactory` pipeline: resolves the shell image for
/// `path` at `size` and rasterizes it to PNG.
pub fn shell_image_png(path: &str, size: u32, flags: SIIGBF) -> Option<Vec<u8>> {
    unsafe {
        // Protocol-handler threads may never have initialized COM; the shell
        // item APIs require an apartment. A thread that already has one gets
        // `S_FALSE`, which is equally usable.
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

/// Copies a shell-produced `HBITMAP` into top-down RGBA pixels and encodes it
/// as PNG.
pub fn bitmap_to_png(bitmap: HBITMAP) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW,
        ReleaseDC,
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

#[cfg(test)]
mod tests {
    use super::{extract_file_icon_png, extract_shell_thumbnail_png};

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
        assert!(extract_shell_thumbnail_png("C:\\does-not-exist.mp4", 64).is_none());
    }
}
