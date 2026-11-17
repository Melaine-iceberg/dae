//! Cross-platform "Open With" application lists.
//!
//! Every platform asks its own database what can open the item: LaunchServices
//! on macOS, glib's application registry on Linux, the shell's association
//! handlers (`SHAssocEnumHandlers`) on Windows. macOS and Linux hand the answer
//! to the in-app picker dialog, which can also register a new default handler.
//! Windows shows the recommended handlers as an Explorer-style context submenu
//! and keeps the native `SHOpenWithDialog` (`open_with` in `commands`) behind
//! "choose another app"; setting a default there stays with the OS dialog,
//! because `UserChoice` is hash-protected and offers no supported programmatic
//! replacement.

use super::error::FileSystemError;
use super::vfs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// An application that can open a file or directory, as listed by the picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWithApp {
    /// Platform identifier: the `.desktop` file id (Linux), the application
    /// bundle path (macOS), or the handler's executable path (Windows).
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
#[derive(Debug, Default, Serialize, Deserialize)]
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

/// See `commands::list_open_with_apps` for the contract; this half is engine-free.
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

    tokio::task::spawn_blocking(move || list_apps(&target))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::open_with_app` for the contract; this half is engine-free.
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

    tokio::task::spawn_blocking(move || open_app(&target, &app_id, set_default))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn list_apps(path: &Path) -> Result<OpenWithChoices, FileSystemError> {
    #[cfg(target_os = "macos")]
    return macos::list_apps(path);

    #[cfg(target_os = "linux")]
    return linux::list_apps(path);

    #[cfg(target_os = "windows")]
    return windows::list_apps(path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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

    #[cfg(target_os = "windows")]
    return windows::open_app(path, app_id, set_default);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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

#[cfg(target_os = "windows")]
mod windows {
    use super::{FileSystemError, OpenWithApp, OpenWithChoices};
    use std::collections::HashSet;
    use std::path::Path;

    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    };
    use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};
    use windows::Win32::UI::Shell::{
        AO_NONE, ASSOC_FILTER_RECOMMENDED, IApplicationActivationManager, IAssocHandler,
        SHAssocEnumHandlers,
    };
    use windows::core::{GUID, HRESULT, PCWSTR, PWSTR};

    const ACTIVATION_MANAGER_CLSID: GUID =
        GUID::from_u128(0x45BA127D_10A8_46EA_8AB7_56EA9078943C);

    /// RAII COM 套间；S_FALSE (0x1) 表示线程已有套间，此时不能反初始化。
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

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Reads a shell-allocated string and releases it with the allocator that
    /// produced it (`CoTaskMemFree`, not Rust's drop).
    fn take_string(raw: PWSTR) -> String {
        if raw.is_null() {
            return String::new();
        }
        let text = unsafe { raw.to_string() }.unwrap_or_default();
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        text
    }

    fn handler_exe(handler: &IAssocHandler) -> String {
        match unsafe { handler.GetName() } {
            Ok(raw) => take_string(raw),
            Err(_) => String::new(),
        }
    }

    /// The shell's recommended handlers for an extension, in shell order —
    /// the same list Explorer's "Open with" submenu draws.
    fn handlers(extension: &str) -> Result<Vec<IAssocHandler>, FileSystemError> {
        let wide_ext = wide(&format!(".{extension}"));
        let enumerator =
            unsafe { SHAssocEnumHandlers(PCWSTR(wide_ext.as_ptr()), ASSOC_FILTER_RECOMMENDED) }
                .map_err(|error| {
                    FileSystemError::Internal(format!("fs.open_with_list_failed: {error}"))
                })?;
        let mut out = Vec::new();
        loop {
            let mut batch: [Option<IAssocHandler>; 8] = Default::default();
            let mut fetched = 0u32;
            let failed = unsafe { enumerator.Next(&mut batch, Some(&mut fetched)) }.is_err();
            out.extend(batch[..fetched as usize].iter().flatten().cloned());
            if failed || fetched < batch.len() as u32 {
                break;
            }
        }
        Ok(out)
    }

    pub fn list_apps(path: &Path) -> Result<OpenWithChoices, FileSystemError> {
        let _apartment = CoApartment::enter()?;
        let mut apps = Vec::new();
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        // Directories and extensionless files have no recommended-handler
        // answer; the submenu still offers the native dialog.
        if !path.is_dir() && !extension.is_empty() {
            let mut seen = HashSet::new();
            for handler in handlers(extension)? {
                let exe = handler_exe(&handler);
                let name = match unsafe { handler.GetUIName() } {
                    Ok(raw) => take_string(raw),
                    Err(_) => String::new(),
                };
                if exe.is_empty() || name.is_empty() || !seen.insert(exe.to_lowercase()) {
                    continue;
                }
                apps.push(OpenWithApp {
                    id: exe.clone(),
                    name,
                    icon_name: Some(exe),
                });
            }
        }
        // One flat group: the submenu mirrors Explorer's recommended list, and
        // the native dialog behind "choose another app" owns the full one.
        Ok(OpenWithChoices {
            recommended: apps,
            ..Default::default()
        })
    }

    pub fn open_app(path: &Path, app_id: &str, set_default: bool) -> Result<(), FileSystemError> {
        if set_default {
            // `UserChoice` is hash-protected; only the system dialog may write it.
            return Err(FileSystemError::Unsupported(
                "fs.open_with_default_unsupported".into(),
            ));
        }
        let _apartment = CoApartment::enter()?;
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        // Matched against the handlers the shell itself offered, so the id from
        // the menu can only name an application this list actually showed.
        let handler = handlers(extension)?
            .into_iter()
            .find(|handler| handler_exe(handler).eq_ignore_ascii_case(app_id))
            .ok_or_else(|| FileSystemError::NotFound("fs.open_with_app_missing".into()))?;
        let exe = handler_exe(&handler);
        let exe_path = Path::new(&exe);
        // `IAssocHandler::Invoke` is not usable here: with no default association
        // for the extension it falls back to the system "choose an app" dialog
        // instead of launching the handler it was given.
        if package_root(exe_path).is_some() {
            activate_packaged(exe_path, path)
        } else {
            launch_command(&exe, path)
        }
    }

    /// The package directory an executable lives in, when it is a packaged
    /// (Microsoft Store) app: the ancestor whose parent is `WindowsApps`.
    fn package_root(exe: &Path) -> Option<&Path> {
        exe.ancestors().find(|ancestor| {
            ancestor
                .parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name.eq_ignore_ascii_case("WindowsApps"))
        })
    }

    /// `Name_publisherhash` from a package directory like
    /// `Microsoft.WindowsNotepad_11.2609.11.0_x64__8wekyb3d8bbwe`.
    fn package_family_name(directory: &str) -> Option<String> {
        let (name, rest) = directory.split_once('_')?;
        let (_, publisher) = rest.rsplit_once("__")?;
        Some(format!("{name}_{publisher}"))
    }

    /// The `Id` of the manifest application whose `Executable` is `relative`,
    /// or the first application when none matches.
    fn application_id(manifest: &str, relative: &str) -> Option<String> {
        let mut first = None;
        let mut rest = manifest;
        while let Some(start) = rest.find("<Application ") {
            let tag = &rest[start..];
            let end = tag.find('>')?;
            let tag = &tag[..end];
            rest = &rest[start + end..];
            let id = attribute(tag, "Id")?;
            if first.is_none() {
                first = Some(id.clone());
            }
            if attribute(tag, "Executable")
                .is_some_and(|executable| executable.eq_ignore_ascii_case(relative))
            {
                return Some(id);
            }
        }
        first
    }

    fn attribute(tag: &str, name: &str) -> Option<String> {
        let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
        let rest = &tag[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_owned())
    }

    fn read_manifest(root: &Path) -> Option<String> {
        let bytes = std::fs::read(root.join("AppxManifest.xml")).ok()?;
        if bytes.starts_with(&[0xFF, 0xFE]) {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        } else {
            String::from_utf8(bytes).ok()
        }
    }

    /// Packaged apps cannot be spawned by exe path; the activation manager
    /// launches them by app-user-model id with the file as a command line.
    fn activate_packaged(exe: &Path, file: &Path) -> Result<(), FileSystemError> {
        let failed = || FileSystemError::Internal("fs.open_with_launch_failed".into());
        let root = package_root(exe).ok_or_else(failed)?;
        let family = root
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(package_family_name)
            .ok_or_else(failed)?;
        let relative = exe
            .strip_prefix(root)
            .ok()
            .and_then(|path| path.to_str())
            .map(|path| path.replace('/', "\\"))
            .ok_or_else(failed)?;
        let id = read_manifest(root)
            .and_then(|manifest| application_id(&manifest, &relative))
            .ok_or_else(failed)?;
        let manager: IApplicationActivationManager = unsafe {
            CoCreateInstance(&ACTIVATION_MANAGER_CLSID, None, CLSCTX_ALL)
        }
        .map_err(|error| {
            FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
        })?;
        let aumid = wide(&format!("{family}!{id}"));
        let arguments = wide(&format!("\"{}\"", file.to_string_lossy()));
        unsafe {
            manager.ActivateApplication(
                PCWSTR(aumid.as_ptr()),
                PCWSTR(arguments.as_ptr()),
                AO_NONE,
            )
        }
        .map_err(|error| {
            FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
        })?;
        Ok(())
    }

    /// The registered `open` verb command for a Win32 handler, with `%1`
    /// replaced by the quoted file; falls back to running the exe directly.
    fn launch_command(exe: &str, file: &Path) -> Result<(), FileSystemError> {
        let quoted = format!("\"{}\"", file.to_string_lossy());
        let template = registry_command(exe).unwrap_or_else(|| format!("\"{exe}\" \"%1\""));
        let command = template
            .replace("\"%1\"", &quoted)
            .replace("\"%L\"", &quoted)
            .replace("%1", &quoted)
            .replace("%L", &quoted);
        let argv = split_command(&command);
        let (program, arguments) = argv
            .split_first()
            .ok_or_else(|| FileSystemError::Internal("fs.open_with_launch_failed".into()))?;
        std::process::Command::new(program)
            .args(arguments)
            .spawn()
            .map_err(|error| {
                FileSystemError::Internal(format!("fs.open_with_launch_failed: {error}"))
            })?;
        Ok(())
    }

    fn registry_command(exe: &str) -> Option<String> {
        let name = Path::new(exe).file_name()?.to_str()?;
        let subkey = wide(&format!("Applications\\{name}\\shell\\open\\command"));
        let mut buffer = [0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                PCWSTR(subkey.as_ptr()),
                PCWSTR::null(),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if !status.is_ok() {
            return None;
        }
        let units = (size as usize) / 2;
        let end = buffer[..units]
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(units);
        Some(String::from_utf16_lossy(&buffer[..end]))
    }

    /// Splits a Windows command line on unquoted whitespace, keeping quoted
    /// spans (including empty ones) as single arguments.
    fn split_command(command: &str) -> Vec<String> {
        let mut argv = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut started = false;
        for character in command.chars() {
            match character {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                whitespace if whitespace.is_whitespace() && !quoted => {
                    if started {
                        argv.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                _ => {
                    current.push(character);
                    started = true;
                }
            }
        }
        if started {
            argv.push(current);
        }
        argv
    }

    #[cfg(test)]
    mod tests {
        use super::{application_id, package_family_name, split_command};

        #[test]
        fn command_lines_split_on_unquoted_whitespace_only() {
            assert_eq!(
                split_command("\"C:\\Program Files\\App\\app.exe\" \"C:\\a b\\c.ini\""),
                ["C:\\Program Files\\App\\app.exe", "C:\\a b\\c.ini"]
            );
            assert_eq!(split_command("app.exe -f \"\" x"), ["app.exe", "-f", "", "x"]);
        }

        #[test]
        fn package_family_name_drops_version_and_architecture() {
            assert_eq!(
                package_family_name("Microsoft.WindowsNotepad_11.2609.11.0_x64__8wekyb3d8bbwe")
                    .as_deref(),
                Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe")
            );
            assert_eq!(package_family_name("plain"), None);
        }

        #[test]
        fn application_id_matches_the_executable_and_falls_back_to_the_first() {
            let manifest = "<Package><Applications>\
                <Application Id=\"Other\" Executable=\"Other\\other.exe\" />\
                <Application Id=\"App\" Executable=\"Notepad\\Notepad.exe\" />\
                </Applications></Package>";
            assert_eq!(
                application_id(manifest, "Notepad\\Notepad.exe").as_deref(),
                Some("App")
            );
            assert_eq!(application_id(manifest, "missing.exe").as_deref(), Some("Other"));
        }
    }
}
