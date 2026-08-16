use super::error::FileSystemError;
use super::local;
use super::progress::{FileOperationKind, FileOperationProgressReporter, emit_preparing};
use super::transfer::{self, TransferSource};
use super::types::{DirectoryView, NewEntryKind, SearchResponse, path_to_string};
use super::vfs::{self, Scheme};
use super::watch::{DirectoryWatcher, WatchHandle, spawn_polling_watcher};
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
///
/// `vfs::resolve` can open a network session (a blocking, runtime-owning
/// operation), so it must run on a blocking thread — never on the async
/// workers, where a nested runtime panics.
#[tauri::command]
#[specta::specta]
pub async fn read_directory(path: String) -> Result<DirectoryView, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || vfs::resolve(&path)?.read_dir(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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

/// Recursively searches entry names beneath one directory. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_directory(
    path: String,
    query: String,
    app: tauri::AppHandle,
) -> Result<SearchResponse, FileSystemError> {
    let generation = app.state::<FileSearchState>().begin();
    let search_app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let backend = vfs::resolve(&path)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&path)?.rename_entry(&path, &new_name)
    })
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
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&directory)?.create_entry(&directory, &name, kind)
    })
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
    emit_preparing(&app, &operation_id, FileOperationKind::Copy);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_sources(sources)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Copy);

        if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| PathBuf::from(&source.path))
                .collect::<Vec<_>>();
            local::copy_entries_with_progress(paths, PathBuf::from(&destination), &progress)
        } else {
            transfer::copy_entries(sources, &destination, &destination_backend, &progress)
        }
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
    emit_preparing(&app, &operation_id, FileOperationKind::Move);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_sources(sources)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Move);

        if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| PathBuf::from(&source.path))
                .collect::<Vec<_>>();
            local::move_entries_with_progress(paths, PathBuf::from(&destination), &progress)
        } else {
            transfer::move_entries(sources, &destination, &destination_backend, &progress)
        }
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
    if paths.is_empty() {
        return Ok(());
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let targets = resolve_sources(paths)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);

        if targets.iter().all(|target| is_local_path(&target.path)) {
            let paths = targets
                .iter()
                .map(|target| PathBuf::from(&target.path))
                .collect::<Vec<_>>();
            local::delete_entries_with_progress(paths, &progress)
        } else {
            transfer::delete_entries(targets, &progress)
        }
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn resolve_sources(paths: Vec<String>) -> Result<Vec<TransferSource>, FileSystemError> {
    paths
        .into_iter()
        .map(|path| {
            let backend = vfs::resolve(&path)?;
            Ok(TransferSource { path, backend })
        })
        .collect()
}

/// Duplicates entries next to their originals ("name 副本"), returning the
/// created paths so the UI can refresh and select them.
#[tauri::command]
#[specta::specta]
pub async fn duplicate_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<String>, FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before duplicating".into(),
        ));
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Copy);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_sources(paths)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Copy);
        transfer::duplicate_sources(sources, &progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn is_local_path(path: &str) -> bool {
    vfs::scheme_of(path).is_ok_and(|scheme| scheme == Scheme::Local)
}

/// True when every source and the destination sit on the local backend, which
/// keeps its native rayon transfer path instead of the streaming engine.
fn is_pure_local(sources: &[TransferSource], destination: &str) -> bool {
    is_local_path(destination) && sources.iter().all(|source| is_local_path(&source.path))
}
