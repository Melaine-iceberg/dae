//! Windows: the shell's own icon, through `IShellItemImageFactory`.
//!
//! The COM plumbing lives in [`crate::fs::windows_image`] because the shell's
//! context-menu code reaches the same extractor to turn an icon path into bytes,
//! and the two callers have to agree on one apartment and one cache rather than
//! maintaining two.
//!
//! What this module adds is the seam: the raw PNG the shell hands over is
//! labelled, and the folder case stops being a special one. `IShellItemImageFactory`
//! resolves a directory exactly as it resolves a file, so `is_dir` needs no
//! branch here — unlike on Linux, where a folder's icon lives in a different
//! context of the theme than a file's does.
//!
//! The one exception is [`extract_type`], which does not go through that pipeline:
//! a type icon is a question about a *registration*, not about a file, and the
//! shell answers it from the name alone.

use super::FileIcon;
use crate::fs::windows_image::{ShellImage, shell_image_png};
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
    SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

pub(super) fn extract(path: &str, size: u32, _is_dir: bool) -> Option<FileIcon> {
    // ICONONLY keeps document thumbnails out — views pair these icons with
    // their own image thumbnails.
    let bytes = shell_image_png(path, size, ShellImage::Icon)?;
    Some(FileIcon {
        mime: "image/png",
        bytes,
    })
}

/// The icon the shell is *registered* to draw for an extension — the same bitmap
/// for every file that shares it, which is what makes one request serve a column.
///
/// `SHGetFileInfoW` with `SHGFI_USEFILEATTRIBUTES` is the extractor that answers
/// that question from the name alone: it resolves the registered class off the
/// extension and never opens the file. `SHDefExtractIconW` would be the more
/// direct call, but on Windows 11 24H2+ it fails with `E_FAIL` for a name with no
/// file behind it — measured on build 26340 — while this path still returns the
/// registered icon, and the generic document icon for a type nothing claims.
///
/// The shell's large icon is 32 px; a row asks in device pixels (18 dp at 1.5
/// scale is 27), so that fits without upscaling. `size` only chooses between the
/// two system sizes, because the large/small pair is all this API offers.
pub(super) fn extract_type(extension: &str, size: u32) -> Option<FileIcon> {
    // A placeholder name: only its extension is read.
    let name: Vec<u16> = format!("type.{extension}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_ICON
        | SHGFI_USEFILEATTRIBUTES
        | if size <= 16 {
            SHGFI_SMALLICON
        } else {
            SHGFI_LARGEICON
        };
    // SAFETY: `name` is a null-terminated buffer that outlives the call, and
    // `info` is a valid out-parameter for the size the API is told.
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if result == 0 || info.hIcon.is_invalid() {
        return None;
    }

    let bytes = icon_to_png(info.hIcon);
    // SAFETY: the icon came from `SHGetFileInfoW`, is owned here, and is
    // destroyed on the way out whether or not the rasterization worked.
    unsafe {
        let _ = DestroyIcon(info.hIcon);
    };

    bytes.map(|bytes| FileIcon {
        mime: "image/png",
        bytes,
    })
}

/// An `HICON` as PNG bytes, through the colour bitmap inside it.
///
/// The same two steps the context-menu code runs for a command's icon resource:
/// `GetIconInfo` for the DIB, then [`crate::fs::windows_image::bitmap_to_png`]
/// out of GDI. A monochrome icon carries no colour bitmap, which the shared
/// conversion reads as the miss it is — the generic glyph beats a black-and-white
/// surprise.
fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    // SAFETY: `icon` is a live icon handle and `info` a valid out-pointer.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;

    let png = crate::fs::windows_image::bitmap_to_png(info.hbmColor);
    // SAFETY: `GetIconInfo` hands over two bitmaps that the caller owns.
    unsafe {
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
    }
    png
}
