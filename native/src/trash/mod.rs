//! Recycle bin / trash view.
//!
//! Mirrors the Tauri shell's `trash_view`: listing is a deliberate scan of the
//! whole bin and runs on a blocking thread, while restore, purge, and empty
//! stream progress. Windows lists by parsing `$Recycle.Bin` metadata directly
//! (see [`recycle_bin`]); freedesktop systems use the `trash` crate; macOS
//! keeps its Trash as a private Finder domain handled by `dae-core`.

use serde::{Deserialize, Serialize};

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
use dae_core::file_system::types::path_to_string;
// The freedesktop helper is host-independent, so the Windows development
// machine type-checks and tests it too.
#[cfg(any(
    test,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
use std::path::{Path, PathBuf};

/// One entry currently sitting in the system trash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    /// System identifier of the item inside the trash: the shell parsing name
    /// on Windows, the `.trashinfo` path on freedesktop systems.
    pub id: String,
    /// Absolute path of the payload inside the trash, when the backend can
    /// name it. This is what `fileicon://` and `thumbnail://` read; `id` is
    /// not a usable path on freedesktop systems.
    pub path: Option<String>,
    /// Display name of the entry as it was deleted.
    pub name: String,
    /// Folder the entry lived in before deletion; restore puts it back there.
    pub original_parent: String,
    /// Unix seconds at which the entry was deleted.
    pub time_deleted: i64,
    /// True when the entry itself is a directory.
    pub is_directory: bool,
    /// File size in bytes; `None` for directories and unknown sizes.
    pub size_bytes: Option<u64>,
}

/// Error message code reported when a restore is requested for an entry whose
/// original location is unknown (no put-back record).
#[cfg(target_os = "macos")]
const TRASH_RESTORE_NO_ORIGIN: &str = "fs.trash_restore_no_origin";

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
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
            path: payload_path(Path::new(&item.id)).map(|path| path_to_string(&path)),
            name: item.name.to_string_lossy().into_owned(),
            original_parent: path_to_string(&item.original_parent),
            time_deleted: item.time_deleted,
            is_directory,
            size_bytes,
        }
    }
}

/// The freedesktop spec keeps an entry's payload and its `.trashinfo` under
/// the same name in sibling `files/` and `info/` directories, while the crate
/// hands back only the info path. Anything that is not an info file has no
/// payload this can name.
#[cfg(any(
    test,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn payload_path(info: &Path) -> Option<PathBuf> {
    let name = info.file_name()?.to_str()?.strip_suffix(".trashinfo")?;
    let trash_root = info.parent()?.parent()?;
    Some(trash_root.join("files").join(name))
}

#[cfg(test)]
mod tests {
    use super::payload_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn derives_a_freedesktop_payload_from_its_trashinfo_file() {
        let info = Path::new("/home/user/.local/share/Trash/info/report.pdf.trashinfo");
        assert_eq!(
            payload_path(info),
            Some(PathBuf::from(
                "/home/user/.local/share/Trash/files/report.pdf"
            ))
        );
    }

    #[test]
    fn derives_a_mounted_volume_payload_too() {
        let info = Path::new("/media/usb/.Trash-1000/info/clip.mp4.trashinfo");
        assert_eq!(
            payload_path(info),
            Some(PathBuf::from("/media/usb/.Trash-1000/files/clip.mp4"))
        );
    }

    #[test]
    fn refuses_names_that_are_not_trashinfo_files() {
        assert_eq!(
            payload_path(Path::new("/home/user/.local/share/Trash/files/report.pdf")),
            None
        );
        assert_eq!(payload_path(Path::new("")), None);
    }
}

#[cfg(target_os = "windows")]
mod recycle_bin;

/// The shell-backed implementation: restore, purge, and empty on every
/// platform the `trash` crate supports, plus listing on freedesktop systems.
#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
mod browse {
    use super::TrashEntry;
    use crate::file_ops::progress::{
        FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
        emit_preparing,
    };
    use dae_core::file_system::error::FileSystemError;
    use std::ffi::OsString;

    /// Lists every entry currently in the system trash, newest deletion first.
    pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
        tokio::task::spawn_blocking(|| {
            let mut entries = collect_entries()?;
            entries.sort_by_key(|entry| std::cmp::Reverse(entry.time_deleted));
            Ok(entries)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    #[cfg(target_os = "windows")]
    fn collect_entries() -> Result<Vec<TrashEntry>, FileSystemError> {
        super::recycle_bin::scan()
    }

    #[cfg(not(target_os = "windows"))]
    fn collect_entries() -> Result<Vec<TrashEntry>, FileSystemError> {
        let items = trash::os_limited::list().map_err(trash_error)?;
        Ok(items.iter().map(TrashEntry::from_item).collect())
    }

    /// Restores trashed entries to their original locations, one shell
    /// operation per entry so progress streams and a later collision does not
    /// roll back earlier successes. Returns the number actually restored.
    pub async fn restore_trash_entries(
        ids: Vec<String>,
        operation_id: String,
    ) -> Result<u64, FileSystemError> {
        if ids.is_empty() {
            return Ok(0);
        }

        emit_preparing(&operation_id, FileOperationKind::Move);

        tokio::task::spawn_blocking(move || {
            let selected = take_trash_items(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Move);
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

    /// Permanently deletes trashed entries. Returns the number actually purged.
    pub async fn delete_trash_entries(
        ids: Vec<String>,
        operation_id: String,
    ) -> Result<u64, FileSystemError> {
        if ids.is_empty() {
            return Ok(0);
        }

        emit_preparing(&operation_id, FileOperationKind::Delete);

        tokio::task::spawn_blocking(move || {
            let selected = take_trash_items(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Delete);
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
    pub async fn empty_trash(operation_id: String) -> Result<u64, FileSystemError> {
        emit_preparing(&operation_id, FileOperationKind::Delete);

        tokio::task::spawn_blocking(move || {
            let items = trash::os_limited::list().map_err(trash_error)?;
            if items.is_empty() {
                return Ok(0);
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Delete);
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

    /// Fetches the current trash contents and keeps only the requested items.
    /// Entries purged or restored elsewhere meanwhile are silently dropped; an
    /// empty result is reported as an error by the caller.
    fn take_trash_items(ids: &[String]) -> Result<Vec<trash::TrashItem>, FileSystemError> {
        let wanted: Vec<OsString> = ids.iter().map(OsString::from).collect();
        let items = trash::os_limited::list().map_err(trash_error)?;
        Ok(items
            .into_iter()
            .filter(|item| wanted.contains(&item.id))
            .collect())
    }

    fn trash_error(error: trash::Error) -> FileSystemError {
        FileSystemError::Internal(error.to_string())
    }
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub use browse::*;

/// macOS implementation: `~/.Trash` is a plain folder, so listing, purging,
/// and emptying are direct filesystem operations, and restores replay the
/// Finder "Put Back" records parsed by `dae-core`'s `macos_trash`.
#[cfg(target_os = "macos")]
mod macos_view {
    use super::TrashEntry;
    use crate::file_ops::progress::{
        FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
        emit_preparing,
    };
    use dae_core::file_system::error::FileSystemError;
    use dae_core::file_system::macos_trash::{self, TrashDiskEntry};
    use dae_core::file_system::types::path_to_string;
    use std::cmp::Reverse;
    use std::path::{Path, PathBuf};

    /// Lists every entry in `~/.Trash`, newest modification first (macOS
    /// records no deletion time).
    pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
        tokio::task::spawn_blocking(|| {
            let mut entries: Vec<TrashEntry> = macos_trash::list_entries()?
                .iter()
                .map(trash_entry)
                .collect();
            entries.sort_by_key(|entry| Reverse(entry.time_deleted));
            Ok(entries)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    /// Puts trashed entries back, replaying the Finder put-back records one
    /// rename at a time. Returns the number actually restored.
    pub async fn restore_trash_entries(
        ids: Vec<String>,
        operation_id: String,
    ) -> Result<u64, FileSystemError> {
        if ids.is_empty() {
            return Ok(0);
        }

        emit_preparing(&operation_id, FileOperationKind::Move);

        tokio::task::spawn_blocking(move || {
            let selected = take_trash_entries(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Move);
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

    /// Permanently deletes trashed entries. Returns the number actually purged.
    pub async fn delete_trash_entries(
        ids: Vec<String>,
        operation_id: String,
    ) -> Result<u64, FileSystemError> {
        if ids.is_empty() {
            return Ok(0);
        }

        emit_preparing(&operation_id, FileOperationKind::Delete);

        tokio::task::spawn_blocking(move || {
            let selected = take_trash_entries(&ids)?;
            if selected.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.trash_entries_missing".into(),
                ));
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Delete);
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
    /// put-back records by removing `.DS_Store`.
    pub async fn empty_trash(operation_id: String) -> Result<u64, FileSystemError> {
        emit_preparing(&operation_id, FileOperationKind::Delete);

        tokio::task::spawn_blocking(move || {
            let entries = macos_trash::list_entries()?;
            if entries.is_empty() {
                return Ok(0);
            }

            let progress =
                FileOperationProgressReporter::new(operation_id, FileOperationKind::Delete);
            progress.start(entries.len() as u64);

            let mut purged = 0u64;
            for entry in &entries {
                macos_trash::purge(&entry.path)?;
                purged += 1;
                progress.advance(&entry.path);
            }
            progress.finish();

            let _ = std::fs::remove_file(macos_trash::trash_dir()?.join(".DS_Store"));
            Ok(purged)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
    }

    fn trash_entry(entry: &TrashDiskEntry) -> TrashEntry {
        TrashEntry {
            id: path_to_string(&entry.path),
            path: Some(path_to_string(&entry.path)),
            name: entry.name.clone(),
            original_parent: entry.original_parent.clone().unwrap_or_default(),
            time_deleted: entry.time_modified,
            is_directory: entry.is_directory,
            size_bytes: entry.size_bytes,
        }
    }

    /// Every id must be a direct child of `~/.Trash`, so a crafted id can
    /// never touch files outside the trash.
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

    fn same_path(left: &Path, right: &Path) -> bool {
        left == right
    }
}

#[cfg(target_os = "macos")]
pub use macos_view::*;

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
mod unsupported {
    use super::TrashEntry;
    use crate::file_ops::progress::FileOperationKind;
    use dae_core::file_system::error::FileSystemError;

    fn unsupported<T>() -> Result<T, FileSystemError> {
        Err(FileSystemError::Unsupported("fs.trash_unsupported".into()))
    }

    pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
        unsupported()
    }

    pub async fn restore_trash_entries(
        _ids: Vec<String>,
        _operation_id: String,
    ) -> Result<u64, FileSystemError> {
        unsupported()
    }

    pub async fn delete_trash_entries(
        _ids: Vec<String>,
        _operation_id: String,
    ) -> Result<u64, FileSystemError> {
        unsupported()
    }

    pub async fn empty_trash(_operation_id: String) -> Result<u64, FileSystemError> {
        unsupported()
    }

    #[allow(dead_code)]
    fn keep_kind_referenced(_kind: FileOperationKind) {}
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub use unsupported::*;
