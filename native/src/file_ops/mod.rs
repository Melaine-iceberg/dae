//! File-operation commands for the Electron core process: rename, create,
//! copy, move, delete, trash, duplicate, properties, and the undo/redo history
//! that backs them.
//!
//! The engines are `dae-core`'s; this module only parses command arguments,
//! owns the process-wide undo stacks, and reports progress on the event bus.

mod progress;
mod undo;

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::local;
use dae_core::file_system::transfer::{self, TransferSource};
use dae_core::file_system::types::{
    ConflictAction, NewEntryKind, PropertyChanges, RenameRequest, TransferItem, TransferPair,
    path_to_string,
};
use dae_core::file_system::vfs;
use progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use undo::{Operation, TrashRecord, UndoRedoOutcome, UndoRedoState};

/// The process-wide history. File operations can arrive from any window, and
/// they all share one Ctrl+Z stack the way a single Explorer window would.
fn undo_state() -> &'static UndoRedoState {
    static STATE: OnceLock<UndoRedoState> = OnceLock::new();
    STATE.get_or_init(UndoRedoState::default)
}

fn parse<T: DeserializeOwned>(args: &Value, command: &str) -> Result<T, String> {
    serde_json::from_value(args.clone())
        .map_err(|error| format!("{command}: invalid args: {error}"))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn reason(error: FileSystemError) -> String {
    error.to_string()
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

fn is_pure_local(sources: &[TransferSource], destination: &str) -> bool {
    vfs::is_local_path(destination) && sources.iter().all(|source| vfs::is_local_path(&source.path))
}

pub(crate) async fn dispatch(command: &str, args: &Value) -> Result<Option<Value>, String> {
    let value = match command {
        "rename_entry" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                path: String,
                new_name: String,
            }
            let args: Args = parse(args, command)?;
            tokio::task::spawn_blocking(move || {
                vfs::resolve(&args.path)?.rename_entry(&args.path, &args.new_name)?;
                if let Some(to) = undo::renamed_path(&args.path, &args.new_name) {
                    if to != args.path {
                        undo_state().record(Operation::Rename {
                            from: args.path,
                            to,
                        });
                    }
                }
                Ok::<_, FileSystemError>(())
            })
            .await
            .map_err(|error| format!("rename join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "rename_entries_batch" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                requests: Vec<RenameRequest>,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            if args.requests.is_empty() {
                return Ok(Some(json!([])));
            }

            emit_preparing(&args.operation_id, FileOperationKind::Move);

            let results = tokio::task::spawn_blocking(move || {
                let mut pairs: Vec<(String, String)> = Vec::with_capacity(args.requests.len());
                let mut seen_sources = HashSet::new();
                let mut seen_destinations = HashSet::new();
                for request in args.requests {
                    let destination =
                        undo::renamed_path(&request.path, &request.new_name).ok_or_else(|| {
                            FileSystemError::InvalidInput(
                                "The root of a volume cannot be renamed".into(),
                            )
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

                // Reject real collisions before mutating anything.
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

                let progress = FileOperationProgressReporter::new(
                    args.operation_id.clone(),
                    FileOperationKind::Move,
                );
                let applied = undo::apply_rename_pairs(&pairs, &progress)?;

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
                    undo_state().record(Operation::RenameBatch { pairs: applied });
                }
                progress.finish();
                Ok(results)
            })
            .await
            .map_err(|error| format!("rename join: {error}"))?
            .map_err(reason)?;
            to_value(results)?
        }

        "create_entry" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                directory: String,
                name: String,
                kind: NewEntryKind,
            }
            let args: Args = parse(args, command)?;
            let created = tokio::task::spawn_blocking(move || {
                let created = vfs::resolve(&args.directory)?.create_entry(
                    &args.directory,
                    &args.name,
                    args.kind,
                )?;
                undo_state().record(Operation::Create {
                    path: created.clone(),
                    kind: args.kind,
                });
                Ok::<_, FileSystemError>(created)
            })
            .await
            .map_err(|error| format!("create join: {error}"))?
            .map_err(reason)?;
            Value::String(created)
        }

        "copy_entries" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                items: Vec<TransferItem>,
                destination: String,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            emit_preparing(&args.operation_id, FileOperationKind::Copy);

            tokio::task::spawn_blocking(move || {
                let sources = resolve_transfer_items(args.items)?;
                let destination_backend = vfs::resolve(&args.destination)?;
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Copy,
                );

                let mut journal: Vec<TransferPair> = Vec::new();
                let result = if is_pure_local(&sources, &args.destination) {
                    let paths = sources
                        .iter()
                        .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                        .collect::<Vec<_>>();
                    local::copy_entries_with_progress(
                        paths,
                        PathBuf::from(&args.destination),
                        &progress,
                        &mut journal,
                    )
                } else {
                    transfer::copy_entries(
                        sources,
                        &args.destination,
                        &destination_backend,
                        &progress,
                        &mut journal,
                    )
                };

                let (recorded_sources, created): (Vec<String>, Vec<String>) = journal
                    .into_iter()
                    .map(|pair| (pair.source, pair.destination))
                    .unzip();
                undo_state().record(Operation::Copy {
                    sources: recorded_sources,
                    destination: args.destination,
                    created,
                });
                result
            })
            .await
            .map_err(|error| format!("copy join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "move_entries" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                items: Vec<TransferItem>,
                destination: String,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            emit_preparing(&args.operation_id, FileOperationKind::Move);

            tokio::task::spawn_blocking(move || {
                let sources = resolve_transfer_items(args.items)?;
                let destination_backend = vfs::resolve(&args.destination)?;
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Move,
                );

                let mut journal: Vec<TransferPair> = Vec::new();
                let result = if is_pure_local(&sources, &args.destination) {
                    let paths = sources
                        .iter()
                        .map(|source| (PathBuf::from(&source.path), source.on_conflict))
                        .collect::<Vec<_>>();
                    local::move_entries_with_progress(
                        paths,
                        PathBuf::from(&args.destination),
                        &progress,
                        &mut journal,
                    )
                } else {
                    transfer::move_entries(
                        sources,
                        &args.destination,
                        &destination_backend,
                        &progress,
                        &mut journal,
                    )
                };

                undo_state().record(Operation::Move { transfers: journal });
                result
            })
            .await
            .map_err(|error| format!("move join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "check_transfer_conflicts" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                sources: Vec<String>,
                destination: String,
            }
            let args: Args = parse(args, command)?;
            let conflicts = tokio::task::spawn_blocking(move || {
                let sources = resolve_sources(args.sources)?;
                let destination_backend = vfs::resolve(&args.destination)?;
                transfer::find_conflicts(sources, &args.destination, &destination_backend)
            })
            .await
            .map_err(|error| format!("conflict join: {error}"))?
            .map_err(reason)?;
            to_value(conflicts)?
        }

        "delete_entries" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                paths: Vec<String>,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            if args.paths.is_empty() {
                return Ok(Some(Value::Null));
            }
            emit_preparing(&args.operation_id, FileOperationKind::Delete);

            tokio::task::spawn_blocking(move || {
                let targets = resolve_sources(args.paths)?;
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Delete,
                );
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
            .map_err(|error| format!("delete join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "trash_entries" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                paths: Vec<String>,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            if args.paths.is_empty() {
                return Ok(Some(Value::Null));
            }
            if args.paths.iter().any(|path| !vfs::is_local_path(path)) {
                return Err("fs.trash_local_only".into());
            }
            emit_preparing(&args.operation_id, FileOperationKind::Delete);

            tokio::task::spawn_blocking(move || {
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Delete,
                );
                progress.start(args.paths.len() as u64);

                let mut records = Vec::with_capacity(args.paths.len());
                let mut first_error: Option<FileSystemError> = None;
                for path in &args.paths {
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

                undo_state().record(Operation::Trash { records });
                progress.finish();

                if let Some(error) = first_error {
                    return Err(error);
                }
                Ok(())
            })
            .await
            .map_err(|error| format!("trash join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "undo_operation" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            let kind = undo_state()
                .peek_undo()
                .map(|operation| operation.undo_kind())
                .unwrap_or(FileOperationKind::Move);
            emit_preparing(&args.operation_id, kind);

            let outcome = tokio::task::spawn_blocking(move || {
                let state = undo_state();
                let Some(operation) = state.pop_undo() else {
                    return Err(FileSystemError::InvalidInput("fs.undo_empty".into()));
                };

                let op = operation.op_code();
                let progress = FileOperationProgressReporter::new(args.operation_id, kind);
                match undo::execute_undo(operation, &progress) {
                    Ok((count, redo_operation)) => {
                        if let Some(redo_operation) = redo_operation {
                            state.push_redo(redo_operation);
                        }
                        undo::emit_changed(state);
                        progress.finish();
                        Ok(UndoRedoOutcome {
                            action: "undo",
                            op,
                            count,
                        })
                    }
                    Err(error) => {
                        undo::emit_changed(state);
                        Err(error)
                    }
                }
            })
            .await
            .map_err(|error| format!("undo join: {error}"))?
            .map_err(reason)?;
            to_value(outcome)?
        }

        "redo_operation" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            let kind = undo_state()
                .peek_redo()
                .map(|operation| operation.redo_kind())
                .unwrap_or(FileOperationKind::Move);
            emit_preparing(&args.operation_id, kind);

            let outcome = tokio::task::spawn_blocking(move || {
                let state = undo_state();
                let Some(operation) = state.pop_redo() else {
                    return Err(FileSystemError::InvalidInput("fs.redo_empty".into()));
                };

                let op = operation.op_code();
                let progress = FileOperationProgressReporter::new(args.operation_id, kind);
                match undo::execute_redo(operation, &progress) {
                    Ok((count, undo_operation)) => {
                        if let Some(undo_operation) = undo_operation {
                            state.push_undo(undo_operation);
                        }
                        undo::emit_changed(state);
                        progress.finish();
                        Ok(UndoRedoOutcome {
                            action: "redo",
                            op,
                            count,
                        })
                    }
                    Err(error) => {
                        undo::emit_changed(state);
                        Err(error)
                    }
                }
            })
            .await
            .map_err(|error| format!("redo join: {error}"))?
            .map_err(reason)?;
            to_value(outcome)?
        }

        "duplicate_entries" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                paths: Vec<String>,
                operation_id: String,
            }
            let args: Args = parse(args, command)?;
            if args.paths.is_empty() {
                return Err("Choose at least one entry before duplicating".into());
            }
            emit_preparing(&args.operation_id, FileOperationKind::Copy);

            let created = tokio::task::spawn_blocking(move || {
                let sources = resolve_sources(args.paths.clone())?;
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Copy,
                );
                let created = transfer::duplicate_sources(sources, &progress)?;
                undo_state().record(Operation::Duplicate {
                    sources: args.paths,
                    created: created.clone(),
                });
                Ok::<_, FileSystemError>(created)
            })
            .await
            .map_err(|error| format!("duplicate join: {error}"))?
            .map_err(reason)?;
            to_value(created)?
        }

        "set_duplicate_suffix" => {
            #[derive(serde::Deserialize)]
            struct Args {
                suffix: String,
            }
            let args: Args = parse(args, command)?;
            transfer::set_duplicate_suffix(&args.suffix);
            Value::Null
        }

        "get_file_properties" => {
            #[derive(serde::Deserialize)]
            struct Args {
                path: String,
            }
            let args: Args = parse(args, command)?;
            let properties = tokio::task::spawn_blocking(move || {
                vfs::resolve(&args.path)?.properties(&args.path)
            })
            .await
            .map_err(|error| format!("properties join: {error}"))?
            .map_err(reason)?;
            to_value(properties)?
        }

        "update_file_properties" => {
            #[derive(serde::Deserialize)]
            struct Args {
                path: String,
                changes: PropertyChanges,
            }
            let args: Args = parse(args, command)?;
            tokio::task::spawn_blocking(move || {
                vfs::resolve(&args.path)?.update_properties(&args.path, &args.changes)
            })
            .await
            .map_err(|error| format!("properties join: {error}"))?
            .map_err(reason)?;
            Value::Null
        }

        "update_file_properties_recursive" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                operation_id: String,
                path: String,
                changes: PropertyChanges,
            }
            let args: Args = parse(args, command)?;
            if !vfs::is_local_path(&args.path) {
                return Err("fs.properties_local_only".into());
            }
            if !Path::new(&args.path).is_dir() {
                return Err(format!("not a directory: {}", args.path));
            }
            emit_preparing(&args.operation_id, FileOperationKind::Properties);

            let outcome = tokio::task::spawn_blocking(move || {
                let progress = FileOperationProgressReporter::new(
                    args.operation_id,
                    FileOperationKind::Properties,
                );
                let outcome = vfs::resolve(&args.path)?.update_properties_recursive(
                    &args.path,
                    &args.changes,
                    &progress,
                )?;
                progress.finish();
                Ok::<_, FileSystemError>(outcome)
            })
            .await
            .map_err(|error| format!("properties join: {error}"))?
            .map_err(reason)?;
            to_value(outcome)?
        }

        _ => return Ok(None),
    };

    Ok(Some(value))
}

/// Blocks until the primary mouse button is released, for the native tab-drag
/// loop. `false` means the timeout expired first.
pub(crate) async fn wait_for_mouse_release(timeout_ms: u64) -> bool {
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            if !primary_button_down() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        false
    })
    .await
    .unwrap_or(false)
}

#[cfg(windows)]
fn primary_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

#[cfg(target_os = "macos")]
fn primary_button_down() -> bool {
    use objc2_app_kit::NSEvent;
    unsafe { NSEvent::pressedMouseButtons() & 1 != 0 }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn primary_button_down() -> bool {
    // No global button state on X11/Wayland; the renderer's frontend-bounds
    // path settles those gestures instead.
    false
}
