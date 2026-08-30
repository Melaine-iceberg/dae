//! Generic transfer engine that moves entries between any two backends
//! through the trait's primitive operations (stat, mkdir, streams, remove).
//! Local-to-local transfers keep the backend-native rayon path in
//! `local/operations.rs`; every other combination lands here.

use super::error::FileSystemError;
use super::progress::FileOperationProgressReporterTrait;
use super::types::{ConflictAction, EntryKind, EntryStat, TransferConflict, TransferPair};
use super::vfs::{FileSystemBackend, SharedBackend};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};

const STREAM_CHUNK_BYTES: usize = 256 * 1024;

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
    let plan = build_plan(sources, destination, destination_backend)?;
    progress.start(
        plan.iter()
            .map(|entry| count_copy_units(entry.source.backend.as_ref(), &entry.source.path, &entry.stat)
                .and_then(|units| add_replacement_units(destination_backend, entry, units)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    for entry in plan {
        progress.begin_entry(Path::new(&entry.source.path));
        remove_replacement(destination_backend, &entry, progress)?;
        copy_node(
            entry.source.backend.as_ref(),
            &entry.source.path,
            &entry.stat,
            destination_backend.as_ref(),
            &entry.destination,
            progress,
        )?;
        journal.push(TransferPair {
            source: entry.source.path.clone(),
            destination: entry.destination.clone(),
        });
    }

    progress.finish();
    Ok(())
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
    let plan = build_plan(sources, destination, destination_backend)?;
    progress.start(
        plan.iter()
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

    for entry in plan {
        progress.begin_entry(Path::new(&entry.source.path));
        remove_replacement(destination_backend, &entry, progress)?;

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

        journal.push(TransferPair {
            source: entry.source.path.clone(),
            destination: entry.destination.clone(),
        });
    }

    progress.finish();
    Ok(())
}

/// Work units contributed by a `Replace` entry's existing destination tree.
fn add_replacement_units(
    destination_backend: &SharedBackend,
    entry: &PlanEntry,
    units: u64,
) -> Result<u64, FileSystemError> {
    match &entry.replacement {
        Some(stat) => Ok(units
            + count_delete_units(destination_backend.as_ref(), &entry.destination, stat)?),
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
        Some(stat) => {
            delete_node(destination_backend.as_ref(), &entry.destination, stat, progress)
        }
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

    let mut conflicts = Vec::new();
    for source in sources {
        let name = last_segment(&source.path)
            .ok_or_else(|| {
                FileSystemError::InvalidInput(format!(
                    "The root of a volume cannot be copied or moved: {}",
                    source.path
                ))
            })?
            .to_owned();
        let target = join_path(destination, &name);

        if Arc::ptr_eq(&source.backend, destination_backend)
            && same_backend_path(&source.path, &target)
        {
            continue;
        }

        let target_stat = match destination_backend.stat(&target) {
            Err(FileSystemError::NotFound(_)) => continue,
            Err(error) => return Err(error),
            Ok(stat) => stat,
        };
        let source_stat = source.backend.stat(&source.path)?;

        conflicts.push(TransferConflict {
            source_path: source.path.clone(),
            target_path: target,
            name,
            source_kind: source_stat.kind,
            source_size: stat_size(&source_stat),
            source_modified_at: source_stat.modified_at,
            target_kind: target_stat.kind,
            target_size: stat_size(&target_stat),
            target_modified_at: target_stat.modified_at,
        });
    }

    Ok(conflicts)
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

    let mut stats = Vec::with_capacity(targets.len());
    for target in &targets {
        stats.push(target.backend.stat(&target.path)?);
    }

    progress.start(
        targets
            .iter()
            .zip(&stats)
            .map(|(target, stat)| count_delete_units(target.backend.as_ref(), &target.path, stat))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    for (target, stat) in targets.into_iter().zip(stats) {
        progress.begin_entry(Path::new(&target.path));
        delete_node(target.backend.as_ref(), &target.path, &stat, progress)?;
    }

    progress.finish();
    Ok(())
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

    let mut stats = Vec::with_capacity(sources.len());
    for source in &sources {
        stats.push(source.backend.stat(&source.path)?);
    }

    progress.start(
        sources
            .iter()
            .zip(&stats)
            .map(|(source, stat)| count_copy_units(source.backend.as_ref(), &source.path, stat))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    let mut created = Vec::with_capacity(sources.len());
    for (source, stat) in sources.into_iter().zip(stats) {
        progress.begin_entry(Path::new(&source.path));
        let destination = unique_duplicate_path(&source)?;

        copy_node(
            source.backend.as_ref(),
            &source.path,
            &stat,
            source.backend.as_ref(),
            &destination,
            progress,
        )?;

        created.push(destination);
    }

    progress.finish();
    Ok(created)
}

/// Computes the first free sibling path for a duplicate of `source`.
fn unique_duplicate_path(source: &TransferSource) -> Result<String, FileSystemError> {
    let name = last_segment(&source.path).ok_or_else(|| {
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

    unique_sibling_path(source.backend.as_ref(), &parent, name, &HashSet::new())
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

    let mut planned_names = HashSet::new();
    let mut plan = Vec::with_capacity(sources.len());

    for source in sources {
        let stat = source.backend.stat(&source.path)?;
        let name = last_segment(&source.path).ok_or_else(|| {
            FileSystemError::InvalidInput(format!(
                "The root of a volume cannot be copied or moved: {}",
                source.path
            ))
        })?;

        if !planned_names.insert(name.to_owned()) {
            return Err(FileSystemError::AlreadyExists(format!(
                "Multiple selected entries have the same name: {name}"
            )));
        }

        let target = join_path(destination, name);

        // A source that would land on itself is a no-op, not a conflict.
        if Arc::ptr_eq(&source.backend, destination_backend)
            && same_backend_path(&source.path, &target)
        {
            continue;
        }

        let (destination_path, replacement) =
            match destination_backend.stat(&target) {
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
                            name,
                            &planned_names,
                        )?;
                        planned_names.insert(last_segment(&kept).unwrap_or_default().to_owned());
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

fn copy_node(
    source: &dyn FileSystemBackend,
    source_path: &str,
    stat: &EntryStat,
    destination_backend: &dyn FileSystemBackend,
    destination_path: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind == EntryKind::Directory {
        destination_backend.mkdir(destination_path)?;
        progress.advance(Path::new(destination_path));

        let view = source.read_dir(source_path)?;
        for entry in &view.entries {
            let entry_stat = source.stat(&entry.path)?;
            copy_node(
                source,
                &entry.path,
                &entry_stat,
                destination_backend,
                &join_path(destination_path, &entry.name),
                progress,
            )?;
        }

        return Ok(());
    }

    // Files and symlinks both stream as content; a symlink materializes as a
    // regular file on the destination, which is the only portable mapping.
    let mut reader = source.open_read(source_path)?;
    let mut writer = destination_backend.open_write(destination_path)?;
    let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        writer.write_all(&buffer[..read])?;
        progress.advance_by(read as u64, Path::new(source_path));
    }

    writer.flush()?;
    Ok(())
}

fn delete_node(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind == EntryKind::Directory {
        let view = backend.read_dir(path)?;
        for entry in &view.entries {
            let entry_stat = backend.stat(&entry.path)?;
            delete_node(backend, &entry.path, &entry_stat, progress)?;
        }
    }

    backend.remove(path)?;
    progress.advance(Path::new(path));
    Ok(())
}

/// Work units for copying: file bytes plus one per directory created, summed
/// over the whole tree so progress totals match what execution advances.
fn count_copy_units(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
) -> Result<u64, FileSystemError> {
    if stat.kind != EntryKind::Directory {
        return Ok(stat.size.max(1));
    }

    let mut total = 1;
    for entry in backend.read_dir(path)?.entries {
        let entry_stat = backend.stat(&entry.path)?;
        total += count_copy_units(backend, &entry.path, &entry_stat)?;
    }

    Ok(total)
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

    let mut total = 1;
    for entry in backend.read_dir(path)?.entries {
        let entry_stat = backend.stat(&entry.path)?;
        total += count_delete_units(backend, &entry.path, &entry_stat)?;
    }

    Ok(total)
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
    use crate::file_system::vfs;
    use std::fs;
    use std::sync::atomic::Ordering as AtomicOrdering;

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
}
