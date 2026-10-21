//! Tauri command adapters for stores whose logic lives in `dae-core`.

use super::cloud::accounts::{
    StoredCloudAccount, delete_cloud_account as delete_cloud_account_core,
    list_cloud_accounts as list_cloud_accounts_core,
};
use super::cloud::oauth::{
    AuthorizeCloudAccountInput, authorize_cloud_account as authorize_cloud_account_core,
};
use super::connections::{
    SaveConnectionInput, StoredConnection, delete_connection as delete_connection_core,
    list_connections as list_connections_core, save_connection as save_connection_core,
};
use super::error::FileSystemError;

#[tauri::command]
#[specta::specta]
pub fn list_connections() -> Result<Vec<StoredConnection>, FileSystemError> {
    list_connections_core()
}

#[tauri::command]
#[specta::specta]
pub fn save_connection(input: SaveConnectionInput) -> Result<StoredConnection, FileSystemError> {
    save_connection_core(input)
}

#[tauri::command]
#[specta::specta]
pub fn delete_connection(id: String) -> Result<(), FileSystemError> {
    delete_connection_core(id)
}

#[tauri::command]
#[specta::specta]
pub async fn test_connection(input: SaveConnectionInput) -> Result<(), FileSystemError> {
    super::smb::test_connection(input).await
}

#[tauri::command]
#[specta::specta]
pub fn list_cloud_accounts() -> Result<Vec<StoredCloudAccount>, FileSystemError> {
    list_cloud_accounts_core()
}

#[tauri::command]
#[specta::specta]
pub fn delete_cloud_account(id: String) -> Result<(), FileSystemError> {
    delete_cloud_account_core(id)
}

#[tauri::command]
#[specta::specta]
pub async fn authorize_cloud_account(
    input: AuthorizeCloudAccountInput,
) -> Result<StoredCloudAccount, FileSystemError> {
    authorize_cloud_account_core(input).await
}
