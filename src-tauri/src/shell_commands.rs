//! The Tauri wrappers around [`dae_core::fs::shell_commands`].
//!
//! The section itself — discovery, matching and invocation for the Windows
//! verbs, macOS services and KDE service menus — lives in the core crate now,
//! because the Qt shell shows the same rows from the same code. What is left
//! here is the part that is genuinely Tauri's: the two `#[tauri::command]`
//! signatures the frontend calls and the `AppHandle` they carry.
//!
//! The `AppHandle` survives the move as a [`Host`]: macOS has to enter its
//! services machinery on the main thread, and the Tauri `Host` implementation
//! is where that thread is reachable in this shell.

use tauri::AppHandle;

use crate::file_system::error::FileSystemError;
use crate::host;

/// Lists the installed apps' right-click commands for this selection.
#[tauri::command]
#[specta::specta]
pub async fn list_shell_commands(
    paths: Vec<String>,
    primary: String,
) -> Result<Vec<dae_core::fs::shell_commands::ShellCommand>, FileSystemError> {
    dae_core::fs::shell_commands::list_shell_commands(paths, primary).await
}

/// Runs one command against the same paths it was listed for.
#[tauri::command]
#[specta::specta]
pub async fn invoke_shell_command(
    app: AppHandle,
    id: String,
    paths: Vec<String>,
) -> Result<(), FileSystemError> {
    dae_core::fs::shell_commands::invoke_shell_command(host::of(&app), id, paths).await
}

/// Primes the shell-command path before the user's first right-click.
#[tauri::command]
#[specta::specta]
pub async fn warm_shell_commands() -> Result<(), FileSystemError> {
    dae_core::fs::shell_commands::warm_shell_commands().await
}
