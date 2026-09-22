//! Linux right-click commands — KDE's *service menus*, the `kio/servicemenus`
//! `.desktop` files that Dolphin and other KIO-based file managers render.
//!
//! # Why this mechanism and not another
//!
//! Linux has no OS-wide context-menu extension point. Every file manager
//! defines its own: Nautilus loads in-process C/Python extensions, Thunar reads
//! its own `uca.xml`, Nemo has another format again. None is hostable by a
//! foreign process and none is shared.
//!
//! KDE's service menus are the exception that matters, for two reasons:
//!
//! 1. **They are declarative.** A service menu is an XDG desktop entry with a
//!    `MimeType` list, an `Actions` list and one `[Desktop Action …]` group per
//!    action — structurally the same shape as the Windows verb model this
//!    module's sibling hosts, a type filter plus a runnable command. That is why
//!    the two share a row type and a grouping rule.
//! 2. **They are self-contained.** An action's `Exec` is a complete command
//!    line, so hosting one costs a `fork`/`exec` and nothing else. On this
//!    platform no foreign code ever enters dae's address space.
//!
//! The trade-off is coverage: a service menu exists only if the user installed
//! the package that ships it, and those are mostly KDE-adjacent (`ark`,
//! `kio-extras`, `dolphin-plugins`). On a GNOME install the section will often be
//! empty — the honest answer rather than a bug.
//!
//! # What this module does not do
//!
//! - **Other file managers' mechanisms.** Nautilus scripts, Thunar custom
//!   actions and the dead `file-manager/actions` format are not read; each is a
//!   separate format with separate semantics and would need its own parser.
//! - **MIME inheritance.** A menu declaring `text/plain` does not match a `.py`
//!   file, though shared-mime-info says it should. See
//!   [`desktop_entry::mime_matches`].
//! - **`Terminal=true`.** KDE runs such an action inside the user's configured
//!   terminal emulator. dae has its own terminal panel, which is not the same
//!   thing, so the command runs without one: the action still works, but a
//!   command-line tool that expects a tty will not.
//!
//! # Where the parsing lives
//!
//! Everything except discovery, spawning and icon lookup is in
//! [`super::desktop_entry`] — a platform-independent module, so its tests run on
//! any host. That split is load-bearing: this file cannot be compiled on the
//! machine it was written on, and argument expansion is exactly the sort of code
//! that is silently wrong rather than obviously broken.

use super::desktop_entry::{
    ANY_FILE, ServiceMenu, expand_exec, mime_matches, parse_service_menu, split_id, user_language,
};
use super::{Selection, SelectionKind, ShellCommand, assign_groups};
use crate::file_system::error::FileSystemError;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a scan of the service-menu directories is reused. An installer
/// dropping a new menu in is rare, and re-reading a few dozen small files on
/// every right-click is not worth it.
const SCAN_TTL: Duration = Duration::from_secs(300);

/// Subdirectories of an icon theme, largest first. A menu row renders at 16px,
/// but scaled displays are common enough that a 48px source is worth preferring
/// over a 16px one.
const ICON_SIZES: &[&str] = &["48x48", "64x64", "32x32", "128x128", "256x256", "scalable"];
/// Themes to try, in the order KDE and GNOME fall back.
const ICON_THEMES: &[&str] = &["hicolor", "Adwaita", "breeze"];

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The XDG data roots, most specific first: `$XDG_DATA_HOME`, then each
/// `$XDG_DATA_DIRS` entry, each defaulted as the spec prescribes.
fn data_roots() -> Vec<PathBuf> {
    resolve_data_roots(
        std::env::var_os("XDG_DATA_HOME").as_deref(),
        std::env::var_os("XDG_DATA_DIRS").as_deref(),
    )
}

/// The root list, from the raw environment values.
///
/// Split from the reads above so the *rules* can be tested rather than the
/// host's environment. The spec is explicit about two cases that are silent
/// failures when unhandled, both of which yield a root list that is quietly
/// wrong instead of empty:
///
/// - An unset **or empty** `XDG_DATA_HOME` means `$HOME/.local/share`. An empty
///   value taken at face value becomes the empty path, so icon lookups would
///   scan `icons/…` relative to the working directory and find nothing.
/// - A relative path in either variable is invalid and is ignored, rather than
///   resolved against the working directory.
fn resolve_data_roots(data_home: Option<&OsStr>, data_dirs: Option<&OsStr>) -> Vec<PathBuf> {
    let mut roots = vec![
        data_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home_dir().join(".local/share")),
    ];

    match data_dirs {
        Some(value) if !value.is_empty() => roots.extend(
            value
                .to_str()
                .into_iter()
                .flat_map(|value| value.split(':'))
                .filter(|entry| !entry.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
        ),
        _ => roots.extend([
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ]),
    }

    roots
}

/// The service-menu directories, most specific first.
///
/// The order is the XDG one, so a user's own menu overrides a packaged menu of
/// the same name — the rule every other XDG consumer follows. Both the current
/// `kio/servicemenus` layout and the pre-5.85 `kservices5` one are read, because
/// packages are still installed against the latter.
fn service_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in data_roots() {
        for suffix in ["kio/servicemenus", "kservices5/ServiceMenus", "kservices5"] {
            dirs.push(root.join(suffix));
        }
    }
    dirs
}

/// Reads every service menu on the machine. Real I/O, so callers keep it off the
/// async command thread and behind the cache below.
fn scan() -> Vec<ServiceMenu> {
    let language = user_language();
    let mut menus = Vec::new();
    // A filename seen once wins: earlier roots are the more specific ones.
    let mut seen = HashSet::new();

    for dir in service_menu_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "desktop")
            })
            .collect();
        // `read_dir` returns an unspecified order; sorting keeps the section
        // stable between right-clicks and between runs.
        files.sort();

        for file in files {
            let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            // A service menu may legitimately be root-owned and unreadable in a
            // half-installed package; one broken file must not hide the rest.
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(menu) = parse_service_menu(&file, &text, language.as_deref()) {
                menus.push(menu);
            }
        }
    }

    menus
}

/// The scan, cached. Mirrors the Windows module's: a right-click must not
/// re-read every file, and a menu installed while dae runs should still appear
/// without a restart.
fn service_menus() -> Vec<ServiceMenu> {
    type ScanCache = Option<(Instant, Vec<ServiceMenu>)>;
    static CACHE: LazyLock<Mutex<ScanCache>> = LazyLock::new(|| Mutex::new(None));

    let mut cached = CACHE.lock().expect("service menu cache poisoned");
    if let Some((scanned_at, menus)) = cached.as_ref()
        && scanned_at.elapsed() < SCAN_TTL
    {
        return menus.clone();
    }

    let menus = scan();
    *cached = Some((Instant::now(), menus.clone()));
    menus
}

/// The MIME type of the selection, for matching against a menu's `MimeType`
/// list.
///
/// A file whose type cannot be guessed from its name reports
/// `application/octet-stream`, which is both what it is and — through
/// [`mime_matches`] — the value that matches every file-scoped menu, which is
/// the right outcome for a file with no extension whose contents are unknown.
fn mime_of(path: &str, kind: SelectionKind) -> String {
    if kind == SelectionKind::Directory {
        return "inode/directory".to_string();
    }
    mime_guess::from_path(path).first().map_or_else(
        || ANY_FILE.to_string(),
        |mime| mime.essence_str().to_string(),
    )
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Resolves an `Icon=` value to a `data:` URL.
///
/// The value is either an absolute path or a themed icon name. `xpm` is
/// deliberately absent from the search: no webview renders it, and a row with a
/// broken image is worse than one with dae's own glyph.
fn resolve_icon(value: &str) -> Option<String> {
    if value.is_empty() || value == "-" {
        return None;
    }
    if value.starts_with('/') {
        return read_icon(Path::new(value));
    }

    for root in data_roots() {
        let icons = root.join("icons");
        for theme in ICON_THEMES {
            for size in ICON_SIZES {
                for extension in ["svg", "png"] {
                    let candidate = icons
                        .join(theme)
                        .join(size)
                        .join("apps")
                        .join(format!("{value}.{extension}"));
                    if let Some(resolved) = read_icon(&candidate) {
                        return Some(resolved);
                    }
                }
            }
        }
    }

    // Last resort: `/usr/share/pixmaps`, where packages without a theme put
    // their icons and which has no theme or size structure at all.
    for extension in ["svg", "png"] {
        if let Some(resolved) =
            read_icon(&Path::new("/usr/share/pixmaps").join(format!("{value}.{extension}")))
        {
            return Some(resolved);
        }
    }

    None
}

/// Reads an icon file as a `data:` URL with the right MIME type, so the SVG case
/// renders as vector rather than as a missing image.
fn read_icon(path: &Path) -> Option<String> {
    use base64::Engine as _;

    let mime = match path.extension().and_then(|extension| extension.to_str()) {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Primes the scan so the first right-click does not pay for it.
///
/// Much cheaper than the Windows warm-up: there are no COM servers to start and
/// no surrogate processes to keep alive, so the only cold cost is reading the
/// service-menu directories.
pub(super) async fn warm() -> Result<(), FileSystemError> {
    let _ = tauri::async_runtime::spawn_blocking(service_menus).await;
    Ok(())
}

pub(super) async fn list(
    paths: Vec<String>,
    primary: String,
) -> Result<Vec<ShellCommand>, FileSystemError> {
    // Discovery, MIME matching, icon lookup and grouping are all synchronous
    // file-system work; only the reply crosses back to the async thread.
    let rows = tauri::async_runtime::spawn_blocking(move || {
        let selection = Selection::for_paths(&primary, &paths)?;
        let mime = mime_of(&primary, selection.kind);

        let mut rows = Vec::new();
        for menu in service_menus() {
            if !menu
                .mime_types
                .iter()
                .any(|declared| mime_matches(declared, &mime, selection.kind))
            {
                continue;
            }
            for action in menu.actions {
                rows.push((
                    menu.name.clone(),
                    ShellCommand {
                        id: super::desktop_entry::id_of(&menu.path, &action.id),
                        label: action.name,
                        icon_data_url: action.icon.as_deref().and_then(resolve_icon),
                        group: None,
                        // Neither concept exists in a `.desktop` service menu,
                        // so the frontend's separator and disabled paths stay
                        // off on this platform.
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
    _app: &tauri::AppHandle,
    id: String,
    paths: Vec<String>,
) -> Result<(), FileSystemError> {
    let Some((desktop_file, action_id)) = split_id(&id) else {
        return Err(FileSystemError::InvalidInput(format!(
            "fs.shell_command_unknown: {id}"
        )));
    };
    let desktop_file = desktop_file.to_string();
    let action_id = action_id.to_string();

    // Re-read the declaring file rather than trusting a scanned copy: the id
    // round-trips through the webview, and the file may have been changed since
    // the menu was built. `%k` wants this path too, so it is already in hand.
    // Owned by the closure rather than borrowed from `id`, which is still needed
    // below for the other failure this can report.
    let unknown_message = id.clone();
    let unknown = move || {
        FileSystemError::InvalidInput(format!("fs.shell_command_unknown: {unknown_message}"))
    };
    let (name, exec) = tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&desktop_file);
        // A menu that vanished between listing and invoking is an ordinary
        // race, not an internal error.
        let text = std::fs::read_to_string(&path).map_err(|_| unknown())?;
        // Called through a borrow at every site rather than handed over once:
        // three different failures here report the same "unknown id", and
        // `ok_or_else` consumes its argument.
        let menu = parse_service_menu(&path, &text, user_language().as_deref())
            .ok_or_else(|| unknown())?;
        menu.actions
            .into_iter()
            .find(|action| action.id == action_id)
            .map(|action| (action.name, action.exec))
            .ok_or_else(|| unknown())
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    let arguments = expand_exec(&exec, &paths, &name, &id);
    let Some((program, rest)) = arguments.split_first() else {
        return Err(FileSystemError::InvalidInput(format!(
            "fs.shell_command_empty_exec: {id}"
        )));
    };

    // Deliberately not through a shell: the `Exec` line is data from a
    // `.desktop` file, and giving it shell semantics would let a selected
    // filename containing `;` or `$(…)` change what runs. The spec's own field
    // codes are the only substitution performed.
    match std::process::Command::new(program).args(rest).spawn() {
        Ok(mut child) => {
            // Reaped on a detached thread. Without this the child stays a
            // zombie for as long as dae runs, once per command the user starts.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        Err(error) => Err(FileSystemError::Internal(format!(
            "fs.shell_command_spawn: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_an_unknown_icon_name_to_nothing() {
        // A name that cannot be on any machine, so this exercises the miss path
        // rather than depending on the host's icon set.
        assert_eq!(resolve_icon("dae-no-such-icon-name-zzz"), None);
        assert_eq!(resolve_icon(""), None);
        assert_eq!(resolve_icon("-"), None);
        assert_eq!(resolve_icon("/nonexistent/path/icon.png"), None);
    }

    #[test]
    fn reads_a_theme_root_from_the_data_directories() {
        // The rules, not the machine's environment. Asserted through
        // `resolve_data_roots` because the interesting inputs are the ones a
        // working desktop never has — `XDG_DATA_HOME` set to nothing is the
        // ordinary way to unset it, and it must not become the empty path.
        //
        // "Rooted" satisfies the shape check as well as "absolute" does: on a
        // Unix host the defaults are absolute, and the fallbacks are `/`-rooted
        // paths whose absoluteness is the host's business, not this module's.
        let rooted = |roots: &[PathBuf]| {
            roots
                .iter()
                .all(|root| !root.as_os_str().is_empty() && (root.is_absolute() || root.has_root()))
        };

        // Nothing set: `$HOME/.local/share` plus the two documented defaults.
        let unset = resolve_data_roots(None, None);
        assert!(rooted(&unset));
        assert!(unset.contains(&PathBuf::from("/usr/share")));

        // Set to the empty string, which the spec reads as unset.
        let empty = resolve_data_roots(Some(OsStr::new("")), Some(OsStr::new("")));
        assert!(rooted(&empty));
        assert!(empty.contains(&PathBuf::from("/usr/share")));

        // A relative element is invalid and dropped rather than resolved
        // against the working directory.
        let relative = resolve_data_roots(
            Some(OsStr::new("share")),
            Some(OsStr::new("share:/usr/share")),
        );
        assert!(rooted(&relative));
        assert!(!relative.contains(&PathBuf::from("share")));

        // What the absolute entries around it do is a Unix question: XDG paths
        // are Unix paths, and on a Windows host — where this module's tests are
        // compiled only to type-check it — `/usr/share` is root-relative and the
        // "ignore relative entries" rule drops it along with the genuinely
        // relative one. Asserted where the rule has its real meaning.
        #[cfg(unix)]
        {
            assert!(relative.contains(&PathBuf::from("/usr/share")));
            assert!(relative.iter().all(|root| root.is_absolute()));
        }
    }

    #[test]
    fn classifies_the_selection_before_matching_mime_types() {
        // A directory is `inode/directory` regardless of its name, which is what
        // keeps a folder from matching a file-scoped `application/octet-stream`
        // menu.
        assert_eq!(
            mime_of("/tmp/some-folder", SelectionKind::Directory),
            "inode/directory"
        );
        // A typed file reports its own type; an untyped one reports the
        // wildcard KDE menus treat as "any file".
        assert_eq!(mime_of("/tmp/a.png", SelectionKind::File), "image/png");
        assert_eq!(mime_of("/tmp/LICENSE", SelectionKind::File), ANY_FILE);
    }
}
