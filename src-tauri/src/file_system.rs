use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::Manager;
use thiserror::Error;

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
    #[error("The directory operation could not complete: {0}")]
    Internal(String),
}

impl From<io::Error> for FileSystemError {
    fn from(error: io::Error) -> Self {
        let message = error.to_string();

        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound(message),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(message),
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
        read_directory_sync,
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
}
