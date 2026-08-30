//! Search commands for the explorer: recursive name search on any backend
//! and ripgrep-backed content search on local paths. A generation counter in
//! [`FileSearchState`] cancels stale traversals when a newer query arrives.

use super::error::FileSystemError;
use super::local;
use super::types::{ContentSearchResponse, SearchResponse};
use super::vfs;
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

/// Searches file contents beneath one local directory with optional regex,
/// case sensitivity, and file-type filtering. Ignores VCS/dependency
/// directories (`.git`, `node_modules`, `target`) by default. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_file_contents(
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
    app: tauri::AppHandle,
) -> Result<ContentSearchResponse, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.content_search_local_only".into(),
        ));
    }

    let generation = app.state::<FileSearchState>().begin();
    let search_app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let state = search_app.state::<FileSearchState>();
        let is_current = move || state.is_current(generation);
        local::search_file_contents_sync(
            PathBuf::from(path),
            &local::ContentSearchParams {
                query: &query,
                is_regex,
                case_sensitive,
                file_filter: file_filter.as_deref(),
            },
            &is_current,
        )
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}
