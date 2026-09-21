use super::error::FileSystemError;
use super::sidebar::write_atomic;
use super::types::{EntryKind, display_name_from_path};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// The recent list, held in memory for the process's lifetime.
///
/// Recording an access runs on every directory visit and every file open, and
/// each call used to read the whole JSON file back, re-sort it, and rewrite it —
/// on the invoke thread, which is the UI thread. Keeping the list here turns a
/// record into an in-memory edit plus one write and leaves the first read as the
/// only one. This process is the file's only writer, so the cache is
/// authoritative.
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
    /// Locks the store, reading the file in on first use.
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

    /// The current list, loading it from disk only on first use.
    pub(super) fn snapshot(&self, path: &Path) -> Result<Vec<RecentItem>, FileSystemError> {
        Ok(self.loaded(path)?.items.clone())
    }

    /// Applies `edit` to the list and writes the result back.
    ///
    /// The lock is held across the write on purpose. `write_atomic` stages
    /// through one fixed temp name, so two records interleaving their
    /// temp-file-and-rename pairs could clobber each other's staging file; the
    /// lock also makes the read-modify-write atomic, which the previous
    /// read-then-rewrite shape was not — two concurrent records could lose one.
    /// Both are acceptable because the whole operation now runs on the blocking
    /// pool rather than the thread that paints.
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

/// Loads the recent items list, most recently used first. Empty on first launch.
#[tauri::command]
#[specta::specta]
pub fn list_recents(app: tauri::AppHandle) -> Result<Vec<RecentItem>, FileSystemError> {
    if let Some(recents) = app
        .state::<super::prefetch::StartupPrefetch>()
        .take_recents()
    {
        return Ok(recents);
    }
    let path = recents_path(&app)?;
    app.state::<RecentsState>().snapshot(&path)
}

/// Records one access, moving an existing entry for the same path to the front.
/// Returns the updated list so callers can sync their local state.
///
/// Blocking file I/O on the blocking pool rather than on the invoke thread: this
/// runs on every directory visit and every file open, and the write is a
/// temp-file-and-rename under the app config directory, which an antivirus
/// scanner or a roamed profile can turn into milliseconds.
#[tauri::command]
#[specta::specta]
pub async fn record_recent(
    app: tauri::AppHandle,
    path: String,
    kind: EntryKind,
    source: RecentSource,
) -> Result<Vec<RecentItem>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let store_path = recents_path(&app)?;
        let item = RecentItem {
            name: display_name_from_path(&path),
            path,
            kind,
            source,
            accessed_at: now_millis(),
        };

        app.state::<RecentsState>().edit(&store_path, |items| {
            upsert_recent(items, item, MAX_RECENT_ITEMS);
            items.clone()
        })
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Removes one path from the recent list, returning the updated list.
#[tauri::command]
#[specta::specta]
pub async fn remove_recent(
    app: tauri::AppHandle,
    path: String,
) -> Result<Vec<RecentItem>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let store_path = recents_path(&app)?;
        app.state::<RecentsState>().edit(&store_path, |items| {
            items.retain(|item| item.path != path);
            items.clone()
        })
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Clears the whole recent list. This never touches the files themselves.
#[tauri::command]
#[specta::specta]
pub async fn clear_recents(app: tauri::AppHandle) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let store_path = recents_path(&app)?;
        app.state::<RecentsState>()
            .edit(&store_path, |items| items.clear())
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
            items.sort_by_key(|item| std::cmp::Reverse(item.accessed_at));
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

        // Re-recording an existing path moves it to the front without duplicating.
        upsert_recent(&mut items, make("b", 4), 10);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].path, "b");
        assert_eq!(items[0].accessed_at, 4);

        // The cap evicts the least recently used entries.
        upsert_recent(&mut items, make("d", 5), 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "d");
        assert_eq!(items[1].path, "b");
    }

    /// 一次记录只应读一次文件，之后缓存就是权威。
    ///
    /// 这条测试是缓存本身的守卫：如果哪天 `edit` 又回到「每次读全文件、改、写回」，
    /// 它会当场失败——`record_recent` 每次进目录、每次打开文件都跑。
    #[test]
    fn reads_the_file_once_then_treats_the_cache_as_authoritative() {
        let directory = temp_recents_dir();
        let path = directory.join(RECENTS_FILE_NAME);
        let state = RecentsState::default();

        // 首次使用：文件不存在，读到空表。
        assert!(state.snapshot(&path).expect("first load").is_empty());

        // 背后把文件改掉。缓存已加载，所以再取快照应该看不到这次改动。
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

        // A fresh state over the same file loads what was written, which is what
        // makes the cache safe to treat as authoritative.
        let reloaded = RecentsState::default();
        assert_eq!(names(reloaded.snapshot(&path).expect("reload")), ["b", "a"]);

        let _ = fs::remove_dir_all(&directory);
    }

    fn temp_recents_dir() -> PathBuf {
        // 每次一个独立目录：测试二进制并行跑，共用目录会互相删文件。
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
