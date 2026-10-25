//! The Tauri entry points.
//!
//! Every function here is a wrapper in the strict sense: it takes what Tauri
//! injects, turns the `AppHandle` into the seam, and calls the service function
//! in [`super::service`]. No logic lives in this file.
//!
//! That split is the point of the migration. The Qt shell does not have commands
//! or an `AppHandle`; it calls `service::*` directly, and the only thing that
//! changes between the two shells is where the `Arc<dyn Host>` comes from.
//!
//! The two exceptions are [`read_directory_packets`] and [`handle_raw_invoke`]:
//! a `Channel` of raw responses has no TypeScript type to generate, so it is
//! dispatched by name in [`handle_raw_invoke`] instead of through `specta`. Both
//! leave with the packet codec when the Qt shell lands.

use super::error::FileSystemError;
use super::listing;
use super::service;
use super::types::{
    DirectoryEntry, DirectoryView, FileProperties, NewEntryKind, PropertyChanges,
    RecursivePropertyUpdateOutcome, RenameRequest, TransferConflict, TransferItem,
};
use super::undo::UndoRedoOutcome;
use crate::host;

/// Returns the operating system's home directory for the initial explorer view.
#[tauri::command]
#[specta::specta]
pub fn get_home_directory() -> Result<String, FileSystemError> {
    service::get_home_directory()
}

/// Reads one directory for the explorer: a first batch of entries that renders
/// immediately, plus the stream the rest of them arrive on.
///
/// Large local directories stream instead of blocking on one big read: the
/// response carries the first batch along with the `stream_id` that the
/// remaining entries arrive under, as `DirectoryEntriesBatch` events (see
/// [`super::listing`]). Every other backend — and any local directory that
/// fits in a single batch — answers with a complete view and a `None` stream
/// id.
///
/// `watcher_id` names the view asking, and arms its change watcher for `path`
/// before the directory is read, so a change landing while the listing runs is
/// either already in it or reported as an event. The explorer passes the id it
/// minted for the pane; reads that only feed a side pane (sidebar tree, Miller
/// columns) pass `None`, because a view that does not display the directory
/// has no business watching it.
///
/// `vfs::resolve` can open a network session (a blocking, runtime-owning
/// operation), so it must run on a blocking thread — never on the async
/// workers, where a nested runtime panics.
#[tauri::command]
#[specta::specta]
pub async fn read_directory(
    path: String,
    stream_id: String,
    watcher_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<DirectoryView, FileSystemError> {
    service::read_directory(path, stream_id, watcher_id, host::of(&app)).await
}

/// Reads exactly the children a change event named, so a listing that is already
/// on screen can be repaired instead of re-walked.
///
/// The alternative is the expensive one: one new file in a 35,803 entry directory
/// used to cost a full re-read — ~12 ms of metadata reads on the backend, ~5.8 MB
/// of listing over IPC, and a comparison of every row to find the one that
/// differed. This answers one entry per name asked for.
///
/// `directory` is the spelling the listing reports (canonical for a local
/// directory), and the paths this returns are built from it, which is what makes
/// a patched row match the row it replaces. `names` come from
/// [`super::DirectoryChanged`]; a name no longer on disk answers `null`, and any
/// other stat failure errors the whole call so the caller re-reads rather than
/// patching from a half-truth.
#[tauri::command]
#[specta::specta]
pub async fn read_directory_changes(
    directory: String,
    names: Vec<String>,
) -> Result<Vec<Option<DirectoryEntry>>, FileSystemError> {
    service::read_directory_changes(directory, names).await
}

/// Renames a single directory entry without allowing a path change.
#[tauri::command]
#[specta::specta]
pub async fn rename_entry(
    path: String,
    new_name: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    service::rename_entry(path, new_name, host::of(&app)).await
}

/// Renames many entries in a single undoable step — the pattern engine of the
/// bulk-rename dialog. Name chains and swaps resolve through staging, so
/// "a→b, b→a" works, and the whole batch reverts with one undo. Returns the
/// destination paths in request order so the UI can reselect them.
#[tauri::command]
#[specta::specta]
pub async fn rename_entries_batch(
    requests: Vec<RenameRequest>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<String>, FileSystemError> {
    service::rename_entries_batch(requests, operation_id, host::of(&app)).await
}

/// Creates a new file or directory inside an existing directory and returns its path.
#[tauri::command]
#[specta::specta]
pub async fn create_entry(
    directory: String,
    name: String,
    kind: NewEntryKind,
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    service::create_entry(directory, name, kind, host::of(&app)).await
}

/// Copies entries into an existing destination directory. Each item's conflict
/// action decides what happens when the target name already exists; the UI
/// usually resolves collisions through `check_transfer_conflicts` first.
#[tauri::command]
#[specta::specta]
pub async fn copy_entries(
    items: Vec<TransferItem>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    service::copy_entries(items, destination, operation_id, host::of(&app)).await
}

/// Moves entries into an existing destination directory. Each item's conflict
/// action decides what happens when the target name already exists; the UI
/// usually resolves collisions through `check_transfer_conflicts` first.
#[tauri::command]
#[specta::specta]
pub async fn move_entries(
    items: Vec<TransferItem>,
    destination: String,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    service::move_entries(items, destination, operation_id, host::of(&app)).await
}

/// Reports the name collisions a copy/move into `destination` would hit, so
/// the UI can show the conflict dialog and collect per-entry resolutions
/// before invoking the transfer.
#[tauri::command]
#[specta::specta]
pub async fn check_transfer_conflicts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<TransferConflict>, FileSystemError> {
    service::check_transfer_conflicts(sources, destination).await
}

/// Permanently deletes entries. The UI must obtain confirmation before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn delete_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    service::delete_entries(paths, operation_id, host::of(&app)).await
}

/// Moves local entries into the system trash (recycle bin) instead of deleting
/// them permanently. The batch stays undoable via [`undo_operation`].
/// Network paths are rejected; the UI routes those to `delete_entries`.
///
/// This only records `(parent, name)` pairs — no trash enumeration here,
/// because `os_limited::list()` scans the whole recycle bin via the shell API
/// and would make every delete visibly slow.
#[tauri::command]
#[specta::specta]
pub async fn trash_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    service::trash_entries(paths, operation_id, host::of(&app)).await
}

/// Undoes the most recent recorded operation (move, rename, copy, trash,
/// create, duplicate) and returns toast text describing what was reverted.
/// A failed step is dropped from the history rather than left stale.
#[tauri::command]
#[specta::specta]
pub async fn undo_operation(
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<UndoRedoOutcome, FileSystemError> {
    service::undo_operation(operation_id, host::of(&app)).await
}

/// Re-applies the most recent undone operation and returns toast text
/// describing what was restored.
#[tauri::command]
#[specta::specta]
pub async fn redo_operation(
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<UndoRedoOutcome, FileSystemError> {
    service::redo_operation(operation_id, host::of(&app)).await
}

/// Duplicates entries next to their originals ("name 副本"), returning the
/// created paths so the UI can refresh and select them.
#[tauri::command]
#[specta::specta]
pub async fn duplicate_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<String>, FileSystemError> {
    service::duplicate_entries(paths, operation_id, host::of(&app)).await
}

/// Pushes the locale's duplicate-name token (e.g. "副本" / "copy") so the
/// backend names Keep-Both conflicts and duplicates in the UI language.
/// Called by the frontend at startup and on language changes.
#[tauri::command]
#[specta::specta]
pub fn set_duplicate_suffix(
    suffix: String,
)  {
    service::set_duplicate_suffix(suffix);
}

/// Reads one entry's full metadata for the properties dialog. Local paths
/// return the platform permission model (POSIX mode bits and ownership, or
/// Windows DOS attributes); other backends degrade to a view-only summary.
#[tauri::command]
#[specta::specta]
pub async fn get_file_properties(
    path: String,
) -> Result<FileProperties, FileSystemError> {
    service::get_file_properties(path).await
}

/// Applies property edits (permissions, ownership, attributes). Fields left
/// unset keep their current value.
#[tauri::command]
#[specta::specta]
pub async fn update_file_properties(
    path: String,
    changes: PropertyChanges,
) -> Result<(), FileSystemError> {
    service::update_file_properties(path, changes).await
}

/// Applies property edits to a directory and everything beneath it (the
/// "apply to enclosed items" option). Progress streams under `operation_id`;
/// the outcome separates entries that were updated from entries that failed,
/// so the UI can surface partial success instead of an all-or-nothing error.
#[tauri::command]
#[specta::specta]
pub async fn update_file_properties_recursive(
    operation_id: String,
    path: String,
    changes: PropertyChanges,
    app: tauri::AppHandle,
) -> Result<RecursivePropertyUpdateOutcome, FileSystemError> {
    service::update_file_properties_recursive(operation_id, path, changes, host::of(&app)).await
}

/// Opens the user's system default terminal at a local directory.
///
/// Each platform honors its own "default terminal" setting: Windows routes
/// the spawned console process to the configured default terminal host
/// (e.g. Windows Terminal), macOS opens Terminal.app, and Linux resolves
/// `$TERMINAL`, `xdg-terminal-exec`, or a common desktop terminal.
#[tauri::command]
#[specta::specta]
pub fn open_terminal(
    path: String,
) -> Result<(), FileSystemError> {
    service::open_terminal(path)
}

/// Opens the system's native "Open With" picker for a local file or folder.
///
/// Windows shows the shell's "How do you want to open this file?" dialog,
/// letting the user pick any installed application; the command resolves once
/// the dialog is dismissed. Since Windows 10 that dialog can no longer change
/// default associations (registration flags are ignored by the OS), so it only
/// opens the item once. macOS and Linux use the in-app picker backed by
/// `file_system::open_with` instead, where this command fails.
#[tauri::command]
#[specta::specta]
pub async fn open_with(
    path: String,
) -> Result<(), FileSystemError> {
    service::open_with(path).await
}

/// The same read as [`read_directory`], but the batches that follow the head
/// arrive as columnar packets on `on_batch` instead of as events.
///
/// The head is still the JSON `DirectoryView` both commands share: it is 512
/// entries, and its error path is the one the explorer already handles. What
/// moves off JSON is the `8192`-entry stream, which for a 35,803 entry
/// directory is 89 % of the payload — the part that cost the webview 25 ms of
/// `eval` on its own thread (measured), against 0.56 ms to hand over a buffer
/// and 0.01 ms to materialise the 40 rows a frame paints.
///
/// The completion marker rides in the last packet's header flags rather than in
/// a separate message, because a channel carrying `InvokeResponseBody` delivers
/// small JSON payloads through `eval` and large ones through `fetch` — the
/// frontend would have to tell an object from an `ArrayBuffer` to know which it
/// got. Raw bytes always arrive as an `ArrayBuffer`, so the stream stays one
/// kind of thing. See [`super::entry_codec::HEADER_FLAG_FINAL`].
///
/// Not registered with specta: a `Channel` of raw responses has no TypeScript
/// type to generate, so the frontend invokes it by name — see
/// [`handle_raw_invoke`].
#[tauri::command]
pub async fn read_directory_packets(
    path: String,
    stream_id: String,
    watcher_id: Option<String>,
    on_batch: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
    app: tauri::AppHandle,
) -> Result<DirectoryView, FileSystemError> {
    service::read_directory_into(
        path,
        stream_id,
        watcher_id,
        host::of(&app),
        listing::BatchSink::Channel(on_batch),
    )
    .await
}

/// Dispatches the commands that carry raw IPC bodies, which cannot live in the
/// generated bindings. Called from `lib.rs` before the specta handler.
pub fn handle_raw_invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    match invoke.message.command() {
        "read_directory_packets" => __cmd__read_directory_packets!(read_directory_packets, invoke),
        _ => false,
    }
}
