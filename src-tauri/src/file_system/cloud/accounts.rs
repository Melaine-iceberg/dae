//! The cloud account registry: persisted account metadata, OAuth credentials
//! in the OS keychain, and cached live backends. Mirrors `connections.rs`.

use super::provider::CloudProviderKind;
use crate::file_system::error::FileSystemError;
use crate::file_system::vfs::SharedBackend;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use tauri::Manager;

const ACCOUNTS_FILE_NAME: &str = "cloud_accounts.json";
const KEYRING_SERVICE: &str = "dae";

/// A saved cloud account as persisted and exposed to the frontend. Secrets
/// never appear here: the OAuth client credentials and refresh token live in
/// the OS keychain (or the session-scoped in-memory fallback), keyed by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredCloudAccount {
    /// Canonical identity, also the explorer path of the drive root:
    /// `cloud://{provider}:{email}`.
    pub id: String,
    pub provider: CloudProviderKind,
    pub email: String,
    pub display_name: String,
}

/// The OAuth material a backend needs to mint access tokens for an account.
#[derive(Debug, Clone)]
pub struct TokenMaterial {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub refresh_token: String,
}

#[derive(Default)]
struct RegistryInner {
    config_dir: Option<PathBuf>,
    saved: Vec<StoredCloudAccount>,
    /// Live backends keyed by account id.
    sessions: HashMap<String, SharedBackend>,
    /// Credentials held only for this app run, when the OS keychain is
    /// unavailable (headless Linux without a Secret Service).
    memory_client_credentials: HashMap<String, String>,
    memory_refresh_tokens: HashMap<String, String>,
}

static REGISTRY: LazyLock<Mutex<RegistryInner>> =
    LazyLock::new(|| Mutex::new(RegistryInner::default()));

/// Loads persisted accounts. Called once from app setup; safe to call again.
pub fn init(app: &tauri::AppHandle) -> Result<(), FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.saved = read_accounts_file(&accounts_path(&config_dir))?;
    Ok(())
}

/// Lists saved accounts sorted by provider then email.
#[tauri::command]
#[specta::specta]
pub fn list_cloud_accounts() -> Result<Vec<StoredCloudAccount>, FileSystemError> {
    let registry = REGISTRY.lock().expect("cloud registry poisoned");
    let mut accounts = registry.saved.clone();
    accounts.sort_by(|a, b| {
        a.provider
            .as_str()
            .cmp(b.provider.as_str())
            .then_with(|| a.email.cmp(&b.email))
    });
    Ok(accounts)
}

/// Removes a saved account, its stored credentials, and any live backend.
#[tauri::command]
#[specta::specta]
pub fn delete_cloud_account(id: String) -> Result<(), FileSystemError> {
    {
        let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
        registry.saved.retain(|existing| existing.id != id);
        registry.sessions.remove(&id);
        registry.memory_client_credentials.remove(&id);
        registry.memory_refresh_tokens.remove(&id);
        persist_locked(&mut registry)?;
    }

    delete_secret(&client_credential_key(&id));
    delete_secret(&refresh_token_key(&id));
    Ok(())
}

/// Returns the account with `id`, if saved.
pub fn account_by_id(id: &str) -> Option<StoredCloudAccount> {
    let registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.saved.iter().find(|account| account.id == id).cloned()
}

/// Creates or replaces an account, storing its OAuth material. Refresh tokens
/// and client credentials go to the OS keychain; when no keychain is usable
/// they fall back to session memory (lost on exit, matching `connections.rs`).
pub fn upsert_account(
    account: StoredCloudAccount,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.saved.retain(|existing| existing.id != account.id);
    registry.saved.push(account.clone());
    registry.sessions.remove(&account.id);
    persist_locked(&mut registry)?;
    drop(registry);

    let credentials = ClientCredentialRecord {
        client_id: client_id.to_owned(),
        client_secret: client_secret
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
            .map(str::to_owned),
    };
    let serialized = serde_json::to_string(&credentials)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    store_secret(&client_credential_key(&account.id), &serialized, &mut |registry| {
        registry.memory_client_credentials.insert(account.id.clone(), serialized.clone());
    });
    store_secret(&refresh_token_key(&account.id), refresh_token, &mut |registry| {
        registry.memory_refresh_tokens.insert(account.id.clone(), refresh_token.to_owned());
    });
    Ok(())
}

/// Resolves the OAuth material for an account.
pub fn token_material(id: &str) -> Result<TokenMaterial, FileSystemError> {
    let serialized = read_secret(&client_credential_key(id), &mut |registry| {
        registry.memory_client_credentials.get(id).cloned()
    })
    .ok_or_else(|| {
        FileSystemError::PermissionDenied(
            "The OAuth client credentials for this cloud account are missing. Remove the account and authorize it again.".into(),
        )
    })?;
    let record: ClientCredentialRecord = serde_json::from_str(&serialized).map_err(|error| {
        FileSystemError::Internal(format!("Corrupt cloud credentials: {error}"))
    })?;

    let refresh_token = read_secret(&refresh_token_key(id), &mut |registry| {
        registry.memory_refresh_tokens.get(id).cloned()
    })
    .ok_or_else(|| {
        FileSystemError::PermissionDenied(
            "The refresh token for this cloud account is missing. Remove the account and authorize it again.".into(),
        )
    })?;

    Ok(TokenMaterial {
        client_id: record.client_id,
        client_secret: record.client_secret,
        refresh_token,
    })
}

/// Persists a rotated refresh token (Dropbox refresh-token rotation).
pub fn save_refresh_token(id: &str, refresh_token: &str) {
    store_secret(&refresh_token_key(id), refresh_token, &mut |registry| {
        registry.memory_refresh_tokens.insert(id.to_owned(), refresh_token.to_owned());
    });
}

/// Returns the cached live backend for an account, if one is open.
pub fn session_for(id: &str) -> Option<SharedBackend> {
    let registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.sessions.get(id).cloned()
}

/// Caches (or replaces) the live backend for an account.
pub fn store_session(id: &str, backend: SharedBackend) {
    let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.sessions.insert(id.to_owned(), backend);
}

#[derive(Serialize, Deserialize)]
struct ClientCredentialRecord {
    client_id: String,
    client_secret: Option<String>,
}

fn client_credential_key(id: &str) -> String {
    format!("{id}:client")
}

fn refresh_token_key(id: &str) -> String {
    format!("{id}:refresh")
}

/// Writes a secret to the keychain, falling back to the provided memory
/// store when no usable keychain exists.
fn store_secret(
    key: &str,
    value: &str,
    memory_fallback: &mut impl FnMut(&mut RegistryInner),
) {
    match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|entry| entry.set_password(value)) {
        Ok(()) => {
            let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
            registry.memory_client_credentials.remove(key);
            registry.memory_refresh_tokens.remove(key);
        }
        Err(_) => {
            let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
            memory_fallback(&mut registry);
        }
    }
}

fn read_secret(key: &str, memory_fallback: &mut impl FnMut(&RegistryInner) -> Option<String>) -> Option<String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key)
        && let Ok(value) = entry.get_password()
    {
        return Some(value);
    }

    let registry = REGISTRY.lock().expect("cloud registry poisoned");
    memory_fallback(&registry)
}

fn delete_secret(key: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = entry.delete_credential();
    }
    let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.memory_client_credentials.remove(key);
    registry.memory_refresh_tokens.remove(key);
}

fn persist_locked(registry: &mut RegistryInner) -> Result<(), FileSystemError> {
    let Some(config_dir) = registry.config_dir.clone() else {
        return Err(FileSystemError::Internal(
            "The cloud account store was not initialized".into(),
        ));
    };

    fs::create_dir_all(&config_dir)?;
    let contents = serde_json::to_string_pretty(&registry.saved)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    crate::file_system::sidebar::write_atomic(&accounts_path(&config_dir), contents.as_bytes())
}

fn accounts_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join(ACCOUNTS_FILE_NAME)
}

fn read_accounts_file(
    path: &std::path::Path,
) -> Result<Vec<StoredCloudAccount>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| FileSystemError::Internal(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

/// Points the store at `config_dir` and reloads from disk, without an app
/// handle. Test-only.
#[cfg(test)]
pub(crate) fn use_config_dir_for_tests(config_dir: PathBuf) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("cloud registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.saved = read_accounts_file(&accounts_path(&config_dir))?;
    registry.sessions.clear();
    registry.memory_client_credentials.clear();
    registry.memory_refresh_tokens.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_accounts_without_persisting_secrets() {
        // Keep secrets out of the real OS credential store. The v1 entry API
        // runs its one-time platform store setup on first use, so trigger it
        // before swapping in the in-memory mock.
        let _ = keyring::Entry::store_status();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("create the mock credential store"),
        );

        let config_dir =
            std::env::temp_dir().join(format!("dae-cloud-accounts-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&config_dir);
        use_config_dir_for_tests(config_dir.clone()).expect("initialize store");

        let account = StoredCloudAccount {
            id: "cloud://dropbox:user@example.com".into(),
            provider: CloudProviderKind::Dropbox,
            email: "user@example.com".into(),
            display_name: "User".into(),
        };
        upsert_account(account.clone(), "client-1", Some("secret-1"), "refresh-1")
            .expect("save account");

        let listed = list_cloud_accounts().expect("list accounts");
        assert_eq!(listed, vec![account.clone()]);

        let persisted =
            fs::read_to_string(config_dir.join(ACCOUNTS_FILE_NAME)).expect("accounts file");
        for secret in ["client-1", "secret-1", "refresh-1", "secret"] {
            assert!(!persisted.contains(secret), "secrets must never be persisted");
        }

        // Secrets resolve through the keychain or its memory fallback.
        let material = token_material(&account.id).expect("token material");
        assert_eq!(material.client_id, "client-1");
        assert_eq!(material.client_secret.as_deref(), Some("secret-1"));
        assert_eq!(material.refresh_token, "refresh-1");

        save_refresh_token(&account.id, "refresh-rotated");
        assert_eq!(
            token_material(&account.id).expect("material").refresh_token,
            "refresh-rotated"
        );

        delete_cloud_account(account.id.clone()).expect("delete account");
        assert!(list_cloud_accounts().expect("list").is_empty());
        assert!(token_material(&account.id).is_err());

        fs::remove_dir_all(config_dir).expect("remove test directory");
    }
}
