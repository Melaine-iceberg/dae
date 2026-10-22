//! OS-level file integration that `dae-core` does not own: the system
//! clipboard's file list, native drag-out, shortcuts, held modifier keys, the
//! external terminal, and the Windows package identity probe.
//!
//! The clipboard and modifier backends are Windows-only, matching the Tauri
//! shell: there is no cross-platform crate that reads a file list off the
//! clipboard or queries global key state with the same fidelity. Shortcuts and
//! the external terminal have native implementations on every desktop.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::vfs;
use serde::Serialize;
use std::path::PathBuf;

/// File paths currently held by the system clipboard, with Explorer's
/// "Preferred DropEffect" marker resolved into a copy/cut flag.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemClipboardFiles {
    pub paths: Vec<String>,
    pub cut: bool,
}

/// The modifier keys held down right now, named like the DOM keyboard events
/// the webview never receives during a native drag-and-drop.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyModifiers {
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub meta_key: bool,
    pub shift_key: bool,
}

// ---------------------------------------------------------------------------
// System clipboard
// ---------------------------------------------------------------------------

pub fn write_files_to_clipboard(paths: Vec<String>, cut: bool) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before copying".into(),
        ));
    }

    write_files_to_clipboard_impl(&paths, cut)
}

pub fn read_files_from_clipboard() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    read_files_from_clipboard_impl()
}

// ---------------------------------------------------------------------------
// Modifier keys
// ---------------------------------------------------------------------------

pub fn get_key_modifiers() -> KeyModifiers {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        };

        // `GetAsyncKeyState` returns the high bit for "down right now" and the
        // low bit for "pressed since the last call" — only the former counts.
        fn held(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
            (unsafe { GetAsyncKeyState(key.0 as i32) }) < 0
        }

        KeyModifiers {
            alt_key: held(VK_MENU),
            ctrl_key: held(VK_CONTROL),
            meta_key: held(VK_LWIN) || held(VK_RWIN),
            shift_key: held(VK_SHIFT),
        }
    }
    #[cfg(not(windows))]
    {
        KeyModifiers::default()
    }
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// Creates links pointing at `sources` inside `destination`: `.lnk` shortcuts
/// on Windows (IShellLink), real symlinks on macOS and Linux. Name collisions
/// get ` (2)`, ` (3)`… suffixes like Explorer, and the created paths come back
/// so the UI can refresh and select them.
pub fn create_shortcuts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<String>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry to link".into(),
        ));
    }

    create_shortcuts_impl(&sources, &destination)
}

// ---------------------------------------------------------------------------
// External terminal
// ---------------------------------------------------------------------------

/// Opens the user's system default terminal at a local directory.
pub fn open_terminal(path: String) -> Result<(), FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.terminal_local_dir_only".into(),
        ));
    }

    let directory = PathBuf::from(&path);
    if !directory.is_dir() {
        return Err(FileSystemError::NotDirectory(path));
    }

    open_system_terminal(&directory)
}

// ---------------------------------------------------------------------------
// Package identity
// ---------------------------------------------------------------------------

/// The Windows package family name this process runs under, or the reason it
/// has none. Only identity-gated APIs (notifications, on-device AI, …) need
/// the packaged form; a plain development launch answers with a reason string.
pub fn package_family_name() -> String {
    #[cfg(target_os = "windows")]
    {
        use windows::ApplicationModel::Package;
        match Package::Current() {
            Ok(package) => match package.Id() {
                Ok(id) => match id.FamilyName() {
                    Ok(name) => name.to_string(),
                    Err(_) => "Error retrieving Family Name".to_string(),
                },
                Err(_) => "Error retrieving Package ID".to_string(),
            },
            Err(_) => "No package identity".to_string(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        "Not running on Windows".to_string()
    }
}

// ---------------------------------------------------------------------------
// Native drag-out
// ---------------------------------------------------------------------------

/// Starts a native drag carrying the given files out of the window. On Windows
/// this runs OLE's modal drag loop and must be called on the Electron main
/// thread; macOS and Linux use Electron's `startDrag` in the main process
/// instead.
#[cfg(windows)]
pub fn start_drag_out(paths: Vec<String>, mode: &str, hwnd: isize) -> Result<(), FileSystemError> {
    use raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    };
    use std::num::NonZeroIsize;
    use std::sync::atomic::Ordering;

    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before dragging".into(),
        ));
    }

    if DRAG_OUT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err(FileSystemError::Internal(
            "A drag-out is already in progress".into(),
        ));
    }
    let _guard = DragOutGuard;

    /// Drag preview shown by the OS while dragging out of the window.
    const DRAG_PREVIEW_ICON: &[u8] = include_bytes!("../../src-tauri/icons/32x32.png");

    struct NativeWindow(isize);

    impl HasWindowHandle for NativeWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let hwnd = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
            // SAFETY: the HWND belongs to the sender window, which outlives the
            // modal drag loop this handle is borrowed for.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    let drag_mode = match mode {
        "copy" => drag::DragMode::Copy,
        "move" => drag::DragMode::Move,
        "link" => drag::DragMode::Link,
        _ => drag::DragMode::Any,
    };

    drag::start_drag(
        &NativeWindow(hwnd),
        drag::DragItem::Files(paths.into_iter().map(PathBuf::from).collect()),
        drag::Image::Raw(DRAG_PREVIEW_ICON.to_vec()),
        |_, _| (),
        drag::Options {
            skip_animatation_on_cancel_or_failure: false,
            mode: drag_mode,
            drag_image_offset: None,
        },
    )
    .map_err(|error| FileSystemError::Internal(format!("fs.drag_out_failed: {error}")))
}

#[cfg(windows)]
static DRAG_OUT_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
struct DragOutGuard;

#[cfg(windows)]
impl Drop for DragOutGuard {
    fn drop(&mut self) {
        DRAG_OUT_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(not(windows))]
pub fn start_drag_out(
    _paths: Vec<String>,
    _mode: &str,
    _hwnd: isize,
) -> Result<(), FileSystemError> {
    Err(FileSystemError::Unsupported(
        "fs.drag_out_unsupported".into(),
    ))
}

// ---------------------------------------------------------------------------
// Windows backends
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use super::SystemClipboardFiles;
    use dae_core::file_system::error::FileSystemError;
    use std::time::Duration;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
        SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE};
    use windows::Win32::UI::Shell::{DROPFILES, DragFinish, DragQueryFileW, HDROP};
    use windows::core::{BOOL, HRESULT, PCWSTR};

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

    fn alloc_filled(size: usize, fill: impl FnOnce(*mut u8)) -> Result<HGLOBAL, FileSystemError> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) }
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

    pub(super) fn write_files_to_clipboard(
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
                let _ = GlobalFree(Some(files));
                let _ = GlobalFree(Some(effect));
            }
            return session.map(|_| ());
        }

        unsafe {
            if let Err(error) = EmptyClipboard() {
                return Err(FileSystemError::Internal(error.to_string()));
            }

            // Once SetClipboardData succeeds the system owns the handles.
            if let Err(error) = SetClipboardData(CF_HDROP.0 as u32, Some(handle_from_global(files)))
            {
                let _ = GlobalFree(Some(files));
                let _ = GlobalFree(Some(effect));
                return Err(FileSystemError::Internal(error.to_string()));
            }

            let format = preferred_drop_effect_format();
            if format != 0
                && let Err(error) = SetClipboardData(format, Some(handle_from_global(effect)))
            {
                // The file list is already on the clipboard; a missing cut
                // marker only downgrades the paste to a copy.
                let _ = GlobalFree(Some(effect));
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

    pub(super) fn create_shortcuts(
        sources: &[String],
        destination: &str,
    ) -> Result<Vec<String>, FileSystemError> {
        use std::iter::once;
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        use windows::core::{Interface, PCWSTR};

        struct CoApartment {
            owned: bool,
        }
        impl CoApartment {
            fn enter() -> Result<Self, FileSystemError> {
                let result: HRESULT =
                    unsafe { CoInitializeEx(Some(std::ptr::null()), COINIT_APARTMENTTHREADED) };
                match result.0 {
                    0 => Ok(Self { owned: true }),
                    1 => Ok(Self { owned: false }),
                    _ => Err(FileSystemError::Internal(format!(
                        "CoInitializeEx failed with HRESULT {:#010x}",
                        result.0
                    ))),
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

            let persist: IPersistFile = link
                .cast::<IPersistFile>()
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            let link_wide: Vec<u16> = link_path.encode_utf16().chain(once(0)).collect();
            unsafe {
                persist
                    .Save(PCWSTR::from_raw(link_wide.as_ptr()), true)
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
    create_shortcuts as create_shortcuts_impl,
    read_files_from_clipboard as read_files_from_clipboard_impl,
    write_files_to_clipboard as write_files_to_clipboard_impl,
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
fn create_shortcuts_impl(
    sources: &[String],
    destination: &str,
) -> Result<Vec<String>, FileSystemError> {
    use std::os::unix::fs::symlink;

    let mut created = Vec::new();
    for source in sources {
        let source_path = std::path::Path::new(source);
        let name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                FileSystemError::InvalidInput(format!("Cannot derive a name from {source}"))
            })?;
        let link_path = unique_symlink_path(destination, name);

        // The target stays absolute: a relative symlink would resolve against
        // the link's own folder and break the moment the link is moved.
        symlink(source_path, &link_path)?;
        created.push(link_path);
    }

    Ok(created)
}

#[cfg(not(windows))]
fn unique_symlink_path(destination: &str, name: &str) -> String {
    let directory = std::path::Path::new(destination);
    let mut link_name = name.to_owned();
    let mut candidate = directory.join(&link_name);
    let mut counter = 2u32;

    while candidate.exists() {
        let source_name = std::path::Path::new(name);
        let stem = source_name
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name);
        link_name = match source_name
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some(extension) => format!("{stem} ({counter}).{extension}"),
            None => format!("{stem} ({counter})"),
        };
        candidate = directory.join(&link_name);
        counter += 1;
    }

    candidate.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// External terminal
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    // Prefer Windows Terminal: `wt -d` opens the user's configured default
    // profile (their chosen shell) at the directory.
    if std::process::Command::new("wt.exe")
        .arg("-d")
        .arg(directory)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // Fallback for systems without Windows Terminal: launch PowerShell in a
    // new console, which Windows routes to the configured default terminal
    // host (e.g. Windows Terminal or conhost).
    std::process::Command::new("powershell.exe")
        .current_dir(directory)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
        .map_err(|error| FileSystemError::Internal(format!("fs.terminal_launch_failed: {error}")))
}

#[cfg(target_os = "macos")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    std::process::Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| FileSystemError::Internal(format!("fs.terminal_launch_failed: {error}")))
}

#[cfg(target_os = "linux")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    use std::process::Command;

    fn spawn(
        program: &str,
        extra_args: &[&str],
        directory: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        let mut command = Command::new(program);
        command.args(extra_args).current_dir(directory).spawn()?;
        Ok(())
    }

    if let Ok(terminal) = std::env::var("TERMINAL") {
        let terminal = terminal.trim();
        if !terminal.is_empty() && spawn(terminal, &[], directory).is_ok() {
            return Ok(());
        }
    }

    let dir = directory.to_string_lossy().into_owned();
    let candidates: Vec<(String, Vec<String>)> = vec![
        ("xdg-terminal-exec".into(), vec![]),
        ("x-terminal-emulator".into(), vec![]),
        (
            "gnome-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "kgx".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        ("konsole".into(), vec!["--workdir".into(), dir.clone()]),
        (
            "xfce4-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "mate-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "tilix".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "wezterm".into(),
            vec!["start".into(), "--cwd".into(), dir.clone()],
        ),
        ("kitty".into(), vec![]),
        ("alacritty".into(), vec![]),
        ("foot".into(), vec![]),
        ("xterm".into(), vec![]),
    ];

    for (program, args) in candidates {
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        if spawn(&program, &arg_refs, directory).is_ok() {
            return Ok(());
        }
    }

    Err(FileSystemError::Internal("fs.terminal_not_found".into()))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn open_system_terminal(_directory: &std::path::Path) -> Result<(), FileSystemError> {
    Err(FileSystemError::Unsupported(
        "fs.terminal_unsupported_platform".into(),
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

        let parsed = file_paths_from_drop_handle(HDROP(handle.0));

        assert_eq!(parsed, paths);
    }

    #[test]
    fn creates_explorer_style_shortcuts() {
        let root = std::env::temp_dir().join(format!("dae-shortcut-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let destination = root.join("destination");
        std::fs::create_dir_all(&destination).expect("create destination");
        let target = root.join("notes.txt");
        std::fs::write(&target, b"linked").expect("write target");

        let sources = vec![target.to_string_lossy().into_owned()];
        let first = super::create_shortcuts_impl(&sources, &destination.to_string_lossy())
            .expect("create the first shortcut");
        let second = super::create_shortcuts_impl(&sources, &destination.to_string_lossy())
            .expect("create the second shortcut");

        assert!(first[0].ends_with("notes - Shortcut.lnk"));
        assert!(second[0].ends_with("notes - Shortcut (2).lnk"));
        assert!(std::path::Path::new(&first[0]).is_file());
        assert!(std::path::Path::new(&second[0]).is_file());

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{create_shortcuts_impl, unique_symlink_path};

    fn test_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("dae-link-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create test root");
        root
    }

    #[test]
    fn unique_symlink_path_avoids_collisions() {
        let root = test_root("collision");

        let first = unique_symlink_path(root.to_str().unwrap(), "report.txt");
        assert_eq!(first, root.join("report.txt").to_string_lossy());

        std::fs::write(&first, b"taken").expect("occupy the name");
        let second = unique_symlink_path(root.to_str().unwrap(), "report.txt");
        assert_eq!(second, root.join("report (2).txt").to_string_lossy());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn create_shortcuts_creates_working_symlinks() {
        let root = test_root("create");
        let destination = root.join("destination");
        std::fs::create_dir_all(&destination).expect("create destination");
        let target = root.join("notes.txt");
        std::fs::write(&target, "linked").expect("write target");

        let created = create_shortcuts_impl(
            &[target.to_string_lossy().into_owned()],
            &destination.to_string_lossy(),
        )
        .expect("create the symlink");

        assert_eq!(created.len(), 1);
        let link = std::path::Path::new(&created[0]);
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(link).unwrap(), target);
        assert_eq!(std::fs::read_to_string(link).unwrap(), "linked");

        let _ = std::fs::remove_dir_all(&root);
    }
}
