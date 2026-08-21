use crate::file_system::error::FileSystemError;
use crate::file_system::progress::FileOperationProgressReporterTrait;
use crate::file_system::transfer::duplicate_name;
use crate::file_system::types::{ConflictAction, NewEntryKind, TransferPair, path_to_string};
use rayon::prelude::*;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
}

pub fn move_entries_with_progress(
    sources: Vec<(PathBuf, ConflictAction)>,
    destination: PathBuf,
    progress: &dyn FileOperationProgressReporterTrait,
    journal: &mut Vec<TransferPair>,
) -> Result<(), FileSystemError> {
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
}

pub fn delete_entries_with_progress(
    paths: Vec<PathBuf>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
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

    let destination = requested_destination.canonicalize()?;
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
                ConflictAction::Replace => {
                    (target.clone(), count_entry_units(&target)?)
                }
                ConflictAction::KeepBoth => {
                    let kept =
                        unique_sibling_path(&destination, &name, &planned_destinations)?;
                    planned_destinations.insert(kept.clone());
                    (kept, 0)
                }
            },
        };

        if metadata.is_dir() {
            let canonical_source = source.canonicalize()?;
            if target.starts_with(&canonical_source) {
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

/// True when both paths resolve to the same on-disk entry.
fn paths_refer_to_same_entry(source: &Path, target: &Path) -> bool {
    match (fs::canonicalize(source), fs::canonicalize(target)) {
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
