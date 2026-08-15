use super::error::FileSystemError;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::Manager;
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
    fn begin_update(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn replace(&self, generation: u64, watcher: RecommendedWatcher) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = Some(watcher);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Breadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    /// The absolute, canonical path that was read.
    pub path: String,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub entries: Vec<DirectoryEntry>,
}

/// Returns the operating system's home directory for the initial explorer view.
#[tauri::command]
#[specta::specta]
pub fn get_home_directory(app: tauri::AppHandle) -> Result<String, FileSystemError> {
    app.path()
        .home_dir()
        .map(|path| path_to_string(&path))
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

/// Reads one directory as an immutable snapshot suitable for rendering in the explorer.
#[tauri::command]
#[specta::specta]
pub async fn read_directory(path: String) -> Result<DirectoryView, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || read_directory_sync(PathBuf::from(path)))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Replaces the active watcher with one that tracks the currently displayed directory.
#[tauri::command]
#[specta::specta]
pub async fn watch_directory(path: String, app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let generation = app.state::<DirectoryWatcher>().begin_update();
    let watcher_app = app.clone();
    let watcher = tauri::async_runtime::spawn_blocking(move || {
        create_directory_watcher(PathBuf::from(path), watcher_app)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    app.state::<DirectoryWatcher>().replace(generation, watcher)
}

fn create_directory_watcher(
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

pub(super) fn read_directory_sync(
    requested_path: PathBuf,
) -> Result<DirectoryView, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let metadata = fs::metadata(&path)?;

    if !metadata.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    let mut entries = fs::read_dir(&path)?
        .map(|entry| {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let metadata = entry.metadata()?;
            let kind = entry_kind(file_type);
            let size = matches!(&kind, EntryKind::File).then_some(metadata.len());

            Ok(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: path_to_string(&entry.path()),
                kind,
                modified_at: modified_at_millis(&metadata),
                size,
            })
        })
        .collect::<Result<Vec<_>, FileSystemError>>()?;

    entries.sort_by_cached_key(entry_sort_key);

    Ok(DirectoryView {
        path: path_to_string(&path),
        breadcrumbs: build_breadcrumbs(&path),
        entries,
    })
}

pub(super) fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

pub(super) fn entry_kind(file_type: fs::FileType) -> EntryKind {
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

pub(super) fn entry_sort_key(entry: &DirectoryEntry) -> (u8, String, String) {
    (
        entry_kind_rank(&entry.kind),
        entry.name.to_lowercase(),
        entry.name.clone(),
    )
}

pub(super) fn entry_kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::Symlink => 1,
        EntryKind::File => 2,
        EntryKind::Other => 3,
    }
}

pub(super) fn build_breadcrumbs(path: &Path) -> Vec<Breadcrumb> {
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

pub(super) fn path_to_string(path: &Path) -> String {
    normalize_path_for_display(&path.to_string_lossy())
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn normalize_path_for_display(path: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }

        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    }

    #[cfg(not(windows))]
    path.to_owned()
}
