//! Where the app keeps its state, resolved without a window handle.
//!
//! Tauri derived this from the bundle identifier (`tauri.conf.json` →
//! `identifier`), and the frontend never saw it. Both shells have to resolve the
//! *same* directory, because this is where `settings.toml`, the favourites, the
//! recent list, the saved connections and the known-host pins live: a shell that
//! computed it differently would look like a fresh install after an upgrade and
//! silently strand every one of them. So the rule is here, in one place, instead
//! of being re-derived per shell.
//!
//! # What the rule is
//!
//! `{system config dir}/{identifier}`, where the system config dir is the same
//! one the `dirs` crate (and therefore Tauri) uses:
//!
//! | Platform | Root |
//! |---|---|
//! | Windows | `%APPDATA%` — the roaming profile, not `%LOCALAPPDATA%` |
//! | macOS | `$HOME/Library/Application Support` |
//! | Linux/BSD | `$XDG_CONFIG_HOME` when absolute, else `$HOME/.config` |
//!
//! Each branch's *rule* is a separate pure function, so it can be tested against
//! synthetic input rather than whichever machine runs the tests — the same split
//! `xdg.rs` uses for datadir resolution, and for the same reason.

use crate::error::FileSystemError;
use std::ffi::OsStr;
use std::path::PathBuf;

/// Must equal `identifier` in `src-tauri/tauri.conf.json` — `the_identifier_matches_the_bundle`
/// fails if the two drift apart.
pub const APP_IDENTIFIER: &str = "com.melaine.dae";

/// The app's config directory. Every persisted document hangs off this.
pub fn config_dir() -> Result<PathBuf, FileSystemError> {
    let root = config_root().ok_or_else(|| {
        FileSystemError::Internal(
            "the environment carries no system config directory".to_owned(),
        )
    })?;

    Ok(root.join(APP_IDENTIFIER))
}

#[cfg(windows)]
fn config_root() -> Option<PathBuf> {
    resolve_config_root(std::env::var_os("APPDATA").as_deref())
}

#[cfg(target_os = "macos")]
fn config_root() -> Option<PathBuf> {
    resolve_config_root(std::env::var_os("HOME").as_deref())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn config_root() -> Option<PathBuf> {
    resolve_config_root(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Windows' root: the roaming app data directory, verbatim.
#[cfg(windows)]
pub fn resolve_config_root(appdata: Option<&OsStr>) -> Option<PathBuf> {
    appdata.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// macOS' root: `Application Support` inside the home directory.
#[cfg(target_os = "macos")]
pub fn resolve_config_root(home: Option<&OsStr>) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join("Library").join("Application Support"))
}

/// Linux/BSD's root: an **absolute** `$XDG_CONFIG_HOME`, else `$HOME/.config`.
///
/// The spec's two silent-failure cases are both handled the way `xdg.rs` handles
/// them: an empty value is unset, and a relative value is ignored rather than
/// resolved against the working directory.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn resolve_config_root(
    config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(config_home) = config_home {
        let path = PathBuf::from(config_home);
        if path.is_absolute() {
            return Some(path);
        }
    }

    home.filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the module: a drift between this constant and the
    /// bundle is a silent "fresh install" for every existing user.
    #[test]
    fn the_identifier_matches_the_bundle() {
        let conf = include_str!("../../../src-tauri/tauri.conf.json");

        assert!(
            conf.contains(&format!("\"identifier\": \"{APP_IDENTIFIER}\"")),
            "APP_IDENTIFIER drifted from tauri.conf.json"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_root_is_roaming_appdata() {
        assert_eq!(
            resolve_config_root(Some(OsStr::new(r"C:\Users\me\AppData\Roaming"))),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming"))
        );
        assert_eq!(resolve_config_root(Some(OsStr::new(""))), None);
        assert_eq!(resolve_config_root(None), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_root_is_application_support() {
        assert_eq!(
            resolve_config_root(Some(OsStr::new("/Users/me"))),
            Some(PathBuf::from("/Users/me/Library/Application Support"))
        );
        assert_eq!(resolve_config_root(None), None);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn a_relative_xdg_config_home_is_ignored_not_resolved() {
        assert_eq!(
            resolve_config_root(Some(OsStr::new("relative/dir")), Some(OsStr::new("/home/me"))),
            Some(PathBuf::from("/home/me/.config"))
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn an_absolute_xdg_config_home_wins() {
        assert_eq!(
            resolve_config_root(Some(OsStr::new("/custom/config")), Some(OsStr::new("/home/me"))),
            Some(PathBuf::from("/custom/config"))
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn a_missing_home_leaves_no_config_dir() {
        assert_eq!(resolve_config_root(None, None), None);
        assert_eq!(resolve_config_root(Some(OsStr::new("")), None), None);
    }
}
