//! The local-disk backend: direct `std::fs` access to drive letters, POSIX
//! paths, and UNC shares. Sibling protocol backends (smb, sftp, ...) will live
//! next to this module as they land.

mod content_search;
mod directory;
mod operations;
mod search;

pub use content_search::{ContentSearchParams, search_file_contents_sync};
pub use directory::create_directory_watcher;
pub use operations::{
    copy_entries_with_progress, delete_entries_with_progress, move_entries_with_progress,
};

#[cfg(test)]
pub use directory::{build_breadcrumbs, read_directory_sync};
#[cfg(test)]
pub use operations::{create_entry_sync, rename_entry_sync};
#[cfg(test)]
pub use search::search_directory_sync;

use crate::file_system::error::FileSystemError;
use crate::file_system::types::{DirectoryView, EntryStat, NewEntryKind, SearchResponse};
use crate::file_system::vfs::FileSystemBackend;
use std::io::{Read, Write};
use std::path::PathBuf;

pub struct LocalBackend;

impl FileSystemBackend for LocalBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        directory::read_directory_sync(PathBuf::from(path))
    }

    fn create_entry(
        &self,
        directory: &str,
        name: &str,
        kind: NewEntryKind,
    ) -> Result<String, FileSystemError> {
        operations::create_entry_sync(PathBuf::from(directory), name.to_owned(), kind)
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        operations::rename_entry_sync(PathBuf::from(path), new_name.to_owned())
    }

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        search::search_directory_sync(PathBuf::from(root), query, is_current)
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let metadata = std::fs::symlink_metadata(PathBuf::from(path))?;

        Ok(EntryStat {
            kind: directory::entry_kind(metadata.file_type()),
            size: metadata.len(),
            modified_at: directory::modified_at_millis(&metadata),
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        std::fs::create_dir(PathBuf::from(path))?;
        Ok(())
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        Ok(Box::new(std::fs::File::open(PathBuf::from(path))?))
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        Ok(Box::new(std::fs::File::create(PathBuf::from(path))?))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let path = PathBuf::from(path);
        let metadata = std::fs::symlink_metadata(&path)?;

        if metadata.is_dir() {
            std::fs::remove_dir(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }

        Ok(())
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        std::fs::rename(PathBuf::from(source), PathBuf::from(destination))?;
        Ok(())
    }
}
