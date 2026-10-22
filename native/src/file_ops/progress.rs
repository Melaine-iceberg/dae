//! File-operation progress, emitted on the same event names the Tauri shell
//! used so the renderer's existing progress panel keeps working unchanged.

pub use dae_core::file_system::progress::FileOperationProgressReporterTrait;
use dae_core::file_system::types::path_to_string;
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant};

const FILE_OPERATION_PROGRESS_INTERVAL: Duration = Duration::from_millis(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum FileOperationKind {
    Copy,
    Move,
    Delete,
    Compress,
    Extract,
    Properties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationPhase {
    Preparing,
    Running,
    #[allow(dead_code)]
    Completed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileOperationProgress {
    operation_id: String,
    operation: FileOperationKind,
    phase: FileOperationPhase,
    completed: u64,
    total: Option<u64>,
    current_path: Option<String>,
}

pub(crate) fn emit_preparing(
    operation_id: &str,
    operation: FileOperationKind,
) {
    emit_file_operation_progress(
        operation_id,
        operation,
        FileOperationPhase::Preparing,
        0,
        None,
        None,
    );
}

struct ReporterState {
    completed: u64,
    total: u64,
    last_emit: Option<Instant>,
}

pub(crate) struct FileOperationProgressReporter {
    operation_id: String,
    operation: FileOperationKind,
    state: std::sync::Mutex<ReporterState>,
}

impl FileOperationProgressReporter {
    pub(crate) fn new(operation_id: String, operation: FileOperationKind) -> Self {
        Self {
            operation_id,
            operation,
            state: std::sync::Mutex::new(ReporterState {
                completed: 0,
                total: 0,
                last_emit: None,
            }),
        }
    }

    fn emit(&self, path: Option<&Path>, force: bool) {
        let mut state = self.state.lock().expect("progress reporter lock poisoned");

        if !force
            && state
                .last_emit
                .is_some_and(|last_emit| last_emit.elapsed() < FILE_OPERATION_PROGRESS_INTERVAL)
        {
            return;
        }

        emit_file_operation_progress(
            &self.operation_id,
            self.operation,
            FileOperationPhase::Running,
            state.completed.min(state.total),
            Some(state.total),
            path.map(path_to_string),
        );
        state.last_emit = Some(Instant::now());
    }
}

impl FileOperationProgressReporterTrait for FileOperationProgressReporter {
    fn start(&self, total: u64) {
        let mut state = self.state.lock().expect("progress reporter lock poisoned");
        state.total = total;
        drop(state);
        self.emit(None, true);
    }

    fn begin_entry(&self, path: &Path) {
        let is_first = {
            let state = self.state.lock().expect("progress reporter lock poisoned");
            state.completed == 0
        };
        self.emit(Some(path), is_first);
    }

    fn advance_by(&self, units: u64, path: &Path) {
        let is_done = {
            let mut state = self.state.lock().expect("progress reporter lock poisoned");
            state.completed = state.completed.saturating_add(units);
            state.completed >= state.total
        };
        self.emit(Some(path), is_done);
    }

    fn finish(&self) {
        {
            let mut state = self.state.lock().expect("progress reporter lock poisoned");
            state.completed = state.total;
        }
        self.emit(None, true);
    }
}

fn emit_file_operation_progress(
    operation_id: &str,
    operation: FileOperationKind,
    phase: FileOperationPhase,
    completed: u64,
    total: Option<u64>,
    current_path: Option<String>,
) {
    let payload = FileOperationProgress {
        operation_id: operation_id.to_owned(),
        operation,
        phase,
        completed,
        total,
        current_path,
    };
    if let Ok(payload) = serde_json::to_string(&payload) {
        crate::emit_event("explorer-file-operation-progress", payload);
    }
}
