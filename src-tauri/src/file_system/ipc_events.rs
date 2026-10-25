//! The wire shapes of the six explorer events, and nothing else.
//!
//! These structs own the event names the frontend binds to, so they stay in the
//! shell: `tauri_specta::Event` is an IPC concept and `dae-core` must not depend
//! on it. The modules that produce them are engine-free and report through
//! [`dae_core::host::Host`]; [`crate::host::TauriHost`] is the one place that
//! rebuilds these payloads.

use super::types::DirectoryEntry;
use dae_core::hashing::FileHashDigests;
use dae_core::progress::{FileOperationKind, FileOperationPhase};
use serde::Serialize;
use specta::Type;

/// Emitted while a long operation runs. `completed` is clamped to `total` so a
/// backend that over-counts cannot push a bar past 100%.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-file-operation-progress")]
#[serde(rename_all = "camelCase")]
pub struct FileOperationProgress {
    pub operation_id: String,
    pub operation: FileOperationKind,
    pub phase: FileOperationPhase,
    pub completed: u64,
    pub total: Option<u64>,
    pub current_path: Option<String>,
}

/// One batch of a streamed listing. The head (`read_directory`'s response) and
/// these batches together form the complete directory.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-entries")]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntriesBatch {
    /// Echoes the `stream_id` the caller passed to `read_directory`.
    pub stream_id: String,
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
    /// True on the final batch: the listing is complete.
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-changed")]
#[serde(rename_all = "camelCase")]
pub struct DirectoryChanged {
    /// The directory that changed, in the spelling its listing reports.
    pub path: String,
    /// The children that changed, by name.
    ///
    /// Empty means the change is *not attributable to a row* — the watcher hit
    /// an error (whose payload may have been dropped), reported the watched
    /// directory itself, or came from a backend that only knows "something
    /// differs". A caller then has to re-read; a non-empty list is everything it
    /// needs to repair a listing it already holds.
    pub names: Vec<String>,
}

/// Emitted whenever the undo/redo stacks change so shortcuts stay accurate.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-undo-redo-changed")]
#[serde(rename_all = "camelCase")]
pub struct UndoRedoChanged {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-size-progress")]
#[serde(rename_all = "camelCase")]
pub struct DirectorySizeProgress {
    pub operation_id: String,
    pub path: String,
    pub size: u64,
    /// False while the scan runs (partial total); true on the final total.
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-file-hash-progress")]
#[serde(rename_all = "camelCase")]
pub struct FileHashProgress {
    pub operation_id: String,
    pub path: String,
    pub bytes_read: u64,
    pub total_bytes: u64,
    /// False while the read runs; true on the final report.
    pub completed: bool,
    /// Present only in the successful final report.
    pub digests: Option<FileHashDigests>,
    /// Present only when the final report carries a failure.
    pub error: Option<String>,
}
