use super::directory::{entry_kind, entry_state_flags, modified_at_millis};
use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    EntryKind, SearchEntry, SearchResponse, entry_kind_rank, path_to_string,
};
use ignore::{WalkBuilder, WalkState};
use std::cmp::Ordering;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

const MAX_SEARCH_RESULTS: usize = 200;

pub fn search_directory_sync(
    requested_path: PathBuf,
    query: &str,
    is_current: &(dyn Fn() -> bool + Send + Sync),
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
                    let mut entries = shared.entries.lock().expect("search results lock poisoned");

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

struct SearchShared<'a> {
    entries: Mutex<Vec<SearchEntry>>,
    truncated: AtomicBool,
    is_current: &'a (dyn Fn() -> bool + Send + Sync),
}

fn build_search_entry(entry: &ignore::DirEntry, name: &str, root: &PathBuf) -> SearchEntry {
    let relative_path = entry.path().strip_prefix(root).unwrap_or(entry.path());
    let kind = entry
        .file_type()
        .map(entry_kind)
        .unwrap_or(EntryKind::Other);
    // With `follow_links(false)` this is the link's own metadata for
    // symlinks, unlike directory listings which follow the target.
    let metadata = entry.metadata().ok();
    let modified_at = metadata.as_ref().and_then(modified_at_millis);
    let size = matches!(&kind, EntryKind::File)
        .then(|| metadata.as_ref().map(|metadata| metadata.len()))
        .flatten();
    let (hidden, read_only) = metadata
        .as_ref()
        .map(|metadata| entry_state_flags(metadata, name))
        .unwrap_or_default();

    SearchEntry {
        name: name.to_owned(),
        path: path_to_string(entry.path()),
        relative_path: path_to_string(relative_path),
        kind,
        modified_at,
        size,
        hidden,
        read_only,
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
