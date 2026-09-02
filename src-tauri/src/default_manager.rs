//! Registers and inspects dae as the operating system's default file manager
//! (the handler for folders / `inode/directory`).
//!
//! Each platform uses its native association mechanism:
//! - **macOS**: LaunchServices `LSSetDefaultRoleHandlerForContentType` for
//!   `public.folder`. Requires the bundle to advertise the folder document
//!   type (see `src-tauri/Info.plist`); unsetting is not exposed by the API.
//! - **Linux**: a generated `.desktop` entry registered through `xdg-mime`.
//! - **Windows**: HKCU ProgID + `Directory` shell/OpenWithProgids keys. Windows
//!   8+ protects the folder `UserChoice` with a per-user hash that cannot be
//!   forged safely, so we register everything and, when the user has not yet
//!   confirmed dae in the OS picker, report `is_registered` (not `is_default`)
//!   and let the UI route them to `ms-settings:defaultapps`.
//!
//! All commands run their blocking work on [`tauri::async_runtime::spawn_blocking`]
//! and update the cached flag in [`crate::settings`] on success.

use crate::file_system::error::FileSystemError;
use crate::settings;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Snapshot of the OS default-file-manager state for the settings UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DefaultFileManagerStatus {
    /// Whether this platform exposes any default-handler mechanism at all.
    pub supported: bool,
    /// The OS currently routes folder opens to dae.
    pub is_default: bool,
    /// dae is registered as a candidate but not yet the confirmed default
    /// (Windows: awaiting the user's choice in system settings).
    pub is_registered: bool,
    /// Optional i18n key / message describing a required manual step.
    pub detail: Option<String>,
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
impl DefaultFileManagerStatus {
    fn unsupported() -> Self {
        Self {
            supported: false,
            is_default: false,
            is_registered: false,
            detail: None,
        }
    }
}

/// Queries the live OS state.
#[tauri::command]
#[specta::specta]
pub async fn get_default_file_manager_status(
    app: tauri::AppHandle,
) -> Result<DefaultFileManagerStatus, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || platform::status(&app))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Registers dae as the default folder handler and returns the new state.
#[tauri::command]
#[specta::specta]
pub async fn set_default_file_manager(
    app: tauri::AppHandle,
) -> Result<DefaultFileManagerStatus, FileSystemError> {
    let status = tauri::async_runtime::spawn_blocking(move || platform::set(&app))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))??;
    // Cache only a confirmed default; a merely-registered state is not yet it.
    let _ = settings::set_default_file_manager_cache(status.is_default);
    Ok(status)
}

/// Removes dae's default-handler registration where the platform allows it.
#[tauri::command]
#[specta::specta]
pub async fn unset_default_file_manager(
    app: tauri::AppHandle,
) -> Result<DefaultFileManagerStatus, FileSystemError> {
    let status = tauri::async_runtime::spawn_blocking(move || platform::unset(&app))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))??;
    let _ = settings::set_default_file_manager_cache(status.is_default);
    Ok(status)
}

/// Resolves the running executable, used to build handler launch commands.
fn current_exe() -> Result<std::path::PathBuf, FileSystemError> {
    std::env::current_exe().map_err(|error| FileSystemError::Internal(error.to_string()))
}

mod platform {
    use super::{DefaultFileManagerStatus, FileSystemError};

    pub fn status(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::status(app);
        #[cfg(target_os = "linux")]
        return super::linux::status(app);
        #[cfg(windows)]
        return super::windows_impl::status(app);
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = app;
            Ok(DefaultFileManagerStatus::unsupported())
        }
    }

    pub fn set(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::set(app);
        #[cfg(target_os = "linux")]
        return super::linux::set(app);
        #[cfg(windows)]
        return super::windows_impl::set(app);
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = app;
            Ok(DefaultFileManagerStatus::unsupported())
        }
    }

    pub fn unset(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::unset(app);
        #[cfg(target_os = "linux")]
        return super::linux::unset(app);
        #[cfg(windows)]
        return super::windows_impl::unset(app);
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = app;
            Ok(DefaultFileManagerStatus::unsupported())
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use super::{DefaultFileManagerStatus, FileSystemError};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use tauri::Manager;

    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    const NO_ERR: i32 = 0;
    const FOLDER_UTI: &str = "public.folder";

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSSetDefaultRoleHandlerForContentType(
            in_content_type: CFStringRef,
            in_role: u32,
            in_handler_bundle_id: CFStringRef,
        ) -> i32;
        fn LSCopyDefaultRoleHandlerForContentType(
            in_content_type: CFStringRef,
            in_role: u32,
        ) -> CFStringRef;
    }

    fn bundle_id(app: &tauri::AppHandle) -> String {
        app.config().identifier.clone()
    }

    pub fn status(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let expected = bundle_id(app);
        let content_type = CFString::new(FOLDER_UTI);
        let handler = unsafe { LSCopyDefaultRoleHandlerForContentType(content_type.as_concrete_TypeRef(), ROLES_ALL) };
        let current = if handler.is_null() {
            None
        } else {
            // Create rule: take ownership so the string is released on drop.
            Some(unsafe { CFString::wrap_under_create_rule(handler) }.to_string())
        };
        let is_default = current.as_deref() == Some(expected.as_str());
        Ok(DefaultFileManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default,
            detail: None,
        })
    }

    pub fn set(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let expected = bundle_id(app);
        let content_type = CFString::new(FOLDER_UTI);
        let handler = CFString::new(&expected);
        let status_code = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                content_type.as_concrete_TypeRef(),
                ROLES_ALL,
                handler.as_concrete_TypeRef(),
            )
        };
        if status_code != NO_ERR {
            return Err(FileSystemError::Internal(
                "default_manager.set_failed".to_string(),
            ));
        }
        // Re-query rather than assume: recent macOS may silently decline.
        status(app)
    }

    pub fn unset(app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        // LaunchServices offers no programmatic "remove default"; the user must
        // pick another app in Finder's Get Info. Report the current state with
        // a detail key so the UI can explain and disable the Remove button.
        let mut current = status(app)?;
        current.detail = Some("default_manager.cannot_unset_macos".to_string());
        Ok(current)
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::{current_exe, DefaultFileManagerStatus, FileSystemError};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    const DESKTOP_ID: &str = "dae-folder.desktop";
    const MIME: &str = "inode/directory";

    fn applications_dir() -> PathBuf {
        if let Ok(data_home) = std::env::var("XDG_DATA_HOME")
            && !data_home.is_empty()
        {
            return PathBuf::from(data_home).join("applications");
        }
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".local/share/applications")
    }

    fn desktop_path() -> PathBuf {
        applications_dir().join(DESKTOP_ID)
    }

    fn mimeapps_path() -> PathBuf {
        if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME")
            && !config_home.is_empty()
        {
            return PathBuf::from(config_home).join("mimeapps.list");
        }
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".config/mimeapps.list")
    }

    pub fn status(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let output = Command::new("xdg-mime")
            .args(["query", "default", MIME])
            .output();
        let current = output
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default();
        let is_default = current == DESKTOP_ID;
        Ok(DefaultFileManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default || desktop_path().exists(),
            detail: None,
        })
    }

    pub fn set(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let exe = current_exe()?;
        let dir = applications_dir();
        fs::create_dir_all(&dir)?;

        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=dae\n\
             Comment=Aesthetic file manager\n\
             Exec=\"{}\" %f\n\
             Icon=dae\n\
             Terminal=false\n\
             Categories=FileManager;Utility;Core;\n\
             MimeType={};\n\
             StartupNotify=true\n",
            exe.to_string_lossy(),
            MIME
        );
        fs::write(desktop_path(), contents)?;

        let status = Command::new("xdg-mime")
            .args(["default", DESKTOP_ID, MIME])
            .status()
            .map_err(|error| FileSystemError::Internal(format!("default_manager.set_failed: {error}")))?;
        if !status.success() {
            return Err(FileSystemError::Internal("default_manager.set_failed".to_string()));
        }
        // Best-effort cache refresh; ignore failures.
        let _ = Command::new("update-desktop-database").arg(&dir).status();
        status_fn()
    }

    pub fn unset(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let _ = fs::remove_file(desktop_path());
        strip_mimeapps_entry();
        let _ = Command::new("update-desktop-database")
            .arg(applications_dir())
            .status();
        status_fn()
    }

    fn status_fn() -> Result<DefaultFileManagerStatus, FileSystemError> {
        // Reuse status without an app handle dependency.
        let output = Command::new("xdg-mime")
            .args(["query", "default", MIME])
            .output();
        let current = output
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default();
        let is_default = current == DESKTOP_ID;
        Ok(DefaultFileManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default || desktop_path().exists(),
            detail: None,
        })
    }

    /// Removes the `inode/directory=dae-folder.desktop` line from mimeapps.list.
    fn strip_mimeapps_entry() {
        let path = mimeapps_path();
        let Ok(existing) = fs::read_to_string(&path) else {
            return;
        };
        let needle = format!("{MIME}={DESKTOP_ID}");
        let filtered: Vec<&str> = existing
            .lines()
            .filter(|line| line.trim() != needle)
            .collect();
        let _ = fs::write(&path, filtered.join("\n") + "\n");
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod windows_impl {
    use super::{current_exe, DefaultFileManagerStatus, FileSystemError};
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_NONE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    const PROGID: &str = "dae.folder";
    const CLASSES_DIR_PROGID: &str = r"Software\Classes\Directory";
    const USERCHOICE_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\Directory\UserChoice";

    /// Encodes a Rust string as a null-terminated UTF-16 vector for PCWSTR.
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Encodes a Rust string as null-terminated UTF-16 little-endian bytes,
    /// the on-disk layout of a REG_SZ value.
    fn wide_bytes(value: &str) -> Vec<u8> {
        wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect()
    }

    /// Creates (or opens) `subkey` under HKCU and returns its handle. The
    /// caller must close it with [`close_key`].
    fn create_key(subkey: &str) -> Result<HKEY, FileSystemError> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` outlives the call and is null-terminated; `handle` is
        // a valid out-pointer. The registry functions are FFI, hence unsafe.
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut handle,
                None,
            )
        }
        .map_err(|error| {
            FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
        })?;
        Ok(handle)
    }

    fn close_key(handle: HKEY) {
        // SAFETY: `handle` came from create/open_key and is closed exactly once.
        unsafe {
            let _ = RegCloseKey(handle);
        }
    }

    /// Sets a REG_SZ value, or a value-name-only REG_NONE marker when `data` is
    /// empty (how OpenWithProgids entries are recorded).
    fn set_value(handle: HKEY, name: &str, data: &str) -> Result<(), FileSystemError> {
        let name_w = wide(name);
        let value_name = if name.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(name_w.as_ptr())
        };

        // SAFETY: `name_w` outlives the call; `value_name` points at it (or is
        // null). `data_bytes` is a valid slice for the REG_SZ branch.
        let result = if data.is_empty() {
            unsafe { RegSetValueExW(handle, value_name, 0, REG_NONE, None) }
        } else {
            let data_bytes = wide_bytes(data);
            unsafe { RegSetValueExW(handle, value_name, 0, REG_SZ, Some(&data_bytes)) }
        };
        result.map_err(|error| {
            FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
        })
    }

    /// Creates `subkey` and sets its `(Default)` value in one step.
    fn write_default(subkey: &str, data: &str) -> Result<(), FileSystemError> {
        let handle = create_key(subkey)?;
        let result = set_value(handle, "", data);
        close_key(handle);
        result
    }

    /// Reads a string value; `name` empty reads the `(Default)` value.
    fn read_string(subkey: &str, name: &str) -> Option<String> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_READ, &mut handle) };
        if opened.is_err() {
            return None;
        }
        let name_w = wide(name);
        let value_name = if name.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(name_w.as_ptr())
        };
        let mut size: u32 = 0;
        let mut kind = REG_SZ;
        // SAFETY: `name_w` outlives the call; `kind`/`size` are valid
        // out-pointers; `lpdata` is null on the size probe.
        let probe = unsafe {
            RegQueryValueExW(handle, value_name, None, Some(&mut kind), None, Some(&mut size))
        };
        if probe.is_err() || size == 0 {
            close_key(handle);
            return None;
        }
        let mut buffer: Vec<u8> = vec![0; size as usize];
        // SAFETY: `buffer` has `size` bytes of capacity for the out-write; the
        // other pointers are as above.
        let read = unsafe {
            RegQueryValueExW(
                handle,
                value_name,
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        close_key(handle);
        if read.is_err() {
            return None;
        }
        // `size` now reflects bytes actually written; drop any slack.
        buffer.truncate(size as usize);
        // REG_SZ is UTF-16LE; strip one trailing null terminator if present.
        let mut as_u16: Vec<u16> = buffer
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if as_u16.last() == Some(&0) {
            as_u16.pop();
        }
        Some(String::from_utf16_lossy(&as_u16))
    }

    fn value_exists(subkey: &str, name: &str) -> bool {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_READ, &mut handle) };
        if opened.is_err() {
            return false;
        }
        let name_w = wide(name);
        let value_name = if name.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(name_w.as_ptr())
        };
        let mut size: u32 = 0;
        // SAFETY: `name_w` outlives the call; `size` is a valid out-pointer;
        // only the size is probed (type/data null).
        let probe = unsafe { RegQueryValueExW(handle, value_name, None, None, None, Some(&mut size)) };
        close_key(handle);
        probe.is_ok()
    }

    fn delete_tree(subkey: &str) {
        let path = wide(subkey);
        // SAFETY: `path` is null-terminated and outlives the call.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()));
        }
    }

    fn delete_value(subkey: &str, name: &str) {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_WRITE, &mut handle) };
        if opened.is_err() {
            return;
        }
        let name_w = wide(name);
        // SAFETY: `name_w` is null-terminated and outlives the call.
        unsafe {
            let _ = RegDeleteValueW(handle, PCWSTR(name_w.as_ptr()));
        }
        close_key(handle);
    }

    fn notify_shell() {
        // SAFETY: SHChangeNotify with an association-changed event takes no
        // item pointers (both null) and returns nothing.
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }

    pub fn status(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let user_choice = read_string(USERCHOICE_KEY, "ProgId");
        let is_default = user_choice.as_deref() == Some(PROGID);
        let is_registered =
            is_default || value_exists(&format!("{CLASSES_DIR_PROGID}\\OpenWithProgids"), PROGID);
        let detail = if is_registered && !is_default {
            Some("default_manager.finish_in_system_settings".to_string())
        } else {
            None
        };
        Ok(DefaultFileManagerStatus {
            supported: true,
            is_default,
            is_registered,
            detail,
        })
    }

    pub fn set(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        let exe = current_exe()?;
        let exe_str = exe.to_string_lossy().into_owned();
        let launch = format!("\"{exe_str}\" \"%1\"");
        let icon = format!("\"{exe_str}\",0");

        // ProgID definition.
        write_default(&format!(r"Software\Classes\{PROGID}"), "dae")?;
        write_default(&format!(r"Software\Classes\{PROGID}\DefaultIcon"), &icon)?;
        write_default(
            &format!(r"Software\Classes\{PROGID}\shell\open\command"),
            &launch,
        )?;

        // Make dae appear in Explorer's "Open with" and the default-apps picker.
        let open_with = format!("{CLASSES_DIR_PROGID}\\OpenWithProgids");
        let handle = create_key(&open_with)?;
        set_value(handle, PROGID, "")?;
        close_key(handle);

        // Immediate right-click "Open with dae" entry (works without UserChoice).
        write_default(
            &format!("{CLASSES_DIR_PROGID}\\shell\\dae"),
            "Open with dae",
        )?;
        write_default(
            &format!("{CLASSES_DIR_PROGID}\\shell\\dae\\command"),
            &launch,
        )?;

        notify_shell();
        status(_app)
    }

    pub fn unset(_app: &tauri::AppHandle) -> Result<DefaultFileManagerStatus, FileSystemError> {
        delete_tree(&format!(r"Software\Classes\{PROGID}"));
        delete_tree(&format!("{CLASSES_DIR_PROGID}\\shell\\dae"));
        delete_value(&format!("{CLASSES_DIR_PROGID}\\OpenWithProgids"), PROGID);
        notify_shell();
        status(_app)
    }
}
