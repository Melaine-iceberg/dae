//! The recent-items store behind the Electron shell.
//!
//! The document (`recent-items.json`) and the directory it lives in are the same
//! ones the Tauri shell used, so a migrated install keeps its history. Recording
//! runs on every directory visit and every file open, so the list is loaded once
//! and kept in memory; this process is the file's only writer.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::fs_atomic::write_atomic;
use dae_core::file_system::types::{EntryKind, display_name_from_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const RECENTS_FILE_NAME: &str = "recent-items.json";
const MAX_RECENT_ITEMS: usize = 300;

/// How the item entered the recent list: a folder the user browsed, or a file
/// the user opened with the system handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecentSource {
    Visited,
    Opened,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentItem {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub source: RecentSource,
    /// Milliseconds since the Unix epoch.
    pub accessed_at: u64,
}

#[derive(Default)]
pub struct RecentsState {
    store: Mutex<RecentStore>,
}

#[derive(Default)]
struct RecentStore {
    /// False until the file has been read at least once.
    loaded: bool,
    items: Vec<RecentItem>,
}

impl RecentsState {
    fn loaded(
        &self,
        path: &Path,
    ) -> Result<std::sync::MutexGuard<'_, RecentStore>, FileSystemError> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| FileSystemError::Internal("fs.recents_lock_poisoned".into()))?;

        if !store.loaded {
            store.items = read_recents(path)?;
            store.loaded = true;
        }

        Ok(store)
    }

    fn snapshot(&self, path: &Path) -> Result<Vec<RecentItem>, FileSystemError> {
        Ok(self.loaded(path)?.items.clone())
    }

    /// Applies `edit` to the list and writes the result back. The lock is held
    /// across the write so two concurrent records cannot lose one another and
    /// cannot stage through the same fixed temp name.
    fn edit<T>(
        &self,
        path: &Path,
        edit: impl FnOnce(&mut Vec<RecentItem>) -> T,
    ) -> Result<T, FileSystemError> {
        let mut store = self.loaded(path)?;
        let result = edit(&mut store.items);
        write_recents(path, &store.items)?;
        Ok(result)
    }
}

static RECENTS: LazyLock<RecentsState> = LazyLock::new(RecentsState::default);

fn recents_path() -> PathBuf {
    crate::config_dir().join(RECENTS_FILE_NAME)
}

/// Loads the recent items list, most recently used first. Empty on first launch.
pub fn list() -> Result<Vec<RecentItem>, FileSystemError> {
    RECENTS.snapshot(&recents_path())
}

/// Records one access, moving an existing entry for the same path to the front.
/// Returns the updated list so callers can sync their local state.
pub fn record(
    path: String,
    kind: EntryKind,
    source: RecentSource,
) -> Result<Vec<RecentItem>, FileSystemError> {
    let store_path = recents_path();
    let item = RecentItem {
        name: display_name_from_path(&path),
        path,
        kind,
        source,
        accessed_at: now_millis(),
    };

    RECENTS.edit(&store_path, |items| {
        upsert_recent(items, item, MAX_RECENT_ITEMS);
        items.clone()
    })
}

/// Removes one path from the recent list, returning the updated list.
pub fn remove(path: String) -> Result<Vec<RecentItem>, FileSystemError> {
    RECENTS.edit(&recents_path(), |items| {
        items.retain(|item| item.path != path);
        items.clone()
    })
}

/// Clears the whole recent list. This never touches the files themselves.
pub fn clear() -> Result<(), FileSystemError> {
    RECENTS.edit(&recents_path(), |items| items.clear())
}

/// Inserts at the front after removing any existing entry for the same path,
/// then enforces the cap.
fn upsert_recent(items: &mut Vec<RecentItem>, item: RecentItem, cap: usize) {
    items.retain(|existing| existing.path != item.path);
    items.insert(0, item);
    items.truncate(cap);
}

fn read_recents(path: &Path) -> Result<Vec<RecentItem>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let mut items: Vec<RecentItem> = serde_json::from_str(&contents)
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            items.sort_by_key(|item| std::cmp::Reverse(item.accessed_at));
            Ok(items)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_recents(path: &Path, items: &[RecentItem]) -> Result<(), FileSystemError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(items)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(path, contents.as_bytes())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_recent_dedupes_orders_and_caps() {
        let make = |path: &str, accessed_at: u64| RecentItem {
            path: path.to_string(),
            name: path.to_string(),
            kind: EntryKind::File,
            source: RecentSource::Opened,
            accessed_at,
        };

        let mut items = vec![make("a", 1), make("b", 2), make("c", 3)];

        upsert_recent(&mut items, make("b", 4), 10);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].path, "b");
        assert_eq!(items[0].accessed_at, 4);

        upsert_recent(&mut items, make("d", 5), 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "d");
        assert_eq!(items[1].path, "b");
    }

    #[test]
    fn reads_the_file_once_then_treats_the_cache_as_authoritative() {
        let directory = temp_recents_dir();
        let path = directory.join(RECENTS_FILE_NAME);
        let state = RecentsState::default();

        assert!(state.snapshot(&path).expect("first load").is_empty());

        write_recents(&path, &[item("external", 1)]).expect("write behind the cache");
        assert!(
            state.snapshot(&path).expect("cached load").is_empty(),
            "the cached list is authoritative; a second read would see this"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_edit_reaches_both_the_cache_and_the_file() {
        let directory = temp_recents_dir();
        let path = directory.join(RECENTS_FILE_NAME);
        let state = RecentsState::default();

        state
            .edit(&path, |items| upsert_recent(items, item("a", 1), 10))
            .expect("record a");
        state
            .edit(&path, |items| upsert_recent(items, item("b", 2), 10))
            .expect("record b");

        let names = |items: Vec<RecentItem>| {
            items
                .into_iter()
                .map(|entry| entry.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(state.snapshot(&path).expect("snapshot")), ["b", "a"]);
        assert_eq!(
            names(read_recents(&path).expect("read back")),
            ["b", "a"],
            "the file has to agree with the cache"
        );

        let reloaded = RecentsState::default();
        assert_eq!(names(reloaded.snapshot(&path).expect("reload")), ["b", "a"]);

        let _ = fs::remove_dir_all(&directory);
    }

    fn temp_recents_dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

        let directory = std::env::temp_dir().join(format!(
            "dae-recents-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create temp dir");
        directory
    }

    fn item(path: &str, accessed_at: u64) -> RecentItem {
        RecentItem {
            path: path.to_string(),
            name: path.to_string(),
            kind: EntryKind::File,
            source: RecentSource::Opened,
            accessed_at,
        }
    }
}
