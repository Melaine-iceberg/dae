//! macOS right-click commands — the *Services* an application declares in its
//! `Info.plist`, as offered by Finder's own "Services" submenu.
//!
//! # Why Services, and what that leaves out
//!
//! A macOS service is an application's public offer to act on a selection: the
//! app puts an `NSServices` array in its `Info.plist` naming a menu title, the
//! UTIs it accepts and the selector that implements it, and the system adds it
//! to the Services menu. Hosting one costs an `Info.plist` read for discovery and
//! a single `NSPerformService` call to run it — the closest macOS analogue of the
//! Windows verb model its sibling module hosts, which is why the two share a row
//! type and a grouping rule.
//!
//! It is **not** the whole of what a Mac shows on a right-click, and the gap is
//! not something this module can close:
//!
//! - **Finder Sync extensions** — the per-folder menu items Dropbox, Figma and
//!   GitHub Desktop ship — are loaded by Finder and by Finder alone. Apple offers
//!   no way for a third-party host to enumerate them and no way to load one, so
//!   this section is narrower than Finder's menu and no amount of work here
//!   changes that.
//! - **Automator Quick Actions** live in `~/Library/Services`, are bundles like
//!   any other, and declare their actions as services — so they *are* picked up,
//!   provided the user saved one as a service rather than as a workflow.
//!
//! # Threading
//!
//! `NSPerformService` is dispatched through `AppHandle::run_on_main_thread`:
//! AppKit reaches the services machinery through the application object, which
//! exists only on the main thread, and this module will not call it on any other
//! one.
//!
//! That dispatch is deliberately **not** awaited. `NSPerformService` blocks until
//! the service finishes, and a service may take as long as it likes — an
//! Automator action can run for minutes. Waiting would freeze dae's event loop
//! for that whole time, so the call is handed to the main queue and `invoke`
//! returns as soon as it is queued. The cost of that choice: the `bool`
//! `NSPerformService` returns is logged rather than reported, so a service that
//! fails to run appears in dae's log and not as an error in the UI. Everything
//! checkable before dispatch — that the bundle still exists, that the declaration
//! is still there, that it still has a title — *is* checked, and those failures
//! do reach the user.
//!
//! # Validation
//!
//! The parsing and id handling live in [`super::plist`], a host-independent
//! module, so their tests run anywhere. What is left here is framework calls and
//! directory walking, and the tests below cover the parts of *that* which are
//! pure logic rather than the parts that need a Mac.

use super::plist::{
    FOLDER_TYPE, ServiceDeclaration, UNKNOWN_FILE_TYPE, bundle_name, contains, id_of, is_dynamic,
    parse_services, split_id,
};
use super::{Selection, SelectionKind, ShellCommand, assign_groups};
use crate::file_system::error::FileSystemError;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a scan of the installed bundles is reused. Installing an application
/// is rare, and Finder itself does not pick up a new service without being
/// restarted.
const SCAN_TTL: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Where application and service bundles live.
///
/// The system locations are spelled out rather than discovered through
/// LaunchServices: `LSCopyApplicationURLsForURL` answers for a given *file*, not
/// for "everything installed", and enumerating the three `Applications` folders
/// plus the two `Services` folders is both complete in practice and far cheaper.
fn bundle_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/System/Library/CoreServices"),
        PathBuf::from("/System/Library/CoreServices/Applications"),
        // The two legacy service locations: the built-in services, and where a
        // user's own Automator actions land.
        PathBuf::from("/System/Library/Services"),
        PathBuf::from("/Library/Services"),
    ];
    let home = home_dir();
    roots.push(home.join("Applications"));
    roots.push(home.join("Library/Services"));
    roots
}

/// One bundle that declares at least one service.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceBundle {
    /// The bundle on disk, absolute. Half of every id its declarations produce.
    path: PathBuf,
    /// The bundle's display name, used as the flyout label when it contributes
    /// more than one command.
    name: String,
    services: Vec<ServiceDeclaration>,
}

/// Renders one bundle's `Info.plist` as JSON, or `None` when it declares no
/// services.
///
/// The byte pre-filter is what makes a full scan affordable: almost every
/// application has an `Info.plist` and almost none declares a service, so the
/// check reduces the cost to one `read` per bundle, plus a `plutil` call for the
/// handful that do.
///
/// JSON is the format `serde_json` parses and a service declaration needs
/// nothing but dictionaries, arrays, strings and booleans. `plutil` refuses the
/// conversion for a plist holding a `<data>` or `<date>`, which drops that one
/// bundle rather than the scan — see [`super::plist`] for why that trade is
/// made.
fn read_info_plist(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents/Info.plist");
    let bytes = std::fs::read(&plist).ok()?;
    if !contains(&bytes, b"NSServices") {
        return None;
    }

    // The system's own converter rather than a plist crate: this is the program
    // that produced the file's encoding, so it cannot disagree with it.
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Reads every bundle that declares a service. Real I/O, so callers keep it off
/// the async command thread and behind the cache below.
fn scan() -> Vec<ServiceBundle> {
    let language = user_language();
    let mut bundles = Vec::new();

    for root in bundle_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path.extension().is_some_and(|extension| {
                        matches!(extension.to_str(), Some("app" | "service" | "workflow"))
                    })
            })
            .collect();
        // `read_dir` returns an unspecified order; sorting keeps the section
        // stable between right-clicks and between runs.
        paths.sort();

        for path in paths {
            // An unreadable bundle is normal — a half-finished install, a
            // quarantined download — and must not hide the rest.
            let Some(declarations) = read_info_plist(&path) else {
                continue;
            };
            let services = parse_services(&declarations, language.as_deref());
            if services.is_empty() {
                continue;
            }
            bundles.push(ServiceBundle {
                name: bundle_name(&path),
                path,
                services,
            });
        }
    }

    bundles
}

/// The scan, cached. Mirrors the other platforms': a right-click must not
/// re-read every `Info.plist` on the machine, and an application installed while
/// dae runs should still appear without a restart.
fn service_bundles() -> Vec<ServiceBundle> {
    type ScanCache = Option<(Instant, Vec<ServiceBundle>)>;
    static CACHE: LazyLock<Mutex<ScanCache>> = LazyLock::new(|| Mutex::new(None));

    let mut cached = CACHE.lock().expect("service bundle cache poisoned");
    if let Some((scanned_at, bundles)) = cached.as_ref()
        && scanned_at.elapsed() < SCAN_TTL
    {
        return bundles.clone();
    }

    let bundles = scan();
    *cached = Some((Instant::now(), bundles.clone()));
    bundles
}

/// The user's language in the `ll_CC` form an `NSMenuItem` dictionary's keys use.
///
/// The environment is checked first, because a build that is launched from a
/// terminal carries the locale there. It is *not* enough on its own: an
/// application launched from the Dock inherits no locale variables at all, which
/// is the ordinary case on this platform, so the system's own preference order
/// is the fallback rather than the exception.
fn user_language() -> Option<String> {
    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable)
            && !value.is_empty()
            && value != "C"
            && value != "POSIX"
        {
            // `zh_CN.UTF-8@variant` → `zh_CN`
            let value = value.split('.').next().unwrap_or(&value);
            let value = value.split('@').next().unwrap_or(value);
            return Some(value.to_string());
        }
    }

    use objc2_foundation::NSLocale;

    let preferred = NSLocale::preferredLanguages();
    preferred.firstObject().map(|language| language.to_string())
}

// ---------------------------------------------------------------------------
// UTI matching
// ---------------------------------------------------------------------------

/// The most specific type that can be claimed for a selection.
fn concrete_type(path: &str, kind: SelectionKind) -> String {
    if kind == SelectionKind::Directory {
        return FOLDER_TYPE.to_string();
    }

    // A file with no extension has no derivable type; neither has one whose
    // extension LaunchServices does not recognize.
    let Some(extension) = Path::new(path).extension() else {
        return UNKNOWN_FILE_TYPE.to_string();
    };

    match uti_identifier(&extension.to_string_lossy()) {
        // A `dyn.` identifier is LaunchServices saying "I have no declaration
        // for this". It would conform to nearly everything, which is not a
        // useful claim, so the honest narrower one is made instead.
        Some(identifier) if !is_dynamic(&identifier) => identifier,
        _ => UNKNOWN_FILE_TYPE.to_string(),
    }
}

/// The UTI LaunchServices derives from a filename extension.
fn uti_identifier(extension: &str) -> Option<String> {
    use objc2_foundation::NSString;
    use objc2_uniform_type_identifiers::UTType;

    let concrete = UTType::typeWithFilenameExtension(&NSString::from_str(extension))?;
    Some(concrete.identifier().to_string())
}

/// Whether a declaration covers the selection.
///
/// Compared by UTI conformance rather than string equality, which is the whole
/// reason for describing types this way: a service declaring `public.image`
/// matches a `.png` file and one declaring `public.item` matches a folder,
/// without either side enumerating the other's members.
fn matches(declaration: &ServiceDeclaration, path: &str, kind: SelectionKind) -> bool {
    let concrete = concrete_type(path, kind);
    declaration
        .send_file_types
        .iter()
        .any(|declared| conforms(&concrete, declared))
}

/// Whether the concrete type conforms to a declared one.
///
/// A UTI that cannot be resolved makes the comparison `false` rather than an
/// error: the declaration is then simply not offered, which is what the Windows
/// module does with an item type it cannot place.
fn conforms(concrete: &str, declared: &str) -> bool {
    use objc2_foundation::NSString;
    use objc2_uniform_type_identifiers::UTType;

    let Some(concrete) = UTType::typeWithIdentifier(&NSString::from_str(concrete)) else {
        return false;
    };
    let Some(declared) = UTType::typeWithIdentifier(&NSString::from_str(declared)) else {
        return false;
    };
    concrete.conformsToType(&declared)
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// Hands the selection to a service through a private pasteboard.
///
/// A unique-named pasteboard, not the general one: a service must not be able to
/// read what the user has on their clipboard, and running a command must not
/// replace it.
///
/// Must run on the main thread — see the module docs. The returned `bool` is
/// `NSPerformService`'s answer to whether the service ran.
fn perform_service(item_name: &str, paths: &[String]) -> bool {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting, NSPerformService};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let pasteboard = NSPasteboard::pasteboardWithUniqueName();
    pasteboard.clearContents();

    let urls: Vec<Retained<NSURL>> = paths
        .iter()
        .map(|path| NSURL::fileURLWithPath(&NSString::from_str(path)))
        .collect();
    let writable: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = urls
        .iter()
        .map(|url| ProtocolObject::from_retained(url.clone()))
        .collect();
    pasteboard.writeObjects(&NSArray::from_retained_slice(&writable));

    NSPerformService(&NSString::from_str(item_name), Some(&pasteboard))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Primes the scan so the first right-click does not pay for it.
///
/// On macOS the cold cost is reading the installed bundles' `Info.plist` files:
/// a few hundred reads plus a `plutil` call for each bundle that mentions
/// services. Unlike Windows there are no COM servers to start, so the warm-up is
/// a single pass and nothing stays resident afterwards.
pub(super) async fn warm() -> Result<(), FileSystemError> {
    let _ = tauri::async_runtime::spawn_blocking(service_bundles).await;
    Ok(())
}

pub(super) async fn list(
    paths: Vec<String>,
    primary: String,
) -> Result<Vec<ShellCommand>, FileSystemError> {
    // Discovery and UTI conformance are both synchronous; only the finished rows
    // cross back to the async thread.
    let rows = tauri::async_runtime::spawn_blocking(move || {
        let selection = Selection::for_paths(&primary, &paths)?;

        let mut rows = Vec::new();
        for bundle in service_bundles() {
            for (index, declaration) in bundle.services.iter().enumerate() {
                if !matches(declaration, &primary, selection.kind) {
                    continue;
                }
                rows.push((
                    bundle.name.clone(),
                    ShellCommand {
                        id: id_of(&bundle.path, index),
                        label: declaration.title.clone(),
                        // An `NSServices` declaration carries no icon, so the
                        // menu draws dae's own glyph for every row here.
                        icon_data_url: None,
                        group: None,
                        // Neither concept exists in an `NSServices` declaration.
                        separator_before: false,
                        disabled: false,
                    },
                ));
            }
        }
        Ok::<_, FileSystemError>(rows)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    if rows.is_empty() {
        return Ok(Vec::new());
    }
    Ok(assign_groups(rows))
}

pub(super) async fn invoke(
    app: &tauri::AppHandle,
    id: String,
    paths: Vec<String>,
) -> Result<(), FileSystemError> {
    let Some((bundle, index)) = split_id(&id) else {
        return Err(FileSystemError::InvalidInput(format!(
            "fs.shell_command_unknown: {id}"
        )));
    };
    let bundle = bundle.to_string();

    // Re-resolve the declaration rather than trusting a cached copy: the id
    // round-trips through the webview and, more to the point, `NSPerformService`
    // is addressed by *title*, so the title is what has to be recovered.
    // Owned by the closure rather than borrowed from `id`: the closure is moved
    // into a blocking task, and `id` does not outlive it.
    let unknown_message = id.clone();
    let unknown = move || {
        FileSystemError::InvalidInput(format!("fs.shell_command_unknown: {unknown_message}"))
    };
    let title = tauri::async_runtime::spawn_blocking(move || {
        let bundle = PathBuf::from(&bundle);
        // Called through a borrow at each site rather than handed over once:
        // both failures here report the same "unknown id", and `ok_or_else`
        // consumes its argument.
        let declarations = read_info_plist(&bundle).ok_or_else(|| unknown())?;
        parse_services(&declarations, user_language().as_deref())
            .into_iter()
            .nth(index)
            .map(|declaration| declaration.title)
            .ok_or_else(|| unknown())
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    // Queued, not awaited — see the module docs for why the result of a slow
    // service must not be waited on from dae's event loop.
    app.run_on_main_thread(move || {
        if !perform_service(&title, &paths) {
            log::warn!("The service {title} did not run");
        }
    })
    .map_err(|error| FileSystemError::Internal(format!("fs.shell_command_dispatch: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_bundle_roots_of_this_machine() {
        // Every root has to be absolute, or the scan silently reads nothing, and
        // the two system-wide locations have to be in the list.
        let roots = bundle_roots();
        assert!(roots.iter().all(|root| root.is_absolute()));
        assert!(roots.contains(&PathBuf::from("/Applications")));
        assert!(roots.contains(&PathBuf::from("/System/Applications")));
        assert!(roots.contains(&PathBuf::from("/System/Library/Services")));
    }

    #[test]
    fn claims_the_narrowest_type_a_selection_supports() {
        // A directory is always `public.folder` and never claims to be a file,
        // which is what keeps a folder from being offered a file-only service.
        assert_eq!(
            concrete_type("/tmp/dir", SelectionKind::Directory),
            FOLDER_TYPE
        );
        // A file with no extension makes no claim beyond "some data".
        assert_eq!(
            concrete_type("/tmp/LICENSE", SelectionKind::File),
            UNKNOWN_FILE_TYPE
        );

        // A known extension resolves through LaunchServices. Asserted as a
        // property rather than an exact UTI, because the declaration is the
        // system's to make and can change between releases.
        let png = concrete_type("/tmp/a.png", SelectionKind::File);
        assert!(!is_dynamic(&png), "png resolved to {png}");
        assert!(
            conforms(&png, "public.image"),
            "png did not conform to public.image"
        );
        assert!(
            conforms(&png, "public.item"),
            "png did not conform to public.item"
        );
    }

    #[test]
    fn falls_back_to_unknown_file_for_an_untyped_extension() {
        let guess = concrete_type("/tmp/a.zzz-no-such-extension", SelectionKind::File);
        assert_eq!(guess, UNKNOWN_FILE_TYPE);
    }

    #[test]
    fn conforms_declared_types_to_the_hierarchy() {
        // The two relations the section depends on: a folder is an item, and an
        // image is a file rather than a folder.
        assert!(conforms(FOLDER_TYPE, "public.item"));
        assert!(conforms(FOLDER_TYPE, FOLDER_TYPE));
        assert!(!conforms(FOLDER_TYPE, "public.data"));

        assert!(conforms("public.image", "public.image"));
        assert!(!conforms("public.image", FOLDER_TYPE));

        // A declared type the system does not know resolves to nothing, so the
        // command is dropped rather than offered against everything.
        assert!(!conforms("public.image", "com.example.no-such-uti"));
        assert!(!conforms("com.example.no-such-uti", "public.item"));
    }

    #[test]
    fn picks_a_language_for_the_menu_titles() {
        // Whatever this returns is used as an `NSMenuItem` dictionary key, so it
        // must be a bare `ll` or `ll_CC` with no codeset or variant attached —
        // and it must never be one of the placeholder locales.
        if let Some(language) = user_language() {
            assert!(!language.contains('.'), "kept its codeset: {language}");
            assert!(!language.contains('@'), "kept its variant: {language}");
            assert!(!language.is_empty());
            assert_ne!(language, "C");
            assert_ne!(language, "POSIX");
        }
    }
}
