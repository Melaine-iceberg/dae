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

use super::archive::{self, ArchiveFormat};
use super::error::FileSystemError;
use super::git::{self, GitBranches, GitDirectoryStatus};
use super::listing;
use super::open_with::{self, OpenWithChoices};
use super::preview::{self, TextPreview};
use super::search;
use super::service;
use super::system_files::{self, KeyModifiers, SystemClipboardFiles};
use super::trash_view::{self, TrashEntry};
use super::cloud::accounts::StoredCloudAccount;
use super::cloud::oauth::AuthorizeCloudAccountInput;
use super::connections::{SaveConnectionInput, StoredConnection};
use super::media_meta::MediaPreview;
use super::recents::{RecentItem, RecentSource};
use super::sidebar::{DiskVolume, Favorite, WslDistro};
use super::spaces::Space;
use super::types::{
    ContentSearchResponse, DirectoryEntry, DirectoryView, EntryKind, FileProperties, NewEntryKind,
    PropertyChanges, RecursivePropertyUpdateOutcome, RenameRequest, SearchResponse,
    TransferConflict, TransferItem,
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


/// 读取一个目录的 Git 装饰信息：当前分支与直接子项的 M/A/U 徽标。
/// 目录不在 Git 工作区内时返回 `None`。git2 的状态扫描在阻塞线程池
/// 执行，不会阻塞异步运行时或 UI；扫描结果按工作区根缓存（见
/// [`GitStatusCache`]），所以同一个工作区里的相邻目录只付一次扫描。
#[tauri::command]
#[specta::specta]
pub async fn get_git_status(
    path: String,
) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    git::get_git_status(path).await
}

/// 列出仓库的本地与远程分支、当前分支及其相对上游的领先/落后计数。
/// 目录不在 Git 工作区内时返回 `None`。
#[tauri::command]
#[specta::specta]
pub async fn list_git_branches(
    path: String,
) -> Result<Option<GitBranches>, FileSystemError> {
    git::list_git_branches(path).await
}

/// 切换分支。`remote` 为真时 `name` 形如 "origin/feature"，
/// 会创建同名的本地跟踪分支再切换。
#[tauri::command]
#[specta::specta]
pub async fn git_checkout_branch(
    path: String,
    name: String,
    remote: bool,
) -> Result<(), FileSystemError> {
    git::git_checkout_branch(path, name, remote).await
}

/// 基于当前 HEAD 创建分支；`checkout` 为真时同时切换过去。
#[tauri::command]
#[specta::specta]
pub async fn git_create_branch(
    path: String,
    name: String,
    checkout: bool,
) -> Result<(), FileSystemError> {
    git::git_create_branch(path, name, checkout).await
}

/// 从远程仓库获取更新（fetch --prune）。走 git 命令行，复用系统凭据
/// （凭据管理器 / SSH agent），本地操作仍由 git2 完成。
#[tauri::command]
#[specta::specta]
pub async fn git_fetch(
    path: String,
) -> Result<(), FileSystemError> {
    git::git_fetch(path).await
}

/// 推送当前分支；无上游时自动 `push --set-upstream` 到首个远程。
#[tauri::command]
#[specta::specta]
pub async fn git_push(
    path: String,
) -> Result<(), FileSystemError> {
    git::git_push(path).await
}

/// 拉取当前分支（fetch + merge/rebase，遵循仓库配置）。
#[tauri::command]
#[specta::specta]
pub async fn git_pull(
    path: String,
) -> Result<(), FileSystemError> {
    git::git_pull(path).await
}

/// 同步：有上游时先拉取再推送；无上游时发布分支（push --set-upstream）。
#[tauri::command]
#[specta::specta]
pub async fn git_sync(
    path: String,
) -> Result<(), FileSystemError> {
    git::git_sync(path).await
}

#[tauri::command]
#[specta::specta]
pub async fn read_text_preview(
    path: String,
    max_bytes: u32,
) -> Result<TextPreview, FileSystemError> {
    preview::read_text_preview(path, max_bytes).await
}

/// Recursively searches entry names beneath one directory. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_directory(
    path: String,
    query: String,
) -> Result<SearchResponse, FileSystemError> {
    search::search_directory(path, query).await
}

/// Stops the active traversal when the search surface is dismissed.
#[tauri::command]
#[specta::specta]
pub fn cancel_search()  {
    search::cancel_search();
}

/// Searches file contents beneath one local directory with optional regex,
/// case sensitivity, and file-type filtering. Ignores VCS/dependency
/// directories (`.git`, `node_modules`, `target`) by default. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_file_contents(
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
) -> Result<ContentSearchResponse, FileSystemError> {
    search::search_file_contents(path, query, is_regex, case_sensitive, file_filter).await
}

/// Lists the applications registered as able to open the local file or
/// directory at `path`, grouped the way a desktop's own picker groups them.
#[tauri::command]
#[specta::specta]
pub async fn list_open_with_apps(
    path: String,
) -> Result<OpenWithChoices, FileSystemError> {
    open_with::list_open_with_apps(path).await
}

/// Opens `path` with the application identified by `app_id`. When
/// `set_default` is set, the application also becomes the default handler for
/// the item's type.
#[tauri::command]
#[specta::specta]
pub async fn open_with_app(
    path: String,
    app_id: String,
    set_default: bool,
) -> Result<(), FileSystemError> {
    open_with::open_with_app(path, app_id, set_default).await
}


/// Places the given paths on the system clipboard as a file list (CF_HDROP on
/// Windows) so Explorer, browsers, and chat apps accept a paste. Setting `cut`
/// also writes Explorer's "Preferred DropEffect" marker, which turns a paste
/// in Explorer or here into a move instead of a copy.
#[tauri::command]
#[specta::specta]
pub async fn write_files_to_clipboard(
    paths: Vec<String>,
    cut: bool,
) -> Result<(), FileSystemError> {
    system_files::write_files_to_clipboard(paths, cut).await
}

/// Reads a file list from the system clipboard, if the clipboard holds one.
#[tauri::command]
#[specta::specta]
pub async fn read_files_from_clipboard() -> Result<Option<SystemClipboardFiles>, FileSystemError> {
    system_files::read_files_from_clipboard().await
}

/// Reports the modifier keys currently held, straight from the OS. Tauri's
/// drag-drop events carry only paths and a position, so a drop of files from
/// another application cannot read Ctrl/Shift/Alt off a DOM event the way the
/// in-window drag does; the frontend polls this instead while a drag hovers.
#[tauri::command]
#[specta::specta]
pub fn get_key_modifiers() -> KeyModifiers {
    system_files::get_key_modifiers()
}

/// Starts a native drag carrying the given files out of the window, e.g. into
/// Explorer, a chat app, or a browser upload field. `mode` decides the effect
/// the target performs: `copy` duplicates the files, `move` relocates them,
/// `link` creates shortcuts. Returns immediately; the drag runs on the main

/// Creates links pointing at `sources` inside `destination`: `.lnk`
/// shortcuts on Windows (IShellLink), real symlinks on macOS and Linux.
/// Resolves name collisions with ` (2)`, ` (3)`… suffixes like Explorer.
/// Returns the created link paths so the UI can refresh and select them.
#[tauri::command]
#[specta::specta]
pub async fn create_shortcuts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<String>, FileSystemError> {
    system_files::create_shortcuts(sources, destination).await
}

/// Compresses entries into an archive of the requested format inside
/// `destination_dir` and returns the archive path. Sources may live on any
/// backend (streamed through the VFS primitives); the archive itself is
/// always a local file.
#[tauri::command]
#[specta::specta]
pub async fn compress_entries(
    sources: Vec<String>,
    destination_dir: String,
    format: ArchiveFormat,
    password: Option<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    archive::compress_entries(sources, destination_dir, format, password, operation_id, host::of(&app)).await
}

/// Extracts a local archive. Without `destination_dir` the entries land in a
/// fresh folder named after the archive stem next to the archive; the folder
/// path is returned.
#[tauri::command]
#[specta::specta]
pub async fn extract_archive(
    archive_path: String,
    destination_dir: Option<String>,
    password: Option<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    archive::extract_archive(archive_path, destination_dir, password, operation_id, host::of(&app)).await
}


/// Lists every entry currently in the system trash, newest deletion first.
///
/// Either implementation walks the whole bin, so this runs on a blocking
/// thread; the frontend only calls it when opening the trash view or after
/// a trash operation.
#[tauri::command]
#[specta::specta]
pub async fn list_trash() -> Result<Vec<TrashEntry>, FileSystemError> {
    trash_view::list_trash().await
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
    trash_view::restore_trash_entries(ids, operation_id, host::of(&app)).await
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
    trash_view::delete_trash_entries(ids, operation_id, host::of(&app)).await
}

/// Permanently deletes everything in the trash. Returns the purged count;
/// the UI must confirm before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn empty_trash(
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<u64, FileSystemError> {
    trash_view::empty_trash(operation_id, host::of(&app)).await
}

/// Lists saved accounts sorted by provider then email.
#[tauri::command]
#[specta::specta]
pub fn list_cloud_accounts() -> Result<Vec<StoredCloudAccount>, FileSystemError> {
    super::cloud::accounts::list_cloud_accounts()
}

/// Removes a saved account, its stored credentials, and any live backend.
#[tauri::command]
#[specta::specta]
pub fn delete_cloud_account(
    id: String,
) -> Result<(), FileSystemError> {
    super::cloud::accounts::delete_cloud_account(id)
}

/// Runs the full browser authorization for one provider and saves the
/// resulting account. Re-authorizing an existing account replaces it.
#[tauri::command]
#[specta::specta]
pub async fn authorize_cloud_account(
    input: AuthorizeCloudAccountInput,
) -> Result<StoredCloudAccount, FileSystemError> {
    super::cloud::oauth::authorize_cloud_account(input).await
}

/// Lists saved connections sorted by protocol then host.
#[tauri::command]
#[specta::specta]
pub fn list_connections() -> Result<Vec<StoredConnection>, FileSystemError> {
    super::connections::list_connections()
}

/// Creates or updates a connection. Passwords, when supplied and remembered,
/// go to the OS keychain; if the keychain is unavailable they fall back to
/// session memory so the connection still works until the app exits.
#[tauri::command]
#[specta::specta]
pub fn save_connection(
    input: SaveConnectionInput,
) -> Result<StoredConnection, FileSystemError> {
    super::connections::save_connection(input)
}

/// Removes a saved connection, its stored credential, and any live session.
#[tauri::command]
#[specta::specta]
pub fn delete_connection(
    id: String,
) -> Result<(), FileSystemError> {
    super::connections::delete_connection(id)
}

/// Starts a background size scan for the given local directories. Partial
/// totals stream as [`DirectorySizeProgress`] events until each directory
/// finishes with `completed: true`. Non-local paths and non-directories are
/// skipped quietly; the caller cancels the operation when the view changes.
#[tauri::command]
#[specta::specta]
pub fn start_directory_size_calculation(
    operation_id: String,
    paths: Vec<String>,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    super::directory_size::start_directory_size_calculation(operation_id, paths, host::of(&app))
}

/// Stops an in-flight size scan when the view navigates away.
#[tauri::command]
#[specta::specta]
pub fn cancel_directory_size_calculation(
    operation_id: String,
)  {
    super::directory_size::cancel_directory_size_calculation(operation_id);
}

/// Starts a background checksum run for one local file. Partial byte counts
/// stream as [`FileHashProgress`] events; the final event carries either the
/// three digests or an error message. Non-local paths and non-files are
/// rejected up front so the frontend gets an immediate failure.
#[tauri::command]
#[specta::specta]
pub fn start_file_hash_calculation(
    operation_id: String,
    path: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    super::hashing::start_file_hash_calculation(operation_id, path, host::of(&app))
}

/// Stops an in-flight checksum run when the dialog closes.
#[tauri::command]
#[specta::specta]
pub fn cancel_file_hash_calculation(
    operation_id: String,
)  {
    super::hashing::cancel_file_hash_calculation(operation_id);
}

/// Stops an in-flight listing when the view navigates away. Ids the backend no
/// longer knows are ignored: the listing may already have finished.
#[tauri::command]
#[specta::specta]
pub fn cancel_directory_listing(
    stream_id: String,
)  {
    super::listing::cancel_directory_listing(stream_id);
}

#[tauri::command]
#[specta::specta]
pub async fn read_media_preview(
    path: String,
) -> Result<MediaPreview, FileSystemError> {
    super::media_meta::read_media_preview(path).await
}

/// Loads the recent items list, most recently used first. Empty on first launch.
#[tauri::command]
#[specta::specta]
pub fn list_recents() -> Result<Vec<RecentItem>, FileSystemError> {
    super::recents::list_recents()
}

/// Records one access, moving an existing entry for the same path to the front.
/// Returns the updated list so callers can sync their local state.
///
/// Blocking file I/O on the blocking pool rather than on the invoke thread: this
/// runs on every directory visit and every file open, and the write is a
/// temp-file-and-rename under the app config directory, which an antivirus
/// scanner or a roamed profile can turn into milliseconds.
#[tauri::command]
#[specta::specta]
pub async fn record_recent(
    path: String,
    kind: EntryKind,
    source: RecentSource,
) -> Result<Vec<RecentItem>, FileSystemError> {
    super::recents::record_recent(path, kind, source).await
}

/// Removes one path from the recent list, returning the updated list.
#[tauri::command]
#[specta::specta]
pub async fn remove_recent(
    path: String,
) -> Result<Vec<RecentItem>, FileSystemError> {
    super::recents::remove_recent(path).await
}

/// Clears the whole recent list. This never touches the files themselves.
#[tauri::command]
#[specta::specta]
pub async fn clear_recents() -> Result<(), FileSystemError> {
    super::recents::clear_recents().await
}

/// Lists local disk volumes with capacity information for the sidebar's disks
/// section. Called lazily — only when the user expands the section.
#[tauri::command]
#[specta::specta]
pub async fn list_disks() -> Result<Vec<DiskVolume>, FileSystemError> {
    super::sidebar::list_disks().await
}

/// Lists installed WSL distributions for the sidebar's WSL section.
///
/// The `\\wsl$` share root only exposes running distros, so discovery goes
/// through `wsl.exe` instead; each distro stays reachable at `\\wsl$\<name>`
/// because accessing that path auto-starts a stopped distro.
///
/// Called lazily — only when the user expands the section — since the probe
/// spawns `wsl.exe` and must not run on the cold-start path.
#[tauri::command]
#[specta::specta]
pub async fn list_wsl_distros() -> Result<Vec<WslDistro>, FileSystemError> {
    super::sidebar::list_wsl_distros().await
}

/// Loads the user's favorite folders, returning an empty list on first launch.
#[tauri::command]
#[specta::specta]
pub fn load_favorites() -> Result<Vec<Favorite>, FileSystemError> {
    super::sidebar::load_favorites()
}

/// Persists the full favorite list, deduplicating entries that share a path.
#[tauri::command]
#[specta::specta]
pub fn save_favorites(
    favorites: Vec<Favorite>,
) -> Result<(), FileSystemError> {
    super::sidebar::save_favorites(favorites)
}

/// Verifies a connection attempt before it is saved, using the credentials
/// typed into the connect dialog rather than any stored ones.
#[tauri::command]
#[specta::specta]
pub async fn test_connection(
    input: SaveConnectionInput,
) -> Result<(), FileSystemError> {
    super::smb::test_connection(input).await
}

/// Lists all spaces, seeding the preset set (Work/Personal/Shared/Archive)
/// on first launch.
#[tauri::command]
#[specta::specta]
pub fn list_spaces() -> Result<Vec<Space>, FileSystemError> {
    super::spaces::list_spaces()
}

/// Creates a new empty space and returns it.
#[tauri::command]
#[specta::specta]
pub fn create_space(
    name: String,
) -> Result<Space, FileSystemError> {
    super::spaces::create_space(name)
}

/// Renames a space. Preset spaces may be renamed but not deleted.
#[tauri::command]
#[specta::specta]
pub fn rename_space(
    space_id: String,
    name: String,
) -> Result<Space, FileSystemError> {
    super::spaces::rename_space(space_id, name)
}

/// Deletes a space. The items inside it are pointers, so no files are removed.
/// Preset spaces cannot be deleted.
#[tauri::command]
#[specta::specta]
pub fn delete_space(
    space_id: String,
) -> Result<(), FileSystemError> {
    super::spaces::delete_space(space_id)
}

/// Pins a path inside a space, returning the updated space. Adding the same
/// path twice keeps the original entry.
#[tauri::command]
#[specta::specta]
pub fn add_space_item(
    space_id: String,
    path: String,
) -> Result<Space, FileSystemError> {
    super::spaces::add_space_item(space_id, path)
}

/// Removes one path from a space, returning the updated space.
#[tauri::command]
#[specta::specta]
pub fn remove_space_item(
    space_id: String,
    path: String,
) -> Result<Space, FileSystemError> {
    super::spaces::remove_space_item(space_id, path)
}

/// Stops the observation a view held. Called when the view goes away — its
/// pane unmounted, its tab closed — and must be safe to call more than once.
#[tauri::command]
#[specta::specta]
pub fn unwatch_directory(
    watcher_id: String,
)  {
    super::watch::unwatch_directory(watcher_id);
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
