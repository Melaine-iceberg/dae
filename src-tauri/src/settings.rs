//! Tauri command surface for settings.
//!
//! The document, its defaults and its clamping rules moved to
//! [`dae_core::settings`] so the Qt shell writes the same `settings.toml`.
//! What remains here is exactly the part that is Tauri-shaped: two command
//! wrappers and the crate-internal accessors other Tauri modules call through.

pub use dae_core::settings::{AppSettings, DefaultFileManagerState, TerminalSettings};
use dae_core::FileSystemError;

pub fn init() -> Result<(), FileSystemError> {
    dae_core::settings::init()
}

#[tauri::command]
#[specta::specta]
pub fn load_settings() -> Result<AppSettings, FileSystemError> {
    dae_core::settings::load_settings()
}

#[tauri::command]
#[specta::specta]
pub fn save_settings(settings: AppSettings) -> Result<AppSettings, FileSystemError> {
    dae_core::settings::save_settings(settings)
}

pub(crate) fn tab_pool_enabled() -> bool {
    dae_core::settings::tab_pool_enabled()
}

pub(crate) fn set_default_file_manager_cache(is_default: bool) -> Result<(), FileSystemError> {
    dae_core::settings::set_default_file_manager_cache(is_default)
}
