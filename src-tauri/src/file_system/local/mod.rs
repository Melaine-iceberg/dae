//! The local-disk backend: direct `std::fs` access to drive letters, POSIX
//! paths, and UNC shares. Sibling protocol backends (smb, sftp, ...) will live
//! next to this module as they land.

mod directory;
mod operations;
mod search;

pub use directory::{DirectoryChanged, DirectoryWatcher, create_directory_watcher};

#[cfg(test)]
pub use directory::{build_breadcrumbs, read_directory_sync};
#[cfg(test)]
pub use operations::{
    copy_entries_with_progress, create_entry_sync, delete_entries_with_progress,
    move_entries_with_progress, rename_entry_sync,
};
#[cfg(test)]
pub use search::search_directory_sync;

use crate::file_system::error::FileSystemError;
use crate::file_system::progress::FileOperationProgressReporterTrait;
use crate::file_system::types::{DirectoryView, NewEntryKind, SearchResponse};
use crate::file_system::vfs::FileSystemBackend;
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

    fn copy(
        &self,
        sources: &[String],
        destination: &str,
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError> {
        operations::copy_entries_with_progress(
            sources.iter().map(PathBuf::from).collect(),
            PathBuf::from(destination),
            progress,
        )
    }

    fn move_entries(
        &self,
        sources: &[String],
        destination: &str,
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError> {
        operations::move_entries_with_progress(
            sources.iter().map(PathBuf::from).collect(),
            PathBuf::from(destination),
            progress,
        )
    }

    fn delete(
        &self,
        paths: &[String],
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError> {
        operations::delete_entries_with_progress(
            paths.iter().map(PathBuf::from).collect(),
            progress,
        )
    }

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        search::search_directory_sync(PathBuf::from(root), query, is_current)
    }
}
