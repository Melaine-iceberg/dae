//! The system clipboard's file list.
//!
//! Explorer, chat apps and browser upload fields exchange files through
//! `CF_HDROP` plus the registered `Preferred DropEffect` format, not through a
//! window toolkit's own clipboard object. This module is the engine-free half
//! of that contract: it writes a path list in those two formats and reads it
//! back with the copy/cut/link marker resolved.
//!
//! Remote paths never reach the system clipboard — a `smb://` URL means
//! nothing to Explorer — so callers filter to local paths before writing and
//! keep their own mirror for the rest.

use super::error::FileSystemError;

/// What the copy source asked the paste target to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEffect {
    Copy,
    Move,
    /// `DROPEFFECT_LINK`: the target should create shortcuts instead of
    /// transferring the files.
    Link,
}

/// A file list on the system clipboard, with the drop effect resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemClipboardFiles {
    pub paths: Vec<String>,
    pub effect: ClipboardEffect,
}

// Plain `DROPEFFECT_*` values so the mapping tests run on every host.
const DROPEFFECT_COPY_VALUE: u32 = 1;
const DROPEFFECT_MOVE_VALUE: u32 = 2;
const DROPEFFECT_LINK_VALUE: u32 = 4;

fn effect_from_drop_effect(value: u32) -> ClipboardEffect {
    match value {
        DROPEFFECT_MOVE_VALUE => ClipboardEffect::Move,
        DROPEFFECT_LINK_VALUE => ClipboardEffect::Link,
        _ => ClipboardEffect::Copy,
    }
}

fn drop_effect_value(effect: ClipboardEffect) -> u32 {
    match effect {
        ClipboardEffect::Copy => DROPEFFECT_COPY_VALUE,
        ClipboardEffect::Move => DROPEFFECT_MOVE_VALUE,
        ClipboardEffect::Link => DROPEFFECT_LINK_VALUE,
    }
}

/// Places `paths` on the system clipboard as a file list carrying `effect`.
#[cfg(windows)]
pub fn write_files_to_clipboard(
    paths: &[String],
    effect: ClipboardEffect,
) -> Result<(), FileSystemError> {
    platform::write_files_to_clipboard(paths, effect)
}

/// Reads the file list the system clipboard holds, if it holds one.
#[cfg(windows)]
pub fn read_files_from_clipboard() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    platform::read_files_from_clipboard()
}

/// The Qt shell is Windows-first; the other platforms still keep their
/// in-app clipboard, so a missing system integration only narrows paste.
#[cfg(not(windows))]
pub fn write_files_to_clipboard(
    _paths: &[String],
    _effect: ClipboardEffect,
) -> Result<(), FileSystemError> {
    Err(FileSystemError::Unsupported(
        "The system file clipboard is only implemented on Windows".into(),
    ))
}

#[cfg(not(windows))]
pub fn read_files_from_clipboard() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    Ok(None)
}

/// Windows' clipboard change counter. A paste compares it with the value
/// recorded when the in-app mirror was written to tell "the clipboard still
/// holds what this shell put there" from "another app changed it".
#[cfg(windows)]
pub fn clipboard_sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
pub fn clipboard_sequence() -> u32 {
    0
}

#[cfg(windows)]
mod platform {
    use std::time::Duration;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
        SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
    use windows::core::{BOOL, PCWSTR};

    use super::{
        ClipboardEffect, SystemClipboardFiles, drop_effect_value, effect_from_drop_effect,
    };
    use crate::error::FileSystemError;

    const CLIPBOARD_OPEN_ATTEMPTS: usize = 10;
    const CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(10);

    fn handle_from_global(global: HGLOBAL) -> HANDLE {
        HANDLE(global.0)
    }

    fn global_from_handle(handle: HANDLE) -> HGLOBAL {
        HGLOBAL(handle.0)
    }

    fn drop_from_handle(handle: HANDLE) -> HDROP {
        HDROP(handle.0)
    }

    /// Closes the clipboard when dropped, even on early returns.
    struct ClipboardSession;

    impl ClipboardSession {
        fn open() -> Result<Self, FileSystemError> {
            for _ in 0..CLIPBOARD_OPEN_ATTEMPTS {
                // Other apps can hold the clipboard briefly; retry a few times.
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Self);
                }
                std::thread::sleep(CLIPBOARD_RETRY_DELAY);
            }

            Err(FileSystemError::Internal(
                "Unable to open the system clipboard".into(),
            ))
        }
    }

    impl Drop for ClipboardSession {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    /// Allocates a movable global block and fills it. `GMEM_MOVEABLE` blocks
    /// must stay locked while writing, which this handles.
    fn alloc_filled(size: usize, fill: impl FnOnce(*mut u8)) -> Result<HGLOBAL, FileSystemError> {
        // `GMEM_ZEROINIT` keeps the `DROPFILES` fields this code does not set
        // (`pt`, `fNC`) at zero instead of whatever the heap last held.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size) }
            .map_err(|error| FileSystemError::Internal(error.to_string()))?;
        let base = unsafe { GlobalLock(handle) }.cast::<u8>();
        if base.is_null() {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
            return Err(FileSystemError::Internal(
                "Unable to lock clipboard memory".into(),
            ));
        }

        fill(base);
        unsafe {
            let _ = GlobalUnlock(handle);
        }
        Ok(handle)
    }

    /// Builds a `CF_HDROP` block: a `DROPFILES` header followed by a
    /// double-null-terminated UTF-16 path list.
    fn build_file_drop_handle(paths: &[String]) -> Result<HGLOBAL, FileSystemError> {
        let mut wide: Vec<u16> = Vec::new();
        for path in paths {
            wide.extend(path.encode_utf16());
            wide.push(0);
        }
        wide.push(0);

        let header_size = std::mem::size_of::<DROPFILES>();
        let size = header_size + wide.len() * 2;
        alloc_filled(size, |base| unsafe {
            let header = base as *mut DROPFILES;
            (*header).pFiles = header_size as u32;
            (*header).fWide = BOOL(1);
            std::ptr::copy_nonoverlapping(
                wide.as_ptr().cast(),
                base.add(header_size),
                wide.len() * 2,
            );
        })
    }

    /// Reads the path list back out of a `CF_HDROP` handle. The handle belongs
    /// to the clipboard, so it is queried but not freed here.
    fn file_paths_from_drop_handle(drop: HDROP) -> Vec<String> {
        unsafe {
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None);
                let mut buffer = vec![0u16; length as usize + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut buffer));
                paths.push(String::from_utf16_lossy(&buffer[..copied as usize]));
            }
            paths
        }
    }

    fn wide_null_terminated(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Explorer communicates the effect through a registered clipboard format
    /// holding a `DROPEFFECT` value.
    fn preferred_drop_effect_format() -> u32 {
        unsafe {
            RegisterClipboardFormatW(PCWSTR(
                wide_null_terminated("Preferred DropEffect").as_ptr(),
            ))
        }
    }

    pub(super) fn write_files_to_clipboard(
        paths: &[String],
        effect: ClipboardEffect,
    ) -> Result<(), FileSystemError> {
        if paths.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Choose at least one entry before copying".into(),
            ));
        }

        let files = build_file_drop_handle(paths)?;
        let effect_handle = alloc_filled(std::mem::size_of::<u32>(), |base| unsafe {
            (base as *mut u32).write(drop_effect_value(effect));
        })?;

        let session = ClipboardSession::open();
        if session.is_err() {
            unsafe {
                let _ = GlobalFree(Some(files));
                let _ = GlobalFree(Some(effect_handle));
            }
            return session.map(|_| ());
        }

        unsafe {
            if let Err(error) = EmptyClipboard() {
                let _ = GlobalFree(Some(files));
                let _ = GlobalFree(Some(effect_handle));
                return Err(FileSystemError::Internal(error.to_string()));
            }

            // Once SetClipboardData succeeds the system owns the handle.
            if let Err(error) = SetClipboardData(CF_HDROP.0 as u32, Some(handle_from_global(files)))
            {
                let _ = GlobalFree(Some(files));
                let _ = GlobalFree(Some(effect_handle));
                return Err(FileSystemError::Internal(error.to_string()));
            }

            let format = preferred_drop_effect_format();
            if format != 0
                && let Err(error) =
                    SetClipboardData(format, Some(handle_from_global(effect_handle)))
            {
                // The file list is already on the clipboard; a missing effect
                // marker only downgrades the paste to a copy.
                let _ = GlobalFree(Some(effect_handle));
                return Err(FileSystemError::Internal(error.to_string()));
            }
        }

        Ok(())
    }

    pub(super) fn read_files_from_clipboard()
    -> Result<Option<SystemClipboardFiles>, FileSystemError> {
        let _session = ClipboardSession::open()?;

        unsafe {
            let Ok(data) = GetClipboardData(CF_HDROP.0 as u32) else {
                return Ok(None);
            };
            if data.is_invalid() {
                return Ok(None);
            }

            let paths = file_paths_from_drop_handle(drop_from_handle(data));
            if paths.is_empty() {
                return Ok(None);
            }

            let mut effect = ClipboardEffect::Copy;
            let format = preferred_drop_effect_format();
            if format != 0
                && let Ok(effect_handle) = GetClipboardData(format)
                && !effect_handle.is_invalid()
            {
                let effect_global = global_from_handle(effect_handle);
                let value = GlobalLock(effect_global).cast::<u32>();
                if !value.is_null() {
                    effect = effect_from_drop_effect(*value);
                    let _ = GlobalUnlock(effect_global);
                }
            }

            Ok(Some(SystemClipboardFiles { paths, effect }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_effect_values_round_trip() {
        for effect in [
            ClipboardEffect::Copy,
            ClipboardEffect::Move,
            ClipboardEffect::Link,
        ] {
            assert_eq!(effect_from_drop_effect(drop_effect_value(effect)), effect);
        }
    }

    #[test]
    fn unknown_effects_fall_back_to_copy() {
        assert_eq!(effect_from_drop_effect(0), ClipboardEffect::Copy);
        assert_eq!(effect_from_drop_effect(0x4004), ClipboardEffect::Copy);
    }
}
