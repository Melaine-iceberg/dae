use super::error::FileSystemError;
use super::local;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::transfer::{self, TransferSource};
use super::types::{
    ConflictAction, ContentSearchResponse, DirectoryView, NewEntryKind, SearchResponse,
    TransferConflict, TransferItem, path_to_string,
};
use super::vfs::{self, Scheme};
use super::watch::{DirectoryWatcher, WatchHandle, spawn_polling_watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::Manager;

#[derive(Default)]
pub struct FileSearchState {
    generation: AtomicU64,
}

impl FileSearchState {
    fn begin(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(AtomicOrdering::Acquire) == generation
    }
}

/// Returns the operating system's home directory for the initial explorer view.
#[tauri::command]
#[specta::specta]
pub fn get_home_directory(app: tauri::AppHandle) -> Result<String, FileSystemError> {
    app.path()
        .home_dir()
        .map(|path| path_to_string(&path))
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

/// Reads one directory as an immutable snapshot suitable for rendering in the explorer.
///
/// `vfs::resolve` can open a network session (a blocking, runtime-owning
/// operation), so it must run on a blocking thread — never on the async
/// workers, where a nested runtime panics.
#[tauri::command]
#[specta::specta]
pub async fn read_directory(path: String) -> Result<DirectoryView, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || vfs::resolve(&path)?.read_dir(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Replaces the active watcher with one that tracks the currently displayed directory.
#[tauri::command]
#[specta::specta]
pub async fn watch_directory(path: String, app: tauri::AppHandle) -> Result<(), FileSystemError> {
    let generation = app.state::<DirectoryWatcher>().begin_update();

    if vfs::split_scheme(&path)?.0 == Scheme::Local {
        let watcher_app = app.clone();
        let watcher = tauri::async_runtime::spawn_blocking(move || {
            local::create_directory_watcher(PathBuf::from(path), watcher_app)
        })
            .await
            .map_err(|error| FileSystemError::Internal(error.to_string()))??;

        app.state::<DirectoryWatcher>()
            .replace(generation, WatchHandle::Notify(watcher))
    } else {
        let watch_path = path.clone();
        let backend = tauri::async_runtime::spawn_blocking(move || vfs::resolve(&watch_path))
            .await
            .map_err(|error| FileSystemError::Internal(error.to_string()))??;

        let handle = spawn_polling_watcher(path, backend, app.clone());
        app.state::<DirectoryWatcher>().replace(generation, handle)
    }
}

/// Recursively searches entry names beneath one directory. A newer request
/// cancels any older traversal.
#[tauri::command]
#[specta::specta]
pub async fn search_directory(
    path: String,
    query: String,
    app: tauri::AppHandle,
) -> Result<SearchResponse, FileSystemError> {
    let generation = app.state::<FileSearchState>().begin();
    let search_app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let backend = vfs::resolve(&path)?;
        let state = search_app.state::<FileSearchState>();
        let is_current = || state.is_current(generation);
        backend.search(&path, &query, &is_current)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Stops the active traversal when the search surface is dismissed.
#[tauri::command]
#[specta::specta]
pub fn cancel_search(app: tauri::AppHandle) {
    app.state::<FileSearchState>().cancel();
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
    app: tauri::AppHandle,
) -> Result<ContentSearchResponse, FileSystemError> {
    if !is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "内容搜索仅支持本地目录".into(),
        ));
    }

    let generation = app.state::<FileSearchState>().begin();
    let search_app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let state = search_app.state::<FileSearchState>();
        let is_current = move || state.is_current(generation);
        local::search_file_contents_sync(
            PathBuf::from(path),
            &local::ContentSearchParams {
                query: &query,
                is_regex,
                case_sensitive,
                file_filter: file_filter.as_deref(),
            },
            &is_current,
        )
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Renames a single directory entry without allowing a path change.
#[tauri::command]
#[specta::specta]
pub async fn rename_entry(path: String, new_name: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&path)?.rename_entry(&path, &new_name)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Creates a new file or directory inside an existing directory and returns its path.
#[tauri::command]
#[specta::specta]
pub async fn create_entry(
    directory: String,
    name: String,
    kind: NewEntryKind,
) -> Result<String, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&directory)?.create_entry(&directory, &name, kind)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
    emit_preparing(&app, &operation_id, FileOperationKind::Copy);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_transfer_items(items)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Copy);

        if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                .collect::<Vec<_>>();
            local::copy_entries_with_progress(paths, PathBuf::from(&destination), &progress)
        } else {
            transfer::copy_entries(sources, &destination, &destination_backend, &progress)
        }
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
    emit_preparing(&app, &operation_id, FileOperationKind::Move);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_transfer_items(items)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Move);

        if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                .collect::<Vec<_>>();
            local::move_entries_with_progress(paths, PathBuf::from(&destination), &progress)
        } else {
            transfer::move_entries(sources, &destination, &destination_backend, &progress)
        }
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_sources(sources)?;
        let destination_backend = vfs::resolve(&destination)?;
        transfer::find_conflicts(sources, &destination, &destination_backend)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Permanently deletes entries. The UI must obtain confirmation before calling this command.
#[tauri::command]
#[specta::specta]
pub async fn delete_entries(
    paths: Vec<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Ok(());
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let targets = resolve_sources(paths)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Delete);

        if targets.iter().all(|target| is_local_path(&target.path)) {
            let paths = targets
                .iter()
                .map(|target| PathBuf::from(&target.path))
                .collect::<Vec<_>>();
            local::delete_entries_with_progress(paths, &progress)
        } else {
            transfer::delete_entries(targets, &progress)
        }
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn resolve_sources(paths: Vec<String>) -> Result<Vec<TransferSource>, FileSystemError> {
    paths
        .into_iter()
        .map(|path| {
            let backend = vfs::resolve(&path)?;
            Ok(TransferSource {
                path,
                backend,
                on_conflict: ConflictAction::Fail,
            })
        })
        .collect()
}

fn resolve_transfer_items(
    items: Vec<TransferItem>,
) -> Result<Vec<TransferSource>, FileSystemError> {
    items
        .into_iter()
        .map(|item| {
            let backend = vfs::resolve(&item.path)?;
            Ok(TransferSource {
                path: item.path,
                backend,
                on_conflict: item.on_conflict,
            })
        })
        .collect()
}

/// One entry remembered from a move-to-trash run: where it lived and what it
/// was called. Kept lightweight so deleting never has to enumerate the trash;
/// [`undo_trash`] resolves these back to `TrashItem`s only when restoring.
struct TrashRecord {
    parent: PathBuf,
    name: std::ffi::OsString,
}

/// Tracks the most recent batch of entries moved to the system trash so that
/// [`undo_trash`] can restore it. A newer batch replaces the previous one.
#[derive(Default)]
pub struct TrashUndoState {
    records: std::sync::Mutex<Vec<TrashRecord>>,
}

/// Moves local entries into the system trash (recycle bin) instead of deleting
/// them permanently. The most recent batch stays undoable via [`undo_trash`].
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
    if paths.is_empty() {
        return Ok(());
    }
    if paths.iter().any(|path| !is_local_path(path)) {
        return Err(FileSystemError::InvalidInput(
            "回收站仅支持本地路径".into(),
        ));
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Delete);

    tauri::async_runtime::spawn_blocking(move || {
        let progress = FileOperationProgressReporter::new(
            app.clone(),
            operation_id,
            FileOperationKind::Delete,
        );
        progress.start(paths.len() as u64);

        let mut records = Vec::with_capacity(paths.len());
        let mut first_error: Option<FileSystemError> = None;
        for path in &paths {
            match trash::delete(path) {
                Ok(()) => {
                    let entry_path = Path::new(path);
                    if let (Some(parent), Some(name)) =
                        (entry_path.parent(), entry_path.file_name())
                    {
                        records.push(TrashRecord {
                            parent: parent.to_path_buf(),
                            name: name.to_os_string(),
                        });
                    }
                    progress.advance(entry_path);
                }
                Err(error) => {
                    first_error = Some(trash_error(error));
                    break;
                }
            }
        }

        // Even when a later entry fails, the ones that did reach the trash
        // stay undoable.
        *app.state::<TrashUndoState>()
            .records
            .lock()
            .expect("trash undo state lock poisoned") = records;
        progress.finish();

        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Restores the most recent [`trash_entries`] batch to its original locations,
/// returning the restored paths so the UI can refresh. Entries no longer in
/// the trash (emptied or restored elsewhere) are skipped.
#[tauri::command]
#[specta::specta]
pub async fn undo_trash(app: tauri::AppHandle) -> Result<Vec<String>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let records = std::mem::take(
            &mut *app
                .state::<TrashUndoState>()
                .records
                .lock()
                .expect("trash undo state lock poisoned"),
        );
        if records.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "没有可撤销的删除操作".into(),
            ));
        }

        let items = trash::os_limited::list().map_err(trash_error)?;
        let mut to_restore = Vec::new();
        let mut restored_paths = Vec::new();
        for record in &records {
            // The same (parent, name) can appear multiple times in the trash
            // from earlier deletions; the newest one is ours.
            let match_item = items.iter().filter(|item| {
                item.name == record.name && same_trash_location(&item.original_parent, &record.parent)
            }).max_by_key(|item| item.time_deleted);

            if let Some(item) = match_item {
                restored_paths.push(path_to_string(&item.original_parent.join(&item.name)));
                to_restore.push(item.clone());
            }
        }

        if to_restore.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "回收站中已找不到这些项目，可能已被清空或还原".into(),
            ));
        }

        trash::os_limited::restore_all(to_restore).map_err(trash_error)?;
        Ok(restored_paths)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Windows paths are case-insensitive, so recycle-bin parents recorded from a
/// deletion must compare that way too.
fn same_trash_location(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn trash_error(error: trash::Error) -> FileSystemError {
    FileSystemError::Internal(error.to_string())
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
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before duplicating".into(),
        ));
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Copy);

    tauri::async_runtime::spawn_blocking(move || {
        let sources = resolve_sources(paths)?;
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Copy);
        transfer::duplicate_sources(sources, &progress)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn is_local_path(path: &str) -> bool {
    vfs::scheme_of(path).is_ok_and(|scheme| scheme == Scheme::Local)
}

/// Opens the user's system default terminal at a local directory.
///
/// Each platform honors its own "default terminal" setting: Windows routes
/// the spawned console process to the configured default terminal host
/// (e.g. Windows Terminal), macOS opens Terminal.app, and Linux resolves
/// `$TERMINAL`, `xdg-terminal-exec`, or a common desktop terminal.
#[tauri::command]
#[specta::specta]
pub fn open_terminal(path: String) -> Result<(), FileSystemError> {
    if !is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "只能在本地目录打开终端".into(),
        ));
    }

    let directory = PathBuf::from(&path);
    if !directory.is_dir() {
        return Err(FileSystemError::NotDirectory(path));
    }

    open_system_terminal(&directory)
}

#[cfg(target_os = "windows")]
fn open_system_terminal(directory: &Path) -> Result<(), FileSystemError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    // Prefer Windows Terminal: `wt -d` opens the user's configured default
    // profile (their chosen shell) at the directory.
    if std::process::Command::new("wt.exe")
        .arg("-d")
        .arg(directory)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // Fallback for systems without Windows Terminal: launch PowerShell in a
    // new console, which Windows routes to the configured default terminal
    // host (e.g. Windows Terminal or conhost).
    std::process::Command::new("powershell.exe")
        .current_dir(directory)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
        .map_err(|error| FileSystemError::Internal(format!("无法启动终端：{error}")))
}

#[cfg(target_os = "macos")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    std::process::Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| FileSystemError::Internal(format!("无法启动终端：{error}")))
}

#[cfg(target_os = "linux")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    use std::process::Command;

    fn spawn(
        program: &str,
        extra_args: &[&str],
        directory: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        let mut command = Command::new(program);
        command.args(extra_args).current_dir(directory).spawn()?;
        Ok(())
    }

    // Honor an explicit user override first.
    if let Ok(terminal) = std::env::var("TERMINAL") {
        let terminal = terminal.trim();
        if !terminal.is_empty() && spawn(terminal, &[], directory).is_ok() {
            return Ok(());
        }
    }

    // Candidates: (program, args). Terminals without a working-directory
    // flag inherit `directory` through `current_dir`.
    let dir = directory.to_string_lossy().into_owned();
    let candidates: Vec<(String, Vec<String>)> = vec![
        // The cross-desktop default-terminal spec.
        ("xdg-terminal-exec".into(), vec![]),
        // Debian's alternatives symlink.
        ("x-terminal-emulator".into(), vec![]),
        (
            "gnome-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "kgx".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        ("konsole".into(), vec!["--workdir".into(), dir.clone()]),
        (
            "xfce4-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "mate-terminal".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "tilix".into(),
            vec!["--working-directory".into(), dir.clone()],
        ),
        (
            "wezterm".into(),
            vec!["start".into(), "--cwd".into(), dir.clone()],
        ),
        ("kitty".into(), vec![]),
        ("alacritty".into(), vec![]),
        ("foot".into(), vec![]),
        ("xterm".into(), vec![]),
    ];

    for (program, args) in candidates {
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        if spawn(&program, &arg_refs, directory).is_ok() {
            return Ok(());
        }
    }

    Err(FileSystemError::Internal(
        "未找到可用的终端，请安装 xdg-terminal-exec 或设置 $TERMINAL".into(),
    ))
}

/// True when every source and the destination sit on the local backend, which
/// keeps its native rayon transfer path instead of the streaming engine.
fn is_pure_local(sources: &[TransferSource], destination: &str) -> bool {
    is_local_path(destination) && sources.iter().all(|source| is_local_path(&source.path))
}
