//! Streaming folder-size calculation for the explorer's detail view.
//!
//! Folder sizes need a recursive scan, so they cannot ship with the
//! `read_dir` snapshot without blocking the listing. The frontend requests
//! sizes for the visible folders after paint and receives partial totals as
//! events while each scan runs — mirroring Windows Explorer's size column.

use super::error::FileSystemError;
use super::types::path_to_string;
use super::vfs::{self, Scheme};
use ignore::{WalkBuilder, WalkState};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tauri_specta::Event;

/// How often partial totals are pushed to the UI while a scan runs.
const SIZE_PROGRESS_INTERVAL_MS: u64 = 250;

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-size-progress")]
#[serde(rename_all = "camelCase")]
pub struct DirectorySizeProgress {
    pub operation_id: String,
    pub path: String,
    pub size: u64,
    /// False while the scan runs (partial total); true on the final total.
    pub completed: bool,
}

/// Live scans keyed by operation id, so navigating away can stop a scan.
#[derive(Default)]
pub struct DirectorySizeState {
    operations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl DirectorySizeState {
    fn register(&self, operation_id: &str) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.operations
            .lock()
            .expect("directory size state lock poisoned")
            .insert(operation_id.to_owned(), Arc::clone(&cancelled));
        cancelled
    }

    fn forget(&self, operation_id: &str) {
        self.operations
            .lock()
            .expect("directory size state lock poisoned")
            .remove(operation_id);
    }

    fn cancel(&self, operation_id: &str) {
        if let Some(cancelled) = self
            .operations
            .lock()
            .expect("directory size state lock poisoned")
            .remove(operation_id)
        {
            cancelled.store(true, AtomicOrdering::Release);
        }
    }
}

/// Starts a background size scan for the given local directories. Partial
/// totals stream as [`DirectorySizeProgress`] events until each directory
/// finishes with `completed: true`. Non-local paths and non-directories are
/// skipped quietly; the caller cancels the operation when the view changes.
#[tauri::command]
#[specta::specta]
pub fn start_directory_size_calculation(
    operation_id: String,
    paths: Vec<String>,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    let directories: Vec<PathBuf> = paths
        .iter()
        .filter(|path| vfs::scheme_of(path).is_ok_and(|scheme| scheme == Scheme::Local))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .collect();

    if directories.is_empty() {
        return Ok(());
    }

    let cancelled = app.state::<DirectorySizeState>().register(&operation_id);
    let done_app = app.clone();
    let done_operation_id = operation_id.clone();

    std::thread::spawn(move || {
        for directory in directories {
            if cancelled.load(AtomicOrdering::Acquire) {
                break;
            }
            scan_directory(&app, &operation_id, &directory, &cancelled);
        }
        done_app.state::<DirectorySizeState>().forget(&done_operation_id);
    });

    Ok(())
}

/// Stops an in-flight size scan when the view navigates away.
#[tauri::command]
#[specta::specta]
pub fn cancel_directory_size_calculation(operation_id: String, app: tauri::AppHandle) {
    app.state::<DirectorySizeState>().cancel(&operation_id);
}

/// Sums every file under `directory` with a parallel walk, streaming partial
/// totals. Symlinks are never followed, so symlink loops cannot hang a scan.
fn scan_directory(
    app: &tauri::AppHandle,
    operation_id: &str,
    directory: &Path,
    cancelled: &AtomicBool,
) {
    let last_emit_ms = AtomicU64::new(0);
    let total = walk_directory_size(directory, cancelled, &|current| {
        emit_size(app, operation_id, directory, current, &last_emit_ms, false);
    });

    if !cancelled.load(AtomicOrdering::Relaxed) {
        emit_size(app, operation_id, directory, total, &last_emit_ms, true);
    }
}

/// Walks `directory` in parallel and sums the bytes of every regular file,
/// reporting the running total to `progress` after each counted file.
/// Returns the final total; a pre-set or mid-walk `cancelled` flag stops the
/// traversal early (the returned sum is then partial).
pub(super) fn walk_directory_size(
    directory: &Path,
    cancelled: &AtomicBool,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> u64 {
    let total = AtomicU64::new(0);

    let mut builder = WalkBuilder::new(directory);
    // Count everything the shell would, including hidden files; none of the
    // standard ignore filters apply to a size report.
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false);

    builder.build_parallel().run(|| {
        Box::new(|result| {
            if cancelled.load(AtomicOrdering::Relaxed) {
                return WalkState::Quit;
            }

            if let Ok(entry) = result
                && entry
                    .file_type()
                    .is_some_and(|file_type| file_type.is_file())
                && let Ok(metadata) = entry.metadata()
            {
                let length = metadata.len();
                let current = total.fetch_add(length, AtomicOrdering::Relaxed) + length;
                progress(current);
            }

            WalkState::Continue
        })
    });

    total.load(AtomicOrdering::Relaxed)
}

/// Emits a running total at most once per throttle window. The
/// compare-exchange lets exactly one walker thread win each window without a
/// lock on the hot path.
fn emit_size(
    app: &tauri::AppHandle,
    operation_id: &str,
    path: &Path,
    size: u64,
    last_emit_ms: &AtomicU64,
    force: bool,
) {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let last_ms = last_emit_ms.load(AtomicOrdering::Relaxed);

    if !force {
        if now_ms < last_ms + SIZE_PROGRESS_INTERVAL_MS {
            return;
        }
        if last_emit_ms
            .compare_exchange(last_ms, now_ms, AtomicOrdering::AcqRel, AtomicOrdering::Relaxed)
            .is_err()
        {
            return;
        }
    }

    let _ = DirectorySizeProgress {
        operation_id: operation_id.to_owned(),
        path: path_to_string(path),
        size,
        completed: force,
    }
    .emit(app);
}
