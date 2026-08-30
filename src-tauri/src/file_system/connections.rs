use super::error::FileSystemError;
use super::vfs::SharedBackend;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use tauri::Manager;

const CONNECTIONS_FILE_NAME: &str = "connections.json";
const KEYRING_SERVICE: &str = "dae";

/// Storage protocols a user can connect to. Mirrors the network variants of
/// [`super::vfs::Scheme`] without the local case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Smb,
    Sftp,
    Ftp,
    WebDav,
}

impl Protocol {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Protocol::Smb => "smb",
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::WebDav => "webdav",
        }
    }
}

/// A saved server connection as persisted and exposed to the frontend.
/// Passwords never appear here: they live in the OS keychain (or the
/// session-scoped in-memory fallback), keyed by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredConnection {
    pub id: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
}

/// Input for creating or updating a connection. `id` is derived from
/// protocol + host + port, so saving the same server again updates it.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionInput {
    pub protocol: Protocol,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub remember_password: bool,
}

#[derive(Default)]
struct RegistryInner {
    config_dir: Option<PathBuf>,
    saved: Vec<StoredConnection>,
    /// Live protocol sessions keyed by `session_key`.
    sessions: HashMap<String, SharedBackend>,
    /// Passwords held only for this app run, when the OS keychain is
    /// unavailable (headless Linux without a Secret Service) or the user
    /// chose not to remember the credential.
    memory_passwords: HashMap<String, String>,
}

static REGISTRY: LazyLock<Mutex<RegistryInner>> =
    LazyLock::new(|| Mutex::new(RegistryInner::default()));

/// Loads persisted connections. Called once from app setup; safe to call
/// again (re-reads from disk).
pub fn init(app: &tauri::AppHandle) -> Result<(), FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let mut registry = REGISTRY.lock().expect("connection registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.saved = read_connections_file(&connections_path(&config_dir))?;
    Ok(())
}

/// Canonical identity of a server connection: `protocol://host[:port]`.
/// Also used as the keyring account and the live-session cache key, so one
/// server maps to exactly one saved connection and one open session.
pub fn session_key(protocol: Protocol, host: &str, port: Option<u16>) -> String {
    let host = normalize_host(host);
    match port {
        Some(port) => format!("{}://{host}:{port}", protocol.as_str()),
        None => format!("{}://{host}", protocol.as_str()),
    }
}

/// Lists saved connections sorted by protocol then host.
#[tauri::command]
#[specta::specta]
pub fn list_connections() -> Result<Vec<StoredConnection>, FileSystemError> {
    let registry = REGISTRY.lock().expect("connection registry poisoned");
    let mut connections = registry.saved.clone();
    connections.sort_by(|a, b| {
        a.protocol
            .as_str()
            .cmp(b.protocol.as_str())
            .then_with(|| a.host.cmp(&b.host))
    });
    Ok(connections)
}

/// Creates or updates a connection. Passwords, when supplied and remembered,
/// go to the OS keychain; if the keychain is unavailable they fall back to
/// session memory so the connection still works until the app exits.
#[tauri::command]
#[specta::specta]
pub fn save_connection(input: SaveConnectionInput) -> Result<StoredConnection, FileSystemError> {
    let host = normalize_host(&input.host);
    if host.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "A server host is required".into(),
        ));
    }

    let id = session_key(input.protocol, &host, input.port);
    let password = input.password.clone();
    let remember_password = input.remember_password;
    let connection = StoredConnection {
        id: id.clone(),
        protocol: input.protocol,
        host,
        port: input.port,
        username: input
            .username
            .map(|username| username.trim().to_owned())
            .filter(|username| !username.is_empty()),
    };

    {
        let mut registry = REGISTRY.lock().expect("connection registry poisoned");
        registry.saved.retain(|existing| existing.id != id);
        registry.saved.push(connection.clone());
        // Credentials may have changed; force a fresh session on next access.
        registry.sessions.remove(&id);
        persist_locked(&mut registry)?;
    }

    apply_password_policy(&id, password.as_deref(), remember_password);
    Ok(connection)
}

/// Removes a saved connection, its stored credential, and any live session.
#[tauri::command]
#[specta::specta]
pub fn delete_connection(id: String) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("connection registry poisoned");
    registry.saved.retain(|existing| existing.id != id);
    registry.memory_passwords.remove(&id);
    registry.sessions.remove(&id);
    persist_locked(&mut registry)?;
    drop(registry);

    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &id) {
        // The credential may never have been stored; ignore that case.
        let _ = entry.delete_credential();
    }
    Ok(())
}

/// The credentials a backend should use when connecting to a server.
/// Anonymous (both `None`) when nothing was saved — the anonymous-first flow.
pub fn resolve_credentials(
    protocol: Protocol,
    host: &str,
    port: Option<u16>,
) -> (Option<String>, Option<String>) {
    let id = session_key(protocol, host, port);

    let username = {
        let registry = REGISTRY.lock().expect("connection registry poisoned");
        registry
            .saved
            .iter()
            .find(|existing| existing.id == id)
            .and_then(|existing| existing.username.clone())
    };

    let password = password_for(&id);
    (username, password)
}

/// Returns the cached live session for a server, if one is open.
pub fn session_for(protocol: Protocol, host: &str, port: Option<u16>) -> Option<SharedBackend> {
    let key = session_key(protocol, host, port);
    let registry = REGISTRY.lock().expect("connection registry poisoned");
    registry.sessions.get(&key).cloned()
}

/// Caches (or replaces) the live session for a server.
pub fn store_session(protocol: Protocol, host: &str, port: Option<u16>, backend: SharedBackend) {
    let key = session_key(protocol, host, port);
    let mut registry = REGISTRY.lock().expect("connection registry poisoned");
    registry.sessions.insert(key, backend);
}

fn password_for(id: &str) -> Option<String> {
    {
        let registry = REGISTRY.lock().expect("connection registry poisoned");
        if let Some(password) = registry.memory_passwords.get(id) {
            return Some(password.clone());
        }
    }

    let entry = keyring::Entry::new(KEYRING_SERVICE, id).ok()?;
    entry.get_password().ok()
}

fn apply_password_policy(id: &str, password: Option<&str>, remember_password: bool) {
    let Some(password) = password.map(str::trim).filter(|p| !p.is_empty()) else {
        // No password supplied: keep whatever credential is already stored.
        return;
    };

    if !remember_password {
        let mut registry = REGISTRY.lock().expect("connection registry poisoned");
        registry
            .memory_passwords
            .insert(id.to_owned(), password.to_owned());
        drop(registry);

        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id) {
            let _ = entry.delete_credential();
        }
        return;
    }

    match keyring::Entry::new(KEYRING_SERVICE, id).and_then(|entry| entry.set_password(password)) {
        Ok(()) => {
            let mut registry = REGISTRY.lock().expect("connection registry poisoned");
            registry.memory_passwords.remove(id);
        }
        Err(_) => {
            // No usable OS keychain (e.g. headless Linux): keep the password
            // for this session only.
            let mut registry = REGISTRY.lock().expect("connection registry poisoned");
            registry
                .memory_passwords
                .insert(id.to_owned(), password.to_owned());
        }
    }
}

fn persist_locked(registry: &mut RegistryInner) -> Result<(), FileSystemError> {
    let Some(config_dir) = registry.config_dir.clone() else {
        return Err(FileSystemError::Internal(
            "The connection store was not initialized".into(),
        ));
    };

    fs::create_dir_all(&config_dir)?;
    let contents = serde_json::to_string_pretty(&registry.saved)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    super::sidebar::write_atomic(&connections_path(&config_dir), contents.as_bytes())
}

fn connections_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join(CONNECTIONS_FILE_NAME)
}

fn read_connections_file(path: &std::path::Path) -> Result<Vec<StoredConnection>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| FileSystemError::Internal(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

/// Points the store at `config_dir` and reloads from disk, without an app
/// handle. Test-only.
#[cfg(test)]
pub(crate) fn use_config_dir_for_tests(config_dir: PathBuf) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("connection registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.saved = read_connections_file(&connections_path(&config_dir))?;
    registry.sessions.clear();
    registry.memory_passwords.clear();
    Ok(())
}

/// Exercises the connection store through its public commands. One test
/// function because the registry is a process-global singleton.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_updates_and_deletes_connections() {
        let config_dir =
            std::env::temp_dir().join(format!("dae-connections-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&config_dir);
        use_config_dir_for_tests(config_dir.clone()).expect("initialize store");

        let saved = save_connection(SaveConnectionInput {
            protocol: Protocol::Smb,
            host: "  MyServer.Local  ".into(),
            port: Some(445),
            username: Some(" alice ".into()),
            password: Some("session-only".into()),
            // Session memory instead of the OS keychain keeps the test hermetic.
            remember_password: false,
        })
        .expect("save connection");

        assert_eq!(saved.id, "smb://myserver.local:445");
        assert_eq!(saved.host, "myserver.local");
        assert_eq!(saved.username.as_deref(), Some("alice"));

        let listed = list_connections().expect("list connections");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], saved);

        let persisted =
            fs::read_to_string(config_dir.join("connections.json")).expect("connections file exists");
        assert!(
            !persisted.contains("password"),
            "passwords must never be persisted"
        );
        assert!(!persisted.contains("session-only"));

        let (username, password) =
            resolve_credentials(Protocol::Smb, "MyServer.Local", Some(445));
        assert_eq!(username.as_deref(), Some("alice"));
        assert_eq!(password.as_deref(), Some("session-only"));

        // Saving the same server again updates in place and keeps the credential.
        save_connection(SaveConnectionInput {
            protocol: Protocol::Smb,
            host: "myserver.local".into(),
            port: Some(445),
            username: Some("bob".into()),
            password: None,
            remember_password: false,
        })
        .expect("update connection");

        let listed = list_connections().expect("list after update");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].username.as_deref(), Some("bob"));
        let (_, password) =
            resolve_credentials(Protocol::Smb, "myserver.local", Some(445));
        assert_eq!(password.as_deref(), Some("session-only"));

        // Reopening the store reloads from disk.
        use_config_dir_for_tests(config_dir.clone()).expect("reload store");
        let listed = list_connections().expect("list after reload");
        assert_eq!(listed.len(), 1);

        delete_connection("smb://myserver.local:445".into()).expect("delete connection");
        assert!(
            list_connections()
                .expect("list after delete")
                .is_empty()
        );
        let (_, password) =
            resolve_credentials(Protocol::Smb, "myserver.local", Some(445));
        assert_eq!(password, None);

        fs::remove_dir_all(config_dir).expect("remove test directory");
    }
}
