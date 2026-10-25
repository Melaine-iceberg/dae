//! Search commands for the explorer: recursive name search on any backend
//! and ripgrep-backed content search on local paths. A generation counter in
//! [`FileSearchState`] cancels stale traversals when a newer query arrives.

use super::error::FileSystemError;
use super::local;
use super::types::{ContentSearchResponse, SearchResponse};
use super::vfs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::OnceLock;

#[derive(Default)]
pub struct FileSearchState {
    generation: AtomicU64,
}

/// The process-wide generation counter. One app per process, so this is the
/// lifetime the managed state had; tests keep constructing their own.
fn state() -> &'static FileSearchState {
    static STATE: OnceLock<FileSearchState> = OnceLock::new();
    STATE.get_or_init(FileSearchState::default)
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

/// See `commands::search_directory` for the contract; this half is engine-free.
pub async fn search_directory(path: String, query: String) -> Result<SearchResponse, FileSystemError> {
    let generation = state().begin();

    tokio::task::spawn_blocking(move || {
        let backend = vfs::resolve(&path)?;
        let is_current = || state().is_current(generation);
        backend.search(&path, &query, &is_current)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::cancel_search` for the contract; this half is engine-free.
pub fn cancel_search() {
    state().cancel();
}

/// See `commands::search_file_contents` for the contract; this half is engine-free.
pub async fn search_file_contents(
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
) -> Result<ContentSearchResponse, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.content_search_local_only".into(),
        ));
    }

    let generation = state().begin();

    tokio::task::spawn_blocking(move || {
        let is_current = move || state().is_current(generation);
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
