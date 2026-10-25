//! The sidebar's well-known directories.
//!
//! Resolving the user's Desktop/Documents/Downloads is `tauri::Manager::path`'s
//! job today. Porting that resolver into `dae_core::paths` is its own
//! decision (seven directories, each with a per-platform rule), so the command
//! stays in the shell; the [`SystemPlace`] values it produces are domain data
//! and live in the core sidebar module.

use super::error::FileSystemError;
use super::sidebar::{PlaceKind, SystemPlace};
use super::types::path_to_string;
use tauri::Manager;

/// Resolves the fixed set of system places, skipping any the OS cannot provide.
/// Resolves the fixed set of system places, skipping any the OS cannot provide.
pub fn resolve_system_places(
    app: &tauri::AppHandle,
) -> Result<Vec<SystemPlace>, FileSystemError> {
    if let Some(places) = super::prefetch::state().take_system_places() {
        return Ok(places);
    }
    let resolver = app.path();
    let candidates = [
        (PlaceKind::Home, resolver.home_dir()),
        (PlaceKind::Desktop, resolver.desktop_dir()),
        (PlaceKind::Documents, resolver.document_dir()),
        (PlaceKind::Downloads, resolver.download_dir()),
        (PlaceKind::Pictures, resolver.picture_dir()),
        (PlaceKind::Music, resolver.audio_dir()),
        (PlaceKind::Videos, resolver.video_dir()),
    ];

    Ok(candidates
        .into_iter()
        .filter_map(|(kind, resolved)| {
            resolved.ok().map(|path| SystemPlace {
                kind,
                path: path_to_string(&path),
            })
        })
        .collect())
}

/// Resolves the fixed set of system places, skipping any the OS cannot provide.
#[tauri::command]
#[specta::specta]
pub fn get_system_places(app: tauri::AppHandle) -> Result<Vec<SystemPlace>, FileSystemError> {
    resolve_system_places(&app)
}
