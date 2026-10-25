//! macOS `~/.Trash` primitives: the `trash` crate supports only `delete` on
//! macOS, so the trash view lists, restores, and purges entries directly.
//!
//! The user's Trash is a plain folder (`~/.Trash`) that Finder curates, and
//! `trash::delete` goes through Finder on macOS, so items deleted from this
//! app land there like any other. The original location of an item — what
//! Finder's "Put Back" uses — is stored only in `~/.Trash/.DS_Store` as
//! per-item `ptbN` (original name) / `ptbL` (original parent) records;
//! [`trash_core::macos`] parses that store. Entries without a put-back
//! record (deleted outside Finder, or Finder not having flushed yet)
//! simply have no known origin and cannot be put back automatically.
//!
//! macOS records no deletion time anywhere, so entries report the item's
//! modification time instead — the best available approximation.

use super::error::FileSystemError;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fs, io};

/// One entry currently sitting in `~/.Trash`.
#[derive(Debug, Clone)]
pub struct TrashDiskEntry {
    /// Absolute path of the item inside the trash folder.
    pub path: PathBuf,
    /// Current file name inside the trash; Finder de-duplicates collisions,
    /// so this need not equal the original name.
    pub name: String,
    /// Original file name at deletion time (`ptbN`), when recorded.
    pub original_name: Option<String>,
    /// Original parent directory (`ptbL`, firmlink-normalized to the
    /// user-visible `/…` form), when recorded.
    pub original_parent: Option<String>,
    /// Modification time in unix seconds — macOS records no deletion time.
    pub time_modified: i64,
    /// True when the item is a directory.
    pub is_directory: bool,
    /// File size in bytes; `None` for directories.
    pub size_bytes: Option<u64>,
}

impl TrashDiskEntry {
    /// Where "put back" would drop the item: the original parent joined with
    /// the original name. `None` when either put-back record is missing.
    pub fn original_destination(&self) -> Option<PathBuf> {
        Some(PathBuf::from(self.original_parent.as_deref()?).join(self.original_name.as_deref()?))
    }
}

/// The user's home trash folder (`~/.Trash`).
pub fn trash_dir() -> Result<PathBuf, FileSystemError> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FileSystemError::Internal("$HOME is not set; cannot locate the Trash".into())
        })?;
    Ok(PathBuf::from(home).join(".Trash"))
}

/// True when `path` refers to a direct child of `~/.Trash` — the only paths
/// the restore/purge commands accept, so a crafted id can never reach files
/// outside the trash.
pub fn is_trash_child(path: &Path) -> bool {
    match trash_dir() {
        Ok(dir) => path.parent() == Some(dir.as_path()),
        Err(_) => false,
    }
}

/// Lists every visible entry in `~/.Trash`. Finder's own `.DS_Store` is
/// skipped; one unreadable entry cannot hide the rest. A trash folder that
/// does not exist yet lists as empty.
pub fn list_entries() -> Result<Vec<TrashDiskEntry>, FileSystemError> {
    let dir = trash_dir()?;
    let mut entries = Vec::new();

    let read_dir = match fs::read_dir(&dir) {
        Ok(read_dir) => read_dir,
        // Nothing has ever been deleted; nothing to list.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(entries),
        Err(error) => return Err(error.into()),
    };

    let put_backs = put_backs();
    for item in read_dir {
        let Ok(item) = item else {
            continue;
        };
        let file_name = item.file_name();
        let name = file_name.to_string_lossy().into_owned();
        if name == ".DS_Store" {
            continue;
        }

        let path = dir.join(&file_name);
        // Symlink metadata so a dangling link still lists (as a file of
        // unknown size), matching the best-effort spirit of the other
        // platforms.
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let is_directory = metadata.is_dir();
        let size_bytes = (!is_directory).then_some(metadata.len());
        let time_modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);

        // Firmlink form (`System/Volumes/Data/…`) never leaks out: keep the
        // normalized parent for display and restores alike.
        let put_back = put_backs.iter().find(|record| record.trash_name == name);
        let (original_name, original_parent) = match put_back {
            Some(record) => {
                let original_parent = record
                    .original_path()
                    .as_deref()
                    .and_then(|path| Path::new(path).parent())
                    .map(|parent| parent.to_string_lossy().into_owned());
                (record.original_name.clone(), original_parent)
            }
            None => (None, None),
        };

        entries.push(TrashDiskEntry {
            path,
            name,
            original_name,
            original_parent,
            time_modified,
            is_directory,
            size_bytes,
        });
    }

    Ok(entries)
}

/// Moves a trash entry back to `destination` — the equivalent of Finder's
/// "Put Back", but via `rename`. The trash lives on the same volume as its
/// origins, so a plain rename suffices. POSIX `rename` overwrites a file
/// that already occupies the destination, but the shell-backed platforms
/// report a collision instead — so an existing destination is rejected
/// the same way rather than silently replacing the target.
pub fn restore(path: &Path, destination: &Path) -> Result<(), FileSystemError> {
    if destination.symlink_metadata().is_ok() {
        return Err(FileSystemError::AlreadyExists(
            destination.to_string_lossy().into_owned(),
        ));
    }
    fs::rename(path, destination).map_err(|error| trash_restore_error(error, path, destination))
}

/// Permanently deletes one trash entry. Directories go recursively.
pub fn purge(path: &Path) -> Result<(), FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Puts `trash_core`'s parse error into context instead of leaking the raw
/// crate error through the command boundary.
fn trash_restore_error(error: io::Error, path: &Path, destination: &Path) -> FileSystemError {
    FileSystemError::Internal(format!(
        "could not put {} back to {}: {error}",
        path.display(),
        destination.display()
    ))
}

/// Put-back records from `~/.Trash/.DS_Store`, best effort: an unreadable or
/// unparsable store simply yields no origins, so one bad metadata file
/// cannot hide the rest of the trash.
fn put_backs() -> Vec<trash_core::macos::PutBack> {
    let Ok(dir) = trash_dir() else {
        return Vec::new();
    };
    let Ok(bytes) = fs::read(dir.join(".DS_Store")) else {
        return Vec::new();
    };
    trash_core::macos::parse_put_back(&bytes).unwrap_or_default()
}
