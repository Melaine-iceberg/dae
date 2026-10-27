//! The real startup profile.
//!
//! The skeleton opened `QStandardPaths::HomeLocation`, which proved the QML →
//! Rust → listing path but skipped every document the Tauri shell already
//! writes. This module reads the same profile directory
//! ([`dae_core::paths::config_dir`]) and answers the questions the shell
//! needs before its first listing:
//!
//! * **Where to start.** A directory passed on the command line or as a
//!   `dae://` deep link wins, then the most recent directory in
//!   `recent-items.json`, then the user's home. A stale recent path is skipped
//!   instead of opening an error.
//! * **How to jump to the path bar.** `settings.toml` carries the same
//!   `app.pathJump` binding the React shell uses (`Mod+P` by default). `Mod`
//!   is translated to Qt's `Ctrl` spelling; Qt maps `Ctrl` to Command on macOS.
//! * **How to open the command palette.** The same table's `app.commandBar`
//!   (`Mod+K` by default), behind the same `Mod` translation.
//!
//! Reading stays small on purpose: the files are a few hundred bytes and this
//! runs once, before the first frame. Writing recents and settings belongs to
//! a later slice; this one must not change a user's profile just by opening it.

use std::path::{Path, PathBuf};

use cxx_qt_lib::QString;

use dae_core::types::EntryKind;

/// The shortcut ids the Tauri shell and the React settings panel use.
const PATH_JUMP_ID: &str = "app.pathJump";
const COMMAND_BAR_ID: &str = "app.commandBar";
const DEFAULT_PATH_JUMP: &str = "Mod+P";
const DEFAULT_COMMAND_BAR: &str = "Mod+K";
const SETTINGS_FILE_NAME: &str = "settings.toml";

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("window.h");
        #[namespace = "dae_qt"]
        #[rust_name = "system_dark"]
        fn dae_qt_system_dark() -> bool;
    }

    unsafe extern "RustQt" {
        /// The profile snapshot, exposed to QML as one read-only object.
        ///
        /// Properties are deliberately plain values rather than paths or
        /// enums: QML only displays them, and the shell owns the rules.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, config_dir, cxx_name = "configDir")]
        #[qproperty(QString, startup_path, cxx_name = "startupPath")]
        #[qproperty(QString, startup_source, cxx_name = "startupSource")]
        #[qproperty(QString, path_jump, cxx_name = "pathJump")]
        #[qproperty(QString, command_bar, cxx_name = "commandBar")]
        #[qproperty(bool, settings_found, cxx_name = "settingsFound")]
        type AppConfig = super::AppConfigRust;
    }
}

/// Where [`Snapshot::start_path`] came from. The shell shows it in the status
/// line, which is how a user can tell "it remembered my last folder" from
/// "it fell back to home".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSource {
    DeepLink,
    CommandLine,
    Recent,
    Home,
}

impl StartSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeepLink => "deepLink",
            Self::CommandLine => "commandLine",
            Self::Recent => "recent",
            Self::Home => "home",
        }
    }
}

/// One read of the profile. Kept separate from the QObject so it can be
/// produced in tests without a QML engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub config_dir: Option<PathBuf>,
    pub start_path: PathBuf,
    pub source: StartSource,
    pub recent_count: usize,
    pub settings_found: bool,
    pub path_jump: String,
    pub command_bar: String,
}

/// Reads the real profile once.
///
/// Ordering is command line → recent → home because that is the order of
/// specificity: an explicit launch target, then the user's last visit, then
/// the fallback every install has.
pub fn snapshot() -> Snapshot {
    let config_dir = dae_core::paths::config_dir().ok();

    let settings_path = config_dir
        .as_deref()
        .map(|dir| dir.join(SETTINGS_FILE_NAME));
    let (settings_found, path_jump, command_bar) = match settings_path {
        Some(path) => read_shortcuts(&path),
        None => (
            false,
            qt_sequence(DEFAULT_PATH_JUMP),
            qt_sequence(DEFAULT_COMMAND_BAR),
        ),
    };

    let recents = dae_core::fs::recents::list_recents().unwrap_or_default();
    let recent_dir = recents
        .iter()
        .filter(|item| item.kind == EntryKind::Directory)
        .map(|item| PathBuf::from(&item.path))
        .find(|path| path.is_dir());

    let working_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let launch_target = std::env::args_os()
        .skip(1)
        .find_map(|arg| crate::activation::target_for_arg(&arg, &working_directory));
    let home = home_dir();
    let (start_path, source) = match (launch_target, recent_dir, home) {
        (Some(target), _, _) => (
            target.path,
            match target.source {
                crate::activation::TargetSource::DeepLink => StartSource::DeepLink,
                crate::activation::TargetSource::CommandLine => StartSource::CommandLine,
            },
        ),
        (_, Some(path), _) => (path, StartSource::Recent),
        (_, _, Some(path)) => (path, StartSource::Home),
        _ => (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            StartSource::Home,
        ),
    };

    Snapshot {
        config_dir,
        start_path,
        source,
        recent_count: recents.len(),
        settings_found,
        path_jump,
        command_bar,
    }
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

/// Reads both shell shortcuts from one pass over `settings.toml`. A missing
/// file or key falls back to the product default rather than blocking startup,
/// matching the Tauri settings loader's fallback.
fn read_shortcuts(path: &Path) -> (bool, String, String) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return (
            false,
            qt_sequence(DEFAULT_PATH_JUMP),
            qt_sequence(DEFAULT_COMMAND_BAR),
        );
    };
    (
        true,
        qt_sequence(
            &parse_binding(&contents, PATH_JUMP_ID)
                .unwrap_or_else(|| DEFAULT_PATH_JUMP.to_owned()),
        ),
        qt_sequence(
            &parse_binding(&contents, COMMAND_BAR_ID)
                .unwrap_or_else(|| DEFAULT_COMMAND_BAR.to_owned()),
        ),
    )
}

fn parse_binding(contents: &str, id: &str) -> Option<String> {
    let document: toml::Value = contents.parse().ok()?;
    document
        .get("shortcuts")?
        .get(id)?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
fn read_path_jump(path: &Path) -> (bool, String) {
    let (found, path_jump, _) = read_shortcuts(path);
    (found, path_jump)
}

#[cfg(test)]
fn parse_path_jump(contents: &str) -> Option<String> {
    parse_binding(contents, PATH_JUMP_ID)
}

/// TanStack Hotkeys spell the platform modifier as `Mod`; Qt wants `Ctrl`.
/// Qt then maps `Ctrl` to Command on macOS, so one translation covers all
/// three platforms.
fn qt_sequence(binding: &str) -> String {
    let binding = binding.trim();
    if binding.is_empty() {
        return String::new();
    }
    binding
        .split('+')
        .map(|part| match part.trim() {
            "Mod" | "Control" | "Ctrl" => "Ctrl",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Release builds have no console (`windows_subsystem = "windows"`), so the
/// startup lines must not panic when stderr is absent; debug runs still print
/// them for the end-to-end check.
fn log(message: String) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
}

pub struct AppConfigRust {
    config_dir: QString,
    startup_path: QString,
    startup_source: QString,
    path_jump: QString,
    command_bar: QString,
    settings_found: bool,
}

/// The OS light/dark preference as Qt reports it. Exposed to the rest of the
/// crate so `Workspace` starts on the same reading the native watcher will
/// later keep current.
pub fn system_dark() -> bool {
    qobject::system_dark()
}

impl Default for AppConfigRust {
    fn default() -> Self {
        let snapshot = snapshot();

        let config_dir = snapshot
            .config_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let start_path = snapshot.start_path.to_string_lossy().into_owned();

        log(format!(
            "dae: profile {} (settings {})",
            if config_dir.is_empty() {
                "<unresolved>"
            } else {
                config_dir.as_str()
            },
            if snapshot.settings_found {
                "read"
            } else {
                "missing"
            }
        ));
        log(format!(
            "dae: start {} ({}, {} recents)",
            start_path,
            snapshot.source.as_str(),
            snapshot.recent_count
        ));
        log(format!("dae: path-jump {}", snapshot.path_jump));
        log(format!("dae: command-bar {}", snapshot.command_bar));

        Self {
            config_dir: QString::from(config_dir.as_str()),
            startup_path: QString::from(start_path.as_str()),
            startup_source: QString::from(snapshot.source.as_str()),
            path_jump: QString::from(snapshot.path_jump.as_str()),
            command_bar: QString::from(snapshot.command_bar.as_str()),
            settings_found: snapshot.settings_found,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_becomes_the_qt_control_modifier() {
        assert_eq!(qt_sequence("Mod+P"), "Ctrl+P");
        assert_eq!(qt_sequence("Mod+Shift+P"), "Ctrl+Shift+P");
        assert_eq!(qt_sequence("Control+`"), "Ctrl+`");
    }

    #[test]
    fn an_explicitly_unbound_action_stays_unbound() {
        assert_eq!(qt_sequence(""), "");
        assert_eq!(qt_sequence("   "), "");
    }

    #[test]
    fn the_path_jump_is_read_from_the_shortcuts_table() {
        let contents = r#"
            [shortcuts]
            "app.pathJump" = "Mod+Shift+P"
            "explorer.copy" = "Mod+C"

            [terminal]
            fontSize = 12
        "#;
        assert_eq!(parse_path_jump(contents).as_deref(), Some("Mod+Shift+P"));
    }

    #[test]
    fn a_partial_or_unknown_settings_file_falls_back() {
        assert_eq!(parse_path_jump(""), None);
        assert_eq!(parse_path_jump("[terminal]\nfontSize = 12\n"), None);
        assert_eq!(
            parse_path_jump("[shortcuts]\n\"explorer.copy\" = \"Mod+C\"\n"),
            None
        );
        assert_eq!(
            parse_path_jump("[shortcuts]\n\"app.pathJump\" = \"\"\n").as_deref(),
            Some("")
        );
    }

    #[test]
    fn the_command_bar_binding_is_read_independently() {
        let contents = r#"
            [shortcuts]
            "app.pathJump" = "Mod+Shift+P"
            "app.commandBar" = "Mod+Shift+K"
        "#;
        assert_eq!(
            parse_binding(contents, COMMAND_BAR_ID).as_deref(),
            Some("Mod+Shift+K")
        );
        assert_eq!(qt_sequence("Mod+Shift+K"), "Ctrl+Shift+K");
    }

    #[test]
    fn a_missing_command_bar_binding_uses_the_default() {
        let missing = std::env::temp_dir().join("dae-qt-settings-command-bar-missing.toml");
        let _ = std::fs::remove_file(&missing);
        let (found, _, command_bar) = read_shortcuts(&missing);
        assert!(!found);
        assert_eq!(command_bar, "Ctrl+K");
    }

    #[test]
    fn a_missing_settings_file_reports_the_default() {
        let missing = std::env::temp_dir().join("dae-qt-settings-does-not-exist.toml");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(read_path_jump(&missing), (false, "Ctrl+P".to_owned()));
    }

    #[test]
    fn start_sources_map_to_status_labels() {
        assert_eq!(StartSource::DeepLink.as_str(), "deepLink");
        assert_eq!(StartSource::CommandLine.as_str(), "commandLine");
        assert_eq!(StartSource::Recent.as_str(), "recent");
        assert_eq!(StartSource::Home.as_str(), "home");
    }
}
