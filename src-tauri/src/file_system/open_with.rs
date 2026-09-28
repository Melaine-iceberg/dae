//! Cross-platform "Open With" application picker for macOS and Linux.
//!
//! Windows keeps its native `SHOpenWithDialog` flow (`open_with` in
//! `commands`): since Windows 10 that dialog ignores the registration flags
//! and can no longer set default associations, and the OS offers no supported
//! programmatic replacement, so no custom picker is provided there. macOS and
//! Linux expose no system picker at all, so these commands ask the platform's
//! own database what can open the item — LaunchServices on macOS, glib's
//! application registry on Linux — and hand the answer to the in-app picker,
//! which opens the item once or registers it as the new default handler.

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

/// The picker's content, in the three groups a desktop file manager uses.
///
/// The shape is not an invention to imitate: it is what GNOME's Files draws, and
/// the groups come from three genuinely different questions rather than one list
/// sliced up. Only `other` is a large list, which is why it is a section behind a
/// filter box rather than a flat continuation of `recommended`.
#[derive(Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenWithChoices {
    /// The handler the desktop would use right now, shown at the top so the
    /// row the user is about to change is also the row they see first.
    pub default: Option<OpenWithApp>,
    /// Applications that name this exact type in their own metadata.
    pub recommended: Vec<OpenWithApp>,
    /// Every other application the desktop would offer. Empty on macOS, where
    /// LaunchServices has no "all applications" answer that is worth a dialog.
    pub other: Vec<OpenWithApp>,
}

/// Lists the applications registered as able to open the local file or
/// directory at `path`, grouped the way a desktop's own picker groups them.
#[tauri::command]
#[specta::specta]
pub async fn list_open_with_apps(path: String) -> Result<OpenWithChoices, FileSystemError> {
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

#[cfg(target_os = "macos")]
mod macos {
    use super::{FileSystemError, OpenWithApp, OpenWithChoices};
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
        // LaunchServices answers one question — what can open this URL — so the
        // macOS picker has one group. The default handler and the "all
        // applications" list would each need a separate query, and the second is
        // a directory walk of every `.app` on the machine.
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
    use super::{FileSystemError, OpenWithApp, OpenWithChoices};
    use gio::prelude::*;
    use std::collections::HashSet;
    use std::path::Path;

    /// The freedesktop type applications get matched on.
    ///
    /// Asked of glib rather than read off the extension, because this one string
    /// decides the whole list: the local file backend consults the shared mime
    /// info globs *and* the file's magic bytes, so an extensionless PNG is
    /// `image/png` and gets image viewers, and `/bin/bash` is
    /// `application/x-executable` rather than "unknown".
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

        // Unreadable, or a type with no rule for it. `octet-stream` is what glib
        // itself calls the unknown, and the picker still has its third section.
        "application/octet-stream".into()
    }

    /// The candidate icon names, colon-joined for the protocol.
    ///
    /// Empty entries are dropped because glib hands them through — an entry whose
    /// `Icon=` is a bare `;` yields a list containing one — and an empty element
    /// would travel as `::` and be looked up as the empty name.
    fn icon_query<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
        let names: Vec<&str> = names.filter(|name| !name.is_empty()).collect();
        (!names.is_empty()).then(|| names.join(":"))
    }

    /// Every name this launcher's icon could answer to, in the order the Icon
    /// Theme spec says to try them.
    ///
    /// glib hands back a `ThemedIcon`, not the raw `Icon=` value, and the list it
    /// carries is longer than what the file says: `dev.zed.Zed.desktop` yields
    /// `zed` as well, which is the name older themes actually ship.
    /// Colon-joined because the protocol takes one parameter and `:` cannot appear
    /// in an icon name. An `Icon=` that points at a file rather than a theme gives
    /// some other icon type, and that row keeps the drawn glyph.
    fn icon_names(icon: Option<&gio::Icon>) -> Option<String> {
        let themed = icon?.downcast_ref::<gio::ThemedIcon>()?;
        icon_query(themed.names().iter().map(|name| name.as_str()))
    }

    /// One row of the picker, or `None` for an entry glib could not name.
    fn row(app: &gio::AppInfo) -> Option<OpenWithApp> {
        Some(OpenWithApp {
            id: app.id()?.into(),
            name: app.display_name().into(),
            icon_name: icon_names(app.icon().as_ref()),
        })
    }

    /// glib's application list, as rows ordered the way the dialog shows them.
    ///
    /// `visible` is the filter a desktop applies to its own launcher list: not
    /// `NoDisplay`, not `Hidden`, and allowed by the running `XDG_CURRENT_DESKTOP`
    /// — which is why a KDE-only entry stays out of the dialog here, exactly as it
    /// stays out of the desktop's overview.
    ///
    /// It keeps `Terminal=true` entries, which the hand-rolled version of this list
    /// dropped for fear of launching a text user interface into nothing. glib wraps
    /// those in a terminal of its own (`xdg-terminal-exec`, then the known
    /// emulators) and reports a real error when the machine has none, so the rows
    /// this filter dropped were simply missing, not safer.
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

        // Three different questions, not one list sliced three ways.
        // `recommended_for_type` is the entries naming this type in their own
        // `MimeType=`; `all()` is every launcher on the machine, which is how an
        // entry declaring no `MimeType=` at all — WeChat, here — ends up in the
        // desktop's dialog for a text file. The gap between the two is wider than
        // it looks: glib resolves `text/x-python` against shared-mime-info's
        // `inherits-from` chain, so an editor that only ever wrote `text/plain` is
        // a handler for a `.py` file and still not a recommendation for one.
        let default = gio::AppInfo::default_for_type(&mime, false)
            .as_ref()
            .and_then(row);
        let recommended = rows(gio::AppInfo::recommended_for_type(&mime), true);

        // The default stays inside 推荐应用: Files draws it in both places, once to
        // say what will happen and once to say it was chosen. Only the tail is
        // trimmed, so no row appears twice.
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
        // Matched against the launchers glib itself found rather than handed to
        // `DesktopAppInfo::new`: the id comes back from the webview, and building
        // an entry by name would accept any readable `.desktop` file on the
        // machine instead of one this list offered.
        let app = gio::AppInfo::all()
            .into_iter()
            .find(|app| app.id().as_deref() == Some(app_id))
            .ok_or_else(|| FileSystemError::NotFound("fs.open_with_app_missing".into()))?;

        let uri = gio::File::for_path(path).uri().to_string();
        // glib expands the `Exec` line, chooses `%f` over `%U` by what the entry
        // declares, does the startup-notification handshake, and reaps the child —
        // four things the hand-built argv did not do, one of which needed a thread
        // of its own.
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn joins_the_theme_candidates_and_drops_the_empty_ones() {
            assert_eq!(
                icon_query(["text-editor", "accessories-text-editor"].into_iter()).as_deref(),
                Some("text-editor:accessories-text-editor")
            );
            assert_eq!(
                icon_query(["a", "", "b"].into_iter()).as_deref(),
                Some("a:b")
            );
            assert_eq!(icon_query(["", ""].into_iter()), None);
            assert_eq!(icon_query(std::iter::empty()), None);
        }

        /// The order the theme asked for is the order the lookup gets: glib puts
        /// the `Icon=` value first and the names derived from the desktop id after
        /// it, and taking only the first would lose every older theme that ships
        /// the shorter name. (`-symbolic` variants trail, which glib appends.)
        #[test]
        fn keeps_the_theme_name_order_the_lookup_has_to_try() {
            let themed = gio::ThemedIcon::from_names(&["dev.zed.Zed", "zed"]);
            assert_eq!(
                icon_names(Some(&themed.upcast())).as_deref(),
                Some("dev.zed.Zed:zed:dev.zed.Zed-symbolic:zed-symbolic")
            );
            // No icon at all is a row with no image, not a row that is missing.
            assert_eq!(icon_names(None), None);
        }

        #[test]
        fn calls_a_directory_and_an_unreadable_file_what_they_are() {
            // A directory is its own type, which is the difference between
            // offering a file manager and offering a text editor.
            assert_eq!(content_type(Path::new("/tmp")), "inode/directory");
            // No magic bytes to read, so no claim better than "data". Guessing
            // from the suffix instead would promise image viewers for a file the
            // user cannot open.
            assert_eq!(
                content_type(Path::new("/nonexistent/image.png")),
                "application/octet-stream"
            );

            // And where the file *is* readable, the answer is not the name: this
            // one has no extension at all, so only the content can say what it is.
            let mut probe = std::env::temp_dir();
            probe.push(format!("dae-open-with-probe-{}", std::process::id()));
            std::fs::write(&probe, "just some words\n").expect("a writable temp dir");
            let kind = content_type(&probe);
            let _ = std::fs::remove_file(&probe);
            assert_eq!(kind, "text/plain");
        }
    }
}
