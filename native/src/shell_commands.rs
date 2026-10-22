//! The context menu's third-party command section — "应用扩展" in the UI,
//! the rows installed applications contribute rather than the ones dae
//! declares itself.
//!
//! Each platform reaches the same three-step shape through a different
//! mechanism:
//!
//! | Platform | Declared in | Hosted by |
//! |---|---|---|
//! | Windows | each package's `AppxManifest.xml` (`desktop5:Verb`) | `CoCreateInstance` → `IExplorerCommand` |
//! | macOS | each bundle's `Info.plist` (`NSServices`) | `NSPerformService` (main process) |
//! | Linux | each service menu's `.desktop` (`Actions`, `MimeType`) | the `Exec` line, run directly |
//!
//! What is shared lives here: the row type, the selection a row is matched
//! against, and the Explorer-style grouping applied to the finished list.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::vfs::is_local_path;
use serde::Serialize;

// The parsers are compiled under `cfg(test)` on every host so their pure logic
// runs on the machine this was written on; the Linux backend is too, so it at
// least type-checks there.
#[cfg(any(target_os = "linux", test))]
mod desktop_entry;
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod plist;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
use windows as backend;

/// One row of the application-extensions section of a context menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCommand {
    /// Identifier of the command, unique within this platform's namespace, and
    /// the value `invoke_shell_command` is called back with. Opaque to the
    /// frontend, which only round-trips it.
    pub id: String,
    /// Menu text, exactly as the owning application worded it — which is why no
    /// row in this section is translated by dae.
    pub label: String,
    /// 16–32px icon as a `data:` URL.
    pub icon_data_url: Option<String>,
    /// Owning app's display name, set only when that app contributes more than
    /// one command to this menu. Explorer groups those under an app flyout; a
    /// lone command is shown inline instead.
    pub group: Option<String>,
    /// The provider asked for a separator above this row. Windows only: no
    /// other platform's declarations carry a separator flag.
    pub separator_before: bool,
    /// The provider reports the command greyed out. Windows only.
    pub disabled: bool,
}

/// What was right-clicked, reduced to what the platform's matching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selection {
    kind: SelectionKind,
    /// Lowercase, leading dot (`".zip"`); empty for a file without extension.
    extension: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    File,
    Directory,
}

impl Selection {
    /// Reduces a selection to a single kind. The right-clicked entry decides:
    /// a mixed selection is still "the file menu" or "the folder menu"
    /// depending on where the click landed, which is the rule Explorer follows.
    fn for_paths(primary: &str, paths: &[String]) -> Result<Self, FileSystemError> {
        if paths.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "fs.shell_command_empty_selection".into(),
            ));
        }

        let metadata = std::fs::metadata(primary).map_err(FileSystemError::from)?;
        Ok(if metadata.is_dir() {
            Self {
                kind: SelectionKind::Directory,
                extension: String::new(),
            }
        } else {
            Self {
                kind: SelectionKind::File,
                extension: extension_of(primary),
            }
        })
    }
}

/// Lowercase extension with a leading dot, or an empty string.
fn extension_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// Collapses per-type registrations into one row per command and gives commands
/// from a multi-command app their Explorer-style group name.
fn assign_groups(rows: Vec<(String, ShellCommand)>) -> Vec<ShellCommand> {
    use std::collections::HashMap;

    let mut per_app: HashMap<String, usize> = HashMap::new();
    for (app, _) in &rows {
        *per_app.entry(app.clone()).or_default() += 1;
    }

    let mut items: Vec<ShellCommand> = rows
        .into_iter()
        .map(|(app, mut item)| {
            if per_app.get(app.as_str()).copied().unwrap_or(0) > 1 {
                item.group = Some(app);
            }
            item
        })
        .collect();

    // Inline rows first, then app flyouts — the order Explorer uses, and the one
    // that keeps a stray installed extension from pushing dae's own items
    // around. Case-insensitive because that is how menus read; the sort is
    // stable, so equal labels keep provider order.
    items.sort_by(|left, right| {
        let key = |item: &ShellCommand| {
            (
                item.group.clone().map(|group| group.to_lowercase()),
                item.label.to_lowercase(),
            )
        };
        match (&left.group, &right.group) {
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            _ => key(left).cmp(&key(right)),
        }
    });
    items
}

/// Lists the installed apps' right-click commands for this selection.
///
/// Returns an empty list when the platform has no extension point, and for
/// selections that cannot be turned into shell items, so the frontend can render
/// one section unconditionally.
pub async fn list_shell_commands(
    paths: Vec<String>,
    primary: String,
) -> Result<Vec<ShellCommand>, FileSystemError> {
    // The mechanisms are all driven by real local paths: a shell item array
    // (Windows), a file URL (macOS), an `Exec` expansion (Linux). A remote
    // backend (SFTP, SMB, cloud) has no such path, and the caller renders no
    // section for it.
    if !is_local_path(&primary) {
        return Ok(Vec::new());
    }
    backend::list(paths, primary).await
}

/// Runs one command against the same paths it was listed for.
pub async fn invoke_shell_command(
    id: String,
    paths: Vec<String>,
) -> Result<(), FileSystemError> {
    backend::invoke(id, paths).await
}

/// Primes the shell-command path before the user's first right-click.
pub async fn warm_shell_commands() -> Result<(), FileSystemError> {
    backend::warm().await
}

/// Resolves a macOS service row to the title `NSPerformService` is addressed
/// by. The core process exposes this so the Electron main process can validate
/// the row before queueing the blocking call on its own thread.
pub fn resolve_service_title(id: &str) -> Result<String, FileSystemError> {
    #[cfg(target_os = "macos")]
    {
        macos::resolve_title(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(FileSystemError::Unsupported(format!(
            "fs.shell_command_not_macos: {id}"
        )))
    }
}

/// Runs a macOS service. Must be called on the Electron main process's UI
/// thread; returns whether the service ran.
pub fn perform_service(title: &str, paths: &[String]) -> Result<bool, FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "fs.shell_command_empty_selection".into(),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        Ok(macos::perform(title, paths))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, paths);
        Err(FileSystemError::Unsupported(
            "fs.shell_command_not_macos".into(),
        ))
    }
}

/// Platforms without a backing module in this core: the section stays empty
/// rather than pretending otherwise.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod backend {
    use super::{FileSystemError, ShellCommand};

    pub(super) async fn list(
        _paths: Vec<String>,
        _primary: String,
    ) -> Result<Vec<ShellCommand>, FileSystemError> {
        Ok(Vec::new())
    }

    pub(super) async fn invoke(
        id: String,
        _paths: Vec<String>,
    ) -> Result<(), FileSystemError> {
        Err(FileSystemError::Unsupported(format!(
            "fs.shell_command_unsupported_platform: {id}"
        )))
    }

    pub(super) async fn warm() -> Result<(), FileSystemError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(label: &str) -> ShellCommand {
        ShellCommand {
            id: label.to_string(),
            label: label.to_string(),
            icon_data_url: None,
            group: None,
            separator_before: false,
            disabled: false,
        }
    }

    #[test]
    fn a_lone_command_stays_inline_and_a_pair_becomes_a_flyout() {
        let items = assign_groups(vec![
            ("Terminal".into(), command("在终端中打开")),
            ("Zed".into(), command("Open in Zed")),
            ("Zed".into(), command("Open Project in Zed")),
        ]);

        assert_eq!(items[0].label, "在终端中打开");
        assert_eq!(items[0].group, None);
        assert_eq!(items[1].group.as_deref(), Some("Zed"));
        assert_eq!(items[2].group.as_deref(), Some("Zed"));
        assert_eq!(items[1].label, "Open in Zed");
        assert_eq!(items[2].label, "Open Project in Zed");
    }
}
