//! Search commands for the explorer: recursive name search on any backend and
//! ripgrep-backed content search on local paths. A generation counter cancels
//! stale traversals when a newer query arrives or the search surface closes.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::types::{ContentSearchResponse, SearchResponse};
use dae_core::file_system::{local, vfs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Claims the next generation for a traversal. Older traversals stop as soon
/// as they observe a generation that is no longer theirs.
pub fn begin() -> u64 {
    GENERATION.fetch_add(1, Ordering::AcqRel) + 1
}

/// Invalidates every search traversal that is still running.
pub fn cancel() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn is_current(generation: u64) -> bool {
    GENERATION.load(Ordering::Acquire) == generation
}

/// Recursively searches entry names beneath one directory.
pub fn directory(
    path: String,
    query: String,
    generation: u64,
) -> Result<SearchResponse, FileSystemError> {
    let backend = vfs::resolve(&path)?;
    let is_current = || is_current(generation);
    backend.search(&path, &query, &is_current)
}

/// Searches file contents beneath one local directory with optional regex,
/// case sensitivity, and file-type filtering. Ignores VCS/dependency
/// directories (`.git`, `node_modules`, `target`) by default.
pub fn file_contents(
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
    generation: u64,
) -> Result<ContentSearchResponse, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.content_search_local_only".into(),
        ));
    }

    let is_current = move || is_current(generation);
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
}
