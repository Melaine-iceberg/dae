//! The "Open With" surface: the native Windows dialog, and the in-app picker's
//! data source on macOS and Linux.
//!
//! Windows keeps its `SHOpenWithDialog` flow; macOS and Linux have no system
//! picker, so `list_open_with_apps` asks LaunchServices / glib what can open
//! the item and `open_with_app` launches it (optionally registering it as the
//! default handler). Ported from the Tauri shell's `open_with` module.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::vfs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWithApp {
    pub id: String,
    pub name: String,
    pub icon_name: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWithChoices {
    pub default: Option<OpenWithApp>,
    pub recommended: Vec<OpenWithApp>,
    pub other: Vec<OpenWithApp>,
}

fn validate_target(path: &str) -> Result<PathBuf, FileSystemError> {
    if !vfs::is_local_path(path) {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_local_only".into(),
        ));
    }
    let target = PathBuf::from(path);
    if !target.is_file() && !target.is_dir() {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_not_found".into(),
        ));
    }
    Ok(target)
}

pub fn open_with(path: String) -> Result<(), FileSystemError> {
    let target = validate_target(&path)?;
    open_system_with_dialog(&target)
}

pub fn list_open_with_apps(path: String) -> Result<OpenWithChoices, FileSystemError> {
    let target = validate_target(&path)?;
    list_apps(&target)
}

pub fn open_with_app(
    path: String,
    app_id: String,
    set_default: bool,
) -> Result<(), FileSystemError> {
    let target = validate_target(&path)?;
    open_app(&target, &app_id, set_default)
}

fn list_apps(path: &Path) -> Result<OpenWithChoices, FileSystemError> {
    #[cfg(target_os = "macos")]
    return macos::list_apps(path);

    #[cfg(target_os = "linux")]
    return linux::list_apps(path);

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(FileSystemError::Unsupported(
            "fs.open_with_picker_unsupported".into(),
        ))
    }
}

fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
    #[cfg(target_os = "macos")]
    return macos::open_app(path, app_id, set_default);

    #[cfg(target_os = "linux")]
    return linux::open_app(path, app_id, set_default);

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, app_id, set_default);
        Err(FileSystemError::Unsupported(
            "fs.open_with_picker_unsupported".into(),
        ))
    }
}

#[cfg(target_os = "windows")]
fn open_system_with_dialog(file: &Path) -> Result<(), FileSystemError> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{
        OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPEN_AS_INFO_FLAGS, OPENASINFO,
        SHOpenWithDialog,
    };
    use windows::core::{HRESULT, PCWSTR};

    /// RAII COM apartment; S_FALSE (0x1) means this thread already had one and
    /// must not uninitialize it.
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
                code => Err(FileSystemError::Internal(format!(
                    "fs.open_with_dialog_failed: 0x{code:08X}"
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

    // The path goes through as UTF-16 rather than via `OpenAs_RunDLLW`, which
    // would round-trip through the ANSI code page and garble non-ASCII names.
    let wide_path: Vec<u16> = file
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let info = OPENASINFO {
        pcszFile: PCWSTR::from_raw(wide_path.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OPEN_AS_INFO_FLAGS(
            OAIF_ALLOW_REGISTRATION.0 | OAIF_REGISTER_EXT.0 | OAIF_EXEC.0,
        ),
    };

    // Modal until dismissed; cancel reports S_FALSE, which is success here.
    unsafe { SHOpenWithDialog(None, &info) }
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{FileSystemError, OpenWithApp, OpenWithChoices};
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::{CFURL, CFURLRef};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    const NO_ERR: i32 = 0;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyApplicationURLsForURL(in_url: CFURLRef, in_role_mask: u32) -> CFArrayRef;
        fn LSSetDefaultRoleHandlerForContentType(
            in_content_type: CFStringRef,
            in_role: u32,
            in_handler_bundle_id: CFStringRef,
        ) -> i32;
        fn UTTypeCreatePreferredIdentifierForTag(
            in_tag_class: CFStringRef,
            in_tag: CFStringRef,
            in_conforming_to_uti: CFStringRef,
        ) -> CFStringRef;
        fn CFArrayGetCount(the_array: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: isize) -> *const std::ffi::c_void;
    }

    pub fn list_apps(path: &Path) -> Result<OpenWithChoices, FileSystemError> {
        let url = CFURL::from_path(path.to_path_buf(), path.is_dir())
            .ok_or_else(|| FileSystemError::Internal("fs.open_with_list_failed".into()))?;

        let mut apps: Vec<OpenWithApp> = Vec::new();
        let array = unsafe { LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), ROLES_ALL) };
        if !array.is_null() {
            let count = unsafe { CFArrayGetCount(array) };
            for index in 0..count {
                let value = unsafe { CFArrayGetValueAtIndex(array, index) } as CFURLRef;
                if value.is_null() {
                    continue;
                }
                let app_url = unsafe { CFURL::wrap_under_get_rule(value) };
                push_app(&mut apps, &app_url);
            }
            drop(unsafe { CFArray::<CFURL>::wrap_under_create_rule(array) });
        }

        if apps.is_empty() {
            let mut dirs = vec![PathBuf::from("/Applications")];
            if let Ok(home) = std::env::var("HOME") {
                dirs.push(PathBuf::from(home).join("Applications"));
            }
            for dir in dirs {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let app_path = entry.path();
                        if app_path.extension().and_then(|e| e.to_str()) == Some("app") {
                            push_path(&mut apps, &app_path);
                        }
                    }
                }
            }
        }

        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(OpenWithChoices {
            recommended: apps,
            ..Default::default()
        })
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        let bundle = PathBuf::from(app_id);
        if !bundle.is_dir() {
            return Err(FileSystemError::NotFound("fs.open_with_app_missing".into()));
        }

        Command::new("open")
            .arg("-a")
            .arg(&bundle)
            .arg(path)
            .spawn()
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
            })?;

        if set_default {
            set_default_handler(path, &bundle)?;
        }
        Ok(())
    }

    fn set_default_handler(path: &Path, bundle: &Path) -> Result<(), FileSystemError> {
        let output = Command::new("defaults")
            .arg("read")
            .arg(bundle.join("Contents/Info"))
            .arg("CFBundleIdentifier")
            .output()
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_default_failed: {error}"))
            })?;
        let bundle_id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || bundle_id.is_empty() {
            return Err(FileSystemError::Internal(
                "fs.open_with_default_failed".into(),
            ));
        }

        let content_type = CFString::new(&content_type_identifier(path));
        let handler = CFString::new(&bundle_id);
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                content_type.as_concrete_TypeRef(),
                ROLES_ALL,
                handler.as_concrete_TypeRef(),
            )
        };
        if status != NO_ERR {
            return Err(FileSystemError::Internal(
                "fs.open_with_default_failed".into(),
            ));
        }
        Ok(())
    }

    fn content_type_identifier(path: &Path) -> String {
        if path.is_dir() {
            return "public.folder".into();
        }
        let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
            return "public.data".into();
        };

        let tag_class = CFString::new("public.filename-extension");
        let tag = CFString::new(extension);
        let uti = unsafe {
            let result = UTTypeCreatePreferredIdentifierForTag(
                tag_class.as_concrete_TypeRef(),
                tag.as_concrete_TypeRef(),
                std::ptr::null(),
            );
            if result.is_null() {
                None
            } else {
                Some(CFString::wrap_under_create_rule(result))
            }
        };
        uti.and_then(|uti| {
            let text = uti.to_string();
            (!text.is_empty()).then_some(text)
        })
        .unwrap_or_else(|| "public.data".into())
    }

    fn push_app(apps: &mut Vec<OpenWithApp>, url: &CFURL) {
        if let Some(app_path) = url.to_path() {
            push_path(apps, &app_path);
        }
    }

    fn push_path(apps: &mut Vec<OpenWithApp>, app_path: &Path) {
        if let Some(name) = app_path.file_stem().and_then(|n| n.to_str()) {
            apps.push(OpenWithApp {
                id: app_path.to_string_lossy().into_owned(),
                name: name.to_owned(),
                icon_name: None,
            });
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{FileSystemError, OpenWithApp, OpenWithChoices};
    use gio::prelude::*;
    use std::collections::HashSet;
    use std::path::Path;

    fn content_type(path: &Path) -> String {
        if path.is_dir() {
            return "inode/directory".into();
        }

        let file = gio::File::for_path(path);
        if let Ok(info) = file.query_info(
            "standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        ) && let Some(kind) = info.content_type()
        {
            return kind.into();
        }

        "application/octet-stream".into()
    }

    fn icon_query<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
        let names: Vec<&str> = names.filter(|name| !name.is_empty()).collect();
        (!names.is_empty()).then(|| names.join(":"))
    }

    fn icon_names(icon: Option<&gio::Icon>) -> Option<String> {
        let themed = icon?.downcast_ref::<gio::ThemedIcon>()?;
        icon_query(themed.names().iter().map(|name| name.as_str()))
    }

    fn row(app: &gio::AppInfo) -> Option<OpenWithApp> {
        Some(OpenWithApp {
            id: app.id()?.into(),
            name: app.display_name().into(),
            icon_name: icon_names(app.icon().as_ref()),
        })
    }

    fn rows(apps: Vec<gio::AppInfo>, visible: bool) -> Vec<OpenWithApp> {
        let mut rows: Vec<OpenWithApp> = apps
            .iter()
            .filter(|app| !visible || app.should_show())
            .filter_map(row)
            .collect();
        rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        rows
    }

    pub fn list_apps(path: &Path) -> Result<OpenWithChoices, FileSystemError> {
        let mime = content_type(path);

        let default = gio::AppInfo::default_for_type(&mime, false)
            .as_ref()
            .and_then(row);
        let recommended = rows(gio::AppInfo::recommended_for_type(&mime), true);

        let mut other = rows(gio::AppInfo::all(), true);
        let taken: HashSet<String> = recommended.iter().map(|app| app.id.clone()).collect();
        other.retain(|app| !taken.contains(&app.id));

        Ok(OpenWithChoices {
            default,
            recommended,
            other,
        })
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        let app = gio::AppInfo::all()
            .into_iter()
            .find(|app| app.id().as_deref() == Some(app_id))
            .ok_or_else(|| FileSystemError::NotFound("fs.open_with_app_missing".into()))?;

        let uri = gio::File::for_path(path).uri().to_string();
        app.launch_uris(&[uri.as_str()], None::<&gio::AppLaunchContext>)
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
            })?;

        if set_default {
            app.set_as_default_for_type(&content_type(path))
                .map_err(|error| {
                    FileSystemError::Internal(format!("fs.open_with_default_failed: {error}"))
                })?;
        }
        Ok(())
    }
}
