//! Trash (recycle bin) browsing: listing, restoring, purging, and emptying.
//!
//! The `trash` crate's `os_limited` API enumerates the whole recycle bin
//! through the shell (IFileOperation on Windows), which is slow enough that
//! the delete flow records `(parent, name)` pairs instead of enumerating —
//! see `commands::trash_entries`. Browsing is a deliberate scan, so this
//! module runs it on blocking threads and streams progress for the long
//! restore/purge operations.
//!
//! `os_limited` only compiles on Windows and freedesktop-compliant Unix:
//! macOS keeps the Trash as a private Finder domain and the crate supports
//! only `delete` there. On macOS the four commands run directly against
//! `~/.Trash` plus the Finder put-back records in `~/.Trash/.DS_Store` —
//! see `macos_trash.rs` — so the whole trash view works there too.

use serde::Serialize;
use specta::Type;

/// `true` on platforms where the `trash` crate exposes the full recycle-bin
/// API (Windows and freedesktop Trash environments).
#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
use super::types::path_to_string;

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

/// Error message code reported when a restore is requested for an entry
/// whose original location is unknown (no put-back record).
#[cfg(target_os = "macos")]
const TRASH_RESTORE_NO_ORIGIN: &str = "fs.trash_restore_no_origin";

#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
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

#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
mod browse {
    use super::super::error::FileSystemError;
    use super::super::progress::{
        FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
        emit_preparing,
    };
    use super::TrashEntry;
    use std::ffi::OsString;

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
}

#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
pub use browse::*;

/// macOS implementation: `~/.Trash` is a plain folder, so listing, purging,
/// and emptying are direct filesystem operations, and restores replay the
/// Finder "Put Back" records parsed by `macos_trash`. The command
/// signatures are identical on every platform, so the generated bindings
/// stay stable.
#[cfg(target_os = "macos")]
mod macos_view {
    use super::super::error::FileSystemError;
    use super::super::progress::{
        FileOperationKind, FileOperationProgressReporter, emit_preparing,
    };
    use super::super::types::path_to_string;
    use super::TrashEntry;
    use crate::file_system::macos_trash::{self, TrashDiskEntry};
    use std::cmp::Reverse;
    use std::path::{Path, PathBuf};

    /// Lists every entry in `~/.Trash`, newest modification first (macOS
    /// records no deletion time; see `macos_trash`). Runs on a blocking
    /// thread like the shell-backed implementations.
    #[tauri::command]
    #[specta::specta]
    pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
        tauri::async_runtime::spawn_blocking(|| {
            let mut entries: Vec<TrashEntry> =
                macos_trash::list_entries()?.iter().map(trash_entry).collect();
            entries.sort_by_key(|entry| Reverse(entry.time_deleted));
            Ok(entries)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    /// Puts trashed entries back to their original locations, replaying the
    /// Finder put-back records one rename at a time so progress streams and
    /// a later failure does not roll back earlier successes. Returns the
    /// number of entries actually restored.
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
            let selected = take_trash_entries(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Move);
            progress.start(selected.len() as u64);

            let mut restored = 0u64;
            for entry in selected {
                let Some(destination) = entry.original_destination() else {
                    return Err(FileSystemError::Unsupported(
                        super::TRASH_RESTORE_NO_ORIGIN.into(),
                    ));
                };
                macos_trash::restore(&entry.path, &destination)?;
                restored += 1;
                progress.advance(&destination);
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
            let selected = take_trash_entries(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);
            progress.start(selected.len() as u64);

            let mut purged = 0u64;
            for entry in selected {
                macos_trash::purge(&entry.path)?;
                purged += 1;
                progress.advance(&entry.path);
            }
            progress.finish();
            Ok(purged)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    /// Permanently deletes everything in `~/.Trash`, then drops the stale
    /// put-back records by removing `.DS_Store` (Finder recreates it on
    /// demand). Returns the purged count; the UI must confirm before calling
    /// this command.
    #[tauri::command]
    #[specta::specta]
    pub async fn empty_trash(
        operation_id: String,
        app: tauri::AppHandle,
    ) -> Result<u64, FileSystemError> {
        emit_preparing(&app, &operation_id, FileOperationKind::Delete);

        tauri::async_runtime::spawn_blocking(move || {
            let entries = macos_trash::list_entries()?;
            if entries.is_empty() {
                return Ok(0);
            }

            let progress =
                FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);
            progress.start(entries.len() as u64);

            let mut purged = 0u64;
            for entry in &entries {
                macos_trash::purge(&entry.path)?;
                purged += 1;
                progress.advance(&entry.path);
            }
            progress.finish();

            // All entries survived; stale put-back metadata can go too.
            let _ = std::fs::remove_file(macos_trash::trash_dir()?.join(".DS_Store"));
            Ok(purged)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    /// One listed entry. The trash path doubles as the id the frontend
    /// round-trips through the restore/purge commands.
    fn trash_entry(entry: &TrashDiskEntry) -> TrashEntry {
        TrashEntry {
            id: path_to_string(&entry.path),
            name: entry.name.clone(),
            original_parent: entry.original_parent.clone().unwrap_or_default(),
            time_deleted: entry.time_modified,
            is_directory: entry.is_directory,
            size_bytes: entry.size_bytes,
        }
    }

    /// Fetches the current trash contents and keeps only the entries whose
    /// id the caller asked for. Every id must be a direct child of
    /// `~/.Trash`, so a crafted id can never touch files outside the trash.
    /// Entries purged or restored elsewhere meanwhile are reported as an
    /// error, like on the other platforms.
    fn take_trash_entries(ids: &[String]) -> Result<Vec<TrashDiskEntry>, FileSystemError> {
        let wanted: Vec<PathBuf> = ids.iter().map(PathBuf::from).collect();
        if wanted.iter().any(|path| !macos_trash::is_trash_child(path)) {
            return Err(FileSystemError::InvalidInput(
                "fs.trash_entries_missing".into(),
            ));
        }
        let entries = macos_trash::list_entries()?;
        Ok(entries
            .into_iter()
            .filter(|entry| wanted.iter().any(|path| same_path(path, &entry.path)))
            .collect())
    }

    /// `~/.Trash` resolves through `$HOME`, so plain component comparison is
    /// exact enough — no canonicalization of a possibly-vanished entry.
    fn same_path(left: &Path, right: &Path) -> bool {
        left == right
    }
}

#[cfg(target_os = "macos")]
pub use macos_view::*;
