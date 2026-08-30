//! Session-scoped undo/redo history for explorer file operations.
//!
//! Every operation that changes the file system appends an [`Operation`]
//! record to the undo stack; a new record clears the redo stack (the standard
//! linear-history model, like Windows Explorer). Partially failed batches
//! still record whatever did land, so Ctrl+Z can revert the executed part.
//!
//! Undo/redo execute through the same engines as the original operations:
//! move-back reuses the transfer pipeline (protocol-native rename when
//! possible), and undoing a copy/duplicate/create removes the created
//! entries through the system trash when they are local, so an undo is
//! never itself destructive. Entries the world has moved on from (deleted,
//! renamed, or occupied elsewhere) are skipped or — for moves — fail with
//! an error, and a failed step is dropped from the history rather than
//! leaving a stale record behind.

use super::error::FileSystemError;
use super::progress::{FileOperationKind, FileOperationProgressReporterTrait};
use super::transfer::{self, TransferSource};
use super::types::{ConflictAction, NewEntryKind, TransferPair, path_to_string};
use super::vfs;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri_specta::Event;

/// How many operations stay undoable; older entries drop off the bottom.
const MAX_HISTORY: usize = 50;

/// Where a trashed entry lived, so undo can restore it.
#[derive(Debug, Clone)]
pub struct TrashRecord {
    pub parent: String,
    pub name: String,
}

/// One history entry: enough state to revert and re-apply an operation.
#[derive(Debug, Clone)]
pub enum Operation {
    /// Batch move; each pair is (original location, current location).
    Move { transfers: Vec<TransferPair> },
    /// One rename; `from`/`to` are the full paths before and after.
    Rename { from: String, to: String },
    /// Batch copy. `created` are the paths the copy produced ("副本"
    /// auto-renames included); `sources`/`destination` re-run the copy on redo.
    Copy {
        sources: Vec<String>,
        destination: String,
        created: Vec<String>,
    },
    /// Batch move-to-trash; the records restore the original locations.
    Trash { records: Vec<TrashRecord> },
    /// One created file or directory.
    Create { path: String, kind: NewEntryKind },
    /// Duplicates produced by the "创建副本" action.
    Duplicate {
        sources: Vec<String>,
        created: Vec<String>,
    },
}

impl Operation {
    fn is_empty(&self) -> bool {
        match self {
            Operation::Move { transfers } => transfers.is_empty(),
            Operation::Rename { .. } => false,
            Operation::Copy { created, .. } => created.is_empty(),
            Operation::Trash { records } => records.is_empty(),
            Operation::Create { .. } => false,
            Operation::Duplicate { created, .. } => created.is_empty(),
        }
    }

    /// Stable code naming the original operation for UI toast translation,
    /// e.g. "move". The frontend maps it to a localized noun.
    pub fn op_code(&self) -> &'static str {
        match self {
            Operation::Move { .. } => "move",
            Operation::Rename { .. } => "rename",
            Operation::Copy { .. } => "copy",
            Operation::Trash { .. } => "trash",
            Operation::Create { .. } => "create",
            Operation::Duplicate { .. } => "duplicate",
        }
    }

    /// The progress-bar operation the undo of this entry performs.
    pub fn undo_kind(&self) -> FileOperationKind {
        match self {
            Operation::Move { .. } | Operation::Rename { .. } => FileOperationKind::Move,
            Operation::Copy { .. }
            | Operation::Trash { .. }
            | Operation::Create { .. }
            | Operation::Duplicate { .. } => FileOperationKind::Delete,
        }
    }

    /// The progress-bar operation the redo of this entry performs.
    pub fn redo_kind(&self) -> FileOperationKind {
        match self {
            Operation::Move { .. } | Operation::Rename { .. } | Operation::Create { .. } => {
                FileOperationKind::Move
            }
            Operation::Copy { .. } | Operation::Duplicate { .. } => FileOperationKind::Copy,
            Operation::Trash { .. } => FileOperationKind::Delete,
        }
    }
}

/// Result of one undo/redo step. The frontend renders the localized toast
/// from these fields, including pluralization for `count`.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoRedoOutcome {
    /// "undo" or "redo".
    pub action: &'static str,
    /// The reverted/re-applied operation code, e.g. "move".
    pub op: &'static str,
    /// How many entries the step touched.
    pub count: u64,
}

/// Emitted whenever the undo/redo stacks change so shortcuts stay accurate.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-undo-redo-changed")]
#[serde(rename_all = "camelCase")]
pub struct UndoRedoChanged {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// The app-wide undo/redo stacks. All access goes through short critical
/// sections; execution happens outside the locks on blocking threads.
#[derive(Default)]
pub struct UndoRedoState {
    undo: std::sync::Mutex<Vec<Operation>>,
    redo: std::sync::Mutex<Vec<Operation>>,
}

impl UndoRedoState {
    /// Appends one executed operation and clears the redo stack.
    pub fn record(&self, app: &tauri::AppHandle, operation: Operation) {
        if operation.is_empty() {
            return;
        }

        {
            let mut undo = self.undo.lock().expect("undo stack lock poisoned");
            undo.push(operation);
            let overflow = undo.len().saturating_sub(MAX_HISTORY);
            undo.drain(..overflow);
        }
        self.redo.lock().expect("redo stack lock poisoned").clear();
        emit_changed(app, self);
    }

    /// The operation Ctrl+Z would run, for progress reporting.
    pub fn peek_undo(&self) -> Option<Operation> {
        self.undo
            .lock()
            .expect("undo stack lock poisoned")
            .last()
            .cloned()
    }

    /// The operation Ctrl+Shift+Z would run, for progress reporting.
    pub fn peek_redo(&self) -> Option<Operation> {
        self.redo
            .lock()
            .expect("redo stack lock poisoned")
            .last()
            .cloned()
    }

    pub fn pop_undo(&self) -> Option<Operation> {
        self.undo.lock().expect("undo stack lock poisoned").pop()
    }

    pub fn pop_redo(&self) -> Option<Operation> {
        self.redo.lock().expect("redo stack lock poisoned").pop()
    }

    /// Pushes the (possibly updated) operation a finished undo produced.
    pub fn push_redo(&self, operation: Operation) {
        let mut redo = self.redo.lock().expect("redo stack lock poisoned");
        redo.push(operation);
    }

    /// Pushes the (possibly updated) operation a finished redo produced.
    pub fn push_undo(&self, operation: Operation) {
        let mut undo = self.undo.lock().expect("undo stack lock poisoned");
        undo.push(operation);
        let overflow = undo.len().saturating_sub(MAX_HISTORY);
        undo.drain(..overflow);
    }

    fn can_undo(&self) -> bool {
        !self
            .undo
            .lock()
            .expect("undo stack lock poisoned")
            .is_empty()
    }

    fn can_redo(&self) -> bool {
        !self
            .redo
            .lock()
            .expect("redo stack lock poisoned")
            .is_empty()
    }
}

/// Notifies the frontend that shortcut availability changed.
pub fn emit_changed(app: &tauri::AppHandle, state: &UndoRedoState) {
    let _ = UndoRedoChanged {
        can_undo: state.can_undo(),
        can_redo: state.can_redo(),
    }
    .emit(app);
}

/// Reverts one operation. Returns the affected-entry count plus the
/// (possibly narrowed) operation to push onto the redo stack — `None` when
/// nothing was actually reverted, so there is nothing to redo either.
pub fn execute_undo(
    operation: Operation,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(u64, Option<Operation>), FileSystemError> {
    match operation {
        Operation::Move { transfers } => {
            let undone = undo_move(&transfers, progress)?;
            let count = undone.len() as u64;
            let redo = (!undone.is_empty()).then_some(Operation::Move { transfers: undone });
            Ok((count, redo))
        }
        Operation::Rename { from, to } => {
            undo_rename(&from, &to)?;
            Ok((1, Some(Operation::Rename { from, to })))
        }
        Operation::Copy {
            sources,
            destination,
            created,
        } => {
            let removed = trash_or_delete(&created, progress)?;
            let count = removed.len() as u64;
            let redo = (!removed.is_empty()).then_some(Operation::Copy {
                sources,
                destination,
                created: removed,
            });
            Ok((count, redo))
        }
        Operation::Trash { records } => {
            let restored = undo_trash(&records, progress)?;
            let count = restored.len() as u64;
            Ok((count, Some(Operation::Trash { records: restored })))
        }
        Operation::Create { path, kind } => {
            let removed = trash_or_delete(std::slice::from_ref(&path), progress)?;
            let count = removed.len() as u64;
            let redo = (!removed.is_empty()).then_some(Operation::Create { path, kind });
            Ok((count, redo))
        }
        Operation::Duplicate { sources, created } => {
            let removed = trash_or_delete(&created, progress)?;
            let count = removed.len() as u64;
            let redo = (!removed.is_empty()).then_some(Operation::Duplicate {
                sources,
                created: removed,
            });
            Ok((count, redo))
        }
    }
}

/// Re-applies one undone operation. Returns the affected-entry count plus
/// the (possibly updated) operation to push back onto the undo stack.
pub fn execute_redo(
    operation: Operation,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(u64, Option<Operation>), FileSystemError> {
    match operation {
        Operation::Move { transfers } => {
            let redone = redo_move(&transfers, progress)?;
            let count = redone.len() as u64;
            let undo = (!redone.is_empty()).then_some(Operation::Move { transfers: redone });
            Ok((count, undo))
        }
        Operation::Rename { from, to } => {
            redo_rename(&from, &to)?;
            Ok((1, Some(Operation::Rename { from, to })))
        }
        Operation::Copy {
            sources,
            destination,
            ..
        } => {
            let created = redo_copy(&sources, &destination, progress)?;
            let count = created.len() as u64;
            let undo = (!created.is_empty()).then_some(Operation::Copy {
                sources,
                destination,
                created,
            });
            Ok((count, undo))
        }
        Operation::Trash { records } => {
            let trashed = redo_trash(&records, progress)?;
            let count = trashed.len() as u64;
            Ok((count, Some(Operation::Trash { records: trashed })))
        }
        Operation::Create { path, kind } => {
            redo_create(&path, kind)?;
            Ok((1, Some(Operation::Create { path, kind })))
        }
        Operation::Duplicate { sources, .. } => {
            let created = redo_duplicate(&sources, progress)?;
            let count = created.len() as u64;
            let undo = (!created.is_empty()).then_some(Operation::Duplicate { sources, created });
            Ok((count, undo))
        }
    }
}

/// The full path an entry gets when renamed to `new_name` within its parent,
/// or `None` for volume roots (which cannot be renamed).
pub fn renamed_path(path: &str, new_name: &str) -> Option<String> {
    if vfs::is_local_path(path) {
        Path::new(path)
            .parent()
            .map(|parent| path_to_string(&parent.join(new_name)))
    } else {
        transfer::parent_path_of(path).map(|parent| format!("{parent}{new_name}"))
    }
}

// -- Move -----------------------------------------------------------------

/// Moves every pair's current entry (`from`) back into the parent of its
/// original location (`to`), grouped by parent so each group is one transfer.
fn move_pairs(
    transfers: &[TransferPair],
    current: fn(&TransferPair) -> &str,
    target: fn(&TransferPair) -> &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<TransferPair>, FileSystemError> {
    // Only pairs whose current location still holds an entry can be replayed.
    let mut active = Vec::new();
    for pair in transfers {
        if entry_exists(current(pair))? {
            active.push(pair.clone());
        }
    }
    if active.is_empty() {
        return Ok(active);
    }

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for pair in &active {
        groups
            .entry(parent_directory(target(pair)))
            .or_default()
            .push(current(pair).to_owned());
    }

    for (parent, paths) in groups {
        move_entries_into(&paths, &parent, progress)?;
    }

    Ok(active)
}

fn undo_move(
    transfers: &[TransferPair],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<TransferPair>, FileSystemError> {
    move_pairs(
        transfers,
        |pair| &pair.destination,
        |pair| &pair.source,
        progress,
    )
}

fn redo_move(
    transfers: &[TransferPair],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<TransferPair>, FileSystemError> {
    move_pairs(
        transfers,
        |pair| &pair.source,
        |pair| &pair.destination,
        progress,
    )
}

/// Moves `paths` into the directory `destination`, failing when a target
/// name is already taken — an undo/redo must never overwrite what is there
/// now. Prefers the backend-native local fast path for pure-local batches.
fn move_entries_into(
    paths: &[String],
    destination: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let mut journal = Vec::new();

    if vfs::is_local_path(destination) && paths.iter().all(|path| vfs::is_local_path(path)) {
        let sources = paths
            .iter()
            .map(|path| (PathBuf::from(path), ConflictAction::Fail))
            .collect();
        super::local::move_entries_with_progress(
            sources,
            PathBuf::from(destination),
            progress,
            &mut journal,
        )
    } else {
        let sources = resolve_sources(paths, ConflictAction::Fail)?;
        let destination_backend = vfs::resolve(destination)?;
        transfer::move_entries(
            sources,
            destination,
            &destination_backend,
            progress,
            &mut journal,
        )
    }
}

// -- Rename ---------------------------------------------------------------

fn undo_rename(from: &str, to: &str) -> Result<(), FileSystemError> {
    // `to` is where the entry lives now; restore its original name. When
    // the entry already sits at `from`, the rename was reverted elsewhere.
    if !entry_exists(to)? {
        if entry_exists(from)? {
            return Ok(());
        }
        return Err(FileSystemError::NotFound(to.to_owned()));
    }

    let name = last_segment_of(from)?;
    vfs::resolve(to)?.rename_entry(to, &name)
}

fn redo_rename(from: &str, to: &str) -> Result<(), FileSystemError> {
    if !entry_exists(from)? {
        if entry_exists(to)? {
            return Ok(());
        }
        return Err(FileSystemError::NotFound(from.to_owned()));
    }

    let name = last_segment_of(to)?;
    vfs::resolve(from)?.rename_entry(from, &name)
}

// -- Copy / duplicate / create undo ----------------------------------------

/// Removes `paths` — through the system trash when they are local, so the
/// undo itself stays reversible; network entries are deleted permanently
/// (no recycle bin exists there). Returns the paths actually removed.
fn trash_or_delete(
    paths: &[String],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<String>, FileSystemError> {
    let mut existing = Vec::with_capacity(paths.len());
    for path in paths {
        if entry_exists(path)? {
            existing.push(path.clone());
        }
    }
    if existing.is_empty() {
        return Ok(existing);
    }

    if existing.iter().all(|path| vfs::is_local_path(path)) {
        progress.start(existing.len() as u64);
        for path in &existing {
            trash::delete(path).map_err(trash_error)?;
            progress.advance(Path::new(path));
        }
        return Ok(existing);
    }

    let targets = resolve_sources(&existing, ConflictAction::Fail)?;
    transfer::delete_entries(targets, progress)?;
    Ok(existing)
}

// -- Copy / duplicate / create redo ----------------------------------------

fn redo_copy(
    sources: &[String],
    destination: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<String>, FileSystemError> {
    let mut journal = Vec::new();

    // KeepBoth so a redo never overwrites whatever claimed the names since;
    // the journal records where the copies actually landed.
    if vfs::is_local_path(destination) && sources.iter().all(|path| vfs::is_local_path(path)) {
        let items = sources
            .iter()
            .map(|path| (PathBuf::from(path), ConflictAction::KeepBoth))
            .collect();
        super::local::copy_entries_with_progress(
            items,
            PathBuf::from(destination),
            progress,
            &mut journal,
        )?;
    } else {
        let items = resolve_sources(sources, ConflictAction::KeepBoth)?;
        let destination_backend = vfs::resolve(destination)?;
        transfer::copy_entries(
            items,
            destination,
            &destination_backend,
            progress,
            &mut journal,
        )?;
    }

    Ok(journal.into_iter().map(|pair| pair.destination).collect())
}

fn redo_duplicate(
    sources: &[String],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<String>, FileSystemError> {
    let items = resolve_sources(sources, ConflictAction::Fail)?;
    transfer::duplicate_sources(items, progress)
}

fn redo_create(path: &str, kind: NewEntryKind) -> Result<(), FileSystemError> {
    let parent = parent_directory(path);
    let name = last_segment_of(path)?;
    vfs::resolve(&parent)?.create_entry(&parent, &name, kind)?;
    Ok(())
}

// -- Trash -----------------------------------------------------------------

/// Restores trashed records to their original locations, returning the
/// records actually restored. Entries no longer in the trash (emptied or
/// restored elsewhere) are skipped.
fn undo_trash(
    records: &[TrashRecord],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<TrashRecord>, FileSystemError> {
    let items = trash::os_limited::list().map_err(trash_error)?;
    let mut to_restore = Vec::new();
    let mut restored = Vec::new();

    for record in records {
        // The same (parent, name) can appear multiple times in the trash
        // from earlier deletions; the newest one is ours.
        let match_item = items
            .iter()
            .filter(|item| {
                item.name.to_string_lossy() == record.name
                    && same_trash_location(&item.original_parent, &record.parent)
            })
            .max_by_key(|item| item.time_deleted);

        if let Some(item) = match_item {
            restored.push(record.clone());
            to_restore.push(item.clone());
        }
    }

    if to_restore.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "fs.undo_trash_missing".into(),
        ));
    }

    progress.start(to_restore.len() as u64);
    trash::os_limited::restore_all(to_restore).map_err(trash_error)?;
    Ok(restored)
}

/// Re-trashes the restored records still sitting at their original spots,
/// returning the records actually moved.
fn redo_trash(
    records: &[TrashRecord],
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<TrashRecord>, FileSystemError> {
    let mut to_trash = Vec::new();
    for record in records {
        if restore_path(record).try_exists()? {
            to_trash.push(record.clone());
        }
    }
    if to_trash.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "fs.redo_trash_source_missing".into(),
        ));
    }

    progress.start(to_trash.len() as u64);
    for record in &to_trash {
        let path = restore_path(record);
        trash::delete(&path).map_err(trash_error)?;
        progress.advance(&path);
    }

    Ok(to_trash)
}

fn restore_path(record: &TrashRecord) -> PathBuf {
    Path::new(&record.parent).join(&record.name)
}

/// Windows paths are case-insensitive, so recycle-bin parents recorded from
/// a deletion must compare that way too.
fn same_trash_location(left: &Path, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == Path::new(right)
    }
}

fn trash_error(error: trash::Error) -> FileSystemError {
    FileSystemError::Internal(error.to_string())
}

// -- Path helpers -----------------------------------------------------------

/// True when the path currently refers to an entry on its backend.
fn entry_exists(path: &str) -> Result<bool, FileSystemError> {
    match vfs::resolve(path)?.stat(path) {
        Ok(_) => Ok(true),
        Err(FileSystemError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

/// The directory containing `path`. Local paths go through the std library
/// (Windows drive roots keep their separator); scheme paths trim the last
/// `/` segment.
fn parent_directory(path: &str) -> String {
    if vfs::is_local_path(path) {
        Path::new(path)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned())
    } else {
        transfer::parent_path_of(path)
            .map(|parent| parent.trim_end_matches('/').to_owned())
            .unwrap_or_else(|| path.to_owned())
    }
}

fn last_segment_of(path: &str) -> Result<String, FileSystemError> {
    transfer::last_segment(path)
        .map(str::to_owned)
        .ok_or_else(|| {
            FileSystemError::InvalidInput(format!("The root of a volume cannot be renamed: {path}"))
        })
}

fn resolve_sources(
    paths: &[String],
    on_conflict: ConflictAction,
) -> Result<Vec<TransferSource>, FileSystemError> {
    paths
        .iter()
        .map(|path| {
            Ok(TransferSource {
                path: path.clone(),
                backend: vfs::resolve(path)?,
                on_conflict,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::local::{move_entries_with_progress, rename_entry_sync};
    use crate::file_system::test_support::TestProgress;
    use std::fs;

    #[test]
    fn undo_and_redo_revert_a_batch_move() {
        let directory =
            std::env::temp_dir().join(format!("dae-undo-move-test-{}", std::process::id()));
        let source = directory.join("source");
        let destination = directory.join("destination");
        fs::create_dir_all(&source).expect("create source directory");
        fs::create_dir_all(&destination).expect("create destination directory");
        fs::write(source.join("a.txt"), "a").expect("create file a");
        fs::write(source.join("b.txt"), "b").expect("create file b");

        let progress = TestProgress::new();
        let mut journal = Vec::new();
        move_entries_with_progress(
            vec![
                (source.join("a.txt"), ConflictAction::Fail),
                (source.join("b.txt"), ConflictAction::Fail),
            ],
            destination.clone(),
            &progress,
            &mut journal,
        )
        .expect("move entries");
        assert_eq!(journal.len(), 2);
        assert!(destination.join("a.txt").exists());
        assert!(!source.join("a.txt").exists());

        let undo_progress = TestProgress::new();
        let (count, redo) =
            execute_undo(Operation::Move { transfers: journal }, &undo_progress)
                .expect("undo move");
        assert_eq!(count, 2);
        assert!(source.join("a.txt").exists());
        assert!(source.join("b.txt").exists());
        assert!(!destination.join("a.txt").exists());

        let redo_progress = TestProgress::new();
        let redo = redo.expect("undo reports a redo operation");
        let (count, undone) =
            execute_redo(redo, &redo_progress).expect("redo move");
        assert_eq!(count, 2);
        assert!(destination.join("a.txt").exists());
        assert!(destination.join("b.txt").exists());
        assert!(!source.join("a.txt").exists());
        assert!(undone.is_some());

        // An entry deleted after the move is skipped; the survivor still reverts
        // and the redo stack only keeps the pair that actually moved back.
        fs::remove_file(destination.join("a.txt")).expect("delete one moved file");
        let journal = match undone.expect("redo reports an undo operation") {
            Operation::Move { transfers } => transfers,
            other => panic!("expected a move operation, got {other:?}"),
        };
        let progress = TestProgress::new();
        let (count, redo) =
            execute_undo(Operation::Move { transfers: journal }, &progress)
                .expect("undo move with a missing entry");
        assert_eq!(count, 1);
        assert!(source.join("b.txt").exists());
        assert!(!source.join("a.txt").exists());
        match redo.expect("redo keeps only the reverted pair") {
            Operation::Move { transfers } => assert_eq!(transfers.len(), 1),
            other => panic!("expected a move operation, got {other:?}"),
        }

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn undo_and_redo_revert_a_rename() {
        let directory =
            std::env::temp_dir().join(format!("dae-undo-rename-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        let file = directory.join("original.txt");
        fs::write(&file, "content").expect("create file");

        let new_name = "renamed.txt";
        rename_entry_sync(file.clone(), new_name.into()).expect("rename file");
        let renamed = directory.join(new_name);
        assert!(renamed.exists());

        let from = path_to_string(&file);
        let to = renamed_path(&from, new_name).expect("derive renamed path");
        let progress = TestProgress::new();
        let (count, redo) =
            execute_undo(Operation::Rename { from, to }, &progress).expect("undo rename");
        assert_eq!(count, 1);
        assert!(file.exists());
        assert!(!renamed.exists());

        let (count, undone) =
            execute_redo(redo.expect("undo reports a redo operation"), &progress)
                .expect("redo rename");
        assert_eq!(count, 1);
        assert!(renamed.exists());
        assert!(!file.exists());
        assert!(undone.is_some());

        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
