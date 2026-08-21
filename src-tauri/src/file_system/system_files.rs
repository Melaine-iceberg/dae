//! OS-level file integration: the system clipboard's file list and native
//! drag-out of files to other applications.

use super::error::FileSystemError;
use serde::{Deserialize, Serialize};
use specta::Type;

/// File paths currently held by the system clipboard, with Explorer's
/// "Preferred DropEffect" marker resolved into a copy/cut flag.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SystemClipboardFiles {
    pub paths: Vec<String>,
    pub cut: bool,
}

/// Places the given paths on the system clipboard as a file list (CF_HDROP on
/// Windows) so Explorer, browsers, and chat apps accept a paste. Setting `cut`
/// also writes Explorer's "Preferred DropEffect" marker, which turns a paste
/// in Explorer or here into a move instead of a copy.
#[tauri::command]
#[specta::specta]
pub async fn write_files_to_clipboard(
    paths: Vec<String>,
    cut: bool,
) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before copying".into(),
        ));
    }

    tauri::async_runtime::spawn_blocking(move || write_files_to_clipboard_impl(&paths, cut))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Reads a file list from the system clipboard, if the clipboard holds one.
#[tauri::command]
#[specta::specta]
pub async fn read_files_from_clipboard() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(read_files_from_clipboard_impl)
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Transfer effect the native drag-out advertises to the drop target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum DragOutMode {
    Copy,
    Move,
    /// Drop targets create shortcuts instead of transferring the files
    /// (DROPEFFECT_LINK — Windows Explorer's Alt-drag behavior).
    Link,
}

/// Starts a native drag carrying the given files out of the window, e.g. into
/// Explorer, a chat app, or a browser upload field. `mode` decides the effect
/// the target performs: `copy` duplicates the files, `move` relocates them,
/// `link` creates shortcuts. Returns immediately; the drag runs on the main
/// thread in a modal loop until the user drops or cancels.
#[tauri::command]
#[specta::specta]
pub fn start_drag_out(
    paths: Vec<String>,
    mode: DragOutMode,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before dragging".into(),
        ));
    }

    start_drag_out_impl(paths, mode, &app)
}

/// Creates `.lnk` shortcuts pointing at `sources` inside `destination`,
/// resolving name collisions with ` (2)`, ` (3)`… suffixes like Explorer.
/// Returns the created shortcut paths so the UI can refresh and select them.
#[tauri::command]
#[specta::specta]
pub async fn create_shortcuts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<String>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry to link".into(),
        ));
    }

    tauri::async_runtime::spawn_blocking(move || create_shortcuts_impl(&sources, &destination))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[cfg(windows)]
mod platform {
    use super::{DragOutMode, SystemClipboardFiles};
    use crate::file_system::error::FileSystemError;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GlobalFree, BOOL, HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
        SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE};
    use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, DROPFILES, HDROP};

    const CLIPBOARD_OPEN_ATTEMPTS: usize = 10;
    const CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(10);

    fn handle_from_global(global: HGLOBAL) -> HANDLE {
        HANDLE(global.0 as isize)
    }

    fn global_from_handle(handle: HANDLE) -> HGLOBAL {
        HGLOBAL(handle.0 as *mut core::ffi::c_void)
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
                if unsafe { OpenClipboard(HWND::default()) }.is_ok() {
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

    /// Allocates a movable global block and fills it; `GMEM_MOVEABLE` blocks
    /// must stay locked while writing, which this handles.
    fn alloc_filled(size: usize, fill: impl FnOnce(*mut u8)) -> Result<HGLOBAL, FileSystemError> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) }
            .map_err(|error| FileSystemError::Internal(error.to_string()))?;
        let base = unsafe { GlobalLock(handle) }.cast::<u8>();
        if base.is_null() {
            unsafe {
                let _ = GlobalFree(handle);
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

    /// Builds a `CF_HDROP` global memory block: a `DROPFILES` header followed
    /// by a double-null-terminated UTF-16 path list.
    pub(super) fn build_file_drop_handle(paths: &[String]) -> Result<HGLOBAL, FileSystemError> {
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

    /// Reads the path list back out of a `CF_HDROP` handle.
    pub(super) fn file_paths_from_drop_handle(drop: HDROP) -> Vec<String> {
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

    /// Explorer communicates copy vs cut through a registered clipboard
    /// format holding a `DROPEFFECT` value.
    fn preferred_drop_effect_format() -> u32 {
        unsafe {
            RegisterClipboardFormatW(PCWSTR(
                wide_null_terminated("Preferred DropEffect").as_ptr(),
            ))
        }
    }

    pub(super) fn write_files_to_clipboard_impl(
        paths: &[String],
        cut: bool,
    ) -> Result<(), FileSystemError> {
        let files = build_file_drop_handle(paths)?;
        let effect_value: u32 = if cut {
            DROPEFFECT_MOVE.0
        } else {
            DROPEFFECT_COPY.0
        };
        let effect = alloc_filled(std::mem::size_of::<u32>(), |base| unsafe {
            (base as *mut u32).write(effect_value);
        })?;

        let session = ClipboardSession::open();
        if session.is_err() {
            unsafe {
                let _ = GlobalFree(files);
                let _ = GlobalFree(effect);
            }
            return session.map(|_| ());
        }

        unsafe {
            if let Err(error) = EmptyClipboard() {
                return Err(FileSystemError::Internal(error.to_string()));
            }

            // Once SetClipboardData succeeds the system owns the handles.
            if let Err(error) = SetClipboardData(CF_HDROP.0 as u32, handle_from_global(files)) {
                let _ = GlobalFree(files);
                let _ = GlobalFree(effect);
                return Err(FileSystemError::Internal(error.to_string()));
            }

            let format = preferred_drop_effect_format();
            if format != 0
                && let Err(error) = SetClipboardData(format, handle_from_global(effect))
            {
                // The file list is already on the clipboard; a missing cut
                // marker only downgrades the paste to a copy.
                let _ = GlobalFree(effect);
                return Err(FileSystemError::Internal(error.to_string()));
            }
        }

        Ok(())
    }

    pub(super) fn read_files_from_clipboard_impl()
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
            DragFinish(drop_from_handle(data));
            if paths.is_empty() {
                return Ok(None);
            }

            let mut cut = false;
            let format = preferred_drop_effect_format();
            if format != 0
                && let Ok(effect_handle) = GetClipboardData(format)
                && !effect_handle.is_invalid()
            {
                let effect_global = global_from_handle(effect_handle);
                let effect = GlobalLock(effect_global).cast::<u32>();
                if !effect.is_null() {
                    cut = *effect == DROPEFFECT_MOVE.0;
                    let _ = GlobalUnlock(effect_global);
                }
            }

            Ok(Some(SystemClipboardFiles { paths, cut }))
        }
    }

    /// Drag preview shown by the OS while dragging out of the window.
    const DRAG_PREVIEW_ICON: &[u8] = include_bytes!("../../icons/32x32.png");

    /// Set while a native drag-out is running. DoDragDrop runs a modal message
    /// loop on the main thread, and closures posted through
    /// `run_on_main_thread` can be dispatched re-entrantly from inside that
    /// loop, so a second gesture arriving mid-drag must not start a nested
    /// DoDragDrop.
    static DRAG_OUT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

    pub(super) fn start_drag_out_impl(
        paths: Vec<String>,
        mode: DragOutMode,
        app: &tauri::AppHandle,
    ) -> Result<(), FileSystemError> {
        use tauri::Manager;

        let Some(window) = app.get_webview_window("main") else {
            return Err(FileSystemError::Internal(
                "The main window was not found".into(),
            ));
        };

        // DoDragDrop must run on the main (STA) thread — the apartment that
        // owns the window, is already OLE-initialized by the runtime, and can
        // receive the drag's input messages. Running it on a background thread
        // leaves the drag dead (no input reaches the modal loop) and each
        // per-gesture thread that initializes OLE and then exits leaks a
        // destroyed apartment, which corrupts process-wide OLE state and later
        // crashes unrelated drag handling on the main thread (stack overflow).
        // This mirrors the official tauri-plugin-drag, which also dispatches
        // drag::start_drag onto the main thread.
        app.run_on_main_thread(move || {
            if DRAG_OUT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                return;
            }

            let item = drag::DragItem::Files(paths.into_iter().map(PathBuf::from).collect());
            let preview = drag::Image::Raw(DRAG_PREVIEW_ICON.to_vec());
            let drag_mode = match mode {
                DragOutMode::Copy => drag::DragMode::Copy,
                DragOutMode::Move => drag::DragMode::Move,
                DragOutMode::Link => drag::DragMode::Link,
            };
            // DoDragDrop blocks in a modal loop that pumps window messages
            // until the user drops or cancels, so the main thread stays
            // responsive for the duration of the gesture.
            if let Err(error) = drag::start_drag(
                &window,
                item,
                preview,
                |_, _| (),
                drag::Options {
                    skip_animatation_on_cancel_or_failure: false,
                    mode: drag_mode,
                },
            ) {
                eprintln!("Unable to start the drag-out: {error:?}");
            }

            DRAG_OUT_IN_PROGRESS.store(false, Ordering::SeqCst);
        })
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

        Ok(())
    }

    pub(super) fn create_shortcuts_impl(
        sources: &[String],
        destination: &str,
    ) -> Result<Vec<String>, FileSystemError> {
        use std::iter::once;
        use windows::core::{ComInterface, PCWSTR};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        /// RAII COM apartment. `owned` records whether this call initialized
        /// COM: S_FALSE (0x1) means the thread already had an apartment, so
        /// dropping must not uninitialize it.
        struct CoApartment { owned: bool }
        impl CoApartment {
            fn enter() -> Result<Self, FileSystemError> {
                unsafe {
                    match CoInitializeEx(Some(std::ptr::null()), COINIT_APARTMENTTHREADED) {
                        Ok(()) => Ok(Self { owned: true }),
                        Err(error) if error.code().0 == 1 => Ok(Self { owned: false }),
                        Err(error) => Err(FileSystemError::Internal(error.to_string())),
                    }
                }
            }
        }
        impl Drop for CoApartment {
            fn drop(&mut self) {
                if self.owned {
                    unsafe { CoUninitialize() };
                }
            }
        }

        let _apartment = CoApartment::enter()?;

        let mut created = Vec::new();
        for source in sources {
            let link: IShellLinkW =
                unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            let source_wide: Vec<u16> = source.encode_utf16().chain(once(0)).collect();
            unsafe {
                link.SetPath(PCWSTR::from_raw(source_wide.as_ptr()))
                    .map_err(|error| FileSystemError::Internal(error.to_string()))?;
                // Explorer-style description so the shortcut hover tooltip
                // mirrors what the shell itself would have written.
                let description: Vec<u16> = source.encode_utf16().chain(once(0)).collect();
                link.SetDescription(PCWSTR::from_raw(description.as_ptr()))
                    .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            }

            let stem = std::path::Path::new(source)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| {
                    FileSystemError::InvalidInput(format!("Cannot derive a name from {source}"))
                })?;
            let link_path = unique_shortcut_path(destination, stem);

            let persist: IPersistFile = link.cast::<IPersistFile>().map_err(|error| {
                FileSystemError::Internal(error.to_string())
            })?;
            unsafe {
                persist
                    .Save(PCWSTR::from_raw(link_path.encode_utf16().collect::<Vec<u16>>().as_ptr()), true)
                    .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            }

            created.push(link_path);
        }

        Ok(created)
    }

    /// Builds `{destination}\{stem} - Shortcut.lnk`, appending ` (2)`, ` (3)`…
    /// while the name is taken — the same scheme Explorer uses.
    fn unique_shortcut_path(destination: &str, stem: &str) -> String {
        let separator = if destination.contains('\\') && !destination.contains('/') {
            '\\'
        } else {
            '/'
        };
        let base = if destination.ends_with('\\') || destination.ends_with('/') {
            destination.to_string()
        } else {
            format!("{destination}{separator}")
        };

        let mut candidate = format!("{base}{stem} - Shortcut.lnk");
        let mut counter = 2u32;
        while std::path::Path::new(&candidate).exists() {
            candidate = format!("{base}{stem} - Shortcut ({counter}).lnk");
            counter += 1;
        }
        candidate
    }
}

#[cfg(windows)]
use platform::{
    create_shortcuts_impl, read_files_from_clipboard_impl, start_drag_out_impl,
    write_files_to_clipboard_impl,
};

#[cfg(not(windows))]
fn write_files_to_clipboard_impl(_paths: &[String], _cut: bool) -> Result<(), FileSystemError> {
    Err(FileSystemError::Internal(
        "System clipboard file lists are only implemented on Windows".into(),
    ))
}

#[cfg(not(windows))]
fn read_files_from_clipboard_impl() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    Err(FileSystemError::Internal(
        "System clipboard file lists are only implemented on Windows".into(),
    ))
}

#[cfg(not(windows))]
fn start_drag_out_impl(
    _paths: Vec<String>,
    _mode: DragOutMode,
    _app: &tauri::AppHandle,
) -> Result<(), FileSystemError> {
    Err(FileSystemError::Internal(
        "Drag-out is only implemented on Windows".into(),
    ))
}

#[cfg(not(windows))]
fn create_shortcuts_impl(
    _sources: &[String],
    _destination: &str,
) -> Result<Vec<String>, FileSystemError> {
    Err(FileSystemError::Internal(
        "Shortcuts are only implemented on Windows".into(),
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::platform::{build_file_drop_handle, file_paths_from_drop_handle};
    use windows::Win32::UI::Shell::HDROP;

    #[test]
    fn file_drop_handle_round_trips() {
        let paths = vec![
            "C:\\temp\\report.txt".to_owned(),
            "C:\\temp\\folder two\\data.bin".to_owned(),
        ];
        let handle = build_file_drop_handle(&paths).expect("drop handle");

        let parsed = file_paths_from_drop_handle(HDROP(handle.0 as isize));

        assert_eq!(parsed, paths);
    }
}
