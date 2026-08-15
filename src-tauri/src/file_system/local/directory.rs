use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, entry_sort_key, path_to_string,
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri_specta::Event;

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-changed")]
pub struct DirectoryChanged(pub String);

#[derive(Default)]
pub struct DirectoryWatcher {
    generation: AtomicU64,
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl DirectoryWatcher {
    pub fn begin_update(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    pub fn replace(&self, generation: u64, watcher: RecommendedWatcher) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = Some(watcher);
        }

        Ok(())
    }

    /// Drops the active watcher, e.g. after navigating somewhere no backend
    /// can observe with OS file notifications.
    pub fn clear(&self, generation: u64) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = None;
        }

        Ok(())
    }
}

pub fn create_directory_watcher(
    requested_path: PathBuf,
    app: tauri::AppHandle,
) -> Result<RecommendedWatcher, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let event_path = path_to_string(&path);
    let mut watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            // Watcher errors (e.g. ReadDirectoryChangesW buffer overflow on network
            // shares) mean changes were dropped, so treat them as "possibly dirty".
            let should_refresh = match &result {
                Ok(event) => !matches!(event.kind, EventKind::Access(_)),
                Err(_) => true,
            };

            if should_refresh {
                let _ = DirectoryChanged(event_path.clone()).emit(&app);
            }
        })
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    Ok(watcher)
}

pub fn read_directory_sync(requested_path: PathBuf) -> Result<DirectoryView, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let metadata = fs::metadata(&path)?;

    if !metadata.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    // Entries whose metadata fails to load (WSL's /proc and /run over the
    // `\\wsl$` 9P share behave this way) are skipped so the directory itself
    // stays readable instead of failing as a whole.
    let mut entries = fs::read_dir(&path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;
            let metadata = entry.metadata().ok()?;
            let kind = entry_kind(file_type);
            let size = matches!(&kind, EntryKind::File).then_some(metadata.len());

            Some(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: path_to_string(&entry.path()),
                kind,
                modified_at: modified_at_millis(&metadata),
                size,
            })
        })
        .collect::<Vec<_>>();

    entries.sort_by_cached_key(entry_sort_key);

    Ok(DirectoryView {
        path: path_to_string(&path),
        breadcrumbs: build_breadcrumbs(&path),
        entries,
    })
}

pub fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

pub fn entry_kind(file_type: fs::FileType) -> EntryKind {
    if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

pub fn build_breadcrumbs(path: &Path) -> Vec<Breadcrumb> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();

    ancestors
        .into_iter()
        .map(|ancestor| Breadcrumb {
            name: ancestor
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_to_string(ancestor)),
            path: path_to_string(ancestor),
        })
        .collect()
}
