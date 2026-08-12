use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::{Emitter, Manager};
use thiserror::Error;

const DIRECTORY_CHANGED_EVENT: &str = "explorer-directory-changed";

#[derive(Default)]
pub struct DirectoryWatcher {
    generation: AtomicU64,
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl DirectoryWatcher {
    fn begin_update(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn replace(&self, generation: u64, watcher: RecommendedWatcher) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = Some(watcher);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Breadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    /// The absolute, canonical path that was read.
    pub path: String,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Error, Serialize, Type)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum FileSystemError {
    #[error("The requested path was not found: {0}")]
    NotFound(String),
    #[error("Permission was denied: {0}")]
    PermissionDenied(String),
    #[error("The requested path is not a directory: {0}")]
    NotDirectory(String),
    #[error("The file system operation failed: {0}")]
    Io(String),
    #[error("An item already exists at the destination: {0}")]
    AlreadyExists(String),
    #[error("The requested operation has invalid input: {0}")]
    InvalidInput(String),
    #[error("The directory operation could not complete: {0}")]
    Internal(String),
}

impl From<io::Error> for FileSystemError {
    fn from(error: io::Error) -> Self {
        let message = error.to_string();

        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound(message),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(message),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(message),
            io::ErrorKind::InvalidInput => Self::InvalidInput(message),
            _ => Self::Io(message),
        }
    }
}

/// Returns the operating system's home directory for the initial explorer view.
#[tauri::command]
#[specta::specta]
pub fn get_home_directory(app: tauri::AppHandle) -> Result<String, FileSystemError> {
    app.path()
        .home_dir()
        .map(|path| path_to_string(&path))
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

/// Reads one directory as an immutable snapshot suitable for rendering in the explorer.
#[tauri::command]
#[specta::specta]
pub async fn read_directory(path: String) -> Result<DirectoryView, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || read_directory_sync(PathBuf::from(path)))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Replaces the active watcher with one that tracks the currently displayed directory.
#[tauri::command]
#[specta::specta]
pub async fn watch_directory(path: String, app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let generation = app.state::<DirectoryWatcher>().begin_update();
    let watcher_app = app.clone();
    let watcher = tauri::async_runtime::spawn_blocking(move || {
        create_directory_watcher(PathBuf::from(path), watcher_app)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))??;

    app.state::<DirectoryWatcher>().replace(generation, watcher)
}

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
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        copy_entries_sync(paths_from_strings(sources), PathBuf::from(destination))
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
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        move_entries_sync(paths_from_strings(sources), PathBuf::from(destination))
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes entries. The UI must obtain confirmation before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn delete_entries(paths: Vec<String>) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || delete_entries_sync(paths_from_strings(paths)))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn create_directory_watcher(
    requested_path: PathBuf,
    app: tauri::AppHandle,
) -> Result<RecommendedWatcher, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let event_path = path_to_string(&path);
    let mut watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            // Watcher errors (e.g. ReadDirectoryChangesW buffer overflow on network
            // shares) mean changes were dropped, so treat them as "possibly dirty".
            let should_refresh = match &result {
                Ok(event) => !matches!(event.kind, EventKind::Access(_)),
                Err(_) => true,
            };

            if should_refresh {
                let _ = app.emit(DIRECTORY_CHANGED_EVENT, &event_path);
            }
        })
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    Ok(watcher)
}

fn read_directory_sync(requested_path: PathBuf) -> Result<DirectoryView, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let metadata = fs::metadata(&path)?;

    if !metadata.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    let mut entries = fs::read_dir(&path)?
        .map(|entry| {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let metadata = entry.metadata()?;
            let kind = entry_kind(file_type);
            let size = matches!(&kind, EntryKind::File).then_some(metadata.len());

            Ok(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: path_to_string(&entry.path()),
                kind,
                modified_at: modified_at_millis(&metadata),
                size,
            })
        })
        .collect::<Result<Vec<_>, FileSystemError>>()?;

    entries.sort_by(compare_entries);

    Ok(DirectoryView {
        path: path_to_string(&path),
        breadcrumbs: build_breadcrumbs(&path),
        entries,
    })
}

fn paths_from_strings(paths: Vec<String>) -> Vec<PathBuf> {
    paths.into_iter().map(PathBuf::from).collect()
}

fn rename_entry_sync(path: PathBuf, new_name: String) -> Result<(), FileSystemError> {
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

fn copy_entries_sync(sources: Vec<PathBuf>, destination: PathBuf) -> Result<(), FileSystemError> {
    let plan = build_transfer_plan(sources, destination)?;

    for entry in plan {
        copy_entry(&entry.source, &entry.destination)?;
    }

    Ok(())
}

fn move_entries_sync(sources: Vec<PathBuf>, destination: PathBuf) -> Result<(), FileSystemError> {
    let plan = build_transfer_plan(sources, destination)?;

    for entry in plan {
        match fs::rename(&entry.source, &entry.destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                copy_entry(&entry.source, &entry.destination)?;
                remove_entry(&entry.source)?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

fn delete_entries_sync(paths: Vec<PathBuf>) -> Result<(), FileSystemError> {
    ensure_unique_paths(&paths)?;

    for path in &paths {
        fs::symlink_metadata(path)?;
    }

    for path in paths {
        remove_entry(&path)?;
    }

    Ok(())
}

#[derive(Debug)]
struct TransferPlanEntry {
    source: PathBuf,
    destination: PathBuf,
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

        plan.push(TransferPlanEntry {
            source,
            destination: target,
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

fn copy_entry(source: &Path, destination: &Path) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_file() {
        fs::copy(source, destination)?;
        return Ok(());
    }

    if metadata.is_dir() {
        return copy_directory(source, destination);
    }

    if metadata.file_type().is_symlink() {
        return copy_symlink(source, destination);
    }

    Err(FileSystemError::InvalidInput(format!(
        "Unsupported entry type: {}",
        path_to_string(source)
    )))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), FileSystemError> {
    fs::create_dir(destination)?;

    for child in fs::read_dir(source)? {
        let child = child?;
        copy_entry(&child.path(), &destination.join(child.file_name()))?;
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

fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn entry_kind(file_type: fs::FileType) -> EntryKind {
    if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

fn compare_entries(left: &DirectoryEntry, right: &DirectoryEntry) -> Ordering {
    let left_rank = entry_kind_rank(&left.kind);
    let right_rank = entry_kind_rank(&right.kind);

    left_rank
        .cmp(&right_rank)
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        .then_with(|| left.name.cmp(&right.name))
}

fn entry_kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::Symlink => 1,
        EntryKind::File => 2,
        EntryKind::Other => 3,
    }
}

fn build_breadcrumbs(path: &Path) -> Vec<Breadcrumb> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();

    ancestors
        .into_iter()
        .map(|ancestor| Breadcrumb {
            name: ancestor
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_to_string(ancestor)),
            path: path_to_string(ancestor),
        })
        .collect()
}

fn path_to_string(path: &Path) -> String {
    normalize_path_for_display(&path.to_string_lossy())
}

fn normalize_path_for_display(path: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }

        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    }

    #[cfg(not(windows))]
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        DirectoryEntry, EntryKind, FileSystemError, build_breadcrumbs, compare_entries,
        copy_entries_sync, delete_entries_sync, move_entries_sync, read_directory_sync,
        rename_entry_sync,
    };
    use std::fs;

    #[cfg(windows)]
    use super::normalize_path_for_display;

    #[cfg(windows)]
    #[test]
    fn removes_windows_verbatim_path_prefixes() {
        assert_eq!(
            normalize_path_for_display(r"\\?\C:\Users\test"),
            r"C:\Users\test"
        );
        assert_eq!(
            normalize_path_for_display(r"\\?\UNC\server\share\folder"),
            r"\\server\share\folder"
        );
    }

    #[test]
    fn builds_clickable_breadcrumbs_from_a_path() {
        let path = std::env::temp_dir().join("dae").join("nested");
        let breadcrumbs = build_breadcrumbs(&path);

        assert_eq!(
            breadcrumbs.last().map(|item| item.path.as_str()),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            breadcrumbs.last().map(|item| item.name.as_str()),
            Some("nested")
        );
    }

    #[test]
    fn places_directories_before_files_case_insensitively() {
        let mut entries = [
            DirectoryEntry {
                name: "zeta.txt".into(),
                path: "zeta.txt".into(),
                kind: EntryKind::File,
                modified_at: None,
                size: None,
            },
            DirectoryEntry {
                name: "alpha".into(),
                path: "alpha".into(),
                kind: EntryKind::Directory,
                modified_at: None,
                size: None,
            },
            DirectoryEntry {
                name: "Beta".into(),
                path: "Beta".into(),
                kind: EntryKind::Directory,
                modified_at: None,
                size: None,
            },
        ];

        entries.sort_by(compare_entries);

        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "Beta");
        assert_eq!(entries[2].name, "zeta.txt");
    }

    #[test]
    fn reads_entries_from_a_directory() {
        let directory =
            std::env::temp_dir().join(format!("dae-file-system-test-{}", std::process::id()));
        let nested_directory = directory.join("folder");
        let file = directory.join("file.txt");

        fs::create_dir_all(&nested_directory).expect("create test directory");
        fs::write(&file, "test").expect("create test file");

        let view = read_directory_sync(directory.clone()).expect("read test directory");
        let names = view
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["folder", "file.txt"]);

        let folder_entry = &view.entries[0];
        let file_entry = &view.entries[1];
        assert!(folder_entry.modified_at.is_some());
        assert_eq!(folder_entry.size, None);
        assert!(file_entry.modified_at.is_some());
        assert_eq!(file_entry.size, Some(4));

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn returns_not_directory_for_a_file_path() {
        let file =
            std::env::temp_dir().join(format!("dae-file-system-test-{}.txt", std::process::id()));
        fs::write(&file, "test").expect("create test file");

        let error = read_directory_sync(file.clone()).expect_err("a file is not a directory");

        fs::remove_file(file).expect("remove test file");

        assert!(matches!(error, FileSystemError::NotDirectory(_)));
    }

    #[test]
    fn performs_file_operations_without_overwriting_entries() {
        let directory =
            std::env::temp_dir().join(format!("dae-file-operation-test-{}", std::process::id()));
        let source = directory.join("source");
        let destination = directory.join("destination");
        let nested_file = source.join("nested.txt");

        fs::create_dir_all(&source).expect("create source directory");
        fs::create_dir_all(&destination).expect("create destination directory");
        fs::write(&nested_file, "copied content").expect("create source file");

        copy_entries_sync(vec![source.clone()], destination.clone()).expect("copy directory");

        let copied_directory = destination.join("source");
        assert_eq!(
            fs::read_to_string(copied_directory.join("nested.txt")).expect("read copied file"),
            "copied content"
        );
        assert!(source.exists());

        let duplicate_error = copy_entries_sync(vec![source.clone()], destination.clone())
            .expect_err("copying over an existing entry should fail");
        assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

        let nested_error = copy_entries_sync(vec![source.clone()], source.clone())
            .expect_err("copying a folder into itself should fail");
        assert!(matches!(nested_error, FileSystemError::InvalidInput(_)));

        rename_entry_sync(nested_file.clone(), "renamed.txt".into()).expect("rename file");
        let renamed_file = source.join("renamed.txt");
        assert!(renamed_file.exists());

        move_entries_sync(vec![renamed_file.clone()], destination.clone()).expect("move file");
        let moved_file = destination.join("renamed.txt");
        assert!(moved_file.exists());
        assert!(!renamed_file.exists());

        delete_entries_sync(vec![moved_file.clone()]).expect("delete file");
        assert!(!moved_file.exists());

        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
