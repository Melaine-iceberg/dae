//! User-defined external actions.
//!
//! `settings.toml` carries an `[[actions]]` table; this module decides which
//! rows apply to a selection and starts the process. Two rules are structural
//! rather than stylistic:
//!
//! * **No shell.** The command and its arguments reach `Command` as a program
//!   and a vector; nothing is joined into a string, so a file called
//!   `a & rm -rf ~` stays one argument.
//! * **Availability is probed, not assumed.** A row whose program is not on
//!   `PATH` is not offered, which is what keeps the built-in "open in IDE"
//!   entry from failing on a machine without VS Code.
//!
//! Placeholders expand per argument: `%f` first path, `%F` every path as its
//! own argument, `%d` the directory, `%u` the first path as a `file:` URL.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::FileSystemError;
use crate::settings::{self, CustomAction};

/// The name a `platforms` entry has for the running build.
pub fn platform_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// True when `command` names something that can be started: an absolute or
/// relative path that exists, or a bare name found on `PATH`.
pub fn command_available(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    if command.contains(['/', '\\']) {
        return Path::new(command).is_file();
    }

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: Vec<String> = if cfg!(windows) {
        // `code` is `code.cmd` on a normal Windows install; the launcher is a
        // script, and `Command::new("code")` resolves it through PATHEXT.
        let mut names = vec![command.to_owned()];
        for extension in ["exe", "cmd", "bat", "com"] {
            names.push(format!("{command}.{extension}"));
        }
        names
    } else {
        vec![command.to_owned()]
    };

    std::env::split_paths(&path)
        .any(|directory| names.iter().any(|name| directory.join(name).is_file()))
}

/// Every action that applies to the current platform and whose program exists.
pub fn available_actions() -> Vec<CustomAction> {
    let platform = platform_name();
    settings::load_settings()
        .map(|settings| {
            settings
                .actions
                .into_iter()
                .filter(|action| {
                    (action.platforms.is_empty()
                        || action.platforms.iter().any(|entry| entry == platform))
                        && command_available(&action.command)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Looks an action up by id, without the availability filter: the caller may be
/// re-running a row the menu offered before a settings change.
pub fn action_by_id(id: &str) -> Option<CustomAction> {
    settings::load_settings()
        .ok()?
        .actions
        .into_iter()
        .find(|action| action.id == id)
}

/// Whether the row belongs on the menu for this selection.
pub fn selection_allows(action: &CustomAction, selection: &[String], directory: &str) -> bool {
    if !action.extensions.is_empty() && !selection.is_empty() {
        let matched = selection.iter().all(|path| {
            Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .is_some_and(|extension| action.extensions.contains(&extension))
        });
        if !matched {
            return false;
        }
    }

    let kinds = |paths: &[String]| -> (usize, usize) {
        let files = paths
            .iter()
            .filter(|path| Path::new(path).is_file())
            .count();
        let directories = paths.iter().filter(|path| Path::new(path).is_dir()).count();
        (files, directories)
    };

    match action.selection.as_str() {
        "file" => {
            let (files, directories) = kinds(selection);
            files > 0 && directories == 0
        }
        "directory" => {
            if selection.is_empty() {
                return !directory.is_empty() && Path::new(directory).is_dir();
            }
            let (files, directories) = kinds(selection);
            directories > 0 && files == 0
        }
        "multiple" => selection.len() >= 2,
        _ => true,
    }
}

/// Expands the placeholders in `args` for one invocation.
pub fn expand_args(action: &CustomAction, selection: &[String], directory: &str) -> Vec<String> {
    let first = selection
        .first()
        .cloned()
        .unwrap_or_else(|| directory.to_owned());
    let url = url_of(&first);
    let mut expanded = Vec::with_capacity(action.args.len());
    for argument in &action.args {
        let mut value = argument.clone();
        if value.contains("%F") {
            // `%F` is the only placeholder that multiplies one argument into
            // several, so it is handled before the single-value substitutions.
            for path in selection {
                expanded.push(argument.replace("%F", path));
            }
            continue;
        }
        value = value.replace("%f", &first);
        value = value.replace("%d", directory);
        value = value.replace("%u", &url);
        expanded.push(value);
    }
    expanded
}

fn url_of(path: &str) -> String {
    url::Url::from_file_path(path)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| path.to_owned())
}

/// Runs one action against a selection. Returns the program it started, which
/// the status center shows; the child is detached from this process.
pub fn run(
    action: &CustomAction,
    selection: &[String],
    directory: &str,
) -> Result<String, FileSystemError> {
    if action.command.trim().is_empty() {
        return Err(FileSystemError::InvalidInput(
            "This action has no command".to_owned(),
        ));
    }
    if !command_available(&action.command) {
        return Err(FileSystemError::InvalidInput(format!(
            "{} was not found on PATH",
            action.command
        )));
    }

    let args = expand_args(action, selection, directory);
    let mut command = Command::new(&action.command);
    let working_directory = directory_of(directory);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !working_directory.as_os_str().is_empty() {
        command.current_dir(working_directory);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // A console program must not flash a console over the shell.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().map_err(|error| {
        FileSystemError::Internal(format!("Could not start the action: {error}"))
    })?;
    Ok(action.command.clone())
}

fn directory_of(directory: &str) -> PathBuf {
    let path = Path::new(directory);
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(selection: &str, extensions: &[&str]) -> CustomAction {
        CustomAction {
            id: "test".to_owned(),
            name: "Test".to_owned(),
            command: "code".to_owned(),
            args: vec!["%f".to_owned(), "%d".to_owned(), "%u".to_owned()],
            platforms: Vec::new(),
            selection: selection.to_owned(),
            extensions: extensions.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn placeholders_expand_per_argument() {
        let directory = std::env::temp_dir();
        let file = directory.join("a b.txt");
        let file = file.to_string_lossy().to_string();
        let expanded = expand_args(
            &action("any", &[]),
            std::slice::from_ref(&file),
            &directory.to_string_lossy(),
        );
        assert_eq!(expanded[0], file);
        assert_eq!(expanded[1], directory.to_string_lossy());
        assert!(expanded[2].starts_with("file://"));
        // A space in a path is not a split point: it stays one argument.
        assert_eq!(expanded.len(), 3);
    }

    #[test]
    fn the_all_files_placeholder_multiplies_the_argument() {
        let mut custom = action("any", &[]);
        custom.args = vec!["--file=%F".to_owned()];
        let expanded = expand_args(&custom, &["/tmp/a".to_owned(), "/tmp/b".to_owned()], "/tmp");
        assert_eq!(expanded, ["--file=/tmp/a", "--file=/tmp/b"]);
    }

    #[test]
    fn with_no_selection_the_first_placeholder_is_the_directory() {
        let expanded = expand_args(&action("any", &[]), &[], "/tmp");
        assert_eq!(expanded[0], "/tmp");
    }

    #[test]
    fn a_missing_program_is_not_offered() {
        assert!(!command_available("dae-definitely-not-a-program"));
        assert!(command_available(
            &std::env::current_exe().unwrap().to_string_lossy()
        ));
    }

    #[test]
    fn selection_modes_and_extension_filters_are_honoured() {
        // The selection is a file this test names itself: the test binary
        // carries `.exe` only on Windows, and the filter reads the name.
        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("probe.exe");
        std::fs::write(&file, b"").expect("write the selection file");
        let files = vec![file.to_string_lossy().to_string()];
        let mut file_only = action("file", &[]);
        file_only.extensions = vec!["exe".to_owned()];
        assert!(selection_allows(&file_only, &files, ""));
        file_only.extensions = vec!["txt".to_owned()];
        assert!(!selection_allows(&file_only, &files, ""));
        assert!(!selection_allows(&action("multiple", &[]), &files, ""));
        assert!(selection_allows(
            &action("directory", &[]),
            &[],
            &std::env::temp_dir().to_string_lossy()
        ));
    }
}
