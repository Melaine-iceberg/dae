use crate::file_system::error::FileSystemError;
use crate::file_system::progress::FileOperationProgressReporterTrait;
use crate::file_system::transfer::{duplicate_name, path_contains};
use crate::file_system::types::{
    ConflictAction, NewEntryKind, TransferPair, canonical_path, path_to_string,
};
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Pool the bulk local tree operations run on.
///
/// `par_iter` would otherwise use rayon's *global* pool — the pool directory
/// listings read per-entry metadata on, which sits on the critical path to
/// painting the explorer. A copy of a large tree running there would starve
/// the very listing that is trying to render, so bulk transfers get their own
/// bounded pool, sized to leave one core for listings, search, and the UI.
///
/// Blocking I/O keeps the pool's threads busy by design; the pool is what caps
/// how much of it can happen at once.
fn transfer_pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();

    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .saturating_sub(1)
            .max(2);

        ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("dae-transfer-{index}"))
            .build()
            .expect("build the transfer thread pool")
    })
}

pub fn rename_entry_sync(path: PathBuf, new_name: String) -> Result<(), FileSystemError> {
    validate_entry_name(&new_name)?;
    fs::symlink_metadata(&path)?;

    let parent = path.parent().ok_or_else(|| {
        FileSystemError::InvalidInput("The root of a volume cannot be renamed".into())
    })?;
    let destination = parent.join(new_name);

    if destination == path {
        return Ok(());
    }

    ensure_path_is_available(&destination)?;
    fs::rename(path, destination)?;
    Ok(())
}

pub fn create_entry_sync(
    directory: PathBuf,
    name: String,
    kind: NewEntryKind,
) -> Result<String, FileSystemError> {
    validate_entry_name(&name)?;

    if !fs::metadata(&directory)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&directory)));
    }

    let target = directory.join(&name);
    ensure_path_is_available(&target)?;

    match kind {
        NewEntryKind::File => {
            fs::File::create(&target)?;
        }
        NewEntryKind::Directory => fs::create_dir(&target)?,
    }

    Ok(path_to_string(&target))
}

pub fn copy_entries_with_progress(
    sources: Vec<(PathBuf, ConflictAction)>,
    destination: PathBuf,
    progress: &dyn FileOperationProgressReporterTrait,
    journal: &mut Vec<TransferPair>,
) -> Result<(), FileSystemError> {
    transfer_pool().install(|| {
        let plan = build_transfer_plan(sources, destination)?;
        progress.start(
            plan.iter()
                .map(|entry| entry.source_units + entry.replacement_units)
                .sum(),
        );

        for entry in plan {
            if entry.replacement_units > 0 {
                delete_entry(&entry.destination, progress)?;
            }
            copy_entry(&entry.source, &entry.destination, progress)?;
            journal.push(TransferPair {
                source: path_to_string(&entry.source),
                destination: path_to_string(&entry.destination),
            });
        }

        progress.finish();
        Ok(())
    })
}

pub fn move_entries_with_progress(
    sources: Vec<(PathBuf, ConflictAction)>,
    destination: PathBuf,
    progress: &dyn FileOperationProgressReporterTrait,
    journal: &mut Vec<TransferPair>,
) -> Result<(), FileSystemError> {
    transfer_pool().install(|| {
        let plan = build_transfer_plan(sources, destination)?;
        progress.start(
            plan.iter()
                .map(|entry| entry.source_units + entry.replacement_units)
                .sum(),
        );

        for entry in plan {
            progress.begin_entry(&entry.source);
            if entry.replacement_units > 0 {
                delete_entry(&entry.destination, progress)?;
            }
            match fs::rename(&entry.source, &entry.destination) {
                Ok(()) => progress.advance_by(entry.source_units, &entry.source),
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                    copy_entry(&entry.source, &entry.destination, progress)?;
                    remove_entry(&entry.source)?;
                }
                Err(error) => return Err(error.into()),
            }

            journal.push(TransferPair {
                source: path_to_string(&entry.source),
                destination: path_to_string(&entry.destination),
            });
        }

        progress.finish();
        Ok(())
    })
}

pub fn delete_entries_with_progress(
    paths: Vec<PathBuf>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    transfer_pool().install(|| {
        ensure_unique_paths(paths.iter().map(|path| path.as_path()))?;

        for path in &paths {
            fs::symlink_metadata(path)?;
        }

        let total = paths
            .iter()
            .try_fold(0_u64, |count, path| -> Result<u64, FileSystemError> {
                Ok(count + count_entry_units(path)?)
            })?;
        progress.start(total);

        for path in paths {
            delete_entry(&path, progress)?;
        }

        progress.finish();
        Ok(())
    })
}

#[derive(Debug)]
struct TransferPlanEntry {
    source: PathBuf,
    destination: PathBuf,
    /// Units the transfer of `source` itself advances (copy or rename).
    source_units: u64,
    /// Units for deleting an existing destination entry (`Replace`); zero
    /// when the destination path is free.
    replacement_units: u64,
}

fn build_transfer_plan(
    sources: Vec<(PathBuf, ConflictAction)>,
    requested_destination: PathBuf,
) -> Result<Vec<TransferPlanEntry>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before pasting".into(),
        ));
    }

    ensure_unique_paths(sources.iter().map(|(path, _)| path.as_path()))?;

    // `canonical_path` rather than `canonicalize`: this is the base every entry
    // of a pasted tree is written to (`copy_directory` joins each child onto it),
    // and `PathBuf::push` rebuilds the whole base when it carries the verbatim
    // prefix (see `types::canonical_path`). Measured on 35,817 children:
    // 13.94 -> 4.75 ms for the joins alone, and one join per copied entry.
    let destination = canonical_path(&requested_destination)?;
    if !fs::metadata(&destination)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&destination)));
    }

    let mut planned_destinations = HashSet::new();
    let mut plan = Vec::with_capacity(sources.len());

    for (source, on_conflict) in sources {
        let metadata = fs::symlink_metadata(&source)?;
        let name = source.file_name().ok_or_else(|| {
            FileSystemError::InvalidInput(format!(
                "The root of a volume cannot be copied or moved: {}",
                path_to_string(&source)
            ))
        })?;
        let target = destination.join(name);

        if !planned_destinations.insert(target.clone()) {
            return Err(FileSystemError::AlreadyExists(format!(
                "Multiple selected entries have the same name: {}",
                path_to_string(&target)
            )));
        }

        let (target, replacement_units) = match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (target, 0),
            Err(error) => return Err(error.into()),
            Ok(_) if paths_refer_to_same_entry(&source, &target) => {
                // Transferring an entry onto itself is a no-op, not a conflict.
                continue;
            }
            Ok(_) => match on_conflict {
                ConflictAction::Fail => {
                    return Err(FileSystemError::AlreadyExists(path_to_string(&target)));
                }
                ConflictAction::Skip => continue,
                ConflictAction::Replace => (target.clone(), count_entry_units(&target)?),
                ConflictAction::KeepBoth => {
                    let kept = unique_sibling_path(&destination, name, &planned_destinations)?;
                    planned_destinations.insert(kept.clone());
                    (kept, 0)
                }
            },
        };

        if metadata.is_dir() {
            // `is_within` rather than `Path::starts_with`: `canonical_path` drops
            // the verbatim prefix per path, so a short source and a destination
            // past `MAX_PATH` come back in different spellings, and comparing
            // those literally would answer "outside" for a folder that is plainly
            // inside its own subtree.
            let canonical_source = canonical_path(&source)?;
            if is_within(&canonical_source, &target) {
                return Err(FileSystemError::InvalidInput(format!(
                    "Cannot paste a folder into itself: {}",
                    path_to_string(&source)
                )));
            }
        }

        let source_units = count_entry_units(&source)?;
        plan.push(TransferPlanEntry {
            source,
            destination: target,
            source_units,
            replacement_units,
        });
    }

    Ok(plan)
}

/// True when `candidate` lies strictly beneath `directory`.
///
/// `canonical_path` decides *per path* whether to drop the verbatim prefix — it
/// keeps it for anything long enough to need it, and for names a plain path
/// cannot spell — so the same directory can come back as `\\?\K:\a` while a
/// descendant comes back as `K:\a\b`. `Path::starts_with` compares prefixes
/// literally and calls that "outside", which is why this guard does not use it.
/// Both sides go through `path_to_string`, so either spelling compares equal.
fn is_within(directory: &Path, candidate: &Path) -> bool {
    path_contains(&path_to_string(directory), &path_to_string(candidate))
}

/// True when both paths resolve to the same on-disk entry.
fn paths_refer_to_same_entry(source: &Path, target: &Path) -> bool {
    match (canonical_path(source), canonical_path(target)) {
        (Ok(source), Ok(target)) => source == target,
        _ => false,
    }
}

/// The first sibling of `name` in `directory` that neither exists on disk nor
/// is already claimed by another planned entry ("副本" naming).
fn unique_sibling_path(
    directory: &Path,
    name: &OsStr,
    reserved: &HashSet<PathBuf>,
) -> Result<PathBuf, FileSystemError> {
    let name = name.to_string_lossy();
    let mut attempt = 0_u32;

    loop {
        let candidate = directory.join(duplicate_name(&name, attempt));
        if !reserved.contains(&candidate) && !candidate.try_exists()? {
            return Ok(candidate);
        }
        attempt += 1;
    }
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(FileSystemError::InvalidInput(
            "The name must not be empty, . or ..".into(),
        ));
    }

    if name.contains(['/', '\\', '\0']) || Path::new(name).components().count() != 1 {
        return Err(FileSystemError::InvalidInput(
            "The name must not contain a path separator".into(),
        ));
    }

    Ok(())
}

fn ensure_unique_paths<'a, I>(paths: I) -> Result<(), FileSystemError>
where
    I: IntoIterator<Item = &'a Path>,
{
    let mut unique_paths = HashSet::new();
    for path in paths {
        if !unique_paths.insert(path) {
            return Err(FileSystemError::InvalidInput(format!(
                "The same entry was selected more than once: {}",
                path_to_string(path)
            )));
        }
    }

    Ok(())
}

fn ensure_path_is_available(path: &Path) -> Result<(), FileSystemError> {
    if path.try_exists()? {
        return Err(FileSystemError::AlreadyExists(path_to_string(path)));
    }

    Ok(())
}

fn count_entry_units(path: &Path) -> Result<u64, FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Ok(1);
    }

    let children = read_child_paths(path)?;
    let child_units: Vec<u64> = children
        .par_iter()
        .map(|(child, _)| count_entry_units(child))
        .collect::<Result<_, _>>()?;

    Ok(1 + child_units.iter().sum::<u64>())
}

fn read_child_paths(directory: &Path) -> Result<Vec<(PathBuf, OsString)>, FileSystemError> {
    fs::read_dir(directory)?
        .map(|child| {
            let child = child?;
            Ok((child.path(), child.file_name()))
        })
        .collect()
}

fn copy_entry(
    source: &Path,
    destination: &Path,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(source)?;
    progress.begin_entry(source);

    if metadata.is_file() {
        fs::copy(source, destination)?;
        progress.advance(source);
        return Ok(());
    }

    if metadata.is_dir() {
        return copy_directory(source, destination, progress);
    }

    if metadata.file_type().is_symlink() {
        copy_symlink(source, destination)?;
        progress.advance(source);
        return Ok(());
    }

    Err(FileSystemError::InvalidInput(format!(
        "Unsupported entry type: {}",
        path_to_string(source)
    )))
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    fs::create_dir(destination)?;
    progress.advance(source);

    let children = read_child_paths(source)?;
    children
        .par_iter()
        .map(|(child, name)| copy_entry(child, &destination.join(name), progress))
        .collect::<Result<(), _>>()
}

fn copy_symlink(source: &Path, destination: &Path) -> Result<(), FileSystemError> {
    let target = fs::read_link(source)?;
    let points_to_directory = fs::metadata(source)?.is_dir();

    create_symlink(&target, destination, points_to_directory)
}

#[cfg(unix)]
fn create_symlink(
    target: &Path,
    destination: &Path,
    _points_to_directory: bool,
) -> Result<(), FileSystemError> {
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(windows)]
fn create_symlink(
    target: &Path,
    destination: &Path,
    points_to_directory: bool,
) -> Result<(), FileSystemError> {
    if points_to_directory {
        std::os::windows::fs::symlink_dir(target, destination)?;
    } else {
        std::os::windows::fs::symlink_file(target, destination)?;
    }

    Ok(())
}

fn remove_entry(path: &Path) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
        return Ok(());
    }

    if metadata.file_type().is_symlink() {
        return fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(FileSystemError::from);
    }

    fs::remove_file(path)?;
    Ok(())
}

fn delete_entry(
    path: &Path,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;
    progress.begin_entry(path);

    if metadata.is_dir() {
        let children = read_child_paths(path)?;
        children
            .par_iter()
            .map(|(child, _)| delete_entry(child, progress))
            .collect::<Result<(), _>>()?;
        fs::remove_dir(path)?;
    } else if metadata.file_type().is_symlink() {
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))?;
    } else {
        fs::remove_file(path)?;
    }

    progress.advance(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::test_support::TestProgress;
    use std::sync::atomic::Ordering as AtomicOrdering;

    #[test]
    fn performs_file_operations_and_reports_entry_progress() {
        let directory =
            std::env::temp_dir().join(format!("dae-file-operation-test-{}", std::process::id()));
        let source = directory.join("source");
        let destination = directory.join("destination");
        let nested_file = source.join("nested.txt");

        fs::create_dir_all(&source).expect("create source directory");
        fs::create_dir_all(&destination).expect("create destination directory");
        fs::write(&nested_file, "copied content").expect("create source file");

        let copy_progress = TestProgress::new();
        copy_entries_with_progress(
            vec![(source.clone(), ConflictAction::Fail)],
            destination.clone(),
            &copy_progress,
            &mut Vec::new(),
        )
        .expect("copy directory");
        assert_eq!(copy_progress.completed.load(AtomicOrdering::Relaxed), 2);
        assert_eq!(copy_progress.total.load(AtomicOrdering::Relaxed), 2);

        let copied_directory = destination.join("source");
        assert_eq!(
            fs::read_to_string(copied_directory.join("nested.txt")).expect("read copied file"),
            "copied content"
        );
        assert!(source.exists());

        let duplicate_progress = TestProgress::new();
        let duplicate_error = copy_entries_with_progress(
            vec![(source.clone(), ConflictAction::Fail)],
            destination.clone(),
            &duplicate_progress,
            &mut Vec::new(),
        )
        .expect_err("copying over an existing entry should fail");
        assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

        let nested_progress = TestProgress::new();
        let nested_error = copy_entries_with_progress(
            vec![(source.clone(), ConflictAction::Fail)],
            source.clone(),
            &nested_progress,
            &mut Vec::new(),
        )
        .expect_err("copying a folder into itself should fail");
        assert!(matches!(nested_error, FileSystemError::InvalidInput(_)));

        rename_entry_sync(nested_file.clone(), "renamed.txt".into()).expect("rename file");
        let renamed_file = source.join("renamed.txt");
        assert!(renamed_file.exists());

        let move_progress = TestProgress::new();
        move_entries_with_progress(
            vec![(renamed_file.clone(), ConflictAction::Fail)],
            destination.clone(),
            &move_progress,
            &mut Vec::new(),
        )
        .expect("move file");
        let moved_file = destination.join("renamed.txt");
        assert!(moved_file.exists());
        assert!(!renamed_file.exists());
        assert_eq!(move_progress.completed.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(move_progress.total.load(AtomicOrdering::Relaxed), 1);

        let delete_progress = TestProgress::new();
        delete_entries_with_progress(vec![moved_file.clone()], &delete_progress)
            .expect("delete file");
        assert!(!moved_file.exists());
        assert_eq!(delete_progress.completed.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(delete_progress.total.load(AtomicOrdering::Relaxed), 1);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn copies_and_deletes_nested_trees_in_parallel() {
        let directory =
            std::env::temp_dir().join(format!("dae-parallel-tree-test-{}", std::process::id()));
        let source = directory.join("source");
        let destination = directory.join("destination");

        for index in 0..3 {
            let nested = source.join(format!("folder-{index}"));
            fs::create_dir_all(&nested).expect("create nested directory");
            fs::write(nested.join("file.txt"), "parallel").expect("create nested file");
        }
        fs::create_dir_all(&destination).expect("create destination directory");

        let copy_progress = TestProgress::new();
        copy_entries_with_progress(
            vec![(source.clone(), ConflictAction::Fail)],
            destination.clone(),
            &copy_progress,
            &mut Vec::new(),
        )
        .expect("copy nested tree");
        assert_eq!(copy_progress.completed.load(AtomicOrdering::Relaxed), 7);
        assert_eq!(copy_progress.total.load(AtomicOrdering::Relaxed), 7);

        let copied_root = destination.join("source");
        for index in 0..3 {
            assert!(
                copied_root
                    .join(format!("folder-{index}"))
                    .join("file.txt")
                    .is_file()
            );
        }

        let delete_progress = TestProgress::new();
        delete_entries_with_progress(vec![copied_root], &delete_progress)
            .expect("delete nested tree");
        assert_eq!(delete_progress.completed.load(AtomicOrdering::Relaxed), 7);
        assert_eq!(delete_progress.total.load(AtomicOrdering::Relaxed), 7);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn creates_files_and_directories_with_validated_names() {
        let directory =
            std::env::temp_dir().join(format!("dae-create-entry-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");

        let file_path =
            create_entry_sync(directory.clone(), "notes.txt".into(), NewEntryKind::File)
                .expect("create file");
        assert_eq!(file_path, path_to_string(&directory.join("notes.txt")));
        assert!(directory.join("notes.txt").is_file());

        let directory_path = create_entry_sync(
            directory.clone(),
            "子文件夹".into(),
            NewEntryKind::Directory,
        )
        .expect("create directory");
        assert_eq!(directory_path, path_to_string(&directory.join("子文件夹")));
        assert!(directory.join("子文件夹").is_dir());

        let duplicate_error =
            create_entry_sync(directory.clone(), "notes.txt".into(), NewEntryKind::File)
                .expect_err("creating over an existing entry should fail");
        assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

        let separator_error =
            create_entry_sync(directory.clone(), "a/b.txt".into(), NewEntryKind::File)
                .expect_err("names with path separators should fail");
        assert!(matches!(separator_error, FileSystemError::InvalidInput(_)));

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn resolves_local_transfer_conflicts_with_replace_skip_and_keep_both() {
        let root =
            std::env::temp_dir().join(format!("dae-conflict-local-test-{}", std::process::id()));
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).expect("create source directory");
        fs::create_dir_all(&destination_dir).expect("create destination directory");

        // Replace: the existing file is deleted, then the source is copied.
        fs::write(source_dir.join("report.txt"), "new content").expect("write source file");
        fs::write(destination_dir.join("report.txt"), "old content").expect("write target file");
        let replace_progress = TestProgress::new();
        copy_entries_with_progress(
            vec![(source_dir.join("report.txt"), ConflictAction::Replace)],
            destination_dir.clone(),
            &replace_progress,
            &mut Vec::new(),
        )
        .expect("copy with replace");
        assert_eq!(
            fs::read_to_string(destination_dir.join("report.txt")).expect("read replaced file"),
            "new content"
        );
        assert_eq!(
            replace_progress.completed.load(AtomicOrdering::Relaxed),
            replace_progress.total.load(AtomicOrdering::Relaxed)
        );
        assert_eq!(replace_progress.total.load(AtomicOrdering::Relaxed), 2);

        // Skip: the destination stays untouched and the source survives.
        let skip_progress = TestProgress::new();
        copy_entries_with_progress(
            vec![(source_dir.join("report.txt"), ConflictAction::Skip)],
            destination_dir.clone(),
            &skip_progress,
            &mut Vec::new(),
        )
        .expect("copy with skip");
        assert_eq!(
            fs::read_to_string(destination_dir.join("report.txt")).expect("skipped file unchanged"),
            "new content"
        );
        assert!(source_dir.join("report.txt").is_file());

        // Keep both: the incoming file lands under a "副本" name.
        let keep_progress = TestProgress::new();
        copy_entries_with_progress(
            vec![(source_dir.join("report.txt"), ConflictAction::KeepBoth)],
            destination_dir.clone(),
            &keep_progress,
            &mut Vec::new(),
        )
        .expect("copy keeping both");
        assert_eq!(
            fs::read_to_string(destination_dir.join("report 副本.txt")).expect("kept copy exists"),
            "new content"
        );
        assert_eq!(
            keep_progress.completed.load(AtomicOrdering::Relaxed),
            keep_progress.total.load(AtomicOrdering::Relaxed)
        );

        // Move with replace removes the source once the target is replaced.
        let move_progress = TestProgress::new();
        move_entries_with_progress(
            vec![(source_dir.join("report.txt"), ConflictAction::Replace)],
            destination_dir.clone(),
            &move_progress,
            &mut Vec::new(),
        )
        .expect("move with replace");
        assert!(!source_dir.join("report.txt").exists());
        assert_eq!(
            fs::read_to_string(destination_dir.join("report.txt")).expect("moved file content"),
            "new content"
        );
        assert_eq!(
            move_progress.completed.load(AtomicOrdering::Relaxed),
            move_progress.total.load(AtomicOrdering::Relaxed)
        );

        // Replacing a directory tree with a file works across kinds.
        fs::create_dir_all(destination_dir.join("bundle")).expect("create target directory");
        fs::write(destination_dir.join("bundle/inner.txt"), "inner").expect("write inner file");
        fs::write(source_dir.join("bundle"), "now a file").expect("write source file");
        copy_entries_with_progress(
            vec![(source_dir.join("bundle"), ConflictAction::Replace)],
            destination_dir.clone(),
            &TestProgress::new(),
            &mut Vec::new(),
        )
        .expect("replace a directory with a file");
        assert_eq!(
            fs::read_to_string(destination_dir.join("bundle")).expect("replaced directory"),
            "now a file"
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn moving_an_entry_onto_itself_is_a_no_op() {
        let root = std::env::temp_dir().join(format!("dae-conflict-self-{}", std::process::id()));
        let directory = root.join("folder");
        fs::create_dir_all(&directory).expect("create directory");
        fs::write(directory.join("file.txt"), "content").expect("write file");

        let move_progress = TestProgress::new();
        move_entries_with_progress(
            vec![(directory.join("file.txt"), ConflictAction::Replace)],
            directory.clone(),
            &move_progress,
            &mut Vec::new(),
        )
        .expect("move onto itself is skipped");
        assert_eq!(
            fs::read_to_string(directory.join("file.txt")).expect("file survives"),
            "content"
        );
        assert_eq!(move_progress.total.load(AtomicOrdering::Relaxed), 0);

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn is_within_rejects_paths_that_merely_share_a_prefix() {
        assert!(!is_within(Path::new("/a/b"), Path::new("/a/bc")));
        assert!(!is_within(Path::new("/a/b"), Path::new("/a/b")));
        assert!(is_within(Path::new("/a/b"), Path::new("/a/b/c")));
        // The root of a volume is the one base that ends in a separator.
        assert!(is_within(Path::new("/"), Path::new("/a/b")));
    }

    #[cfg(windows)]
    #[test]
    fn is_within_treats_the_two_canonical_spellings_as_one_path() {
        // `canonical_path` drops `\\?\` per path, so a source short enough to
        // keep the plain spelling and a descendant past `MAX_PATH` really do
        // come back spelled differently.
        assert!(is_within(
            Path::new(r"K:\folder"),
            Path::new(r"\\?\K:\folder\pasted")
        ));
        assert!(is_within(
            Path::new(r"\\?\K:\folder"),
            Path::new(r"K:\folder\pasted")
        ));

        assert!(!is_within(
            Path::new(r"K:\folder"),
            Path::new(r"K:\folder-2")
        ));
        assert!(is_within(Path::new(r"K:\"), Path::new(r"K:\folder\file")));
    }

    #[cfg(windows)]
    #[test]
    fn plans_paste_destinations_without_the_verbatim_prefix() {
        let root =
            std::env::temp_dir().join(format!("dae-transfer-plan-spelling-{}", std::process::id()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).expect("create the source");
        fs::create_dir_all(&destination).expect("create the destination");
        fs::write(source.join("pasted.txt"), "content").expect("write the source file");

        assert!(
            destination
                .canonicalize()
                .expect("canonicalize the fixture")
                .to_string_lossy()
                .starts_with(r"\\?\"),
            "the fixture no longer exercises the prefix this test is about"
        );

        let plan = build_transfer_plan(
            vec![(source.clone(), ConflictAction::Fail)],
            destination.clone(),
        )
        .expect("plan the paste");

        // Every entry of a pasted tree is joined onto this base, and a verbatim
        // base is rebuilt on every one of those joins (`canonical_path`).
        let planned = plan[0].destination.to_string_lossy().into_owned();
        assert!(
            !planned.starts_with(r"\\?\"),
            "a paste base of {planned} rebuilds that base for every entry"
        );

        fs::remove_dir_all(root).expect("remove test directory");
    }
}
