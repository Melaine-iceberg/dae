//! Directory change observation across backends. The local backend uses OS
//! file notifications; every other backend falls back to snapshot polling.

use crate::file_system::error::FileSystemError;
use crate::file_system::local;
use crate::file_system::types::{DirectoryView, entry_kind_rank, path_to_string};
use crate::file_system::vfs::SharedBackend;
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

/// Arms the OS watcher for a local directory, replacing whatever was watched
/// before.
///
/// `canonical_path` must be canonical (see [`crate::file_system::types::canonical_path`])
/// — the listing this accompanies reports that spelling, and the events have
/// to name the same directory.
///
/// Failing to watch is not fatal: the explorer still lists the directory, it
/// just stops seeing live changes, so the error is logged rather than
/// propagated into the read the user is waiting for.
pub fn arm_local_watcher(app: &tauri::AppHandle, canonical_path: PathBuf) {
    let generation = app.state::<DirectoryWatcher>().begin_update();

    match local::create_directory_watcher(canonical_path.clone(), app.clone()) {
        Ok(watcher) => {
            if let Err(error) = app
                .state::<DirectoryWatcher>()
                .replace(generation, WatchHandle::Notify(watcher))
            {
                eprintln!(
                    "Unable to store the directory watcher for {}: {error}",
                    path_to_string(&canonical_path)
                );
            }
        }
        Err(error) => eprintln!(
            "Unable to watch {} for changes: {error}",
            path_to_string(&canonical_path)
        ),
    }
}

/// Arms a snapshot-polling watcher for a non-local directory, replacing
/// whatever was watched before. Failures are logged, never fatal (see
/// [`arm_local_watcher`]).
pub fn arm_polling_watcher(app: &tauri::AppHandle, path: &str, backend: SharedBackend) {
    let generation = app.state::<DirectoryWatcher>().begin_update();
    let handle = spawn_polling_watcher(path.to_owned(), backend, app.clone());

    if let Err(error) = app.state::<DirectoryWatcher>().replace(generation, handle) {
        eprintln!("Unable to store the polling watcher for {path}: {error}");
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
