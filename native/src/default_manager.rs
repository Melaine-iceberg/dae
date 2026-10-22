//! Registers dae as the OS handler for folders (`inode/directory`).
//!
//! Ported from the Tauri shell unchanged apart from the app handle: the bundle
//! identifier is fixed, and the running executable is resolved from the
//! process. Windows registers an HKCU ProgID and reports `is_registered` until
//! the user confirms dae in the OS picker (Windows 8+ protects the folder
//! `UserChoice` hash); macOS uses LaunchServices; Linux writes a `.desktop`
//! entry and calls `xdg-mime`.

use dae_core::file_system::error::FileSystemError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultFileManagerStatus {
    pub supported: bool,
    pub is_default: bool,
    pub is_registered: bool,
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

pub fn get_status() -> Result<DefaultFileManagerStatus, FileSystemError> {
    platform::status()
}

pub fn set() -> Result<DefaultFileManagerStatus, FileSystemError> {
    let status = platform::set()?;
    // Cache only a confirmed default; a merely-registered state is not yet it.
    let _ = crate::settings::set_default_file_manager_cache(status.is_default);
    Ok(status)
}

pub fn unset() -> Result<DefaultFileManagerStatus, FileSystemError> {
    let status = platform::unset()?;
    let _ = crate::settings::set_default_file_manager_cache(status.is_default);
    Ok(status)
}

fn current_exe() -> Result<std::path::PathBuf, FileSystemError> {
    std::env::current_exe().map_err(|error| FileSystemError::Internal(error.to_string()))
}

mod platform {
    use super::{DefaultFileManagerStatus, FileSystemError};

    pub fn status() -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::status();
        #[cfg(target_os = "linux")]
        return super::linux::status();
        #[cfg(windows)]
        return super::windows_impl::status();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        Ok(DefaultFileManagerStatus::unsupported())
    }

    pub fn set() -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::set();
        #[cfg(target_os = "linux")]
        return super::linux::set();
        #[cfg(windows)]
        return super::windows_impl::set();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        Ok(DefaultFileManagerStatus::unsupported())
    }

    pub fn unset() -> Result<DefaultFileManagerStatus, FileSystemError> {
        #[cfg(target_os = "macos")]
        return super::macos::unset();
        #[cfg(target_os = "linux")]
        return super::linux::unset();
        #[cfg(windows)]
        return super::windows_impl::unset();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        Ok(DefaultFileManagerStatus::unsupported())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{DefaultFileManagerStatus, FileSystemError};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    const NO_ERR: i32 = 0;
    const FOLDER_UTI: &str = "public.folder";
    const BUNDLE_ID: &str = "com.melaine.dae";

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

    pub fn status() -> Result<DefaultFileManagerStatus, FileSystemError> {
        let content_type = CFString::new(FOLDER_UTI);
        let handler = unsafe {
            LSCopyDefaultRoleHandlerForContentType(content_type.as_concrete_TypeRef(), ROLES_ALL)
        };
        let current = if handler.is_null() {
            None
        } else {
            Some(unsafe { CFString::wrap_under_create_rule(handler) }.to_string())
        };
        let is_default = current.as_deref() == Some(BUNDLE_ID);
        Ok(DefaultFileManagerStatus {
            supported: true,
            is_default,
            is_registered: is_default,
            detail: None,
        })
    }

    pub fn set() -> Result<DefaultFileManagerStatus, FileSystemError> {
        let content_type = CFString::new(FOLDER_UTI);
        let handler = CFString::new(BUNDLE_ID);
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
        status()
    }

    pub fn unset() -> Result<DefaultFileManagerStatus, FileSystemError> {
        let mut current = status()?;
        current.detail = Some("default_manager.cannot_unset_macos".to_string());
        Ok(current)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{DefaultFileManagerStatus, FileSystemError, current_exe};
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

    fn query_status() -> Result<DefaultFileManagerStatus, FileSystemError> {
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

    pub fn status() -> Result<DefaultFileManagerStatus, FileSystemError> {
        query_status()
    }

    pub fn set() -> Result<DefaultFileManagerStatus, FileSystemError> {
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
            .map_err(|error| {
                FileSystemError::Internal(format!("default_manager.set_failed: {error}"))
            })?;
        if !status.success() {
            return Err(FileSystemError::Internal(
                "default_manager.set_failed".to_string(),
            ));
        }
        let _ = Command::new("update-desktop-database").arg(&dir).status();
        query_status()
    }

    pub fn unset() -> Result<DefaultFileManagerStatus, FileSystemError> {
        let _ = fs::remove_file(desktop_path());
        strip_mimeapps_entry();
        let _ = Command::new("update-desktop-database")
            .arg(applications_dir())
            .status();
        query_status()
    }

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

#[cfg(windows)]
mod windows_impl {
    use super::{DefaultFileManagerStatus, FileSystemError, current_exe};
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

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_bytes(value: &str) -> Vec<u8> {
        wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect()
    }

    fn create_key(subkey: &str) -> Result<HKEY, FileSystemError> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
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
        unsafe {
            let _ = RegCloseKey(handle);
        }
    }

    fn set_value(handle: HKEY, name: &str, data: &str) -> Result<(), FileSystemError> {
        let name_w = wide(name);
        let value_name = if name.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(name_w.as_ptr())
        };

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

    fn write_default(subkey: &str, data: &str) -> Result<(), FileSystemError> {
        let handle = create_key(subkey)?;
        let result = set_value(handle, "", data);
        close_key(handle);
        result
    }

    fn read_string(subkey: &str, name: &str) -> Option<String> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
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
        let mut as_u16: Vec<u16> = buffer
            .as_chunks::<2>()
            .0
            .iter()
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
        let probe =
            unsafe { RegQueryValueExW(handle, value_name, None, None, None, Some(&mut size)) };
        close_key(handle);
        probe.is_ok()
    }

    fn delete_tree(subkey: &str) {
        let path = wide(subkey);
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()));
        }
    }

    fn delete_value(subkey: &str, name: &str) {
        let path = wide(subkey);
        let mut handle = HKEY::default();
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
        unsafe {
            let _ = RegDeleteValueW(handle, PCWSTR(name_w.as_ptr()));
        }
        close_key(handle);
    }

    fn notify_shell() {
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }

    pub fn status() -> Result<DefaultFileManagerStatus, FileSystemError> {
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

    pub fn set() -> Result<DefaultFileManagerStatus, FileSystemError> {
        let exe = current_exe()?;
        let exe_str = exe.to_string_lossy().into_owned();
        let launch = format!("\"{exe_str}\" \"%1\"");
        let icon = format!("\"{exe_str}\",0");

        write_default(&format!(r"Software\Classes\{PROGID}"), "dae")?;
        write_default(&format!(r"Software\Classes\{PROGID}\DefaultIcon"), &icon)?;
        write_default(
            &format!(r"Software\Classes\{PROGID}\shell\open\command"),
            &launch,
        )?;

        let open_with = format!("{CLASSES_DIR_PROGID}\\OpenWithProgids");
        let handle = create_key(&open_with)?;
        set_value(handle, PROGID, "")?;
        close_key(handle);

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

    pub fn unset() -> Result<DefaultFileManagerStatus, FileSystemError> {
        delete_tree(&format!(r"Software\Classes\{PROGID}"));
        delete_tree(&format!("{CLASSES_DIR_PROGID}\\shell\\dae"));
        delete_value(&format!("{CLASSES_DIR_PROGID}\\OpenWithProgids"), PROGID);
        notify_shell();
        status()
    }
}
