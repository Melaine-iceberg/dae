//! Tauri command adapters over `dae_core::file_system::git`.

use dae_core::file_system::error::FileSystemError;

pub use dae_core::file_system::git::{
    GitBranches, GitDirectoryStatus, GitStatusCache,
};

use tauri::Manager;

#[tauri::command]
#[specta::specta]
pub async fn get_git_status(
    path: String,
    app: tauri::AppHandle,
) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache = app.state::<GitStatusCache>();
        dae_core::file_system::git::get_git_status(path, &cache)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn list_git_branches(path: String) -> Result<Option<GitBranches>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        dae_core::file_system::git::list_git_branches(path)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_checkout_branch(
    path: String,
    name: String,
    remote: bool,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        dae_core::file_system::git::git_checkout_branch(path, name, remote)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_create_branch(
    path: String,
    name: String,
    checkout: bool,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        dae_core::file_system::git::git_create_branch(path, name, checkout)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_fetch(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || dae_core::file_system::git::git_fetch(path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_push(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || dae_core::file_system::git::git_push(path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || dae_core::file_system::git::git_pull(path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[tauri::command]
#[specta::specta]
pub async fn git_sync(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || dae_core::file_system::git::git_sync(path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}
