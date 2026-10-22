//! Windows: the shell's own icon, through `IShellItemImageFactory`.
//!
//! The implementation stays where it was written — `file_system::preview`'s COM
//! pipeline — because `shell_commands` reaches for the same function to turn the
//! icon paths an `IExplorerCommand` reports into bytes, and the two callers have
//! to agree on one apartment and one cache rather than maintaining two.
//!
//! What this module adds is the seam: the raw PNG the shell hands over is
//! labelled, and the folder case stops being a special one. `IShellItemImageFactory`
//! resolves a directory exactly as it resolves a file, so `is_dir` needs no
//! branch here — unlike on Linux, where a folder's icon lives in a different
//! context of the theme than a file's does.
//!
//! The one exception is [`extract_type`], which does not go through that pipeline:
//! a type icon is a question about a *registration*, not about a file, and the
//! shell has a separate extractor for it.

use super::FileIcon;
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{
    SHDefExtractIconW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON,
    SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

pub(super) fn extract(path: &str, size: u32, _is_dir: bool) -> Option<FileIcon> {
    let bytes = crate::file_system::preview::extract_file_icon_png(path, size)?;
    Some(FileIcon {
        mime: "image/png",
        bytes,
    })
}

/// The icon the shell is *registered* to draw for an extension — the same bitmap
/// for every file that shares it, which is what makes one request serve a column.
///
/// Two extractors, because they disagree about names that are not paths:
/// `SHDefExtractIconW` can render at the requested size but wants a real file
/// behind the name on some Windows builds, and `SHGetFileInfoW` with
/// `SHGFI_USEFILEATTRIBUTES` is the API that explicitly resolves a type without
/// touching the filesystem. The first is preferred for its size control; the
/// second is what answers when the placeholder name does not exist.
///
/// A name with no class registered still gets Windows' generic document icon, and
/// a type the shell truly has nothing for is a `None` here, which the protocol
/// turns into a 404 and the row keeps the glyph the frontend draws.
pub(super) fn extract_type(extension: &str, size: u32) -> Option<FileIcon> {
    // A placeholder name: only its extension is read, and the caller's value is
    // filtered to type-shaped characters before it gets here.
    let name: Vec<u16> = format!("type.{extension}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut icon = HICON::default();
    // SAFETY: `name` is a null-terminated buffer that outlives the call, and
    // `icon` is a valid out-pointer for the large-icon slot.
    let extracted =
        unsafe { SHDefExtractIconW(PCWSTR(name.as_ptr()), 0, 0, Some(&mut icon), None, size) };
    if extracted.is_ok() && !icon.is_invalid() {
        return icon_png(icon);
    }

    let mut info = SHFILEINFOW::default();
    let size_flag = if size <= 16 {
        SHGFI_SMALLICON
    } else {
        SHGFI_LARGEICON
    };
    // SAFETY: `name` is null-terminated; `info` is a valid out-structure. The
    // attributes flag is what tells the shell not to stat the name.
    let found = unsafe {
        SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_USEFILEATTRIBUTES | size_flag,
        )
    };
    if found == 0 || info.hIcon.is_invalid() {
        return None;
    }

    icon_png(info.hIcon)
}

/// Rasterizes an owned icon and destroys the handle.
fn icon_png(icon: HICON) -> Option<FileIcon> {
    let bytes = icon_to_png(icon);
    // SAFETY: the icon is owned here, and is destroyed whether or not the
    // rasterization worked.
    unsafe {
        let _ = DestroyIcon(icon);
    }

    bytes.map(|bytes| FileIcon {
        mime: "image/png",
        bytes,
    })
}

/// An `HICON` as PNG bytes, through the colour bitmap inside it.
///
/// The same two steps `shell_commands` runs for a command's icon resource:
/// `GetIconInfo` for the DIB, then `preview`'s shared copy out of GDI. A
/// monochrome icon carries no colour bitmap, which `bitmap_to_png` reads as the
/// miss it is — the generic glyph beats a black-and-white surprise.
fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    // SAFETY: `icon` is a live icon handle and `info` a valid out-pointer.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;

    let png = crate::file_system::preview::bitmap_to_png(info.hbmColor);
    // SAFETY: `GetIconInfo` hands over two bitmaps that the caller owns.
    unsafe {
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
    }
    png
}
