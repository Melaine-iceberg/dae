//! Cross-platform "Open With" application picker for macOS and Linux.
//!
//! Windows keeps its native `SHOpenWithDialog` flow (`open_with` in
//! `commands`): since Windows 10 that dialog ignores the registration flags
//! and can no longer set default associations, and the OS offers no supported
//! programmatic replacement, so no custom picker is provided there. macOS and
//! Linux expose no system picker at all, so these commands enumerate candidate
//! applications (LaunchServices on macOS, freedesktop `.desktop` entries on
//! Linux) for the in-app picker, which can open the item once or set a new
//! default handler.

use super::error::FileSystemError;
use super::vfs;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// An application that can open a file or directory, as listed by the picker.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenWithApp {
    /// Platform identifier: the `.desktop` file id (Linux) or the application
    /// bundle path (macOS).
    pub id: String,
    /// Display name.
    pub name: String,
}

/// Lists the applications registered as able to open the local file or
/// directory at `path`. When no application advertises the item's type, all
/// visible applications are returned so the picker still has content.
#[tauri::command]
#[specta::specta]
pub async fn list_open_with_apps(path: String) -> Result<Vec<OpenWithApp>, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_local_only".into(),
        ));
    }

    let target = PathBuf::from(&path);
    if !target.is_file() && !target.is_dir() {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_not_found".into(),
        ));
    }

    tauri::async_runtime::spawn_blocking(move || list_apps(&target))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Opens `path` with the application identified by `app_id`. When
/// `set_default` is set, the application also becomes the default handler for
/// the item's type.
#[tauri::command]
#[specta::specta]
pub async fn open_with_app(
    path: String,
    app_id: String,
    set_default: bool,
) -> Result<(), FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_local_only".into(),
        ));
    }

    let target = PathBuf::from(&path);
    if !target.is_file() && !target.is_dir() {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_not_found".into(),
        ));
    }

    tauri::async_runtime::spawn_blocking(move || open_app(&target, &app_id, set_default))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn list_apps(path: &Path) -> Result<Vec<OpenWithApp>, FileSystemError> {
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

#[cfg(target_os = "macos")]
mod macos {
    use super::{FileSystemError, OpenWithApp};
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::{CFURL, CFURLRef};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// LaunchServices role mask meaning "any role" (viewer, editor, or shell).
    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    const NO_ERR: i32 = 0;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        /// Applications LaunchServices knows can handle the given URL.
        fn LSCopyApplicationURLsForURL(in_url: CFURLRef, in_role_mask: u32) -> CFArrayRef;
        /// Registers the default handler for a content type (deprecated but
        /// still functional; newer macOS versions may still honor it for file
        /// types even though URL-scheme defaults moved to System Settings).
        fn LSSetDefaultRoleHandlerForContentType(
            in_content_type: CFStringRef,
            in_role: u32,
            in_handler_bundle_id: CFStringRef,
        ) -> i32;
        /// Maps a tag (here: a filename extension) to its preferred UTI.
        fn UTTypeCreatePreferredIdentifierForTag(
            in_tag_class: CFStringRef,
            in_tag: CFStringRef,
            in_conforming_to_uti: CFStringRef,
        ) -> CFStringRef;
        fn CFArrayGetCount(the_array: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: isize) -> *const std::ffi::c_void;
    }

    pub fn list_apps(path: &Path) -> Result<Vec<OpenWithApp>, FileSystemError> {
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
            // Take ownership of the create-rule reference so it is released.
            drop(unsafe { CFArray::<CFURL>::wrap_under_create_rule(array) });
        }

        if apps.is_empty() {
            // Nothing registered for this type: fall back to the standard
            // application folders so the picker still offers a choice.
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
        Ok(apps)
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        let bundle = PathBuf::from(app_id);
        if !bundle.is_dir() {
            return Err(FileSystemError::NotFound(
                "fs.open_with_app_missing".into(),
            ));
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

    /// Registers `bundle` as the default handler for `path`'s content type.
    fn set_default_handler(path: &Path, bundle: &Path) -> Result<(), FileSystemError> {
        // The bundle identifier is what LaunchServices stores as the handler;
        // `defaults read` pulls it straight out of the bundle's Info.plist.
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

    /// Maps an item to the Uniform Type Identifier LaunchServices keys by.
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
            });
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{FileSystemError, OpenWithApp};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// One parsed `[Desktop Entry]` group.
    struct DesktopEntry {
        name: String,
        exec: String,
        mime_types: Vec<String>,
    }

    /// XDG data dirs' `applications` directories, user scope first so its
    /// `.desktop` files win the id deduplication.
    fn application_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        let data_home = std::env::var("XDG_DATA_HOME")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var("HOME").ok().map(|home| format!("{home}/.local/share")));
        if let Some(data_home) = data_home {
            dirs.push(PathBuf::from(data_home).join("applications"));
        }

        let data_dirs =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
        dirs
    }

    /// Parses the keys the picker needs from a desktop entry file, skipping
    /// entries that stay out of menus (`NoDisplay`/`Hidden`) or need a
    /// terminal host (`Terminal=true`, which cannot be launched reliably from
    /// a GUI context without desktop-specific wrapping).
    fn parse_desktop_entry(content: &str) -> Option<DesktopEntry> {
        let mut name: Option<String> = None;
        let mut exec: Option<String> = None;
        let mut mime_types: Vec<String> = Vec::new();
        let mut terminal = false;
        let mut no_display = false;
        let mut hidden = false;
        let mut in_main_group = false;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_main_group = line == "[Desktop Entry]";
                continue;
            }
            if !in_main_group {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "Name" => name = Some(value.to_owned()),
                "Exec" => exec = Some(value.to_owned()),
                "MimeType" => {
                    mime_types = value
                        .split(';')
                        .filter(|mime| !mime.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                "Terminal" => terminal = value.eq_ignore_ascii_case("true"),
                "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
                "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }

        if no_display || hidden || terminal {
            return None;
        }
        Some(DesktopEntry {
            name: name?,
            exec: exec?,
            mime_types,
        })
    }

    /// Reads every candidate desktop entry, keyed by its desktop id (the
    /// file name), keeping the first copy found in the higher-priority dir.
    fn collect_entries() -> Vec<(String, DesktopEntry)> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();

        for dir in application_dirs() {
            let Ok(files) = std::fs::read_dir(&dir) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                    continue;
                };
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(entry) = parse_desktop_entry(&content) {
                    entries.push((id, entry));
                }
            }
        }
        entries
    }

    /// The freedesktop MIME type LaunchServices-style matching keys on.
    fn detect_mime(path: &Path) -> String {
        if path.is_dir() {
            return "inode/directory".into();
        }
        mime_guess::from_path(path)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_owned()
    }

    pub fn list_apps(path: &Path) -> Result<Vec<OpenWithApp>, FileSystemError> {
        let mime = detect_mime(path);
        let entries = collect_entries();

        let mut apps: Vec<OpenWithApp> = entries
            .iter()
            .filter(|(_, entry)| entry.mime_types.iter().any(|candidate| candidate == &mime))
            .map(|(id, entry)| OpenWithApp {
                id: id.clone(),
                name: entry.name.clone(),
            })
            .collect();

        if apps.is_empty() {
            // Unrecognized types still deserve a picker: offer everything.
            apps = entries
                .iter()
                .map(|(id, entry)| OpenWithApp {
                    id: id.clone(),
                    name: entry.name.clone(),
                })
                .collect();
        }

        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(apps)
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        let entries = collect_entries();
        let entry = entries
            .iter()
            .find(|(id, _)| id == app_id)
            .map(|(_, entry)| entry)
            .ok_or_else(|| FileSystemError::NotFound("fs.open_with_app_missing".into()))?;

        let file = path.to_string_lossy().into_owned();
        let (program, args) = expand_exec(&entry.exec, &file, &entry.name);
        if program.is_empty() {
            return Err(FileSystemError::Internal(
                "fs.open_with_launch_failed".into(),
            ));
        }

        Command::new(&program)
            .args(&args)
            .spawn()
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
            })?;

        if set_default {
            set_default_handler(app_id, &detect_mime(path))?;
        }
        Ok(())
    }

    /// Registers the desktop entry as the default handler via `xdg-mime`.
    fn set_default_handler(app_id: &str, mime: &str) -> Result<(), FileSystemError> {
        let status = Command::new("xdg-mime")
            .args(["default", app_id, mime])
            .status()
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_default_failed: {error}"))
            })?;
        if !status.success() {
            return Err(FileSystemError::Internal(
                "fs.open_with_default_failed".into(),
            ));
        }
        Ok(())
    }

    /// Splits a desktop entry `Exec` value into program and arguments with the
    /// file substituted into the `%f`/`%F`/`%u`/`%U` field codes. Quoted
    /// arguments survive intact; unknown codes stay literal.
    fn expand_exec(exec: &str, file: &str, app_name: &str) -> (String, Vec<String>) {
        let tokens = tokenize_exec(exec);
        let mut program = String::new();
        let mut args: Vec<String> = Vec::new();
        let mut has_file = false;

        for (index, token) in tokens.iter().enumerate() {
            let mut expanded = String::new();
            let mut chars = token.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    expanded.push(c);
                    continue;
                }
                match chars.next() {
                    Some('f') | Some('F') | Some('u') | Some('U') => {
                        expanded.push_str(file);
                        has_file = true;
                    }
                    // The application's display name substitutes `%c`.
                    Some('c') => expanded.push_str(app_name),
                    // `%i` (icon) and `%k` (desktop file path) carry no
                    // useful value in this context.
                    Some('i') | Some('k') => {}
                    Some('%') => expanded.push('%'),
                    Some(other) => {
                        expanded.push('%');
                        expanded.push(other);
                    }
                    None => expanded.push('%'),
                }
            }

            if index == 0 {
                program = expanded;
            } else {
                args.push(expanded);
            }
        }

        // Entries without any file field code take the path as a trailing
        // argument, matching the desktop entry spec's recommendation.
        if !has_file {
            args.push(file.to_owned());
        }
        (program, args)
    }

    /// Tokenizes an `Exec` value per the desktop entry spec: spaces separate
    /// arguments, double quotes group them, and backslash escapes a quoted
    /// reserved character.
    fn tokenize_exec(exec: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;

        let mut chars = exec.chars();
        while let Some(c) = chars.next() {
            if in_quotes {
                if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                } else if c == '"' {
                    in_quotes = false;
                } else {
                    current.push(c);
                }
            } else if c == '"' {
                in_quotes = true;
            } else if c.is_whitespace() {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            } else {
                current.push(c);
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }
}

// The helpers under test live in the Linux-only module, so the tests follow it.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::linux::expand_exec;

    #[test]
    fn expands_single_file_code() {
        let (program, args) = expand_exec("gedit %f", "/tmp/a b.txt", "Gedit");
        assert_eq!(program, "gedit");
        assert_eq!(args, ["/tmp/a b.txt"]);
    }

    #[test]
    fn appends_file_without_field_code() {
        let (program, args) = expand_exec("code", "/tmp/a.txt", "Code");
        assert_eq!(program, "code");
        assert_eq!(args, ["/tmp/a.txt"]);
    }

    #[test]
    fn keeps_quoted_arguments() {
        let (program, args) = expand_exec(
            "\"/opt/My App/run\" --profile %u",
            "/tmp/a.txt",
            "My App",
        );
        assert_eq!(program, "/opt/My App/run");
        assert_eq!(args, ["--profile", "/tmp/a.txt"]);
    }
}
