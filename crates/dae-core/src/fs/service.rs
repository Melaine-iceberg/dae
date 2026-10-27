use super::error::FileSystemError;
use super::listing;
use super::local;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::transfer::{self, TransferSource};
use super::types::{
    ConflictAction, DirectoryEntry, DirectoryView, FileProperties, NewEntryKind, PropertyChanges,
    RecursivePropertyUpdateOutcome, RenameRequest, TransferConflict, TransferItem, TransferPair,
    path_to_string,
};
use super::undo::{self, Operation, TrashRecord, UndoRedoOutcome};
use super::vfs::{self, SharedBackend};
use std::collections::HashSet;
use crate::fs::listing::{BatchTarget, EventsTarget};
use crate::host::Host;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// See `commands::get_home_directory` for the contract; this half is engine-free.
pub fn get_home_directory() -> Result<String, FileSystemError> {
    home_dir()
        .map(|path| path_to_string(&path))
        .ok_or_else(|| FileSystemError::Internal("the environment carries no home directory".into()))
}

/// The OS home directory.
///
/// Windows answers with `USERPROFILE`, falling back to `HOMEDRIVE` + `HOMEPATH`
/// (the pair a domain login sets); everything else uses `HOME`. This is the one
/// environment read the shell used to do through `tauri::Manager::path`, and it
/// is deliberately *not* `xdg::home_dir`: that one defaults to `/` and exists to
/// answer where XDG data lives, not where the user keeps files.
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from).or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(drive).join(path))
        })
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// See `commands::read_directory` for the contract; this half is engine-free.
pub async fn read_directory(
    path: String,
    stream_id: String,
    watcher_id: Option<String>,
    host: Arc<dyn Host>,
) -> Result<DirectoryView, FileSystemError> {
    read_directory_into(
        path,
        stream_id,
        watcher_id,
        Arc::clone(&host),
        Arc::new(EventsTarget::new(host)),
    )
    .await
}

/// See `commands::read_directory_changes` for the contract; this half is engine-free.
pub async fn read_directory_changes(
    directory: String,
    names: Vec<String>,
) -> Result<Vec<Option<DirectoryEntry>>, FileSystemError> {
    // Remote backends have no cheap per-entry read and their watcher reports
    // "something differs" rather than which, so a patch request for one is a bug
    // rather than a slow path. Re-read is what the caller does with an error.
    if !vfs::is_local_path(&directory) {
        return Err(FileSystemError::Internal(format!(
            "{directory} is not a local directory"
        )));
    }

    let path = PathBuf::from(&directory);
    tokio::task::spawn_blocking(move || local::stat_named_children(&path, names))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// The read both directory commands perform; they differ only in where the
/// streamed batches are delivered.
pub async fn read_directory_into(
    path: String,
    stream_id: String,
    watcher_id: Option<String>,
    host: Arc<dyn Host>,
    target: Arc<dyn BatchTarget>,
) -> Result<DirectoryView, FileSystemError> {
    // 这里原先会消费 `prefetch::warm_startup_data` 预读的目录快照，但那条路径
    // 从来没有生效过：唯一会被预读的是 home 目录，而它只由 `initialize()` 读取，
    // 且带着 watcher（`navigation.ts`）——带 watcher 的读**故意不接受**快照
    // （快照在窗口存在之前就读好了，可能已经陈旧，中间又没有 watcher 兜底）。
    // 于是那次预读只是和真正的首读抢磁盘与分配器，现已删掉（连同这条分支）。
    // 若将来要让首屏用上预热，正确做法是让快照能被接受**并**在它之上挂 watcher。

    // Remote backends enumerate through their own protocol APIs and have no
    // iterator to hand over, so they keep answering in one piece. Their
    // watcher polls, and is armed before the read for the same reason the
    // local one is (see [`listing::open_streamed_listing`]).
    if !vfs::is_local_path(&path) {
        let watch_host = Arc::clone(&host);
        return tokio::task::spawn_blocking(move || {
            let backend = vfs::resolve(&path)?;

            if let Some(watcher_id) = &watcher_id {
                super::watch::arm_polling_watcher(
                    watch_host,
                    &path,
                    SharedBackend::clone(&backend),
                    watcher_id,
                );
            }

            backend.read_dir(&path)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    }

    let listing_host = Arc::clone(&host);
    let listing_path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || {
        listing::open_streamed_listing(
            listing_host,
            listing_path,
            stream_id,
            watcher_id,
            target,
        )
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::rename_entry` for the contract; this half is engine-free.
pub async fn rename_entry(
    path: String,
    new_name: String,
    host: Arc<dyn Host>,
) -> Result<(), FileSystemError> {
    tokio::task::spawn_blocking(move || {
        vfs::resolve(&path)?.rename_entry(&path, &new_name)?;
        if let Some(to) = undo::renamed_path(&path, &new_name)
            && to != path
        {
            undo::state()
                .record(host.as_ref(), Operation::Rename { from: path, to });
        }
        Ok(())
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::rename_entries_batch` for the contract; this half is engine-free.
pub async fn rename_entries_batch(
    requests: Vec<RenameRequest>,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<Vec<String>, FileSystemError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Move);

    tokio::task::spawn_blocking(move || {
        // Derive full destination paths and drop no-ops before touching disk.
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(requests.len());
        let mut seen_sources = HashSet::new();
        let mut seen_destinations = HashSet::new();
        for request in requests {
            let destination =
                undo::renamed_path(&request.path, &request.new_name).ok_or_else(|| {
                    FileSystemError::InvalidInput("The root of a volume cannot be renamed".into())
                })?;
            if destination == request.path {
                continue;
            }
            if !seen_sources.insert(request.path.clone()) {
                return Err(FileSystemError::InvalidInput(format!(
                    "The same entry was selected more than once: {}",
                    request.path
                )));
            }
            if !seen_destinations.insert(destination.to_ascii_lowercase()) {
                return Err(FileSystemError::AlreadyExists(destination));
            }
            pairs.push((request.path, destination));
        }
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        // Reject real collisions before mutating anything: a destination
        // already claimed by an entry that is not part of the batch.
        for (_, destination) in &pairs {
            if !undo::entry_exists(destination)? {
                continue;
            }
            let claimed_externally = pairs
                .iter()
                .all(|(source, _)| !source.eq_ignore_ascii_case(destination));
            if claimed_externally {
                return Err(FileSystemError::AlreadyExists(destination.clone()));
            }
        }

        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Move);
        let applied = undo::apply_rename_pairs(&pairs, &progress)?;

        // Report destinations in request order so the UI can reselect them;
        // entries that failed before applying keep their old selection.
        let applied_lookup: std::collections::HashMap<&str, &str> = applied
            .iter()
            .map(|pair| (pair.source.as_str(), pair.destination.as_str()))
            .collect();
        let results: Vec<String> = pairs
            .iter()
            .map(|(source, destination)| {
                applied_lookup
                    .get(source.as_str())
                    .map(|applied| (*applied).to_owned())
                    .unwrap_or_else(|| destination.clone())
            })
            .collect();

        if !applied.is_empty() {
            undo::state()
                .record(host.as_ref(), Operation::RenameBatch { pairs: applied });
        }
        progress.finish();

        Ok(results)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::create_entry` for the contract; this half is engine-free.
pub async fn create_entry(
    directory: String,
    name: String,
    kind: NewEntryKind,
    host: Arc<dyn Host>,
) -> Result<String, FileSystemError> {
    tokio::task::spawn_blocking(move || {
        let created = vfs::resolve(&directory)?.create_entry(&directory, &name, kind)?;
        undo::state().record(
            host.as_ref(),
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

/// See `commands::copy_entries` for the contract; this half is engine-free.
pub async fn copy_entries(
    items: Vec<TransferItem>,
    destination: String,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<(), FileSystemError> {
    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Copy);

    tokio::task::spawn_blocking(move || {
        let sources = resolve_transfer_items(items)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Copy);

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
        undo::state().record(
            host.as_ref(),
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

/// See `commands::move_entries` for the contract; this half is engine-free.
pub async fn move_entries(
    items: Vec<TransferItem>,
    destination: String,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<(), FileSystemError> {
    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Move);

    tokio::task::spawn_blocking(move || {
        let sources = resolve_transfer_items(items)?;
        let destination_backend = vfs::resolve(&destination)?;
        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Move);

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

        undo::state()
            .record(host.as_ref(), Operation::Move { transfers: journal });

        result
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::check_transfer_conflicts` for the contract; this half is engine-free.
pub async fn check_transfer_conflicts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<TransferConflict>, FileSystemError> {
    tokio::task::spawn_blocking(move || {
        let sources = resolve_sources(sources)?;
        let destination_backend = vfs::resolve(&destination)?;
        transfer::find_conflicts(sources, &destination, &destination_backend)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::delete_entries` for the contract; this half is engine-free.
pub async fn delete_entries(
    paths: Vec<String>,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Ok(());
    }

    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Delete);

    tokio::task::spawn_blocking(move || {
        let targets = resolve_sources(paths)?;
        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Delete);

        if targets
            .iter()
            .all(|target| vfs::is_local_path(&target.path))
        {
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

/// See `commands::trash_entries` for the contract; this half is engine-free.
pub async fn trash_entries(
    paths: Vec<String>,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<(), FileSystemError> {
    if paths.is_empty() {
        return Ok(());
    }
    if paths.iter().any(|path| !vfs::is_local_path(path)) {
        return Err(FileSystemError::InvalidInput("fs.trash_local_only".into()));
    }

    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Delete);

    tokio::task::spawn_blocking(move || {
        let progress = FileOperationProgressReporter::new(
            Arc::clone(&host),
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
        undo::state()
            .record(host.as_ref(), Operation::Trash { records });
        progress.finish();

        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::undo_operation` for the contract; this half is engine-free.
pub async fn undo_operation(
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<UndoRedoOutcome, FileSystemError> {
    let kind = undo::state()
        .peek_undo()
        .map(|operation| operation.undo_kind())
        .unwrap_or(FileOperationKind::Move);
    emit_preparing(host.as_ref(), &operation_id, kind);

    tokio::task::spawn_blocking(move || {
        let state = undo::state();
        let Some(operation) = state.pop_undo() else {
            return Err(FileSystemError::InvalidInput("fs.undo_empty".into()));
        };

        let op = operation.op_code();
        let progress = FileOperationProgressReporter::new(Arc::clone(&host), operation_id, kind);
        let result = undo::execute_undo(operation, &progress);

        match result {
            Ok((count, redo_operation)) => {
                if let Some(redo_operation) = redo_operation {
                    state.push_redo(redo_operation);
                }
                undo::emit_changed(host.as_ref(), state);
                progress.finish();
                Ok(UndoRedoOutcome {
                    action: "undo",
                    op,
                    count,
                })
            }
            Err(error) => {
                // The entry stays consumed; report what blocked the undo.
                undo::emit_changed(host.as_ref(), state);
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::redo_operation` for the contract; this half is engine-free.
pub async fn redo_operation(
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<UndoRedoOutcome, FileSystemError> {
    let kind = undo::state()
        .peek_redo()
        .map(|operation| operation.redo_kind())
        .unwrap_or(FileOperationKind::Move);
    emit_preparing(host.as_ref(), &operation_id, kind);

    tokio::task::spawn_blocking(move || {
        let state = undo::state();
        let Some(operation) = state.pop_redo() else {
            return Err(FileSystemError::InvalidInput("fs.redo_empty".into()));
        };

        let op = operation.op_code();
        let progress = FileOperationProgressReporter::new(Arc::clone(&host), operation_id, kind);
        let result = undo::execute_redo(operation, &progress);

        match result {
            Ok((count, undo_operation)) => {
                if let Some(undo_operation) = undo_operation {
                    state.push_undo(undo_operation);
                }
                undo::emit_changed(host.as_ref(), state);
                progress.finish();
                Ok(UndoRedoOutcome {
                    action: "redo",
                    op,
                    count,
                })
            }
            Err(error) => {
                undo::emit_changed(host.as_ref(), state);
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::duplicate_entries` for the contract; this half is engine-free.
pub async fn duplicate_entries(
    paths: Vec<String>,
    operation_id: String,
    host: Arc<dyn Host>,
) -> Result<Vec<String>, FileSystemError> {
    if paths.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before duplicating".into(),
        ));
    }

    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Copy);

    tokio::task::spawn_blocking(move || {
        let sources = resolve_sources(paths.clone())?;
        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Copy);
        let created = transfer::duplicate_sources(sources, &progress)?;

        undo::state().record(
            host.as_ref(),
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

/// See `commands::set_duplicate_suffix` for the contract; this half is engine-free.
pub fn set_duplicate_suffix(suffix: String) {
    super::transfer::set_duplicate_suffix(&suffix);
}

/// Creates a shortcut in `destination` for every path in `sources`.
pub async fn create_shortcuts(
    sources: Vec<String>,
    destination: String,
) -> Result<Vec<String>, FileSystemError> {
    tokio::task::spawn_blocking(move || super::shortcuts::create_shortcuts(&sources, &destination))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Places `paths` on the system clipboard as a file list carrying `effect`.
pub async fn write_files_to_clipboard(
    paths: Vec<String>,
    effect: super::clipboard::ClipboardEffect,
) -> Result<(), FileSystemError> {
    tokio::task::spawn_blocking(move || super::clipboard::write_files_to_clipboard(&paths, effect))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Reads the file list the system clipboard holds, if it holds one.
pub async fn read_files_from_clipboard()
-> Result<Option<super::clipboard::SystemClipboardFiles>, FileSystemError> {
    tokio::task::spawn_blocking(super::clipboard::read_files_from_clipboard)
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::get_file_properties` for the contract; this half is engine-free.
pub async fn get_file_properties(path: String) -> Result<FileProperties, FileSystemError> {
    tokio::task::spawn_blocking(move || vfs::resolve(&path)?.properties(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::update_file_properties` for the contract; this half is engine-free.
pub async fn update_file_properties(
    path: String,
    changes: PropertyChanges,
) -> Result<(), FileSystemError> {
    tokio::task::spawn_blocking(move || {
        vfs::resolve(&path)?.update_properties(&path, &changes)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::update_file_properties_recursive` for the contract; this half is engine-free.
pub async fn update_file_properties_recursive(
    operation_id: String,
    path: String,
    changes: PropertyChanges,
    host: Arc<dyn Host>,
) -> Result<RecursivePropertyUpdateOutcome, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.properties_local_only".into(),
        ));
    }

    if !Path::new(&path).is_dir() {
        return Err(FileSystemError::NotDirectory(path));
    }

    emit_preparing(host.as_ref(), &operation_id, FileOperationKind::Properties);

    tokio::task::spawn_blocking(move || {
        let progress =
            FileOperationProgressReporter::new(Arc::clone(&host), operation_id, FileOperationKind::Properties);
        let outcome =
            vfs::resolve(&path)?.update_properties_recursive(&path, &changes, &progress)?;
        progress.finish();
        Ok(outcome)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// See `commands::open_terminal` for the contract; this half is engine-free.
pub fn open_terminal(path: String) -> Result<(), FileSystemError> {
    if !vfs::is_local_path(&path) {
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

/// See `commands::open_with` for the contract; this half is engine-free.
pub async fn open_with(path: String) -> Result<(), FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_local_only".into(),
        ));
    }

    let target = PathBuf::from(&path);
    if !target.is_file() && !target.is_dir() {
        return Err(FileSystemError::InvalidInput(
            "fs.open_with_not_found".into(),
        ));
    }

    // SHOpenWithDialog 以模态方式运行到用户关闭为止，放到阻塞线程池执行。
    tokio::task::spawn_blocking(move || open_system_with_dialog(&target))
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

    Err(FileSystemError::Internal("fs.terminal_not_found".into()))
}

#[cfg(target_os = "windows")]
fn open_system_with_dialog(file: &Path) -> Result<(), FileSystemError> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{
        OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPEN_AS_INFO_FLAGS, OPENASINFO,
        SHOpenWithDialog,
    };
    use windows::core::{HRESULT, PCWSTR};

    /// RAII COM 套间；S_FALSE (0x1) 表示线程已有套间，此时不能反初始化。
    struct CoApartment {
        owned: bool,
    }

    impl CoApartment {
        fn enter() -> Result<Self, FileSystemError> {
            // windows 0.62 返回裸 HRESULT：S_OK (0) 表示新初始化了套间，
            // 需要在 drop 时反初始化；S_FALSE (0x1) 表示线程已有套间，不能反初始化。
            let result: HRESULT =
                unsafe { CoInitializeEx(Some(std::ptr::null()), COINIT_APARTMENTTHREADED) };
            match result.0 {
                0 => Ok(Self { owned: true }),
                1 => Ok(Self { owned: false }),
                _ => Err(FileSystemError::Internal(format!(
                    "CoInitializeEx failed with HRESULT {:#010x}",
                    result.0
                ))),
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
    Err(FileSystemError::Internal("fs.open_with_unsupported".into()))
}

/// True when every source and the destination sit on the local backend, which
/// keeps its native rayon transfer path instead of the streaming engine.
fn is_pure_local(sources: &[TransferSource], destination: &str) -> bool {
    vfs::is_local_path(destination)
        && sources
            .iter()
            .all(|source| vfs::is_local_path(&source.path))
}
