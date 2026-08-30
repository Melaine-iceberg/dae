//! Directory change observation across backends. The local backend uses OS
//! file notifications; every other backend falls back to snapshot polling.

use crate::file_system::error::FileSystemError;
use crate::file_system::local;
use crate::file_system::types::{entry_kind_rank, DirectoryView};
use crate::file_system::vfs::{self, Scheme, SharedBackend};
use notify::RecommendedWatcher;
use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;
use tauri::Manager;
use tauri_specta::Event;

const POLL_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-changed")]
pub struct DirectoryChanged(pub String);

/// The active observation. Dropping a `Notify` handle stops its OS watcher;
/// a `Poll` handle owns a stop flag that its poller thread checks each tick.
pub enum WatchHandle {
    Notify(RecommendedWatcher),
    Poll(Arc<AtomicBool>),
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        match self {
            // Dropping the notify watcher stops it; pollers need the signal.
            WatchHandle::Notify(watcher) => {
                let _ = &*watcher;
            }
            WatchHandle::Poll(stop) => stop.store(true, AtomicOrdering::Relaxed),
        }
    }
}

#[derive(Default)]
pub struct DirectoryWatcher {
    generation: AtomicU64,
    watcher: Mutex<Option<WatchHandle>>,
}

impl DirectoryWatcher {
    pub fn begin_update(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    pub fn replace(&self, generation: u64, handle: WatchHandle) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = Some(handle);
        }

        Ok(())
    }
}

/// Replaces the active watcher with one that tracks the currently displayed directory.
#[tauri::command]
#[specta::specta]
pub async fn watch_directory(path: String, app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let generation = app.state::<DirectoryWatcher>().begin_update();

    if vfs::split_scheme(&path)?.0 == Scheme::Local {
        let watcher_app = app.clone();
        let watcher = tauri::async_runtime::spawn_blocking(move || {
            local::create_directory_watcher(PathBuf::from(path), watcher_app)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))??;

        app.state::<DirectoryWatcher>()
            .replace(generation, WatchHandle::Notify(watcher))
    } else {
        let watch_path = path.clone();
        let backend = tauri::async_runtime::spawn_blocking(move || vfs::resolve(&watch_path))
            .await
            .map_err(|error| FileSystemError::Internal(error.to_string()))??;

        let handle = spawn_polling_watcher(path, backend, app.clone());
        app.state::<DirectoryWatcher>().replace(generation, handle)
    }
}

/// Watches `path` on `backend` by diffing directory snapshots until the
/// returned handle is dropped. Transient read errors skip a tick instead of
/// killing the poller, so a briefly unreachable server does not blind the
/// explorer permanently.
pub fn spawn_polling_watcher(
    path: String,
    backend: SharedBackend,
    app: tauri::AppHandle,
) -> WatchHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    thread::spawn(move || {
        let mut snapshot = match backend.read_dir(&path) {
            Ok(view) => fingerprint(&view),
            // The explorer just read this directory; failing here means the
            // session went away between the two calls. Nothing to watch.
            Err(_) => return,
        };

        loop {
            thread::sleep(POLL_INTERVAL);

            if stop_flag.load(AtomicOrdering::Relaxed) {
                return;
            }

            match backend.read_dir(&path) {
                Ok(view) => {
                    let current = fingerprint(&view);
                    if current != snapshot {
                        snapshot = current;
                        let _ = DirectoryChanged(view.path.clone()).emit(&app);
                    }
                }
                Err(_) => continue,
            }
        }
    });

    WatchHandle::Poll(stop)
}

/// Folds the snapshot into a 64-bit hash instead of building one formatted
/// `String` per entry every poll tick, which keeps polling watchers free of
/// per-entry heap allocations.
fn fingerprint(view: &DirectoryView) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    view.entries.len().hash(&mut hasher);
    for entry in &view.entries {
        entry.name.hash(&mut hasher);
        entry_kind_rank(&entry.kind).hash(&mut hasher);
        entry.size.hash(&mut hasher);
        entry.modified_at.hash(&mut hasher);
    }
    hasher.finish()
}
