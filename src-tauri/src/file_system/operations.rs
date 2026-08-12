use super::directory::path_to_string;
use super::error::FileSystemError;
use super::progress::{
    FileOperationProgressReporter, FileOperationProgressReporterTrait, emit_preparing,
};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Renames a single directory entry without allowing a path change.
#[tauri::command]
#[specta::specta]
pub async fn rename_entry(path: String, new_name: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || rename_entry_sync(PathBuf::from(path), new_name))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Copies entries into an existing destination directory. Existing files are never overwritten.
#[tauri::command]
#[specta::specta]
pub async fn copy_entries(
    sources: Vec<String>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    emit_preparing(&app, &operation_id, "copy");

    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = FileOperationProgressReporter::new(app, operation_id, "copy");
        copy_entries_with_progress(
            paths_from_strings(sources),
            PathBuf::from(destination),
            &mut progress,
        )
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Moves entries into an existing destination directory. Existing files are never overwritten.
#[tauri::command]
#[specta::specta]
pub async fn move_entries(
    sources: Vec<String>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    emit_preparing(&app, &operation_id, "move");

    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = FileOperationProgressReporter::new(app, operation_id, "move");
        move_entries_with_progress(
            paths_from_strings(sources),
            PathBuf::from(destination),
            &mut progress,
        )
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes entries. The UI must obtain confirmation before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn delete_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    emit_preparing(&app, &operation_id, "delete");

    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = FileOperationProgressReporter::new(app, operation_id, "delete");
        delete_entries_with_progress(paths_from_strings(paths), &mut progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn paths_from_strings(paths: Vec<String>) -> Vec<PathBuf> {
    paths.into_iter().map(PathBuf::from).collect()
}

pub(super) fn rename_entry_sync(path: PathBuf, new_name: String) -> Result<(), FileSystemError> {
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

pub(super) fn copy_entries_with_progress<P: FileOperationProgressReporterTrait>(
    sources: Vec<PathBuf>,
    destination: PathBuf,
    progress: &mut P,
) -> Result<(), FileSystemError> {
    let plan = build_transfer_plan(sources, destination)?;
    progress.start(plan.iter().map(|entry| entry.work_units).sum());

    for entry in plan {
        copy_entry(&entry.source, &entry.destination, progress)?;
    }

    progress.finish();
    Ok(())
}

pub(super) fn move_entries_with_progress<P: FileOperationProgressReporterTrait>(
    sources: Vec<PathBuf>,
    destination: PathBuf,
    progress: &mut P,
) -> Result<(), FileSystemError> {
    let plan = build_transfer_plan(sources, destination)?;
    progress.start(plan.iter().map(|entry| entry.work_units).sum());

    for entry in plan {
        progress.begin_entry(&entry.source);
        match fs::rename(&entry.source, &entry.destination) {
            Ok(()) => progress.advance_by(entry.work_units, &entry.source),
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                copy_entry(&entry.source, &entry.destination, progress)?;
                remove_entry(&entry.source)?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    progress.finish();
    Ok(())
}

pub(super) fn delete_entries_with_progress<P: FileOperationProgressReporterTrait>(
    paths: Vec<PathBuf>,
    progress: &mut P,
) -> Result<(), FileSystemError> {
    ensure_unique_paths(&paths)?;

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
    work_units: u64,
}

fn build_transfer_plan(
    sources: Vec<PathBuf>,
    requested_destination: PathBuf,
) -> Result<Vec<TransferPlanEntry>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before pasting".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    let destination = requested_destination.canonicalize()?;
    if !fs::metadata(&destination)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&destination)));
    }

    let mut planned_destinations = HashSet::new();
    let mut plan = Vec::with_capacity(sources.len());

    for source in sources {
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

        ensure_path_is_available(&target)?;

        if metadata.is_dir() {
            let canonical_source = source.canonicalize()?;
            if destination.starts_with(&canonical_source) {
                return Err(FileSystemError::InvalidInput(format!(
                    "Cannot paste a folder into itself: {}",
                    path_to_string(&source)
                )));
            }
        }

        let work_units = count_entry_units(&source)?;

        plan.push(TransferPlanEntry {
            source,
            destination: target,
            work_units,
        });
    }

    Ok(plan)
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

fn ensure_unique_paths(paths: &[PathBuf]) -> Result<(), FileSystemError> {
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

    fs::read_dir(path)?.try_fold(1, |count, child| {
        Ok(count + count_entry_units(&child?.path())?)
    })
}

fn copy_entry<P: FileOperationProgressReporterTrait>(
    source: &Path,
    destination: &Path,
    progress: &mut P,
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

fn copy_directory<P: FileOperationProgressReporterTrait>(
    source: &Path,
    destination: &Path,
    progress: &mut P,
) -> Result<(), FileSystemError> {
    fs::create_dir(destination)?;
    progress.advance(source);

    for child in fs::read_dir(source)? {
        let child = child?;
        copy_entry(
            &child.path(),
            &destination.join(child.file_name()),
            progress,
        )?;
    }

    Ok(())
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

fn delete_entry<P: FileOperationProgressReporterTrait>(
    path: &Path,
    progress: &mut P,
) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;
    progress.begin_entry(path);

    if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            delete_entry(&child?.path(), progress)?;
        }
        fs::remove_dir(path)?;
    } else if metadata.file_type().is_symlink() {
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))?;
    } else {
        fs::remove_file(path)?;
    }

    progress.advance(path);
    Ok(())
}
