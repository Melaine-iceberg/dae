//! Streaming folder-size calculation for the explorer's detail view.
//!
//! Folder sizes need a recursive scan, so they cannot ship with the
//! `read_dir` snapshot without blocking the listing. The frontend requests
//! sizes for the visible folders after paint and receives partial totals as
//! events while each scan runs — mirroring Windows Explorer's size column.

use super::error::FileSystemError;
use super::types::path_to_string;
use super::url::Scheme;
use super::vfs;
use ignore::{WalkBuilder, WalkState};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::host::{Event as UiEvent, Host};
use std::sync::OnceLock;

/// How often partial totals are pushed to the UI while a scan runs.
const SIZE_PROGRESS_INTERVAL_MS: u64 = 250;

/// Entries a size walk visits on the calling thread before it hands the tree
/// to the parallel walker. The parallel pool costs ~4 ms to start (measured on
/// a 16-thread machine), while a serial entry costs well under a microsecond,
/// so every small folder is cheaper without it; this bound keeps the detour a
/// large tree pays below that fixed cost.
const SERIAL_ENTRY_LIMIT: u64 = 512;

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

/// See `commands::start_directory_size_calculation` for the contract; this half is engine-free.
pub fn start_directory_size_calculation(
    operation_id: String,
    paths: Vec<String>,
    host: std::sync::Arc<dyn crate::host::Host>,
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

    let cancelled = state().register(&operation_id);
    let done_operation_id = operation_id.clone();

    std::thread::spawn(move || {
        for directory in directories {
            if cancelled.load(AtomicOrdering::Acquire) {
                break;
            }
            scan_directory(host.as_ref(), &operation_id, &directory, &cancelled);
        }
        state().forget(&done_operation_id);
    });

    Ok(())
}

/// See `commands::cancel_directory_size_calculation` for the contract; this half is engine-free.
pub fn cancel_directory_size_calculation(operation_id: String) {
    state().cancel(&operation_id);
}

/// The process-wide registry of size scans. One app per process, so this is the
/// lifetime the managed state had; tests keep constructing their own.
fn state() -> &'static DirectorySizeState {
    static STATE: OnceLock<DirectorySizeState> = OnceLock::new();
    STATE.get_or_init(DirectorySizeState::default)
}

/// Sums every file under `directory` with a parallel walk, streaming partial
/// totals. Symlinks are never followed, so symlink loops cannot hang a scan.
fn scan_directory(host: &dyn Host, operation_id: &str, directory: &Path, cancelled: &AtomicBool) {
    let last_emit_ms = AtomicU64::new(0);
    let total = walk_directory_size(directory, cancelled, &|current| {
        emit_size(host, operation_id, directory, current, &last_emit_ms, false);
    });

    if !cancelled.load(AtomicOrdering::Relaxed) {
        emit_size(host, operation_id, directory, total, &last_emit_ms, true);
    }
}

/// Sums one directory without streaming progress, for callers that want the
/// number for a whole selection at once (the listing's "calculate size").
/// The cancellation flag is checked per entry, so a cancelled walk returns the
/// partial total it had reached.
pub fn measure_directory(directory: &Path, cancelled: &AtomicBool) -> u64 {
    walk_directory_size(directory, cancelled, &|_| {})
}

/// Walks `directory` and sums the bytes of every regular file, reporting the
/// running total to `progress` after each counted file.
///
/// Small trees are summed on the calling thread: `ignore` splits *directories*
/// across threads, never the entries inside one, so a folder that fits in
/// [`SERIAL_ENTRY_LIMIT`] entries gains nothing from a pool that costs ~4 ms to
/// start. A larger tree restarts under the parallel walker, so the serial
/// attempt is the only extra work it pays.
///
/// Returns the final total; a pre-set or mid-walk `cancelled` flag stops the
/// traversal early (the returned sum is then partial).
pub(super) fn walk_directory_size(
    directory: &Path,
    cancelled: &AtomicBool,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> u64 {
    if let Some((total, reported)) = walk_small_tree(directory, cancelled) {
        // Replayed only once the serial walk owns the answer: a partial total
        // from an attempt that handed the tree to the parallel walker must not
        // reach the UI, and the progress contract is one report per counted
        // file either way.
        for current in reported {
            progress(current);
        }
        return total;
    }

    let total = AtomicU64::new(0);

    size_builder(directory).build_parallel().run(|| {
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

/// The options every size walk uses: everything the shell would count, no
/// standard ignore filters, and symlinks never followed.
fn size_builder(directory: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(directory);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false);
    builder
}

/// Sums the tree on the calling thread while it stays within
/// [`SERIAL_ENTRY_LIMIT`] entries, buffering the per-file running totals the
/// caller replays through `progress` once this arm owns the answer. `None`
/// means the tree is larger and the caller has to restart it under the
/// parallel walker, discarding the partial total reached here.
fn walk_small_tree(directory: &Path, cancelled: &AtomicBool) -> Option<(u64, Vec<u64>)> {
    let mut total = 0u64;
    let mut visited = 0u64;
    let mut reported = Vec::new();

    for result in size_builder(directory).build() {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Some((total, reported));
        }

        visited += 1;
        if visited > SERIAL_ENTRY_LIMIT {
            return None;
        }

        if let Ok(entry) = result
            && entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            && let Ok(metadata) = entry.metadata()
        {
            total += metadata.len();
            reported.push(total);
        }
    }

    Some((total, reported))
}

/// Emits a running total at most once per throttle window. The
/// compare-exchange lets exactly one walker thread win each window without a
/// lock on the hot path.
fn emit_size(
    host: &dyn Host,
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
            .compare_exchange(
                last_ms,
                now_ms,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Relaxed,
            )
            .is_err()
        {
            return;
        }
    }

    host.emit(UiEvent::DirectorySizeProgress {
        operation_id: operation_id.to_owned(),
        path: path_to_string(path),
        size,
        completed: force,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn walks_directory_sizes_including_nested_and_hidden_files() {
        let directory =
            std::env::temp_dir().join(format!("dae-folder-size-test-{}", std::process::id()));
        let nested = directory.join("nested");
        fs::create_dir_all(&nested).expect("create test directory");
        fs::write(directory.join("root.txt"), "root").expect("write root file");
        fs::write(directory.join(".hidden"), "hidden!").expect("write hidden file");
        fs::write(nested.join("leaf.bin"), "leaf-content").expect("write nested file");

        let cancelled = AtomicBool::new(false);
        let last_reported = AtomicU64::new(0);
        let report_count = AtomicU64::new(0);

        let total = walk_directory_size(&directory, &cancelled, &|current| {
            last_reported.store(current, AtomicOrdering::Relaxed);
            report_count.fetch_add(1, AtomicOrdering::Relaxed);
        });

        // Every file — including the hidden and the nested one — is counted
        // exactly once, so the last progress report carries the final total.
        assert_eq!(total, 4 + 7 + 12);
        assert_eq!(last_reported.load(AtomicOrdering::Relaxed), total);
        assert_eq!(report_count.load(AtomicOrdering::Relaxed), 3);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    /// Prints what one folder-size walk costs on a real tree.
    ///
    /// ```bash
    /// cd crates/dae-core
    /// DAE_DIRECTORY_SIZE_BENCH=K:/dae-listing-bench/big \
    ///   cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_directory_size_walk
    /// ```
    ///
    /// The tree is whatever the variable names; `bench/listing-pipeline
    /// prepare` writes a 35,803-entry one from the repository root. `files` is
    /// how many regular files the walk counted, so `per file` is the fixed cost
    /// of visiting one entry and `throughput` the bytes half — a tree of empty
    /// files isolates the former. A tree within `SERIAL_ENTRY_LIMIT` entries is
    /// summed on the calling thread; past it the parallel walker takes over.
    /// That walker spreads *directories* across threads, never the files inside
    /// one directory, so a single 35,803-entry directory stays on one thread in
    /// both arms. Run a flat tree and a deep one when judging a change. Compare
    /// this test against itself, before and after a change; absolute
    /// milliseconds are per-machine.
    #[test]
    #[ignore = "prints measurements for the directory in DAE_DIRECTORY_SIZE_BENCH"]
    fn reports_the_cost_of_a_directory_size_walk() {
        use std::time::Instant;

        let Some(root) = std::env::var_os("DAE_DIRECTORY_SIZE_BENCH") else {
            panic!("set DAE_DIRECTORY_SIZE_BENCH to a directory to measure");
        };
        let root = PathBuf::from(root);
        assert!(root.is_dir(), "{} is not a directory", root.display());

        // One untimed walk counts the files the timed walk will visit, so the
        // per-entry figure and the shipped answer come from the same entries.
        let cancelled = AtomicBool::new(false);
        let counted = AtomicU64::new(0);
        let expected = walk_directory_size(&root, &cancelled, &|_| {
            counted.fetch_add(1, AtomicOrdering::Relaxed);
        });
        let files = counted.load(AtomicOrdering::Relaxed);

        // Two warm passes, then seven timed ones; a walk that summed a
        // different number is a bug, not a faster run.
        for _ in 0..2 {
            assert_eq!(measure_directory(&root, &cancelled), expected);
        }
        let mut samples = Vec::with_capacity(7);
        for _ in 0..7 {
            let started = Instant::now();
            let measured = measure_directory(&root, &cancelled);
            samples.push(started.elapsed().as_nanos());
            assert_eq!(measured, expected);
        }
        samples.sort_unstable();

        let median_ms = samples[samples.len() / 2] as f64 / 1e6;
        let mebibytes = expected as f64 / (1024.0 * 1024.0);

        println!("root       {}", root.display());
        println!("files      {files}");
        println!("bytes      {expected}  ({mebibytes:.1} MiB)");
        println!("median     {median_ms:.2} ms");
        println!(
            "min/max    {:.2} / {:.2} ms",
            samples[0] as f64 / 1e6,
            samples[samples.len() - 1] as f64 / 1e6
        );
        println!(
            "per file   {:.2} us/file",
            median_ms * 1000.0 / files.max(1) as f64
        );
        println!(
            "throughput {:.1} MiB/s",
            mebibytes / (median_ms / 1000.0).max(f64::MIN_POSITIVE)
        );
    }

    /// Past `SERIAL_ENTRY_LIMIT` entries the serial arm hands the tree to the
    /// parallel walker; the restart must count every file exactly once.
    #[test]
    fn falls_back_to_the_parallel_walk_past_the_serial_limit() {
        let directory =
            std::env::temp_dir().join(format!("dae-folder-size-limit-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        let files = SERIAL_ENTRY_LIMIT as usize + 8;
        for index in 0..files {
            fs::write(directory.join(format!("file-{index:04}.txt")), "x")
                .expect("write test file");
        }

        let cancelled = AtomicBool::new(false);
        let total = measure_directory(&directory, &cancelled);

        assert_eq!(total, files as u64);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn cancels_directory_size_walks_before_they_start() {
        let directory =
            std::env::temp_dir().join(format!("dae-folder-size-cancel-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(directory.join("file.txt"), "content").expect("write file");

        let cancelled = AtomicBool::new(true);
        let report_count = AtomicU64::new(0);
        let total = walk_directory_size(&directory, &cancelled, &|_| {
            report_count.fetch_add(1, AtomicOrdering::Relaxed);
        });

        assert_eq!(total, 0);
        assert_eq!(report_count.load(AtomicOrdering::Relaxed), 0);

        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
