//! Trash (recycle bin) browsing: listing, restoring, purging, and emptying.
//!
//! The `trash` crate's `os_limited` API enumerates the whole recycle bin
//! through the shell (IFileOperation on Windows), which is slow enough that
//! the delete flow records `(parent, name)` pairs instead of enumerating —
//! see `commands::trash_entries`. Browsing is a deliberate scan, so this
//! module runs it on blocking threads and streams progress for the long
//! restore/purge operations.

use super::error::FileSystemError;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::types::path_to_string;
use serde::Serialize;
use specta::Type;
use std::ffi::OsString;

/// One entry currently sitting in the system trash.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    /// System identifier of the item inside the trash: the shell parsing
    /// name on Windows, the `.trashinfo` path on freedesktop systems.
    pub id: String,
    /// Display name of the entry as it was deleted.
    pub name: String,
    /// Folder the entry lived in before deletion; restore puts it back there.
    pub original_parent: String,
    /// Unix seconds at which the entry was deleted.
    pub time_deleted: i64,
    /// True when shell metadata reported the entry as a directory.
    pub is_directory: bool,
    /// File size in bytes; `None` for directories and unknown sizes.
    pub size_bytes: Option<u64>,
}

impl TrashEntry {
    fn from_item(item: &trash::TrashItem) -> Self {
        // Metadata resolution is best effort: an unreadable shell item still
        // lists (as a file of unknown size) so one bad entry cannot hide the
        // rest of the trash.
        let (is_directory, size_bytes) = match trash::os_limited::metadata(item) {
            Ok(metadata) => match metadata.size {
                trash::TrashItemSize::Bytes(bytes) => (false, Some(bytes)),
                trash::TrashItemSize::Entries(_) => (true, None),
            },
            Err(_) => (false, None),
        };

        Self {
            id: item.id.to_string_lossy().into_owned(),
            name: item.name.to_string_lossy().into_owned(),
            original_parent: path_to_string(&item.original_parent),
            time_deleted: item.time_deleted,
            is_directory,
            size_bytes,
        }
    }
}

/// Lists every entry currently in the system trash, newest deletion first.
///
/// This scans the whole recycle bin through the shell API, so it runs on a
/// blocking thread; the frontend only calls it when opening the trash view
/// or after a trash operation.
#[tauri::command]
#[specta::specta]
pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(|| {
        let items = trash::os_limited::list().map_err(trash_error)?;
        let mut entries: Vec<TrashEntry> =
            items.iter().map(TrashEntry::from_item).collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.time_deleted));
        Ok(entries)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Restores trashed entries (selected by their trash ids) to their original
/// locations, one shell operation per entry so progress streams and a later
/// collision does not roll back earlier successes. Returns the number of
/// entries actually restored.
#[tauri::command]
#[specta::specta]
pub async fn restore_trash_entries(
    ids: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<u64, FileSystemError> {
    if ids.is_empty() {
        return Ok(0);
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Move);

    tauri::async_runtime::spawn_blocking(move || {
        let selected = take_trash_items(&ids)?;
        if selected.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "fs.trash_entries_missing".into(),
            ));
        }

        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Move);
        progress.start(selected.len() as u64);

        let mut restored = 0u64;
        for item in selected {
            let original = item.original_path();
            trash::os_limited::restore_all(std::iter::once(item)).map_err(trash_error)?;
            restored += 1;
            progress.advance(&original);
        }
        progress.finish();
        Ok(restored)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes trashed entries (selected by their trash ids).
/// Returns the number of entries actually purged.
#[tauri::command]
#[specta::specta]
pub async fn delete_trash_entries(
    ids: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<u64, FileSystemError> {
    if ids.is_empty() {
        return Ok(0);
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let selected = take_trash_items(&ids)?;
        if selected.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "fs.trash_entries_missing".into(),
            ));
        }

        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);
        progress.start(selected.len() as u64);

        let mut purged = 0u64;
        for item in selected {
            let original = item.original_path();
            trash::os_limited::purge_all(std::iter::once(item)).map_err(trash_error)?;
            purged += 1;
            progress.advance(&original);
        }
        progress.finish();
        Ok(purged)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes everything in the trash. Returns the purged count;
/// the UI must confirm before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn empty_trash(
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<u64, FileSystemError> {
    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let items = trash::os_limited::list().map_err(trash_error)?;
        if items.is_empty() {
            return Ok(0);
        }

        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);
        progress.start(items.len() as u64);

        let mut purged = 0u64;
        for item in items {
            let original = item.original_path();
            trash::os_limited::purge_all(std::iter::once(item)).map_err(trash_error)?;
            purged += 1;
            progress.advance(&original);
        }
        progress.finish();
        Ok(purged)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Fetches the current trash contents and keeps only the items whose id the
/// caller asked for. Entries purged or restored elsewhere meanwhile are
/// silently dropped; an empty result is reported as an error by the caller.
fn take_trash_items(ids: &[String]) -> Result<Vec<trash::TrashItem>, FileSystemError> {
    let wanted: Vec<OsString> = ids.iter().map(OsString::from).collect();
    let items = trash::os_limited::list().map_err(trash_error)?;
    Ok(items.into_iter().filter(|item| wanted.contains(&item.id)).collect())
}

fn trash_error(error: trash::Error) -> FileSystemError {
    FileSystemError::Internal(error.to_string())
}
