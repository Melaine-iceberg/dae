use super::directory::path_to_string;
use serde::Serialize;
use specta::Type;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::Emitter;

const FILE_OPERATION_PROGRESS_EVENT: &str = "explorer-file-operation-progress";
const FILE_OPERATION_PROGRESS_INTERVAL: Duration = Duration::from_millis(60);

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationProgress {
    pub operation_id: String,
    pub operation: String,
    pub phase: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub current_path: Option<String>,
}

pub(super) fn emit_preparing(app: &tauri::AppHandle, operation_id: &str, operation: &str) {
    emit_file_operation_progress(app, operation_id, operation, "preparing", 0, None, None);
}

pub(super) trait FileOperationProgressReporterTrait {
    fn start(&mut self, total: u64);
    fn begin_entry(&mut self, path: &Path);
    fn advance(&mut self, path: &Path) {
        self.advance_by(1, path);
    }
    fn advance_by(&mut self, units: u64, path: &Path);
    fn finish(&mut self);
}

pub(super) struct FileOperationProgressReporter {
    app: tauri::AppHandle,
    operation_id: String,
    operation: &'static str,
    completed: u64,
    total: u64,
    last_emit: Option<Instant>,
}

impl FileOperationProgressReporter {
    pub(super) fn new(
        app: tauri::AppHandle,
        operation_id: String,
        operation: &'static str,
    ) -> Self {
        Self {
            app,
            operation_id,
            operation,
            completed: 0,
            total: 0,
            last_emit: None,
        }
    }

    fn emit(&mut self, path: Option<&Path>, force: bool) {
        if !force
            && self
                .last_emit
                .is_some_and(|last_emit| last_emit.elapsed() < FILE_OPERATION_PROGRESS_INTERVAL)
        {
            return;
        }

        emit_file_operation_progress(
            &self.app,
            &self.operation_id,
            self.operation,
            "running",
            self.completed,
            Some(self.total),
            path.map(path_to_string),
        );
        self.last_emit = Some(Instant::now());
    }
}

impl FileOperationProgressReporterTrait for FileOperationProgressReporter {
    fn start(&mut self, total: u64) {
        self.total = total;
        self.emit(None, true);
    }

    fn begin_entry(&mut self, path: &Path) {
        self.emit(Some(path), self.completed == 0);
    }

    fn advance_by(&mut self, units: u64, path: &Path) {
        self.completed = self.completed.saturating_add(units).min(self.total);
        self.emit(Some(path), self.completed == self.total);
    }

    fn finish(&mut self) {
        self.completed = self.total;
        self.emit(None, true);
    }
}

fn emit_file_operation_progress(
    app: &tauri::AppHandle,
    operation_id: &str,
    operation: &str,
    phase: &str,
    completed: u64,
    total: Option<u64>,
    current_path: Option<String>,
) {
    let _ = app.emit(
        FILE_OPERATION_PROGRESS_EVENT,
        FileOperationProgress {
            operation_id: operation_id.to_owned(),
            operation: operation.to_owned(),
            phase: phase.to_owned(),
            completed,
            total,
            current_path,
        },
    );
}
