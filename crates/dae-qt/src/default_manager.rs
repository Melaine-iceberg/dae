//! Registers and inspects dae as the operating system's default file manager
//! (the handler for folders / `inode/directory`).
//!
//! Each platform uses its native association mechanism:
//! - **macOS**: LaunchServices `LSSetDefaultRoleHandlerForContentType` for
//!   `public.folder`. The bundle has to advertise the folder document type
//!   (see `packaging/qt/Info.plist`); LaunchServices exposes no unset.
//! - **Linux**: a generated `.desktop` entry registered through `xdg-mime`.
//! - **Windows**: HKCU ProgID plus `Directory` shell/OpenWithProgids keys.
//!   Windows 8+ protects the folder `UserChoice` with a per-user hash that
//!   cannot be forged safely, so we register everything and report
//!   `is_registered` (not `is_default`) until the user confirms dae in the
//!   system default-apps page.
//!
//! This is the Qt shell's port of `src-tauri/src/default_manager.rs`; both
//! shells use the same registry keys and desktop-entry id, so switching shells
//! does not leave two competing registrations.

use dae_core::FileSystemError;
use std::path::PathBuf;

/// Snapshot of the OS default-file-manager state for the settings UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefaultManagerStatus {
    /// Whether this platform exposes any default-handler mechanism at all.
    pub supported: bool,
    /// The OS currently routes folder opens to dae.
    pub is_default: bool,
    /// dae is registered as a candidate but not yet the confirmed default.
    pub is_registered: bool,
    /// Stable code for a required manual step, translated for the panel.
    pub detail: Option<String>,
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
impl DefaultManagerStatus {
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
pub(crate) fn status() -> Result<DefaultManagerStatus, FileSystemError> {
    platform::status()
}

/// Registers dae as the folder handler and returns the new state.
pub(crate) fn set() -> Result<DefaultManagerStatus, FileSystemError> {
    platform::set()
}

/// Removes dae's registration where the platform allows it.
pub(crate) fn unset() -> Result<DefaultManagerStatus, FileSystemError> {
    platform::unset()
}

/// Resolves the running executable, used to build handler launch commands.
fn current_exe() -> Result<PathBuf, FileSystemError> {
    std::env::current_exe().map_err(|error| {
        FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
    })
}

mod platform {
    use super::{DefaultManagerStatus, FileSystemError};

    pub fn status() -> Result<DefaultManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::status();
        #[cfg(target_os = "linux")]
        return super::linux::status();
        #[cfg(windows)]
        return super::windows_impl::status();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            Ok(DefaultManagerStatus::unsupported())
        }
    }

    pub fn set() -> Result<DefaultManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::set();
        #[cfg(target_os = "linux")]
        return super::linux::set();
        #[cfg(windows)]
        return super::windows_impl::set();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            Ok(DefaultManagerStatus::unsupported())
        }
    }

    pub fn unset() -> Result<DefaultManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::unset();
        #[cfg(target_os = "linux")]
        return super::linux::unset();
        #[cfg(windows)]
        return super::windows_impl::unset();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            Ok(DefaultManagerStatus::unsupported())
        }
    }
}

/// Linux desktop-entry text. Kept free of OS calls so its shape is testable on
/// every host.
#[cfg(any(target_os = "linux", test))]
mod desktop {
    pub(super) const DESKTOP_ID: &str = "dae-folder.desktop";
    pub(super) const MIME: &str = "inode/directory";

    pub(super) fn contents(exe: &str) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=dae\n\
             Comment=Aesthetic file manager\n\
             Exec=\"{exe}\" %f\n\
             Icon=dae\n\
             Terminal=false\n\
             Categories=FileManager;Utility;Core;\n\
             MimeType={MIME};\n\
             StartupNotify=true\n"
        )
    }

    /// Removes only the exact `inode/directory=dae-folder.desktop` entry, so
    /// other folders' defaults in `mimeapps.list` survive an unset.
    pub(super) fn strip_mimeapps_entry(existing: &str) -> String {
        let needle = format!("{MIME}={DESKTOP_ID}");
        let filtered: Vec<&str> = existing
            .lines()
            .filter(|line| line.trim() != needle)
            .collect();
        filtered.join("\n") + "\n"
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{DefaultManagerStatus, FileSystemError};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

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

    fn bundle_id() -> &'static str {
        dae_core::paths::APP_IDENTIFIER
    }

    pub fn status() -> Result<DefaultManagerStatus, FileSystemError> {
        let content_type = CFString::new(FOLDER_UTI);
        // SAFETY: the content type outlives the call; a non-null result is a
        // create-rule reference that `wrap_under_create_rule` takes ownership
        // of.
        let handler = unsafe {
            LSCopyDefaultRoleHandlerForContentType(content_type.as_concrete_TypeRef(), ROLES_ALL)
        };
        let current = if handler.is_null() {
            None
        } else {
            Some(unsafe { CFString::wrap_under_create_rule(handler) }.to_string())
        };
        let is_default = current.as_deref() == Some(bundle_id());
        Ok(DefaultManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default,
            detail: None,
        })
    }

    pub fn set() -> Result<DefaultManagerStatus, FileSystemError> {
        let content_type = CFString::new(FOLDER_UTI);
        let handler = CFString::new(bundle_id());
        // SAFETY: both Core Foundation strings outlive the call.
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
        // Recent macOS versions may silently decline, so re-query instead of
        // assuming the call took effect.
        status()
    }

    pub fn unset() -> Result<DefaultManagerStatus, FileSystemError> {
        let mut current = status()?;
        current.detail = Some("default_manager.cannot_unset_macos".to_string());
        Ok(current)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::desktop::{self, DESKTOP_ID, MIME};
    use super::{DefaultManagerStatus, FileSystemError, current_exe};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

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

    fn query_default() -> String {
        Command::new("xdg-mime")
            .args(["query", "default", MIME])
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default()
    }

    fn status_now() -> DefaultManagerStatus {
        let is_default = query_default() == DESKTOP_ID;
        DefaultManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default || desktop_path().exists(),
            detail: None,
        }
    }

    pub fn status() -> Result<DefaultManagerStatus, FileSystemError> {
        Ok(status_now())
    }

    pub fn set() -> Result<DefaultManagerStatus, FileSystemError> {
        let exe = current_exe()?;
        let dir = applications_dir();
        fs::create_dir_all(&dir).map_err(|error| {
            FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
        })?;
        fs::write(desktop_path(), desktop::contents(&exe.to_string_lossy())).map_err(|error| {
            FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
        })?;

        let registered = Command::new("xdg-mime")
            .args(["default", DESKTOP_ID, MIME])
            .status()
            .map_err(|error| {
                FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
            })?;
        if !registered.success() {
            return Err(FileSystemError::Internal(
                "default_manager.set_failed".to_string(),
            ));
        }
        // Best-effort cache refresh; a missing tool is not a failed set.
        let _ = Command::new("update-desktop-database").arg(&dir).status();
        Ok(status_now())
    }

    pub fn unset() -> Result<DefaultManagerStatus, FileSystemError> {
        let _ = fs::remove_file(desktop_path());
        strip_mimeapps_entry();
        let _ = Command::new("update-desktop-database")
            .arg(applications_dir())
            .status();
        Ok(status_now())
    }

    fn strip_mimeapps_entry() {
        let path = mimeapps_path();
        let Ok(existing) = fs::read_to_string(&path) else {
            return;
        };
        let _ = fs::write(&path, desktop::strip_mimeapps_entry(&existing));
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::{DefaultManagerStatus, FileSystemError, current_exe};
    use std::path::Path;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_NONE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows::core::PCWSTR;

    const PROGID: &str = "dae.folder";
    const CLASSES_DIR_PROGID: &str = r"Software\Classes\Directory";
    const USERCHOICE_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\Directory\UserChoice";
    const FINISH_IN_SETTINGS: &str = "default_manager.finish_in_system_settings";

    fn launch_command(exe: &Path) -> String {
        format!("\"{}\" \"%1\"", exe.to_string_lossy())
    }

    fn icon_value(exe: &Path) -> String {
        format!("\"{}\",0", exe.to_string_lossy())
    }

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
        // a valid out-pointer.
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut handle,
                None,
            )
        };
        if created.is_err() {
            return Err(FileSystemError::Internal(format!(
                "default_manager.set_failed: {}",
                created.0
            )));
        }
        Ok(handle)
    }

    fn close_key(handle: HKEY) {
        // SAFETY: `handle` came from create/open and is closed exactly once.
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
        // SAFETY: `name_w` outlives the call; `data_bytes` is valid for the
        // REG_SZ branch.
        let result = if data.is_empty() {
            unsafe { RegSetValueExW(handle, value_name, Some(0), REG_NONE, None) }
        } else {
            let data_bytes = wide_bytes(data);
            unsafe { RegSetValueExW(handle, value_name, Some(0), REG_SZ, Some(&data_bytes)) }
        };
        if result.is_err() {
            return Err(FileSystemError::Internal(format!(
                "default_manager.set_failed: {}",
                result.0
            )));
        }
        Ok(())
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
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut handle,
            )
        };
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
            RegQueryValueExW(
                handle,
                value_name,
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if probe.is_err() || size == 0 {
            close_key(handle);
            return None;
        }
        let mut buffer: Vec<u8> = vec![0; size as usize];
        // SAFETY: `buffer` has `size` bytes for the out-write; the other
        // pointers are as above.
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
        buffer.truncate(size as usize);
        // REG_SZ is UTF-16LE; strip one trailing null terminator if present.
        let mut units: Vec<u16> = buffer
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        Some(String::from_utf16_lossy(&units))
    }

    fn value_exists(subkey: &str, name: &str) -> bool {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut handle,
            )
        };
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
        // SAFETY: `name_w` outlives the call; only the size is probed.
        let probe =
            unsafe { RegQueryValueExW(handle, value_name, None, None, None, Some(&mut size)) };
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
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                KEY_WRITE,
                &mut handle,
            )
        };
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

    pub fn status() -> Result<DefaultManagerStatus, FileSystemError> {
        let user_choice = read_string(USERCHOICE_KEY, "ProgId");
        let is_default = user_choice.as_deref() == Some(PROGID);
        let is_registered =
            is_default || value_exists(&format!("{CLASSES_DIR_PROGID}\\OpenWithProgids"), PROGID);
        let detail = if is_registered && !is_default {
            Some(FINISH_IN_SETTINGS.to_string())
        } else {
            None
        };
        Ok(DefaultManagerStatus {
            supported: true,
            is_default,
            is_registered,
            detail,
        })
    }

    pub fn set() -> Result<DefaultManagerStatus, FileSystemError> {
        let exe = current_exe()?;
        let launch = launch_command(&exe);
        let icon = icon_value(&exe);

        write_default(&format!(r"Software\Classes\{PROGID}"), "dae")?;
        write_default(&format!(r"Software\Classes\{PROGID}\DefaultIcon"), &icon)?;
        write_default(
            &format!(r"Software\Classes\{PROGID}\shell\open\command"),
            &launch,
        )?;

        // Make dae appear in Explorer's "Open with" and the default-apps picker.
        let open_with = format!("{CLASSES_DIR_PROGID}\\OpenWithProgids");
        let handle = create_key(&open_with)?;
        let marker = set_value(handle, PROGID, "");
        close_key(handle);
        marker?;

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
        status()
    }

    pub fn unset() -> Result<DefaultManagerStatus, FileSystemError> {
        delete_tree(&format!(r"Software\Classes\{PROGID}"));
        delete_tree(&format!("{CLASSES_DIR_PROGID}\\shell\\dae"));
        delete_value(&format!("{CLASSES_DIR_PROGID}\\OpenWithProgids"), PROGID);
        notify_shell();
        status()
    }

    #[cfg(test)]
    mod tests {
        use super::{icon_value, launch_command};
        use std::path::Path;

        #[test]
        fn commands_quote_an_executable_with_spaces() {
            let exe = Path::new(r"C:\Program Files\dae\dae.exe");
            assert_eq!(
                launch_command(exe),
                "\"C:\\Program Files\\dae\\dae.exe\" \"%1\""
            );
            assert_eq!(icon_value(exe), "\"C:\\Program Files\\dae\\dae.exe\",0");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::desktop;

    #[test]
    fn desktop_entry_advertises_the_folder_mime_type() {
        let contents = desktop::contents("/opt/dae/dae");
        assert!(contents.contains("Exec=\"/opt/dae/dae\" %f"));
        assert!(contents.contains("MimeType=inode/directory;"));
    }

    #[test]
    fn unset_removes_only_the_dae_folder_entry() {
        let existing = "[Default Applications]\n\
                        inode/directory=dae-folder.desktop\n\
                        image/png=viewer.desktop\n";
        let stripped = desktop::strip_mimeapps_entry(existing);
        assert!(!stripped.contains("inode/directory=dae-folder.desktop"));
        assert!(stripped.contains("image/png=viewer.desktop"));
    }
}
