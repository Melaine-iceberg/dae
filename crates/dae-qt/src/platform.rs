//! OS integration the Qt shell needs but Qt itself does not expose.
//!
//! Three questions live here: hand a path to its default handler, show a path
//! selected in the file manager, and read the user's accent colour. They are
//! the same platform calls the Tauri shell made through plugins, written
//! against the OS rather than a webview so the Qt shell can use them before a
//! window exists.

use std::path::Path;

/// Shows the operating system's own properties sheet for `path`.
///
/// Windows has one, and there it is the truthful answer: the same sheet
/// Explorer and the Save dialogs show, including the Security, Details and
/// Previous Versions pages dae would otherwise have to reimplement. Every
/// other platform answers `false`, and the caller keeps the in-app dialog.
///
/// The sheet is shown modeless, so this returns before the user closes it. A
/// failure — a path the shell has no property handler for, or a remote one
/// that slipped through — returns `false` and the caller falls back to the
/// in-app dialog.
pub fn show_properties(path: &str) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        use windows::Win32::UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SW_SHOWNORMAL};
        use windows::core::{HSTRING, PCWSTR};

        // The sheet is a shell COM object and the "properties" verb is
        // dispatched through the context-menu handler, both of which need an
        // STA on the calling thread. Qt initializes OLE for drag-and-drop, but
        // not necessarily before the first properties call.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

        let verb = HSTRING::from("properties");
        let file = HSTRING::from(path);
        // The sheet is owned by the window the command came from. This runs on
        // the GUI thread right after the user picked the menu row, so the
        // foreground window is that window.
        let hwnd = unsafe { GetForegroundWindow() };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            // `INVOKEIDLIST` is what routes the verb through the shell's own
            // context menu, which is where the modeless property sheet lives.
            fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_FLAG_NO_UI,
            hwnd,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: `info` is fully initialized, the strings outlive the call,
        // and the shell copies what it keeps.
        unsafe { ShellExecuteExW(&mut info) }.is_ok()
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Opens `path` with the handler the OS would use for its type.
pub fn open_path(path: &str) -> Result<(), String> {
    let target = Path::new(path);
    if !target.exists() {
        return Err("找不到该项目".to_owned());
    }

    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, PCWSTR};

        let operation = HSTRING::from("open");
        let file = HSTRING::from(path);
        // SAFETY: both HSTRINGs outlive the call and are valid null-terminated
        // wide strings; the other parameters are explicitly null.
        let result = unsafe {
            ShellExecuteW(
                None,
                &operation,
                &file,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success as a value greater than 32.
        if result.0 as isize <= 32 {
            return Err(format!("系统无法打开 {path}"));
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = target;
        Err("this platform has no default handler".to_owned())
    }
}

/// Shows `path` selected in the platform file manager.
pub fn reveal_path(path: &str) -> Result<(), String> {
    let target = Path::new(path);
    if !target.exists() {
        return Err("找不到该项目".to_owned());
    }

    #[cfg(windows)]
    {
        // A file is selected in its parent; a directory is opened directly.
        let mut command = std::process::Command::new("explorer.exe");
        if target.is_dir() {
            command.arg(path);
        } else {
            command.arg("/select,").arg(path);
        }
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let directory = if target.is_dir() {
            target
        } else {
            target.parent().unwrap_or(target)
        };
        std::process::Command::new("xdg-open")
            .arg(directory)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = target;
        Err("this platform has no file manager".to_owned())
    }
}

/// The OS accent colour as `#rrggbb`, or the shell's default blue where the
/// platform publishes none.
pub fn accent_hex() -> String {
    accent().unwrap_or_else(|| "#2f6fdf".to_owned())
}

/// The OS accent colour, or `None` where the platform publishes none. The
/// native watcher re-reads through here so startup and every later change
/// share one decoder.
pub(crate) fn accent() -> Option<String> {
    platform_accent()
}

#[cfg(windows)]
fn platform_accent() -> Option<String> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";
    const ACCENT_VALUE: &str = "AccentColor";
    const COLORIZATION_VALUE: &str = "ColorizationColor";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_dword(name: &str) -> Option<u32> {
        let key_path = wide(DWM_KEY);
        let mut key = HKEY::default();
        // SAFETY: key_path is null-terminated and outlives the call; key is a
        // valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut key,
            )
        };
        if opened.is_err() {
            return None;
        }

        let name = wide(name);
        let mut kind = REG_DWORD;
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: name outlives the call; the out-pointers are valid and the
        // buffer is exactly one DWORD, which `size` announces.
        let read = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(&mut value as *mut u32 as *mut u8),
                Some(&mut size),
            )
        };
        let _ = unsafe { RegCloseKey(key) };
        (read.is_ok() && kind == REG_DWORD && size >= 4).then_some(value)
    }

    // `AccentColor` is 0xAABBGGRR (a COLORREF); `ColorizationColor` is
    // 0xAARRGGBB. Both are present on a normal Windows 10/11 install, and the
    // first is the one the Settings page writes.
    read_dword(ACCENT_VALUE)
        .and_then(from_abgr)
        .or_else(|| read_dword(COLORIZATION_VALUE).and_then(from_argb))
}

#[cfg(not(windows))]
fn platform_accent() -> Option<String> {
    None
}

#[cfg_attr(not(windows), allow(dead_code))]
fn srgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Unpacks `0xAABBGGRR`, the layout `HKCU\...\DWM!AccentColor` uses.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_abgr(packed: u32) -> Option<String> {
    (packed != 0).then(|| srgb_hex(packed as u8, (packed >> 8) as u8, (packed >> 16) as u8))
}

/// Unpacks `0xAARRGGBB`, the layout `ColorizationColor` uses.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_argb(packed: u32) -> Option<String> {
    (packed != 0).then(|| srgb_hex((packed >> 16) as u8, (packed >> 8) as u8, packed as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two neighbouring registry values decode to the same colour; the
    /// byte orders are easy to swap and the failure (orange instead of blue)
    /// is invisible in a test that only checks the shape.
    #[test]
    fn the_two_accent_layouts_agree() {
        assert_eq!(from_abgr(0xffd47800).as_deref(), Some("#0078d4"));
        assert_eq!(from_argb(0xc40078d4).as_deref(), Some("#0078d4"));
    }

    #[test]
    fn a_zero_accent_is_no_reading() {
        assert_eq!(from_abgr(0), None);
        assert_eq!(from_argb(0), None);
    }
}
