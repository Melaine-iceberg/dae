use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, entry_sort_key, path_to_string,
};
use crate::file_system::watch::DirectoryChanged;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs;
use std::path::{Path, PathBuf};
use tauri_specta::Event;

#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY};

pub fn create_directory_watcher(
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
                let _ = DirectoryChanged(event_path.clone()).emit(&app);
            }
        })
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    Ok(watcher)
}

/// The beginning of a directory listing: the entries that were read, plus a
/// cursor onto the entries that are still unread.
///
/// The split is what makes large directories streamable: the head renders
/// immediately while [`DirectoryListingCursor`] keeps the OS directory
/// iterator alive on the thread that streams the rest (see
/// [`super::super::listing`]).
pub struct DirectoryListing {
    pub view: DirectoryView,
    /// `None` once the directory turned out to fit in the batch, which makes
    /// `view` complete.
    pub cursor: Option<DirectoryListingCursor>,
}

/// The unread remainder of a directory listing: the live `read_dir` iterator.
///
/// `std::fs::ReadDir` is `Send`, so the reader thread that streams the
/// remaining batches can own it. Entries whose metadata cannot be read are
/// skipped rather than failing the whole listing — WSL's `/proc` and `/run`
/// over the `\\wsl$` 9P share behave that way.
pub struct DirectoryListingCursor {
    iterator: fs::ReadDir,
}

impl DirectoryListingCursor {
    /// Pulls up to `count` entries, sorted for display. The boolean reports
    /// that the directory is exhausted, so the caller can close the stream.
    pub fn take(&mut self, count: usize) -> (Vec<DirectoryEntry>, bool) {
        // `count` may be `usize::MAX` when a caller wants everything at once;
        // never reserve that much up front.
        let mut entries = Vec::with_capacity(count.min(MAX_BATCH_RESERVE));
        let mut exhausted = false;

        for _ in 0..count {
            match self.iterator.next() {
                Some(Ok(entry)) => {
                    if let Some(entry) = directory_entry(entry) {
                        entries.push(entry);
                    }
                }
                // A failing entry is skipped; iteration continues (or ends)
                // on the next call.
                Some(Err(_)) => continue,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }

        entries.sort_by_cached_key(entry_sort_key);
        (entries, exhausted)
    }
}

/// Ceiling for the pre-allocation above, so a huge `count` cannot ask the
/// allocator for gigabytes before a single entry has been read.
const MAX_BATCH_RESERVE: usize = 8192;

/// Reads the first `first_batch` entries of a directory. The returned cursor
/// holds the unread remainder and is `None` when the directory was exhausted,
/// which means the view is already complete.
pub fn open_directory_listing(
    requested_path: PathBuf,
    first_batch: usize,
) -> Result<DirectoryListing, FileSystemError> {
    let path = requested_path.canonicalize()?;
    let metadata = fs::metadata(&path)?;

    if !metadata.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    let mut cursor = DirectoryListingCursor {
        iterator: fs::read_dir(&path)?,
    };
    let (entries, exhausted) = cursor.take(first_batch);

    Ok(DirectoryListing {
        view: DirectoryView {
            path: path_to_string(&path),
            breadcrumbs: build_breadcrumbs(&path),
            entries,
            stream_id: None,
        },
        cursor: (!exhausted).then_some(cursor),
    })
}

/// Reads a directory as one complete, sorted snapshot.
pub fn read_directory_sync(requested_path: PathBuf) -> Result<DirectoryView, FileSystemError> {
    let DirectoryListing { mut view, cursor } = open_directory_listing(requested_path, usize::MAX)?;

    if let Some(mut cursor) = cursor {
        // Unreachable with a `usize::MAX` batch size, but draining keeps the
        // two entry points equivalent if that ever changes.
        let (rest, _) = cursor.take(usize::MAX);
        view.entries.extend(rest);
        view.entries.sort_by_cached_key(entry_sort_key);
    }

    Ok(view)
}

/// Converts one raw directory entry, or `None` when its type or metadata
/// cannot be read.
fn directory_entry(entry: fs::DirEntry) -> Option<DirectoryEntry> {
    let file_type = entry.file_type().ok()?;
    let metadata = entry.metadata().ok()?;
    let kind = entry_kind(file_type);
    let size = matches!(&kind, EntryKind::File).then_some(metadata.len());
    let name = entry.file_name().to_string_lossy().into_owned();
    let (hidden, read_only) = entry_state_flags(&metadata, &name);

    Some(DirectoryEntry {
        name,
        path: path_to_string(&entry.path()),
        kind,
        modified_at: modified_at_millis(&metadata),
        size,
        hidden,
        read_only,
    })
}


pub fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

pub fn entry_kind(file_type: fs::FileType) -> EntryKind {
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

/// `(hidden, read_only)` derived from metadata already fetched during
/// listing — no extra system calls. Windows reads the DOS attribute bits;
/// Unix approximates with the dot prefix and the owner write bit.
#[cfg(windows)]
pub fn entry_state_flags(metadata: &fs::Metadata, _name: &str) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;

    let attributes = metadata.file_attributes();
    (
        attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
        attributes & FILE_ATTRIBUTE_READONLY.0 != 0,
    )
}

#[cfg(unix)]
pub fn entry_state_flags(metadata: &fs::Metadata, name: &str) -> (bool, bool) {
    use std::os::unix::fs::PermissionsExt;

    (
        name.starts_with('.'),
        metadata.permissions().mode() & 0o200 == 0,
    )
}

#[cfg(not(any(windows, unix)))]
pub fn entry_state_flags(_metadata: &fs::Metadata, _name: &str) -> (bool, bool) {
    (false, false)
}

pub fn build_breadcrumbs(path: &Path) -> Vec<Breadcrumb> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    use super::super::properties::update_properties;
    #[cfg(windows)]
    use crate::file_system::types::PropertyChanges;

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
    #[allow(clippy::permissions_set_readonly_false)] // temp fixture, deleted right after
    fn reports_read_only_and_hidden_entry_flags() {
        let directory =
            std::env::temp_dir().join(format!("dae-entry-flags-test-{}", std::process::id()));
        let plain_file = directory.join("plain.txt");
        let read_only_file = directory.join("locked.txt");
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(&plain_file, "plain").expect("create plain file");
        fs::write(&read_only_file, "locked").expect("create read-only file");

        let mut permissions = fs::metadata(&read_only_file)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&read_only_file, permissions).expect("mark file read-only");

        #[cfg(unix)]
        let hidden_file = {
            let hidden_file = directory.join(".hidden.txt");
            fs::write(&hidden_file, "hidden").expect("create hidden file");
            hidden_file
        };
        #[cfg(windows)]
        let hidden_file = {
            let hidden_file = directory.join("hidden.txt");
            fs::write(&hidden_file, "hidden").expect("create hidden file");
            update_properties(
                &hidden_file,
                &PropertyChanges {
                    hidden: Some(true),
                    ..Default::default()
                },
            )
            .expect("mark file hidden");
            hidden_file
        };

        let view = read_directory_sync(directory.clone()).expect("read test directory");
        let by_name = |name: &str| {
            view.entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap_or_else(|| panic!("entry {name} is missing from the listing"))
        };

        assert!(!by_name("plain.txt").hidden);
        assert!(!by_name("plain.txt").read_only);
        assert!(by_name("locked.txt").read_only);
        let hidden_name = hidden_file
            .file_name()
            .expect("hidden file has a name")
            .to_string_lossy()
            .into_owned();
        assert!(by_name(&hidden_name).hidden);

        // Restore writability so cleanup succeeds on every platform.
        let mut permissions = fs::metadata(&read_only_file)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&read_only_file, permissions).expect("clear read-only");

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
