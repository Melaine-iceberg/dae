use super::directory::{
    EntryKind, entry_kind, entry_kind_rank, modified_at_millis, path_to_string,
};
use super::error::FileSystemError;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Ordering;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::Manager;

const MAX_SEARCH_RESULTS: usize = 200;

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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntry {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub entries: Vec<SearchEntry>,
    pub truncated: bool,
}

/// Recursively searches entry names beneath one directory using ripgrep's
/// filesystem traversal engine. A newer request cancels any older traversal.
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
        search_directory_sync(PathBuf::from(path), &query, || {
            search_app.state::<FileSearchState>().is_current(generation)
        })
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

pub(super) fn search_directory_sync(
    requested_path: PathBuf,
    query: &str,
    is_current: impl Fn() -> bool,
) -> Result<SearchResponse, FileSystemError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchResponse {
            entries: Vec::new(),
            truncated: false,
        });
    }

    let path = requested_path.canonicalize()?;
    if !fs::metadata(&path)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    let folded_query = query.to_lowercase();
    let mut builder = WalkBuilder::new(&path);
    builder.standard_filters(false).follow_links(false);

    let mut entries = Vec::new();
    let mut truncated = false;

    for result in builder.build() {
        if !is_current() {
            break;
        }

        let entry = match result {
            Ok(entry) => entry,
            // Inaccessible descendants should not prevent useful partial results.
            Err(_) => continue,
        };

        if entry.depth() == 0 {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().contains(&folded_query) {
            continue;
        }

        if entries.len() == MAX_SEARCH_RESULTS {
            truncated = true;
            break;
        }

        let relative_path = entry.path().strip_prefix(&path).unwrap_or(entry.path());
        let kind = entry
            .file_type()
            .map(entry_kind)
            .unwrap_or(EntryKind::Other);
        let metadata = entry.metadata().ok();
        let modified_at = metadata.as_ref().and_then(modified_at_millis);
        let size = matches!(&kind, EntryKind::File)
            .then(|| metadata.as_ref().map(|metadata| metadata.len()))
            .flatten();

        entries.push(SearchEntry {
            name,
            path: path_to_string(entry.path()),
            relative_path: path_to_string(relative_path),
            kind,
            modified_at,
            size,
        });
    }

    entries.sort_by(|left, right| compare_search_entries(left, right, &folded_query));

    Ok(SearchResponse { entries, truncated })
}

fn compare_search_entries(left: &SearchEntry, right: &SearchEntry, query: &str) -> Ordering {
    let left_name = left.name.to_lowercase();
    let right_name = right.name.to_lowercase();

    match_rank(&left_name, query)
        .cmp(&match_rank(&right_name, query))
        .then_with(|| entry_kind_rank(&left.kind).cmp(&entry_kind_rank(&right.kind)))
        .then_with(|| left_name.cmp(&right_name))
        .then_with(|| left.relative_path.cmp(&right.relative_path))
}

fn match_rank(name: &str, query: &str) -> u8 {
    if name == query {
        0
    } else if name.starts_with(query) {
        1
    } else {
        2
    }
}
