use super::directory::{
    EntryKind, entry_kind, entry_kind_rank, modified_at_millis, path_to_string,
};
use super::error::FileSystemError;
use ignore::{WalkBuilder, WalkState};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Ordering;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::Mutex;
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
        search_directory_sync(PathBuf::from(path), &query, move || {
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

pub(super) fn search_directory_sync<F>(
    requested_path: PathBuf,
    query: &str,
    is_current: F,
) -> Result<SearchResponse, FileSystemError>
where
    F: Fn() -> bool + Send + Sync + 'static,
{
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
    let shared = Arc::new(SearchShared {
        entries: Mutex::new(Vec::new()),
        truncated: AtomicBool::new(false),
        is_current,
    });
    let root = Arc::new(path);

    let mut builder = WalkBuilder::new(root.as_path());
    builder.standard_filters(false).follow_links(false);

    {
        let run_shared = Arc::clone(&shared);
        let run_root = Arc::clone(&root);
        let run_query: Arc<str> = Arc::from(folded_query.as_str());

        builder.build_parallel().run(move || {
            let shared = Arc::clone(&run_shared);
            let root = Arc::clone(&run_root);
            let folded_query = Arc::clone(&run_query);

            Box::new(move |result| {
                if shared.truncated.load(AtomicOrdering::Relaxed) || !(shared.is_current)() {
                    return WalkState::Quit;
                }

                let entry = match result {
                    Ok(entry) => entry,
                    // Inaccessible descendants should not prevent useful partial results.
                    Err(_) => return WalkState::Continue,
                };

                if entry.depth() == 0 {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.to_lowercase().contains(&*folded_query) {
                    return WalkState::Continue;
                }

                let should_stop = {
                    let mut entries = shared
                        .entries
                        .lock()
                        .expect("search results lock poisoned");

                    if entries.len() >= MAX_SEARCH_RESULTS {
                        true
                    } else {
                        entries.push(build_search_entry(&entry, &name, &root));
                        false
                    }
                };

                if should_stop {
                    shared.truncated.store(true, AtomicOrdering::Relaxed);
                    return WalkState::Quit;
                }

                WalkState::Continue
            })
        });
    }

    let mut entries = shared
        .entries
        .lock()
        .expect("search results lock poisoned")
        .drain(..)
        .collect::<Vec<_>>();
    let truncated = shared.truncated.load(AtomicOrdering::Relaxed);

    entries.sort_by(|left, right| compare_search_entries(left, right, &folded_query));

    Ok(SearchResponse { entries, truncated })
}

struct SearchShared<F> {
    entries: Mutex<Vec<SearchEntry>>,
    truncated: AtomicBool,
    is_current: F,
}

fn build_search_entry(
    entry: &ignore::DirEntry,
    name: &str,
    root: &PathBuf,
) -> SearchEntry {
    let relative_path = entry.path().strip_prefix(root).unwrap_or(entry.path());
    let kind = entry
        .file_type()
        .map(entry_kind)
        .unwrap_or(EntryKind::Other);
    let metadata = entry.metadata().ok();
    let modified_at = metadata.as_ref().and_then(modified_at_millis);
    let size = matches!(&kind, EntryKind::File)
        .then(|| metadata.as_ref().map(|metadata| metadata.len()))
        .flatten();

    SearchEntry {
        name: name.to_owned(),
        path: path_to_string(entry.path()),
        relative_path: path_to_string(relative_path),
        kind,
        modified_at,
        size,
    }
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
