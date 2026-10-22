//! Saved remote connections and cloud accounts.
//!
//! `dae-core` owns both stores and the OAuth flow; this module only forwards
//! the renderer's commands, exactly like the Tauri shell's `network` module.

use dae_core::file_system::cloud::accounts::{self, StoredCloudAccount};
use dae_core::file_system::cloud::oauth::{self, AuthorizeCloudAccountInput};
use dae_core::file_system::connections::{self, SaveConnectionInput, StoredConnection};
use dae_core::file_system::error::FileSystemError;

pub fn list_connections() -> Result<Vec<StoredConnection>, FileSystemError> {
    connections::list_connections()
}

pub fn save_connection(input: SaveConnectionInput) -> Result<StoredConnection, FileSystemError> {
    connections::save_connection(input)
}

pub fn delete_connection(id: String) -> Result<(), FileSystemError> {
    connections::delete_connection(id)
}

pub async fn test_connection(input: SaveConnectionInput) -> Result<(), FileSystemError> {
    dae_core::file_system::smb::test_connection(input).await
}

pub fn list_cloud_accounts() -> Result<Vec<StoredCloudAccount>, FileSystemError> {
    accounts::list_cloud_accounts()
}

pub fn delete_cloud_account(id: String) -> Result<(), FileSystemError> {
    accounts::delete_cloud_account(id)
}

pub async fn authorize_cloud_account(
    input: AuthorizeCloudAccountInput,
) -> Result<StoredCloudAccount, FileSystemError> {
    oauth::authorize_cloud_account(input).await
}

/// Points both stores at the app config directory. Called once from
/// `initialize`; the Tauri shell did the same during setup.
pub fn init(config_dir: std::path::PathBuf) {
    let _ = connections::init(config_dir.clone());
    let _ = accounts::init(config_dir);
}
