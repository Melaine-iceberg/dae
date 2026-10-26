//! Windows shell bitmaps as PNG bytes, shared by icons and thumbnails.
//!
//! `IShellItemImageFactory` is the one API that answers "draw what Explorer
//! would draw" for a path, and it hands back an `HBITMAP` that only GDI can
//! read. Two callers want that bitmap as bytes: the icon path asks for
//! `SIIGBF_ICONONLY` (a document keeps its type icon next to a separate image
//! thumbnail), and the thumbnail path wants whatever the registered handler
//! produces. The apartment setup, the colour-bitmap copy and the PNG encode are
//! identical, so they live here rather than in both callers.

use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};

/// Which question the shell is being asked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShellImage {
    /// The entry's icon. `SIIGBF_ICONONLY` keeps document thumbnails out.
    Icon,
    /// A desktop-produced thumbnail. `SIIGBF_THUMBNAILONLY` makes a file with
    /// no thumbnail handler a miss instead of a 384 px copy of its icon.
    Thumbnail,
}

/// The shell's bitmap for `path` at `size`, encoded as PNG.
pub fn shell_image_png(path: &str, size: u32, kind: ShellImage) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_ICONONLY,
        SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
    };
    use windows::core::HSTRING;

    let flags: SIIGBF = match kind {
        ShellImage::Icon => SIIGBF_ICONONLY | SIIGBF_RESIZETOFIT,
        ShellImage::Thumbnail => SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT,
    };

    unsafe {
        // Callers run on worker threads that may never have initialized COM;
        // the shell item APIs require an apartment.
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
///
/// Also the second half of the shell's icon-resource extraction: an
/// `IExplorerCommand` can name an icon by resource id, which never reaches
/// `IShellItemImageFactory`.
pub fn bitmap_to_png(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Vec<u8>> {
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
        // GetDIBits wants a real DC for format negotiation; the screen DC is
        // process-wide and costs nothing here.
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
    use super::*;

    #[test]
    fn extracts_png_icon_for_executable() {
        let bytes = shell_image_png("C:\\Windows\\System32\\notepad.exe", 64, ShellImage::Icon)
            .expect("notepad.exe exposes a shell icon");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(bytes.len() > 100);
    }

    #[test]
    fn missing_path_yields_none() {
        assert!(shell_image_png("C:\\does-not-exist.lnk", 64, ShellImage::Icon).is_none());
    }

    #[test]
    fn extracts_a_png_type_icon_for_a_registered_extension() {
        let icon = crate::fs::icons::extract_type("json", 36)
            .expect("the shell has a registered icon for .json");
        assert_eq!(icon.mime, "image/png");
        assert_eq!(&icon.bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
