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
    /// What to draw the row with: a freedesktop icon name or, where the entry
    /// gives one, an absolute icon path — handed to `fileicon://?name=` rather
    /// than resolved here, because a list of these as base64 runs to megabytes.
    /// `None` when the launcher names no icon, and always on macOS, whose shell
    /// keys icons on a file rather than a name.
    pub icon_name: Option<String>,
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
                // The bundle's `.icns` is reachable through `file_icons`, but
                // that means an AppKit call per row on a blocking thread that is
                // not the render pool, so the picker keeps its drawn glyph.
                icon_name: None,
            });
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{FileSystemError, OpenWithApp};
    use crate::shell_commands::SelectionKind;
    use crate::shell_commands::desktop_entry::{
        expand_exec, localized, mime_matches, parse_groups, split_list, user_language,
    };
    use crate::xdg::data_roots;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// One application launcher, reduced to what the picker needs.
    struct Application {
        /// The `.desktop` file name, which is both the id the frontend sends
        /// back and the value `xdg-mime` stores as a handler.
        id: String,
        /// The `.desktop` file itself, absolute — what `%k` expands to.
        path: PathBuf,
        /// Display name, localized the way the desktop's own menus localize.
        name: String,
        /// The raw `Exec` line, field codes still in place.
        exec: String,
        /// The raw `Icon=` value: a themed name, or an absolute path. Both are
        /// answered by the same theme search.
        icon: Option<String>,
        /// The declared `MimeType` list, lowercased.
        mime_types: Vec<String>,
    }

    /// The `applications` directory under each XDG data root, most specific
    /// first, so a user's own entry wins the deduplication below.
    fn application_dirs() -> Vec<PathBuf> {
        data_roots()
            .into_iter()
            .map(|root| root.join("applications"))
            .collect()
    }

    /// Reads the keys the picker needs from a desktop entry file, skipping the
    /// entries that stay out of menus (`NoDisplay`/`Hidden`), that need a
    /// terminal host (`Terminal=true`, which cannot be launched reliably from a
    /// GUI context without desktop-specific wrapping), and that are not
    /// launchers at all (`Type=Link`, `Type=Directory`).
    fn parse_application(
        path: &Path,
        content: &str,
        language: Option<&str>,
    ) -> Option<Application> {
        let entry = parse_groups(content).remove("Desktop Entry")?;

        let is_disabled = |key: &str| {
            entry
                .get(key)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        };
        if is_disabled("NoDisplay") || is_disabled("Hidden") || is_disabled("Terminal") {
            return None;
        }
        // Omitted means `Application`, which is what the pre-spec files are.
        match entry.get("Type").map(String::as_str) {
            None | Some("Application") => {}
            _ => return None,
        }

        let name = localized(&entry, "Name", language)?.to_owned();
        let exec = entry
            .get("Exec")
            .filter(|value| !value.trim().is_empty())?
            .clone();

        Some(Application {
            id: path.file_name()?.to_string_lossy().into_owned(),
            path: path.to_path_buf(),
            name,
            exec,
            icon: localized(&entry, "Icon", language).map(str::to_owned),
            mime_types: entry
                .get("MimeType")
                .map(|value| split_list(value).map(str::to_lowercase).collect())
                .unwrap_or_default(),
        })
    }

    /// Reads every candidate launcher on the machine. Real file-system work, so
    /// callers keep it off the async command thread.
    fn collect_applications() -> Vec<Application> {
        let language = user_language();
        let mut seen = HashSet::new();
        let mut applications = Vec::new();

        for dir in application_dirs() {
            let Ok(files) = std::fs::read_dir(&dir) else {
                continue;
            };
            // `read_dir` yields an unspecified order; sorting keeps the list
            // stable between runs, so the picker does not reshuffle.
            let mut files: Vec<PathBuf> = files.flatten().map(|file| file.path()).collect();
            files.sort();

            for path in files {
                if path.extension().and_then(|name| name.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !seen.insert(name.to_owned()) {
                    continue;
                }
                // A root-owned entry from a half-installed package must not
                // hide the rest of the list.
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(application) = parse_application(&path, &content, language.as_deref()) {
                    applications.push(application);
                }
            }
        }
        applications
    }

    /// The freedesktop MIME type the matching keys on.
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
        let kind = if path.is_dir() {
            SelectionKind::Directory
        } else {
            SelectionKind::File
        };
        let applications = collect_applications();

        let mut matching: Vec<&Application> = applications
            .iter()
            .filter(|application| {
                application
                    .mime_types
                    .iter()
                    .any(|declared| mime_matches(declared, &mime, kind))
            })
            .collect();

        if matching.is_empty() {
            // Unrecognized types still deserve a picker: offer everything.
            matching = applications.iter().collect();
        }

        matching.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        Ok(matching
            .iter()
            .map(|application| OpenWithApp {
                id: application.id.clone(),
                name: application.name.clone(),
                icon_name: application.icon.clone(),
            })
            .collect())
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        let application = collect_applications()
            .into_iter()
            .find(|application| application.id == app_id)
            .ok_or_else(|| FileSystemError::NotFound("fs.open_with_app_missing".into()))?;

        let file = path.to_string_lossy().into_owned();
        let desktop_file = application.path.to_string_lossy().into_owned();
        // Expands to an argument vector with the program first. Deliberately not
        // through a shell — the `Exec` line is data from a `.desktop` file, and
        // shell semantics would let a selected filename containing `;` or `$(…)`
        // change what runs.
        let arguments = expand_exec(
            &application.exec,
            std::slice::from_ref(&file),
            &application.name,
            &desktop_file,
        );
        let (program, rest) = arguments
            .split_first()
            .ok_or_else(|| FileSystemError::Internal("fs.open_with_launch_failed".into()))?;

        let child = Command::new(program).args(rest).spawn().map_err(|error| {
            FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
        })?;
        reap(child);

        if set_default {
            set_default_handler(app_id, &detect_mime(path))?;
        }
        Ok(())
    }

    /// Reaps the launched application so it does not outlive its parent as a
    /// zombie. Its exit status is of no interest here; only that it is collected.
    fn reap(mut child: std::process::Child) {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
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

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse(text: &str) -> Option<Application> {
            parse_application(
                Path::new("/usr/share/applications/tool.desktop"),
                text,
                Some("zh_CN"),
            )
        }

        #[test]
        fn takes_the_users_language_and_the_icon_the_entry_names() {
            // The two things a hand-rolled parser of this file's own design got
            // wrong: an entry that ships `Name[zh_CN]` has no business drawing
            // English next to a Chinese menu, and an entry with an icon is not
            // the generic window glyph.
            let application = parse(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Archive Manager\n\
                 Name[zh_CN]=归档管理器\n\
                 Icon=org.kde.ark\n\
                 Exec=ark %f\n\
                 MimeType=application/zip;text/plain;\n",
            )
            .expect("a launcher");

            assert_eq!(application.id, "tool.desktop");
            assert_eq!(application.name, "归档管理器");
            assert_eq!(application.icon.as_deref(), Some("org.kde.ark"));
            assert_eq!(application.exec, "ark %f");
            assert_eq!(
                application.mime_types,
                ["application/zip".to_string(), "text/plain".to_string()]
            );
        }

        #[test]
        fn falls_back_to_the_bare_name_when_the_locale_is_absent() {
            let application = parse(
                "[Desktop Entry]\n\
                 Name=Firefox\n\
                 Icon=firefox\n\
                 Exec=firefox %u\n",
            )
            .expect("a launcher");

            assert_eq!(application.name, "Firefox");
        }

        #[test]
        fn keeps_an_entry_that_declares_no_type() {
            // `Type` is required by the spec but its only meaningful default is
            // `Application`, and pre-spec entries omit it.
            assert!(parse("[Desktop Entry]\nName=T\nExec=t\n").is_some());
        }

        #[test]
        fn skips_what_a_desktop_would_not_offer() {
            for entry in [
                // Hidden from menus by the packager.
                "[Desktop Entry]\nName=T\nExec=t\nNoDisplay=true\n",
                "[Desktop Entry]\nName=T\nExec=t\nHidden=true\n",
                // Needs a terminal this picker cannot give it.
                "[Desktop Entry]\nName=T\nExec=t\nTerminal=true\n",
                // Not a launcher: a bookmark and an icon-theme directory.
                "[Desktop Entry]\nType=Link\nName=T\nURL=https://example.com\n",
                "[Desktop Entry]\nType=Directory\nName=T\n",
                // Nothing to run.
                "[Desktop Entry]\nName=T\nExec=   \n",
                // No name to show.
                "[Desktop Entry]\nExec=t\n",
            ] {
                assert!(parse(entry).is_none(), "accepted {entry:?}");
            }
        }
    }
}
