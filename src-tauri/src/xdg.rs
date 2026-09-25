//! XDG base-directory resolution, shared by every consumer that reads data
//! dropped outside the app's own directory tree.
//!
//! Two modules need it today: `shell_commands::linux` finds the `.desktop`
//! service menus a packaged application installed, and `file_icons::linux`
//! finds the icon themes. Both had to implement the same spec rules, and the
//! rules are the sort that fail quietly — an unset `XDG_DATA_HOME` that reads
//! as the empty path turns a theme lookup into a scan of the working directory
//! and yields *nothing* rather than an error.
//!
//! Pure path arithmetic, so it compiles and tests on any host: which is the
//! same reason `shell_commands::desktop_entry` is platform-independent.
//!
//! Both consumers are Linux-only, so on the other two platforms every function
//! here is dead until `cargo test` builds the Linux module to type-check it —
//! which is exactly the arrangement, and why the warning is silenced rather than
//! the module cfg-gated away.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsStr;
use std::path::PathBuf;

/// `$HOME`, or `/` when the environment does not carry a usable one.
pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The XDG data roots, most specific first: `$XDG_DATA_HOME`, then each
/// `$XDG_DATA_DIRS` entry, each defaulted as the spec prescribes.
pub(crate) fn data_roots() -> Vec<PathBuf> {
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
///   value taken at face value becomes the empty path, so a lookup would scan
///   `icons/…` relative to the working directory and find nothing.
/// - A relative path in either variable is invalid and is ignored, rather than
///   resolved against the working directory.
pub(crate) fn resolve_data_roots(
    data_home: Option<&OsStr>,
    data_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
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

/// `$XDG_CONFIG_HOME`, defaulting to `$HOME/.config`.
pub(crate) fn config_roots() -> Vec<PathBuf> {
    resolve_config_roots(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// See [`resolve_data_roots`] for why the two environment reads are split out.
/// The rules are the same; only the defaults differ — an unset or empty
/// `XDG_CONFIG_HOME` means `$HOME/.config`, and a missing `$HOME` leaves no
/// user config directory at all rather than inventing one.
pub(crate) fn resolve_config_roots(
    config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Vec<PathBuf> {
    match config_home.map(PathBuf::from).filter(|path| path.is_absolute()) {
        Some(path) if !path.as_os_str().is_empty() => return vec![path],
        _ => {}
    }

    home.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| vec![path.join(".config")])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_data_root_from_the_data_directories() {
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
        let relative =
            resolve_data_roots(Some(OsStr::new("share")), Some(OsStr::new("share:/usr/share")));
        assert!(rooted(&relative));
        assert!(!relative.contains(&PathBuf::from("share")));

        // What the absolute entries around it do is a Unix question: XDG paths
        // are Unix paths, and on a Windows host — where this module is compiled
        // only to type-check it — `/usr/share` is root-relative and the
        // "ignore relative entries" rule drops it along with the genuinely
        // relative one. Asserted where the rule has its real meaning.
        #[cfg(unix)]
        {
            assert!(relative.contains(&PathBuf::from("/usr/share")));
            assert!(relative.iter().all(|root| root.is_absolute()));
        }
    }

    #[test]
    fn a_missing_home_leaves_no_user_config_directory() {
        // `$HOME` unset is not a reason to guess one. An empty list means "look
        // at the system roots only", which is what a headless run wants.
        assert!(resolve_config_roots(None, None).is_empty());
    }

    /// The path *values*, asserted where their syntax means what it says.
    ///
    /// `is_absolute()` is the check the production code has to make — XDG paths
    /// are Unix paths and a relative one is invalid, not resolved — but on a
    /// Windows host it reports `/usr/share` and `/home/u` both relative, so
    /// every branch below would take its fallback. Same split the data-roots
    /// test above already makes for the same reason.
    #[cfg(unix)]
    #[test]
    fn takes_the_config_home_when_it_names_a_directory() {
        assert_eq!(
            resolve_config_roots(Some(OsStr::new("/cfg")), Some(OsStr::new("/home/u"))),
            vec![PathBuf::from("/cfg")]
        );
        // Empty reads as unset, which is `$HOME/.config`.
        assert_eq!(
            resolve_config_roots(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            vec![PathBuf::from("/home/u/.config")]
        );
        // A relative `XDG_CONFIG_HOME` is invalid, so it falls back to `$HOME`
        // rather than resolving against the working directory.
        assert_eq!(
            resolve_config_roots(Some(OsStr::new("cfg")), Some(OsStr::new("/home/u"))),
            vec![PathBuf::from("/home/u/.config")]
        );
    }
}
