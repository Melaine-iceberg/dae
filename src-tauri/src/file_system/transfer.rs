//! Generic transfer engine that moves entries between any two backends
//! through the trait's primitive operations (stat, mkdir, streams, remove).
//! Local-to-local transfers keep the backend-native rayon path in
//! `local/operations.rs`; every other combination lands here.

use super::error::FileSystemError;
use super::progress::FileOperationProgressReporterTrait;
use super::types::{
    ConflictAction, DirectoryEntry, EntryKind, EntryStat, TransferConflict, TransferPair,
};
use super::vfs::{FileSystemBackend, SharedBackend};
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{channel, sync_channel};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, RwLock};
use std::thread;

const STREAM_CHUNK_BYTES: usize = 256 * 1024;

/// Requests the engine keeps in flight against a remote backend.
///
/// Every backend reached through this module is network-bound, so the number
/// that decides throughput is how many requests are outstanding, not how many
/// cores the machine has. Single digits is deliberate: the per-request win is
/// mostly spent by four, while SMB servers, SFTP sessions, and cloud APIs all
/// start throttling — or failing requests outright — when a client opens a
/// core-count-sized burst.
const CROSS_BACKEND_CONCURRENCY: usize = 4;

/// Pool the cross-backend engine runs on.
///
/// The local bulk path has its own core-sized pool in `local::operations`;
/// this one is small and fixed because it caps *outstanding requests*, not CPU
/// work. Keeping it separate also means a large remote transfer cannot starve
/// the directory listings that paint the explorer.
fn cross_backend_pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();

    POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(CROSS_BACKEND_CONCURRENCY)
            .thread_name(|index| format!("dae-remote-{index}"))
            .build()
            .expect("build the cross-backend transfer pool")
    })
}

/// Collects results from concurrent tasks back into submission order.
///
/// The journal handed to the undo layer reads far better — and tests stay
/// deterministic — when entries appear in the order the caller asked for,
/// even though the work finishes in whatever order the network allows.
struct OrderedSink<T> {
    slots: Mutex<Vec<(usize, T)>>,
}

impl<T> OrderedSink<T> {
    fn new() -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, index: usize, value: T) {
        self.lock().push((index, value));
    }

    /// Appends everything collected so far to `out`, in the order the values
    /// were indexed. Call it even when the work failed: entries that did land
    /// still belong in the journal so the caller can undo a partial transfer.
    fn drain_into(self, out: &mut Vec<T>) {
        let mut slots = self
            .slots
            .into_inner()
            .unwrap_or_else(|error| error.into_inner());
        slots.sort_by_key(|(index, _)| *index);
        out.extend(slots.into_iter().map(|(_, value)| value));
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(usize, T)>> {
        self.slots.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// Chunks that may queue between the reading and the writing side. Two is the
/// minimum that keeps a read outstanding while a write is in flight; a deeper
/// queue would only hold more of the file in memory.
const STREAM_PIPELINE_DEPTH: usize = 2;

/// Locale-dependent token appended to duplicate names ("副本" / "copy" / …).
/// The frontend pushes the localized token at startup and on language
/// changes; the Chinese default keeps unit tests independent of UI state.
static DUPLICATE_SUFFIX: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new("副本".to_owned()));

/// Sets the suffix used for duplicate naming (Keep-Both conflicts and the
/// duplicate action). Empty input is ignored to avoid corrupting names.
pub fn set_duplicate_suffix(suffix: &str) {
    if suffix.is_empty() {
        return;
    }
    if let Ok(mut lock) = DUPLICATE_SUFFIX.write() {
        *lock = suffix.to_owned();
    }
}

/// One source entry paired with the backend that serves it.
pub struct TransferSource {
    pub path: String,
    pub backend: SharedBackend,
    /// How the destination is resolved when the target name already exists.
    pub on_conflict: ConflictAction,
}

struct PlanEntry {
    source: TransferSource,
    stat: EntryStat,
    destination: String,
    /// The existing destination entry a `Replace` entry deletes first.
    replacement: Option<EntryStat>,
}

/// Copies sources into `destination` (a directory on `destination_backend`).
/// Each source's [`ConflictAction`] decides what happens when the target
/// name already exists; `Fail` keeps the legacy "never overwrite" behavior.
/// Every completed entry is appended to `journal` so callers can build an
/// undo record even when a later entry fails.
pub fn copy_entries(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
    progress: &dyn FileOperationProgressReporterTrait,
    journal: &mut Vec<TransferPair>,
) -> Result<(), FileSystemError> {
    cross_backend_pool().install(|| {
        let plan = build_plan(sources, destination, destination_backend)?;
        progress.start(
            plan.par_iter()
                .map(|entry| {
                    count_copy_units(
                        entry.source.backend.as_ref(),
                        &entry.source.path,
                        &entry.stat,
                    )
                    .and_then(|units| add_replacement_units(destination_backend, entry, units))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum(),
        );

        let completed = OrderedSink::new();
        let outcome = plan.par_iter().enumerate().try_for_each(
            |(index, entry)| -> Result<(), FileSystemError> {
                progress.begin_entry(Path::new(&entry.source.path));
                remove_replacement(destination_backend, entry, progress)?;
                copy_node(
                    entry.source.backend.as_ref(),
                    &entry.source.path,
                    &entry.stat,
                    destination_backend.as_ref(),
                    &entry.destination,
                    progress,
                )?;
                completed.push(
                    index,
                    TransferPair {
                        source: entry.source.path.clone(),
                        destination: entry.destination.clone(),
                    },
                );
                Ok(())
            },
        );

        // Journalled whether or not the batch finished: a copy that failed
        // halfway still needs an undo record for what it did land.
        completed.drain_into(journal);
        outcome?;

        progress.finish();
        Ok(())
    })
}

/// Moves sources into `destination`, preferring a protocol-native rename when
/// source and destination share one backend, falling back to copy + delete.
/// Each source's [`ConflictAction`] decides what happens when the target name
/// already exists. Every completed entry is appended to `journal` so callers
/// can build an undo record even when a later entry fails.
pub fn move_entries(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
    progress: &dyn FileOperationProgressReporterTrait,
    journal: &mut Vec<TransferPair>,
) -> Result<(), FileSystemError> {
    cross_backend_pool().install(|| {
        let plan = build_plan(sources, destination, destination_backend)?;
        progress.start(
            plan.par_iter()
                .map(|entry| {
                    count_copy_units(
                        entry.source.backend.as_ref(),
                        &entry.source.path,
                        &entry.stat,
                    )
                    .and_then(|copy| {
                        count_delete_units(
                            entry.source.backend.as_ref(),
                            &entry.source.path,
                            &entry.stat,
                        )
                        .map(|delete| copy + delete)
                    })
                    .and_then(|units| add_replacement_units(destination_backend, entry, units))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum(),
        );

        let completed = OrderedSink::new();
        let move_one = |index: usize, entry: &PlanEntry| -> Result<(), FileSystemError> {
            progress.begin_entry(Path::new(&entry.source.path));
            remove_replacement(destination_backend, entry, progress)?;

            let native_rename = if Arc::ptr_eq(&entry.source.backend, destination_backend) {
                entry
                    .source
                    .backend
                    .rename_to(&entry.source.path, &entry.destination)
                    .is_ok()
            } else {
                false
            };

            if native_rename {
                let units = count_copy_units(
                    entry.source.backend.as_ref(),
                    &entry.source.path,
                    &entry.stat,
                )? + count_delete_units(
                    entry.source.backend.as_ref(),
                    &entry.source.path,
                    &entry.stat,
                )?;
                progress.advance_by(units, Path::new(&entry.source.path));
            } else {
                copy_node(
                    entry.source.backend.as_ref(),
                    &entry.source.path,
                    &entry.stat,
                    destination_backend.as_ref(),
                    &entry.destination,
                    progress,
                )?;
                delete_node(
                    entry.source.backend.as_ref(),
                    &entry.source.path,
                    &entry.stat,
                    progress,
                )?;
            }

            completed.push(
                index,
                TransferPair {
                    source: entry.source.path.clone(),
                    destination: entry.destination.clone(),
                },
            );
            Ok(())
        };

        let outcome = plan
            .par_iter()
            .enumerate()
            .try_for_each(|(index, entry)| move_one(index, entry));

        completed.drain_into(journal);
        outcome?;

        progress.finish();
        Ok(())
    })
}

/// Work units contributed by a `Replace` entry's existing destination tree.
fn add_replacement_units(
    destination_backend: &SharedBackend,
    entry: &PlanEntry,
    units: u64,
) -> Result<u64, FileSystemError> {
    match &entry.replacement {
        Some(stat) => {
            Ok(units + count_delete_units(destination_backend.as_ref(), &entry.destination, stat)?)
        }
        None => Ok(units),
    }
}

/// Deletes the existing destination tree of a `Replace` entry before the
/// source is transferred onto its path.
fn remove_replacement(
    destination_backend: &SharedBackend,
    entry: &PlanEntry,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    match &entry.replacement {
        Some(stat) => delete_node(
            destination_backend.as_ref(),
            &entry.destination,
            stat,
            progress,
        ),
        None => Ok(()),
    }
}

/// Reports every source whose target name already exists in `destination`,
/// with both sides' metadata for the conflict dialog. Sources that would land
/// on themselves (a no-op the engine skips) are not conflicts.
pub fn find_conflicts(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
) -> Result<Vec<TransferConflict>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before pasting".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    if destination_backend.stat(destination)?.kind != EntryKind::Directory {
        return Err(FileSystemError::NotDirectory(destination.to_owned()));
    }

    // Each source costs up to two round trips (the target's stat and its own),
    // so the whole batch is resolved concurrently and reported in the order
    // the caller listed it.
    let reported: Result<Vec<Option<TransferConflict>>, FileSystemError> = cross_backend_pool()
        .install(|| {
            sources
                .par_iter()
                .map(|source| {
                    let name = source.backend.entry_name(&source.path).map_err(|_| {
                        FileSystemError::InvalidInput(format!(
                            "The root of a volume cannot be copied or moved: {}",
                            source.path
                        ))
                    })?;
                    let target = join_path(destination, &name);

                    if Arc::ptr_eq(&source.backend, destination_backend)
                        && same_backend_path(&source.path, &target)
                    {
                        return Ok(None);
                    }

                    let target_stat = match destination_backend.stat(&target) {
                        Err(FileSystemError::NotFound(_)) => return Ok(None),
                        Err(error) => return Err(error),
                        Ok(stat) => stat,
                    };
                    let source_stat = source.backend.stat(&source.path)?;

                    Ok(Some(TransferConflict {
                        source_path: source.path.clone(),
                        target_path: target,
                        name,
                        source_kind: source_stat.kind,
                        source_size: stat_size(&source_stat),
                        source_modified_at: source_stat.modified_at,
                        target_kind: target_stat.kind,
                        target_size: stat_size(&target_stat),
                        target_modified_at: target_stat.modified_at,
                    }))
                })
                .collect()
        });

    Ok(reported?.into_iter().flatten().collect())
}

fn stat_size(stat: &EntryStat) -> Option<u64> {
    (stat.kind != EntryKind::Directory).then_some(stat.size)
}

/// Deletes entries anywhere in the VFS, depth-first.
pub fn delete_entries(
    targets: Vec<TransferSource>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if targets.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before deleting".into(),
        ));
    }

    ensure_unique_paths(&targets)?;

    cross_backend_pool().install(|| {
        let stats = targets
            .par_iter()
            .map(|target| target.backend.stat(&target.path))
            .collect::<Result<Vec<_>, FileSystemError>>()?;

        progress.start(
            targets
                .par_iter()
                .zip(stats.par_iter())
                .map(|(target, stat)| {
                    count_delete_units(target.backend.as_ref(), &target.path, stat)
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum(),
        );

        targets
            .par_iter()
            .zip(stats.par_iter())
            .try_for_each(|(target, stat)| {
                progress.begin_entry(Path::new(&target.path));
                delete_node(target.backend.as_ref(), &target.path, stat, progress)
            })?;

        progress.finish();
        Ok(())
    })
}

/// Duplicates entries next to their originals with "副本" suffixes
/// ("报告.txt" → "报告 副本.txt", then "报告 副本 2.txt", …). Works on any
/// backend because each copy runs through the same engine as copy/paste.
pub fn duplicate_sources(
    sources: Vec<TransferSource>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<String>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before duplicating".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    cross_backend_pool().install(|| {
        let stats = sources
            .par_iter()
            .map(|source| source.backend.stat(&source.path))
            .collect::<Result<Vec<_>, FileSystemError>>()?;

        progress.start(
            sources
                .par_iter()
                .zip(stats.par_iter())
                .map(|(source, stat)| count_copy_units(source.backend.as_ref(), &source.path, stat))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum(),
        );

        // Two sources can only collide on a duplicate name when they sit in the
        // same directory with the same name, which `ensure_unique_paths` above
        // already rejected — so concurrent naming stays collision-free.
        let created = OrderedSink::new();
        let outcome = sources
            .par_iter()
            .zip(stats.par_iter())
            .enumerate()
            .try_for_each(|(index, (source, stat))| -> Result<(), FileSystemError> {
                progress.begin_entry(Path::new(&source.path));
                let destination = unique_duplicate_path(source)?;

                copy_node(
                    source.backend.as_ref(),
                    &source.path,
                    stat,
                    source.backend.as_ref(),
                    &destination,
                    progress,
                )?;

                created.push(index, destination);
                Ok(())
            });

        let mut created_paths = Vec::with_capacity(sources.len());
        created.drain_into(&mut created_paths);
        outcome?;

        progress.finish();
        Ok(created_paths)
    })
}

/// Computes the first free sibling path for a duplicate of `source`.
fn unique_duplicate_path(source: &TransferSource) -> Result<String, FileSystemError> {
    let name = source.backend.entry_name(&source.path).map_err(|_| {
        FileSystemError::InvalidInput(format!(
            "The root of a volume cannot be duplicated: {}",
            source.path
        ))
    })?;

    let parent = parent_path_of(&source.path).ok_or_else(|| {
        FileSystemError::InvalidInput(format!(
            "The root of a volume cannot be duplicated: {}",
            source.path
        ))
    })?;

    unique_sibling_path(source.backend.as_ref(), &parent, &name, &HashSet::new())
}

/// The first sibling of `name` in `parent` that neither exists on `backend`
/// nor appears in `reserved` (names other planned entries will claim).
fn unique_sibling_path(
    backend: &dyn FileSystemBackend,
    parent: &str,
    name: &str,
    reserved: &HashSet<String>,
) -> Result<String, FileSystemError> {
    let mut attempt = 0_u32;
    loop {
        let candidate_name = duplicate_name(name, attempt);
        let candidate = join_path(parent, &candidate_name);

        if reserved.contains(&candidate_name) {
            attempt += 1;
            continue;
        }

        match backend.stat(&candidate) {
            Err(FileSystemError::NotFound(_)) => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => attempt += 1,
        }
    }
}

/// The directory containing `path`, keeping the trailing separator style.
pub(super) fn parent_path_of(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let separator_index = trimmed.rfind(['/', '\\'])?;
    Some(trimmed[..=separator_index].to_owned())
}

/// "report.txt" → "report 副本.txt" → "report 副本 2.txt"; directories keep
/// their full name because they have no extension to preserve. The suffix
/// token follows the UI locale (see [`set_duplicate_suffix`]).
pub fn duplicate_name(name: &str, attempt: u32) -> String {
    let token = match DUPLICATE_SUFFIX.read() {
        Ok(lock) => lock.clone(),
        Err(poisoned) => poisoned.into_inner().to_owned(),
    };
    let suffix = if attempt == 0 {
        token
    } else {
        format!("{token} {}", attempt + 1)
    };

    let Some((stem, extension)) = name.rsplit_once('.') else {
        return format!("{name} {suffix}");
    };

    if stem.is_empty() {
        return format!("{name} {suffix}");
    }

    format!("{stem} {suffix}.{extension}")
}

fn build_plan(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
) -> Result<Vec<PlanEntry>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before pasting".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    if destination_backend.stat(destination)?.kind != EntryKind::Directory {
        return Err(FileSystemError::NotDirectory(destination.to_owned()));
    }

    // One `stat` (and, on cloud storage, one name lookup) per source is a
    // round trip each, so they go out together before the sequential part of
    // planning — which is what depends on the order they arrive in — runs.
    let resolved: Vec<Result<(EntryStat, String), FileSystemError>> = sources
        .par_iter()
        .map(|source| {
            let stat = source.backend.stat(&source.path)?;
            let name = source.backend.entry_name(&source.path).map_err(|_| {
                FileSystemError::InvalidInput(format!(
                    "The root of a volume cannot be copied or moved: {}",
                    source.path
                ))
            })?;
            Ok((stat, name))
        })
        .collect();

    let mut planned_names = HashSet::new();
    let mut plan = Vec::with_capacity(sources.len());

    for (source, resolved) in sources.into_iter().zip(resolved) {
        let (stat, name) = resolved?;

        if !planned_names.insert(name.clone()) {
            return Err(FileSystemError::AlreadyExists(format!(
                "Multiple selected entries have the same name: {name}"
            )));
        }

        let target = join_path(destination, &name);

        // A source that would land on itself is a no-op, not a conflict.
        if Arc::ptr_eq(&source.backend, destination_backend)
            && same_backend_path(&source.path, &target)
        {
            continue;
        }

        let (destination_path, replacement) = match destination_backend.stat(&target) {
            Err(FileSystemError::NotFound(_)) => (target, None),
            Err(error) => return Err(error),
            Ok(target_stat) => match source.on_conflict {
                ConflictAction::Fail => {
                    return Err(FileSystemError::AlreadyExists(target));
                }
                ConflictAction::Skip => continue,
                ConflictAction::Replace => (target, Some(target_stat)),
                ConflictAction::KeepBoth => {
                    let kept = unique_sibling_path(
                        destination_backend.as_ref(),
                        destination,
                        &name,
                        &planned_names,
                    )?;
                    let kept_name = destination_backend
                        .entry_name(&kept)
                        .unwrap_or_else(|_| last_segment(&kept).unwrap_or_default().to_owned());
                    planned_names.insert(kept_name);
                    (kept, None)
                }
            },
        };

        if stat.kind == EntryKind::Directory
            && Arc::ptr_eq(&source.backend, destination_backend)
            && path_contains(&source.path, &destination_path)
        {
            return Err(FileSystemError::InvalidInput(format!(
                "Cannot paste a folder into itself: {}",
                source.path
            )));
        }

        plan.push(PlanEntry {
            source,
            stat,
            destination: destination_path,
            replacement,
        });
    }

    Ok(plan)
}

/// Path equality on one backend: exact on POSIX, case-insensitive with
/// unified separators on Windows (sources arrive with `\` while joined
/// targets use `/`).
fn same_backend_path(left: &str, right: &str) -> bool {
    fn normalize(path: &str) -> String {
        let trimmed = path.trim_end_matches(['/', '\\']);

        #[cfg(windows)]
        {
            trimmed.replace('\\', "/").to_lowercase()
        }

        #[cfg(not(windows))]
        {
            trimmed.to_owned()
        }
    }

    normalize(left) == normalize(right)
}

/// Builds the `EntryStat` for a child entry from what its listing already
/// reported, so walking a tree does not spend a round trip per entry on
/// metadata that has already been handed over.
///
/// Every backend's `read_dir` reports the same `kind` its `stat` would, and
/// the same size for regular files, which is what makes the sized case usable
/// as-is. Everything else — symlinks, and whatever else a backend cannot size
/// — still goes through `stat`, so those keep the type that backend reports
/// for them instead of being second-guessed here.
///
/// Directories report a size of zero: nothing downstream reads it, because
/// copy units come from the children and delete units count the node itself.
/// Skipping the lookup therefore cannot change what is copied or counted.
fn child_stat(
    backend: &dyn FileSystemBackend,
    entry: &DirectoryEntry,
) -> Result<EntryStat, FileSystemError> {
    match entry.size {
        Some(size) => Ok(EntryStat {
            kind: entry.kind,
            size,
            modified_at: entry.modified_at,
        }),
        None if entry.kind == EntryKind::Directory => Ok(EntryStat {
            kind: EntryKind::Directory,
            size: 0,
            modified_at: entry.modified_at,
        }),
        None => backend.stat(&entry.path),
    }
}

/// Copies one node. A directory is created, then its children are copied
/// concurrently — siblings are independent, and both the pool the engine runs
/// on and `try_for_each` bound how many of them reach the backend at once and
/// stop handing out new ones as soon as a child fails.
///
/// A file or symlink streams as content; a symlink materializes as a regular
/// file on the destination, which is the only portable mapping.
fn copy_node(
    source: &dyn FileSystemBackend,
    source_path: &str,
    stat: &EntryStat,
    destination_backend: &dyn FileSystemBackend,
    destination_path: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind != EntryKind::Directory {
        let mut reader = source.open_read(source_path)?;
        let writer = destination_backend.open_write(destination_path)?;
        return copy_stream(reader.as_mut(), writer, source_path, progress);
    }

    destination_backend.mkdir(destination_path)?;
    progress.advance(Path::new(destination_path));

    let view = source.read_dir(source_path)?;
    view.entries.par_iter().try_for_each(|entry| {
        let entry_stat = child_stat(source, entry)?;
        copy_node(
            source,
            &entry.path,
            &entry_stat,
            destination_backend,
            &join_path(destination_path, &entry.name),
            progress,
        )
    })
}

/// One chunk handed from the reading side to the writing side, carrying its
/// buffer back for reuse once it has been written.
struct StreamChunk {
    bytes: Vec<u8>,
    len: usize,
}

/// Streams `reader` into `writer` with the reads and the writes running
/// concurrently, so a destination whose writes are slow to acknowledge (SMB,
/// SFTP, cloud) no longer makes every read wait behind one.
///
/// The caller keeps reading on its own thread while a writer thread drains
/// chunks over a bounded queue sized [`STREAM_PIPELINE_DEPTH`]; buffers are
/// recycled along the way rather than reallocated per chunk. `progress` is
/// advanced on the reading side, which keeps the reporter single-threaded and
/// means the reported count can lead the bytes that have actually landed by at
/// most the queue depth.
fn copy_stream(
    reader: &mut dyn Read,
    mut writer: Box<dyn Write + Send>,
    source_path: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let (chunk_tx, chunk_rx) = sync_channel::<StreamChunk>(STREAM_PIPELINE_DEPTH);
    let (free_tx, free_rx) = channel::<Vec<u8>>();
    let write_error = Arc::new(Mutex::new(None));

    let writer_error = Arc::clone(&write_error);
    let writer_thread = thread::spawn(move || {
        while let Ok(chunk) = chunk_rx.recv() {
            if let Err(error) = writer.write_all(&chunk.bytes[..chunk.len]) {
                *writer_error.lock().expect("stream write error lock") =
                    Some(FileSystemError::from(error));
                return;
            }

            // A failed send means the reading side is gone; the buffer is
            // dropped with it.
            let _ = free_tx.send(chunk.bytes);
        }

        if let Err(error) = writer.flush() {
            *writer_error.lock().expect("stream write error lock") =
                Some(FileSystemError::from(error));
        }
    });

    // The first `STREAM_PIPELINE_DEPTH` chunks are read into fresh buffers,
    // after which every chunk reuses one the writer handed back.
    let mut allocate_buffers = STREAM_PIPELINE_DEPTH;
    let mut read_error = None;

    loop {
        let mut bytes = if allocate_buffers > 0 {
            allocate_buffers -= 1;
            vec![0_u8; STREAM_CHUNK_BYTES]
        } else {
            // Only ever blocks while the writer is alive and writing: it
            // returns a buffer per chunk, and drops its sender on failure,
            // which is what unblocks this.
            match free_rx.recv() {
                Ok(bytes) => bytes,
                Err(_) => break,
            }
        };

        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(read) => {
                // Dropping `chunk_tx` below is what ends the writer; a failure
                // here means the writer already stopped, so there is nothing
                // left to drain.
                if chunk_tx.send(StreamChunk { bytes, len: read }).is_err() {
                    break;
                }
                progress.advance_by(read as u64, Path::new(source_path));
            }
            Err(error) => {
                read_error = Some(FileSystemError::from(error));
                break;
            }
        }
    }

    // Dropping the sender ends the writer's receive loop. Joining it both
    // keeps a failed transfer from finishing in the background after it was
    // reported, and turns a panicking writer into a failure instead of a
    // truncated file that looks like a success.
    drop(chunk_tx);
    let writer_panicked = writer_thread.join().is_err();

    if let Some(error) = read_error {
        return Err(error);
    }

    if let Some(error) = write_error.lock().expect("stream write error lock").take() {
        return Err(error);
    }

    if writer_panicked {
        return Err(FileSystemError::Internal(format!(
            "Writing {source_path} stopped unexpectedly"
        )));
    }

    Ok(())
}

/// Deletes `path`, depth-first: every child goes before the directory that
/// holds it. Siblings are removed concurrently, which is what lets a tree on a
/// remote backend clear at request concurrency instead of one round trip at a
/// time.
fn delete_node(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind == EntryKind::Directory {
        let view = backend.read_dir(path)?;
        view.entries.par_iter().try_for_each(|entry| {
            let entry_stat = child_stat(backend, entry)?;
            delete_node(backend, &entry.path, &entry_stat, progress)
        })?;
    }

    backend.remove(path)?;
    progress.advance(Path::new(path));
    Ok(())
}

/// Work units for copying: file bytes plus one per directory created, summed
/// over the whole tree so progress totals match what execution advances.
///
/// The walk is itself concurrent, and it runs before the first byte moves, so
/// its latency is the delay before the progress bar has a total. On a remote
/// backend that is now one listing per directory rather than a listing plus a
/// `stat` per entry, because the entries come back already sized — see
/// [`child_stat`].
fn count_copy_units(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
) -> Result<u64, FileSystemError> {
    if stat.kind != EntryKind::Directory {
        return Ok(stat.size.max(1));
    }

    let children: Result<Vec<u64>, FileSystemError> = backend
        .read_dir(path)?
        .entries
        .par_iter()
        .map(|entry| {
            let entry_stat = child_stat(backend, entry)?;
            count_copy_units(backend, &entry.path, &entry_stat)
        })
        .collect();

    Ok(1 + children?.into_iter().sum::<u64>())
}

/// Work units for deleting: one per removed node, summed over the whole tree.
fn count_delete_units(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
) -> Result<u64, FileSystemError> {
    if stat.kind != EntryKind::Directory {
        return Ok(1);
    }

    let children: Result<Vec<u64>, FileSystemError> = backend
        .read_dir(path)?
        .entries
        .par_iter()
        .map(|entry| {
            let entry_stat = child_stat(backend, entry)?;
            count_delete_units(backend, &entry.path, &entry_stat)
        })
        .collect();

    Ok(1 + children?.into_iter().sum::<u64>())
}

fn ensure_unique_paths(sources: &[TransferSource]) -> Result<(), FileSystemError> {
    let mut unique_paths = HashSet::new();
    for source in sources {
        if !unique_paths.insert(source.path.clone()) {
            return Err(FileSystemError::InvalidInput(format!(
                "The same entry was selected more than once: {}",
                source.path
            )));
        }
    }

    Ok(())
}

pub(super) fn last_segment(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|segment| !segment.is_empty())
}

fn join_path(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches(['/', '\\']), name)
}

/// True when `descendant` lies inside `ancestor`, on the same backend.
fn path_contains(ancestor: &str, descendant: &str) -> bool {
    let ancestor = ancestor.trim_end_matches(['/', '\\']);
    let descendant = descendant.trim_end_matches(['/', '\\']);
    descendant.len() > ancestor.len()
        && descendant.starts_with(ancestor)
        && descendant[ancestor.len()..]
            .chars()
            .next()
            .is_some_and(|next| next == '/' || next == '\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::local::LocalBackend;
    use crate::file_system::test_support::TestProgress;
    use crate::file_system::types::{DirectoryView, NewEntryKind, SearchResponse};
    use crate::file_system::vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// Passes every call through to the local backend while recording how many
    /// of them were in flight at the same time, which is what tells real
    /// concurrency apart from a loop that merely looks parallel.
    ///
    /// Each call parks briefly so the window is wide enough to overlap: the
    /// local disk answers faster than the threads can be scheduled, and on a
    /// remote backend the round trip itself is the delay being modelled.
    struct ProbeBackend {
        inner: LocalBackend,
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        /// Calls to `stat`. A walk that reuses what its listing reported must
        /// not move this number, which is what makes the saving observable
        /// rather than merely asserted in a comment.
        stats: AtomicUsize,
        /// Paths whose `open_write` always fails, so a batch can be made to
        /// fail partway through while the rest of it still lands.
        poison: Option<String>,
    }

    const PROBE_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

    impl ProbeBackend {
        fn new(poison: Option<&str>) -> Self {
            Self {
                inner: LocalBackend,
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                stats: AtomicUsize::new(0),
                poison: poison.map(str::to_owned),
            }
        }

        fn enter(&self) {
            let now = self.in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.peak.fetch_max(now, AtomicOrdering::SeqCst);
            std::thread::sleep(PROBE_DELAY);
        }

        fn leave(&self) {
            self.in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
        }

        fn peak(&self) -> usize {
            self.peak.load(AtomicOrdering::SeqCst)
        }

        fn stat_calls(&self) -> usize {
            self.stats.load(AtomicOrdering::SeqCst)
        }
    }

    impl FileSystemBackend for ProbeBackend {
        fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
            self.inner.read_dir(path)
        }

        fn create_entry(
            &self,
            directory: &str,
            name: &str,
            kind: NewEntryKind,
        ) -> Result<String, FileSystemError> {
            self.inner.create_entry(directory, name, kind)
        }

        fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
            self.inner.rename_entry(path, new_name)
        }

        fn search(
            &self,
            root: &str,
            query: &str,
            is_current: &(dyn Fn() -> bool + Send + Sync),
        ) -> Result<SearchResponse, FileSystemError> {
            self.inner.search(root, query, is_current)
        }

        fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
            self.stats.fetch_add(1, AtomicOrdering::SeqCst);
            self.enter();
            let stat = self.inner.stat(path);
            self.leave();
            stat
        }

        fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
            self.inner.mkdir(path)
        }

        fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
            self.enter();
            let reader = self.inner.open_read(path);
            self.leave();
            reader
        }

        fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
            if self
                .poison
                .as_deref()
                .is_some_and(|poison| path.ends_with(poison))
            {
                return Err(FileSystemError::Io(format!("refusing to write {path}")));
            }

            self.enter();
            let writer = self.inner.open_write(path);
            self.leave();
            writer
        }

        fn remove(&self, path: &str) -> Result<(), FileSystemError> {
            self.inner.remove(path)
        }

        fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
            self.inner.rename_to(source, destination)
        }
    }

    /// Walking a tree must not spend a round trip per entry on metadata the
    /// listing already handed over. On a remote backend that lookup was a
    /// second request for something the previous one had answered, which made
    /// a deep tree cost roughly twice what it had to.
    #[test]
    fn walks_a_tree_without_a_round_trip_per_entry() {
        use std::sync::Arc;

        let root =
            std::env::temp_dir().join(format!("dae-no-per-entry-stat-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(source_dir.join("nested/deeper")).expect("create nested source");
        for index in 0..4 {
            fs::write(source_dir.join(format!("file-{index}.txt")), "payload")
                .expect("write source file");
            fs::write(
                source_dir.join(format!("nested/leaf-{index}.txt")),
                "payload",
            )
            .expect("write nested file");
        }
        fs::write(source_dir.join("nested/deeper/deep.txt"), "payload").expect("write deep file");
        fs::create_dir_all(&destination_dir).expect("create destination");

        // Distinct `Arc`s force the streaming engine instead of a fast path.
        let source_probe = Arc::new(ProbeBackend::new(None));
        let source: Arc<dyn FileSystemBackend> = source_probe.clone();
        let destination: Arc<dyn FileSystemBackend> = Arc::new(ProbeBackend::new(None));

        let progress = TestProgress::new();
        copy_entries(
            vec![TransferSource {
                path: source_dir.to_string_lossy().into_owned(),
                backend: source,
                on_conflict: ConflictAction::Fail,
            }],
            &destination_dir.to_string_lossy(),
            &destination,
            &progress,
            &mut Vec::new(),
        )
        .expect("copy the tree");

        // Planning stats the selected root once. The three directories and
        // nine files under it all come back sized from the listings that
        // found them, so none of them may cost a lookup.
        assert_eq!(
            source_probe.stat_calls(),
            1,
            "a tree walk should stat only the root it was handed"
        );
        // The source lands inside the destination directory under its own name.
        assert!(
            destination_dir
                .join("source/nested/deeper/deep.txt")
                .is_file()
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }

    /// Reusing what a listing reported is only sound while that listing
    /// reports what `stat` would. Pin the two sides together against the real
    /// backend, so neither can drift without a test noticing.
    #[test]
    fn listings_report_what_stat_would() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-listing-parity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("folder")).expect("create folder");
        fs::write(root.join("file.txt"), b"payload").expect("write file");
        fs::write(root.join("folder/nested.txt"), b"nested").expect("write nested file");

        // A link is the one kind a listing deliberately leaves unsized, so it
        // is also the one that must still be looked up.
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("file.txt"), root.join("link.txt"))
            .expect("create symlink");
        #[cfg(windows)]
        let _ = std::os::windows::fs::symlink_file(root.join("file.txt"), root.join("link.txt"));

        let backend = Arc::new(ProbeBackend::new(None));
        let view = backend
            .read_dir(&root.to_string_lossy())
            .expect("list directory");
        assert!(!view.entries.is_empty(), "the fixture should list entries");
        #[cfg(unix)]
        assert!(
            view.entries
                .iter()
                .any(|entry| entry.kind == EntryKind::Symlink),
            "the fixture should include a link to exercise the unsized branch"
        );

        for entry in &view.entries {
            let before = backend.stat_calls();
            let from_listing = child_stat(backend.as_ref(), entry).expect("child stat");
            let looked_up = backend.stat_calls() > before;
            let from_stat = backend.stat(&entry.path).expect("stat");

            assert_eq!(
                from_listing.kind, from_stat.kind,
                "kind disagrees for {}",
                entry.name
            );

            match entry.kind {
                // The kinds a listing can size must be taken from it, and
                // must agree with the lookup they replace.
                EntryKind::File => {
                    assert!(!looked_up, "{} should not need a lookup", entry.name);
                    assert_eq!(
                        from_listing.size, from_stat.size,
                        "size disagrees for {}",
                        entry.name
                    );
                }
                EntryKind::Directory => {
                    assert!(!looked_up, "{} should not need a lookup", entry.name);
                }
                // Everything else keeps whatever the backend says about it.
                _ => assert!(looked_up, "{} should still be looked up", entry.name),
            }
        }

        // Creating a real link needs a privilege Windows may not grant, so
        // drive the unsized branch from a hand-built entry as well: it must
        // be looked up, and the backend's answer must be the one that wins.
        let unsized_entry = DirectoryEntry {
            name: "link.txt".into(),
            path: root.join("file.txt").to_string_lossy().into_owned(),
            kind: EntryKind::Symlink,
            modified_at: None,
            size: None,
            hidden: false,
            read_only: false,
        };

        let before = backend.stat_calls();
        let resolved = child_stat(backend.as_ref(), &unsized_entry).expect("resolve the entry");
        assert_eq!(
            backend.stat_calls(),
            before + 1,
            "an unsized entry must still be looked up"
        );
        assert_eq!(
            resolved.kind,
            EntryKind::File,
            "the backend decides what an unsized entry is"
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn copies_a_directory_tree_with_several_requests_in_flight() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-parallel-tree-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(source_dir.join("nested/deeper")).expect("create nested source");
        for index in 0..8 {
            fs::write(source_dir.join(format!("file-{index}.txt")), "payload")
                .expect("write source file");
            fs::write(
                source_dir.join(format!("nested/deeper/leaf-{index}.txt")),
                "payload",
            )
            .expect("write nested source file");
        }
        fs::create_dir_all(&destination_dir).expect("create destination");

        // Distinct `Arc`s force the streaming engine instead of a fast path.
        let source: Arc<dyn FileSystemBackend> = Arc::new(ProbeBackend::new(None));
        let probe = Arc::new(ProbeBackend::new(None));
        let destination: Arc<dyn FileSystemBackend> = probe.clone();

        let progress = TestProgress::new();
        let mut journal = Vec::new();
        copy_entries(
            vec![TransferSource {
                path: source_dir.to_string_lossy().into_owned(),
                backend: source,
                on_conflict: ConflictAction::Fail,
            }],
            &destination_dir.to_string_lossy(),
            &destination,
            &progress,
            &mut journal,
        )
        .expect("copy the tree");

        let copied = destination_dir.join("source");
        assert_eq!(
            fs::read_to_string(copied.join("file-7.txt")).expect("read copied file"),
            "payload"
        );
        assert_eq!(
            fs::read_to_string(copied.join("nested/deeper/leaf-3.txt")).expect("read nested copy"),
            "payload"
        );
        assert_eq!(
            progress.completed.load(AtomicOrdering::Relaxed),
            progress.total.load(AtomicOrdering::Relaxed)
        );
        assert_eq!(journal.len(), 1);

        // The pool runs four requests wide; three is enough to prove the engine
        // is not walking siblings one at a time.
        assert!(
            probe.peak() >= 3,
            "expected concurrent requests, saw a peak of {}",
            probe.peak()
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn journals_what_landed_when_a_batch_fails_partway() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-partial-journal-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).expect("create source directory");
        fs::create_dir_all(&destination_dir).expect("create destination directory");
        for name in ["first.txt", "poison.bin", "last.txt"] {
            fs::write(source_dir.join(name), "payload").expect("write source file");
        }

        let source: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
        let destination: Arc<dyn FileSystemBackend> =
            Arc::new(ProbeBackend::new(Some("poison.bin")));

        let progress = TestProgress::new();
        let mut journal = Vec::new();
        let error = copy_entries(
            vec![
                TransferSource {
                    path: source_dir.join("first.txt").to_string_lossy().into_owned(),
                    backend: source.clone(),
                    on_conflict: ConflictAction::Fail,
                },
                TransferSource {
                    path: source_dir.join("poison.bin").to_string_lossy().into_owned(),
                    backend: source.clone(),
                    on_conflict: ConflictAction::Fail,
                },
                TransferSource {
                    path: source_dir.join("last.txt").to_string_lossy().into_owned(),
                    backend: source,
                    on_conflict: ConflictAction::Fail,
                },
            ],
            &destination_dir.to_string_lossy(),
            &destination,
            &progress,
            &mut journal,
        )
        .expect_err("the poisoned destination must fail the batch");
        assert!(matches!(error, FileSystemError::Io(_)), "got {error:?}");

        // The batch stops as soon as one entry fails, so which of the other two
        // made it is up to the scheduler — but whatever is journalled must have
        // actually landed, must not name the failed entry, and must stay in
        // plan order so the partial copy can be undone.
        let landed: Vec<&str> = journal
            .iter()
            .map(|pair| last_segment(&pair.source).unwrap_or_default())
            .collect();
        assert!(
            !landed.is_empty(),
            "entries that landed before the failure must still be journalled"
        );
        for name in &landed {
            assert_ne!(
                *name, "poison.bin",
                "the failed entry must not be journalled"
            );
            assert!(
                destination_dir.join(name).is_file(),
                "{name} was journalled but never landed"
            );
        }

        let mut plan_order = landed.clone();
        plan_order.sort_by_key(|name| usize::from(*name == "last.txt"));
        assert_eq!(landed, plan_order, "the journal must stay in plan order");
        assert!(!destination_dir.join("poison.bin").exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn transfers_trees_between_distinct_backend_instances() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-transfer-test-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(source_dir.join("nested")).expect("create source tree");
        fs::write(source_dir.join("root.txt"), "root content").expect("write root file");
        fs::write(source_dir.join("nested/leaf.bin"), vec![7_u8; 600 * 1024])
            .expect("write multi-chunk file");
        fs::create_dir_all(&destination_dir).expect("create destination");

        // Two distinct Arcs force the streaming path instead of any fast path.
        let source_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
        let destination_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
        let source_path = source_dir.to_string_lossy().into_owned();
        let destination_path = destination_dir.to_string_lossy().into_owned();

        let copy_progress = TestProgress::new();
        copy_entries(
            vec![TransferSource {
                path: source_path.clone(),
                backend: source_backend.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            &destination_path,
            &destination_backend,
            &copy_progress,
            &mut Vec::new(),
        )
        .expect("copy tree across backends");

        let copied_root = destination_dir.join("source");
        assert_eq!(
            fs::read_to_string(copied_root.join("root.txt")).expect("read copied root file"),
            "root content"
        );
        let leaf = fs::read(copied_root.join("nested/leaf.bin")).expect("read copied leaf");
        assert_eq!(leaf.len(), 600 * 1024);
        assert!(leaf.iter().all(|byte| *byte == 7));
        assert_eq!(
            copy_progress.completed.load(AtomicOrdering::Relaxed),
            copy_progress.total.load(AtomicOrdering::Relaxed)
        );

        let duplicate_progress = TestProgress::new();
        let duplicate_error = copy_entries(
            vec![TransferSource {
                path: source_path.clone(),
                backend: source_backend.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            &destination_path,
            &destination_backend,
            &duplicate_progress,
            &mut Vec::new(),
        )
        .expect_err("overwriting must be blocked");
        assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

        let move_destination_dir = root.join("destination-moved");
        fs::create_dir_all(&move_destination_dir).expect("create move destination");
        let move_progress = TestProgress::new();
        move_entries(
            vec![TransferSource {
                path: source_path.clone(),
                backend: source_backend.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            &move_destination_dir.to_string_lossy(),
            &destination_backend,
            &move_progress,
            &mut Vec::new(),
        )
        .expect("move tree across backends");
        assert!(!source_dir.exists());
        assert!(
            move_destination_dir
                .join("source/nested/leaf.bin")
                .is_file()
        );
        assert_eq!(
            move_progress.completed.load(AtomicOrdering::Relaxed),
            move_progress.total.load(AtomicOrdering::Relaxed)
        );

        let delete_progress = TestProgress::new();
        delete_entries(
            vec![TransferSource {
                path: move_destination_dir
                    .join("source")
                    .to_string_lossy()
                    .into_owned(),
                backend: destination_backend.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            &delete_progress,
        )
        .expect("delete tree through engine");
        assert!(!move_destination_dir.join("source").exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn reports_conflicts_for_the_dialog_and_skips_self_transfers() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-conflict-report-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).expect("create source directory");
        fs::create_dir_all(&destination_dir).expect("create destination directory");
        fs::write(source_dir.join("clashing.txt"), "source bytes").expect("write clashing source");
        fs::write(source_dir.join("fresh.txt"), "fresh bytes").expect("write fresh source");
        // A file already sitting in the destination directory: moving the whole
        // batch into its own parent must not report it as a conflict.
        fs::write(destination_dir.join("clashing.txt"), "target bytes")
            .expect("write clashing target");

        let backend: Arc<dyn FileSystemBackend> = vfs::resolve(&source_dir.to_string_lossy())
            .expect("resolve local backend");

        let source = |name: &str| TransferSource {
            path: source_dir.join(name).to_string_lossy().into_owned(),
            backend: backend.clone(),
            on_conflict: ConflictAction::Fail,
        };

        let conflicts = find_conflicts(
            vec![source("clashing.txt"), source("fresh.txt")],
            &destination_dir.to_string_lossy(),
            &backend,
        )
        .expect("find conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].name, "clashing.txt");
        assert_eq!(conflicts[0].source_size, Some("source bytes".len() as u64));
        assert_eq!(conflicts[0].target_size, Some("target bytes".len() as u64));
        assert!(conflicts[0].source_modified_at.is_some());
        assert!(conflicts[0].target_modified_at.is_some());

        // A file moved into its own directory lands on itself: no conflict.
        let self_conflicts = find_conflicts(
            vec![TransferSource {
                path: destination_dir
                    .join("clashing.txt")
                    .to_string_lossy()
                    .into_owned(),
                backend: backend.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            &destination_dir.to_string_lossy(),
            &backend,
        )
        .expect("find self conflicts");
        assert!(self_conflicts.is_empty());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn resolves_streaming_transfer_conflicts_across_backends() {
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("dae-conflict-stream-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).expect("create source directory");
        fs::create_dir_all(&destination_dir).expect("create destination directory");
        fs::write(source_dir.join("data.bin"), "streamed").expect("write source file");
        fs::write(destination_dir.join("data.bin"), "existing").expect("write target file");

        // Two distinct Arcs force the streaming engine.
        let source_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
        let destination_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);

        let keep_progress = TestProgress::new();
        copy_entries(
            vec![TransferSource {
                path: source_dir.join("data.bin").to_string_lossy().into_owned(),
                backend: source_backend.clone(),
                on_conflict: ConflictAction::KeepBoth,
            }],
            &destination_dir.to_string_lossy(),
            &destination_backend,
            &keep_progress,
            &mut Vec::new(),
        )
        .expect("stream copy keeping both");
        assert_eq!(
            fs::read_to_string(destination_dir.join("data 副本.bin")).expect("kept streamed copy"),
            "streamed"
        );

        let replace_progress = TestProgress::new();
        copy_entries(
            vec![TransferSource {
                path: source_dir.join("data.bin").to_string_lossy().into_owned(),
                backend: source_backend.clone(),
                on_conflict: ConflictAction::Replace,
            }],
            &destination_dir.to_string_lossy(),
            &destination_backend,
            &replace_progress,
            &mut Vec::new(),
        )
        .expect("stream copy with replace");
        assert_eq!(
            fs::read_to_string(destination_dir.join("data.bin")).expect("replaced streamed copy"),
            "streamed"
        );
        assert_eq!(
            replace_progress.completed.load(AtomicOrdering::Relaxed),
            replace_progress.total.load(AtomicOrdering::Relaxed)
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn pipelines_a_stream_many_chunks_long() {
        let root =
            std::env::temp_dir().join(format!("dae-stream-pipeline-test-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create test directory");
        let destination = root.join("pipelined.bin");

        // Comfortably more chunks than the pipeline is deep, so buffers are
        // being recycled while the reading side is still reading.
        let payload = (0..STREAM_CHUNK_BYTES * 3 + 4097)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<u8>>();
        let progress = TestProgress::new();
        progress.start(payload.len() as u64);

        let mut reader = std::io::Cursor::new(payload.clone());
        copy_stream(
            &mut reader,
            Box::new(fs::File::create(&destination).expect("create destination file")),
            &destination.to_string_lossy(),
            &progress,
        )
        .expect("stream the payload");

        assert_eq!(fs::read(&destination).expect("read destination"), payload);
        progress.finish();

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn reports_a_destination_failure_instead_of_giving_up_the_thread() {
        /// Accepts nothing, the way a full or read-only destination behaves.
        struct FailingDestination;

        impl Write for FailingDestination {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("destination is full"))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let progress = TestProgress::new();
        let mut reader = std::io::Cursor::new(vec![0_u8; STREAM_CHUNK_BYTES * 4]);
        let error = copy_stream(
            &mut reader,
            Box::new(FailingDestination),
            "src/failing.bin",
            &progress,
        )
        .expect_err("a failing destination must fail the transfer");

        assert!(matches!(error, FileSystemError::Io(_)), "got {error:?}");
    }
}
