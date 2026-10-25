//! Throttled delivery of `dae_core::progress` to the webview.
//!
//! The types live in `dae_core` (they are part of the IPC contract), the event
//! goes through [`dae_core::host::Host`], and what stays here is the part that
//! is genuinely shell-specific: how often a webview wants to be told. A Qt
//! queued signal would pick its own rate; this one is 60 ms unless something
//! crossed a boundary, because the frontend recomputes a progress bar on every
//! event and a per-entry rate on a 35,000-entry copy is not a rate at all.
//!
//! [`FileOperationProgress`] is the wire type — it owns the
//! `explorer-file-operation-progress` name — but nothing in this module emits it
//! directly any more: [`TauriHost`] does, from the core event.
//!
//! [`TauriHost`]: crate::host::TauriHost

use super::types::path_to_string;
use dae_core::host::{Event as UiEvent, Host};
use serde::Serialize;
use specta::Type;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use dae_core::progress::{FileOperationKind, FileOperationPhase};

const FILE_OPERATION_PROGRESS_INTERVAL: Duration = Duration::from_millis(60);

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

pub(super) fn emit_preparing(
    host: &dyn Host,
    operation_id: &str,
    operation: FileOperationKind,
) {
    emit_file_operation_progress(
        host,
        operation_id,
        operation,
        FileOperationPhase::Preparing,
        0,
        None,
        None,
    );
}

pub trait FileOperationProgressReporterTrait: Send + Sync {
    fn start(&self, total: u64);
    fn begin_entry(&self, path: &Path);
    fn advance(&self, path: &Path) {
        self.advance_by(1, path);
    }
    fn advance_by(&self, units: u64, path: &Path);
    fn finish(&self);
}

struct ReporterState {
    completed: u64,
    total: u64,
    last_emit: Option<Instant>,
}

pub(super) struct FileOperationProgressReporter {
    /// The seam, not an `AppHandle`: a transfer thread outlives every window and
    /// has no business holding one. One `Arc` per operation, cloned into the
    /// worker.
    host: Arc<dyn Host>,
    operation_id: String,
    operation: FileOperationKind,
    state: Mutex<ReporterState>,
}

impl FileOperationProgressReporter {
    pub(super) fn new(
        host: Arc<dyn Host>,
        operation_id: String,
        operation: FileOperationKind,
    ) -> Self {
        Self {
            host,
            operation_id,
            operation,
            state: Mutex::new(ReporterState {
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
            self.host.as_ref(),
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
    host: &dyn Host,
    operation_id: &str,
    operation: FileOperationKind,
    phase: FileOperationPhase,
    completed: u64,
    total: Option<u64>,
    current_path: Option<String>,
) {
    host.emit(UiEvent::FileOperationProgress {
        operation_id: operation_id.to_owned(),
        operation,
        phase,
        completed,
        total,
        current_path,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testing::RecordingHost;

    /// Every emit in this sequence is forced (first entry, then finish), so the
    /// assertions do not depend on how long the test machine took between calls.
    #[test]
    fn forces_the_boundaries_and_reports_them_in_order() {
        let recorder = Arc::new(RecordingHost::default());
        let host: Arc<dyn Host> = recorder.clone();
        let reporter =
            FileOperationProgressReporter::new(host, "op-1".to_owned(), FileOperationKind::Copy);

        reporter.start(3);
        reporter.begin_entry(Path::new("a.txt"));
        reporter.finish();

        let events = recorder.take();
        assert_eq!(events.len(), 3, "start, first entry and finish all force");

        let UiEvent::FileOperationProgress {
            phase,
            completed,
            total,
            current_path,
            operation,
            operation_id,
        } = &events[0]
        else {
            panic!("expected a progress event, got {:?}", events[0]);
        };
        assert_eq!(*phase, FileOperationPhase::Running);
        assert_eq!(*operation, FileOperationKind::Copy);
        assert_eq!(operation_id, "op-1");
        assert_eq!(*completed, 0);
        assert_eq!(*total, Some(3));
        assert_eq!(*current_path, None);

        // The first entry is reported even though no time has passed, because a
        // bar that sits at zero until 60 ms is a bar that looks stuck.
        let UiEvent::FileOperationProgress { current_path, .. } = &events[1] else {
            panic!("expected a progress event");
        };
        assert!(
            current_path.as_deref().is_some_and(|p| p.ends_with("a.txt")),
            "got {current_path:?}"
        );

        // `finish` clamps to the total rather than trusting the last advance.
        let UiEvent::FileOperationProgress { completed, total, .. } = &events[2] else {
            panic!("expected a progress event");
        };
        assert_eq!(*completed, 3);
        assert_eq!(*total, Some(3));
    }

    /// Between boundaries the 60 ms window applies: a second advance that
    /// crosses no boundary is dropped. This is the only place the throttle is
    /// pinned — it is what keeps a 35,000-entry copy from posting 35,000 events.
    #[test]
    fn an_unforced_tick_inside_the_window_is_dropped() {
        let recorder = Arc::new(RecordingHost::default());
        let host: Arc<dyn Host> = recorder.clone();
        let reporter =
            FileOperationProgressReporter::new(host, "op-2".to_owned(), FileOperationKind::Move);

        reporter.start(1000);
        reporter.advance_by(1, Path::new("a.txt"));
        reporter.advance_by(1, Path::new("b.txt"));

        // `start` forced one event; both advances are inside the window and
        // cross no boundary, so nothing else arrives.
        assert_eq!(recorder.take().len(), 1);
    }

    #[test]
    fn preparing_reports_no_total() {
        let recorder = Arc::new(RecordingHost::default());
        let host: Arc<dyn Host> = recorder.clone();

        emit_preparing(host.as_ref(), "op-3", FileOperationKind::Extract);

        let events = recorder.take();
        let [
            UiEvent::FileOperationProgress {
                phase,
                completed,
                total,
                ..
            },
        ] = events.as_slice()
        else {
            panic!("expected exactly one progress event, got {events:?}");
        };
        assert_eq!(*phase, FileOperationPhase::Preparing);
        assert_eq!(*completed, 0);
        assert_eq!(*total, None);
    }
}
