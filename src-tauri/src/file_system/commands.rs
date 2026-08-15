use super::error::FileSystemError;
use super::local::{self, DirectoryWatcher};
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, emit_preparing,
};
use super::types::{DirectoryView, NewEntryKind, SearchResponse, path_to_string};
use super::vfs::{self, Scheme};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::Manager;

#[derive(Default)]
pub struct FileSearchState {
    generation: AtomicU64,
}

impl FileSearchState {
    fn begin(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(AtomicOrdering::Acquire) == generation
    }
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
    let backend = vfs::resolve(&path)?;

    tauri::async_runtime::spawn_blocking(move || backend.read_dir(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Replaces the active watcher with one that tracks the currently displayed directory.
#[tauri::command]
#[specta::specta]
pub async fn watch_directory(path: String, app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let generation = app.state::<DirectoryWatcher>().begin_update();

    // Only the local backend exposes OS file notifications today; network
    // backends will plug polling or protocol-native notifications in here.
    if vfs::split_scheme(&path)?.0 != Scheme::Local {
        return app.state::<DirectoryWatcher>().clear(generation);
    }

    let watcher_app = app.clone();
    let watcher = tauri::async_runtime::spawn_blocking(move || {
        local::create_directory_watcher(PathBuf::from(path), watcher_app)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    app.state::<DirectoryWatcher>().replace(generation, watcher)
}

/// Recursively searches entry names beneath one directory. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_directory(
    path: String,
    query: String,
    app: tauri::AppHandle,
) -> Result<SearchResponse, FileSystemError> {
    let backend = vfs::resolve(&path)?;
    let generation = app.state::<FileSearchState>().begin();
    let search_app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let state = search_app.state::<FileSearchState>();
        let is_current = || state.is_current(generation);
        backend.search(&path, &query, &is_current)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Stops the active traversal when the search surface is dismissed.
#[tauri::command]
#[specta::specta]
pub fn cancel_search(app: tauri::AppHandle) {
    app.state::<FileSearchState>().cancel();
}

/// Renames a single directory entry without allowing a path change.
#[tauri::command]
#[specta::specta]
pub async fn rename_entry(path: String, new_name: String) -> Result<(), FileSystemError> {
    let backend = vfs::resolve(&path)?;

    tauri::async_runtime::spawn_blocking(move || backend.rename_entry(&path, &new_name))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Creates a new file or directory inside an existing directory and returns its path.
#[tauri::command]
#[specta::specta]
pub async fn create_entry(
    directory: String,
    name: String,
    kind: NewEntryKind,
) -> Result<String, FileSystemError> {
    let backend = vfs::resolve(&directory)?;

    tauri::async_runtime::spawn_blocking(move || backend.create_entry(&directory, &name, kind))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Copies entries into an existing destination directory. Existing files are never overwritten.
#[tauri::command]
#[specta::specta]
pub async fn copy_entries(
    sources: Vec<String>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    vfs::ensure_same_backend(&sources, &destination, "Copying")?;
    let backend = vfs::resolve(&destination)?;
    emit_preparing(&app, &operation_id, FileOperationKind::Copy);

    tauri::async_runtime::spawn_blocking(move || {
        let progress = FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Copy);
        backend.copy(&sources, &destination, &progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Moves entries into an existing destination directory. Existing files are never overwritten.
#[tauri::command]
#[specta::specta]
pub async fn move_entries(
    sources: Vec<String>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    vfs::ensure_same_backend(&sources, &destination, "Moving")?;
    let backend = vfs::resolve(&destination)?;
    emit_preparing(&app, &operation_id, FileOperationKind::Move);

    tauri::async_runtime::spawn_blocking(move || {
        let progress = FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Move);
        backend.move_entries(&sources, &destination, &progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes entries. The UI must obtain confirmation before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn delete_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    if let Some(first) = paths.first() {
        vfs::ensure_same_backend(&paths, first, "Deleting")?;
    }
    let backend = vfs::resolve(paths.first().map(String::as_str).unwrap_or_default())?;
    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let progress = FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);
        backend.delete(&paths, &progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}
