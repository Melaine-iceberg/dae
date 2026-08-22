use super::error::FileSystemError;
use super::local;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::transfer::{self, TransferSource};
use super::types::{
    ConflictAction, ContentSearchResponse, DirectoryView, FileProperties, NewEntryKind,
    PropertyChanges, SearchResponse, TransferConflict, TransferItem, TransferPair,
    path_to_string,
};
use super::undo::{self, Operation, TrashRecord, UndoRedoOutcome, UndoRedoState};
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
            "fs.content_search_local_only".into(),
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
pub async fn rename_entry(
    path: String,
    new_name: String,
    app: tauri::AppHandle,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&path)?.rename_entry(&path, &new_name)?;
        if let Some(to) = undo::renamed_path(&path, &new_name) {
            if to != path {
                app.state::<UndoRedoState>().record(
                    &app,
                    Operation::Rename {
                        from: path,
                        to,
                    },
                );
            }
        }
        Ok(())
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
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let created = vfs::resolve(&directory)?.create_entry(&directory, &name, kind)?;
        app.state::<UndoRedoState>().record(
            &app,
            Operation::Create {
                path: created.clone(),
                kind,
            },
        );
        Ok(created)
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
            FileOperationProgressReporter::new(app.clone(), operation_id, FileOperationKind::Copy);

        // The journal records every entry that actually landed on disk, so a
        // partially failed copy still gets an undo record for its successes.
        let mut journal: Vec<TransferPair> = Vec::new();
        let result = if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                .collect::<Vec<_>>();
            local::copy_entries_with_progress(
                paths,
                PathBuf::from(&destination),
                &progress,
                &mut journal,
            )
        } else {
            transfer::copy_entries(
                sources,
                &destination,
                &destination_backend,
                &progress,
                &mut journal,
            )
        };

        let (recorded_sources, created): (Vec<String>, Vec<String>) = journal
            .into_iter()
            .map(|pair| (pair.source, pair.destination))
            .unzip();
        app.state::<UndoRedoState>().record(
            &app,
            Operation::Copy {
                sources: recorded_sources,
                destination,
                created,
            },
        );

        result
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
            FileOperationProgressReporter::new(app.clone(), operation_id, FileOperationKind::Move);

        // The journal records every entry that actually moved, so a partially
        // failed move still gets an undo record for its successes.
        let mut journal: Vec<TransferPair> = Vec::new();
        let result = if is_pure_local(&sources, &destination) {
            let paths = sources
                .iter()
                .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                .collect::<Vec<_>>();
            local::move_entries_with_progress(
                paths,
                PathBuf::from(&destination),
                &progress,
                &mut journal,
            )
        } else {
            transfer::move_entries(
                sources,
                &destination,
                &destination_backend,
                &progress,
                &mut journal,
            )
        };

        app.state::<UndoRedoState>().record(
            &app,
            Operation::Move {
                transfers: journal,
            },
        );

        result
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
    if paths.is_empty() {
        return Ok(());
    }
    if paths.iter().any(|path| !is_local_path(path)) {
        return Err(FileSystemError::InvalidInput("fs.trash_local_only".into()));
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
                            parent: path_to_string(parent),
                            name: name.to_string_lossy().into_owned(),
                        });
                    }
                    progress.advance(entry_path);
                }
                Err(error) => {
                    first_error = Some(FileSystemError::Internal(error.to_string()));
                    break;
                }
            }
        }

        // Even when a later entry fails, the ones that did reach the trash
        // stay undoable.
        app.state::<UndoRedoState>().record(
            &app,
            Operation::Trash { records },
        );
        progress.finish();

        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
    let kind = app
        .state::<UndoRedoState>()
        .peek_undo()
        .map(|operation| operation.undo_kind())
        .unwrap_or(FileOperationKind::Move);
    emit_preparing(&app, &operation_id, kind);

    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UndoRedoState>();
        let Some(operation) = state.pop_undo() else {
            return Err(FileSystemError::InvalidInput("fs.undo_empty".into()));
        };

        let op = operation.op_code();
        let progress = FileOperationProgressReporter::new(app.clone(), operation_id, kind);
        let result = undo::execute_undo(operation, &progress);

        match result {
            Ok((count, redo_operation)) => {
                if let Some(redo_operation) = redo_operation {
                    state.push_redo(redo_operation);
                }
                undo::emit_changed(&app, &state);
                progress.finish();
                Ok(UndoRedoOutcome {
                    action: "undo",
                    op,
                    count,
                })
            }
            Err(error) => {
                // The entry stays consumed; report what blocked the undo.
                undo::emit_changed(&app, &state);
                Err(error)
            }
        }
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Re-applies the most recent undone operation and returns toast text
/// describing what was restored.
#[tauri::command]
#[specta::specta]
pub async fn redo_operation(
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<UndoRedoOutcome, FileSystemError> {
    let kind = app
        .state::<UndoRedoState>()
        .peek_redo()
        .map(|operation| operation.redo_kind())
        .unwrap_or(FileOperationKind::Move);
    emit_preparing(&app, &operation_id, kind);

    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UndoRedoState>();
        let Some(operation) = state.pop_redo() else {
            return Err(FileSystemError::InvalidInput("fs.redo_empty".into()));
        };

        let op = operation.op_code();
        let progress = FileOperationProgressReporter::new(app.clone(), operation_id, kind);
        let result = undo::execute_redo(operation, &progress);

        match result {
            Ok((count, undo_operation)) => {
                if let Some(undo_operation) = undo_operation {
                    state.push_undo(undo_operation);
                }
                undo::emit_changed(&app, &state);
                progress.finish();
                Ok(UndoRedoOutcome {
                    action: "redo",
                    op,
                    count,
                })
            }
            Err(error) => {
                undo::emit_changed(&app, &state);
                Err(error)
            }
        }
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
        let sources = resolve_sources(paths.clone())?;
        let progress =
            FileOperationProgressReporter::new(app.clone(), operation_id, FileOperationKind::Copy);
        let created = transfer::duplicate_sources(sources, &progress)?;

        app.state::<UndoRedoState>().record(
            &app,
            Operation::Duplicate {
                sources: paths,
                created: created.clone(),
            },
        );
        Ok(created)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn is_local_path(path: &str) -> bool {
    vfs::scheme_of(path).is_ok_and(|scheme| scheme == Scheme::Local)
}

/// Pushes the locale's duplicate-name token (e.g. "副本" / "copy") so the
/// backend names Keep-Both conflicts and duplicates in the UI language.
/// Called by the frontend at startup and on language changes.
#[tauri::command]
#[specta::specta]
pub fn set_duplicate_suffix(suffix: String) {
    super::transfer::set_duplicate_suffix(&suffix);
}

/// Reads one entry's full metadata for the properties dialog. Local paths
/// return the platform permission model (POSIX mode bits and ownership, or
/// Windows DOS attributes); other backends degrade to a view-only summary.
#[tauri::command]
#[specta::specta]
pub async fn get_file_properties(path: String) -> Result<FileProperties, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || vfs::resolve(&path)?.properties(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Applies property edits (permissions, ownership, attributes). Fields left
/// unset keep their current value.
#[tauri::command]
#[specta::specta]
pub async fn update_file_properties(
    path: String,
    changes: PropertyChanges,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        vfs::resolve(&path)?.update_properties(&path, &changes)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
            "fs.terminal_local_dir_only".into(),
        ));
    }

    let directory = PathBuf::from(&path);
    if !directory.is_dir() {
        return Err(FileSystemError::NotDirectory(path));
    }

    open_system_terminal(&directory)
}

/// Opens the system's native "Open With" picker for a local file.
///
/// Windows shows the shell's "How do you want to open this file?" dialog,
/// letting the user pick any installed application (or set a new default);
/// the command resolves once the dialog is dismissed. macOS and Linux expose
/// no native picker, so the command fails there.
#[tauri::command]
#[specta::specta]
pub async fn open_with(path: String) -> Result<(), FileSystemError> {
    if !is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_local_only".into(),
        ));
    }

    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err(FileSystemError::InvalidInput("fs.open_with_file_only".into()));
    }

    // SHOpenWithDialog 以模态方式运行到用户关闭为止，放到阻塞线程池执行。
    tauri::async_runtime::spawn_blocking(move || open_system_with_dialog(&file))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
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
        .map_err(|error| FileSystemError::Internal(format!("fs.terminal_launch_failed: {error}")))
}

#[cfg(target_os = "macos")]
fn open_system_terminal(directory: &std::path::Path) -> Result<(), FileSystemError> {
    std::process::Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| FileSystemError::Internal(format!("fs.terminal_launch_failed: {error}")))
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
        "fs.terminal_not_found".into(),
    ))
}

#[cfg(target_os = "windows")]
fn open_system_with_dialog(file: &Path) -> Result<(), FileSystemError> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPENASINFO, OPEN_AS_INFO_FLAGS,
        SHOpenWithDialog,
    };

    /// RAII COM 套间；S_FALSE (0x1) 表示线程已有套间，此时不能反初始化。
    struct CoApartment {
        owned: bool,
    }

    impl CoApartment {
        fn enter() -> Result<Self, FileSystemError> {
            unsafe {
                match CoInitializeEx(Some(std::ptr::null()), COINIT_APARTMENTTHREADED) {
                    Ok(()) => Ok(Self { owned: true }),
                    Err(error) if error.code().0 == 1 => Ok(Self { owned: false }),
                    Err(error) => Err(FileSystemError::Internal(error.to_string())),
                }
            }
        }
    }

    impl Drop for CoApartment {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    let _apartment = CoApartment::enter()?;

    // 路径以 UTF-16 直传。不走 rundll32 的 OpenAs_RunDLLW——那条路径会经过
    // ANSI 代码页转换，非 ASCII 文件名在对话框里会显示成乱码。
    let wide_path: Vec<u16> = file
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let info = OPENASINFO {
        pcszFile: PCWSTR::from_raw(wide_path.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OPEN_AS_INFO_FLAGS(
            OAIF_ALLOW_REGISTRATION.0 | OAIF_REGISTER_EXT.0 | OAIF_EXEC.0,
        ),
    };

    // 模态对话框：确认后立即用所选程序打开文件；取消返回 S_FALSE，视为成功。
    unsafe { SHOpenWithDialog(None, &info) }
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

#[cfg(not(target_os = "windows"))]
fn open_system_with_dialog(_file: &Path) -> Result<(), FileSystemError> {
    Err(FileSystemError::Internal(
        "fs.open_with_unsupported".into(),
    ))
}

/// True when every source and the destination sit on the local backend, which
/// keeps its native rayon transfer path instead of the streaming engine.
fn is_pure_local(sources: &[TransferSource], destination: &str) -> bool {
    is_local_path(destination) && sources.iter().all(|source| is_local_path(&source.path))
}
