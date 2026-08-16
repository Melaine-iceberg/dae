use super::error::FileSystemError;
use super::sidebar::write_atomic;
use super::types::{display_name_from_path, EntryKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const RECENTS_FILE_NAME: &str = "recent-items.json";
const MAX_RECENT_ITEMS: usize = 300;

/// How the item entered the recent list: a folder the user browsed, or a file
/// the user opened with the system handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum RecentSource {
    Visited,
    Opened,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentItem {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub source: RecentSource,
    /// Milliseconds since the Unix epoch.
    pub accessed_at: u64,
}

/// Loads the recent items list, most recently used first. Empty on first launch.
#[tauri::command]
#[specta::specta]
pub fn list_recents(app: tauri::AppHandle) -> Result<Vec<RecentItem>, FileSystemError> {
    let path = recents_path(&app)?;
    read_recents(&path)
}

/// Records one access, moving an existing entry for the same path to the front.
/// Returns the updated list so callers can sync their local state.
#[tauri::command]
#[specta::specta]
pub fn record_recent(
    app: tauri::AppHandle,
    path: String,
    kind: EntryKind,
    source: RecentSource,
) -> Result<Vec<RecentItem>, FileSystemError> {
    let store_path = recents_path(&app)?;
    let mut items = read_recents(&store_path)?;

    let item = RecentItem {
        name: display_name_from_path(&path),
        path,
        kind,
        source,
        accessed_at: now_millis(),
    };
    upsert_recent(&mut items, item, MAX_RECENT_ITEMS);
    write_recents(&store_path, &items)?;

    Ok(items)
}

/// Removes one path from the recent list, returning the updated list.
#[tauri::command]
#[specta::specta]
pub fn remove_recent(
    app: tauri::AppHandle,
    path: String,
) -> Result<Vec<RecentItem>, FileSystemError> {
    let store_path = recents_path(&app)?;
    let mut items = read_recents(&store_path)?;
    items.retain(|item| item.path != path);
    write_recents(&store_path, &items)?;
    Ok(items)
}

/// Clears the whole recent list. This never touches the files themselves.
#[tauri::command]
#[specta::specta]
pub fn clear_recents(app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let store_path = recents_path(&app)?;
    write_recents(&store_path, &[])
}

/// Inserts at the front after removing any existing entry for the same path,
/// then enforces the cap. Extracted for unit testing.
pub(super) fn upsert_recent(items: &mut Vec<RecentItem>, item: RecentItem, cap: usize) {
    items.retain(|existing| existing.path != item.path);
    items.insert(0, item);
    items.truncate(cap);
}

fn read_recents(path: &std::path::Path) -> Result<Vec<RecentItem>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let mut items: Vec<RecentItem> = serde_json::from_str(&contents)
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            items.sort_by(|a, b| b.accessed_at.cmp(&a.accessed_at));
            Ok(items)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_recents(path: &std::path::Path, items: &[RecentItem]) -> Result<(), FileSystemError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(items)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(path, contents.as_bytes())
}

fn recents_path(app: &tauri::AppHandle) -> Result<PathBuf, FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    Ok(config_dir.join(RECENTS_FILE_NAME))
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
