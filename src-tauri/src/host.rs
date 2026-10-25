//! The Tauri implementation of [`dae_core::host::Host`] — the one place a
//! engine-agnostic event is turned back into a webview event.
//!
//! This file is meant to stay this small. Everything that grows here is a sign
//! that an engine detail is leaking into the core instead of being adapted at
//! the boundary.

use dae_core::host::{Event, Host};
use std::sync::Arc;
use tauri::AppHandle;
use tauri_specta::Event as _;

/// Wraps an `AppHandle` as the seam.
///
/// One `Arc` per operation, not per emit: the reporter/emitter keeps it for its
/// lifetime. `Host` cannot be implemented on `AppHandle` itself — both the trait
/// and the type are foreign to this crate — so this newtype is the price of the
/// orphan rule, and it is a cheap one.
pub(crate) fn of(app: &AppHandle) -> Arc<dyn Host> {
    Arc::new(TauriHost::new(app.clone()))
}

/// Reports through the same `tauri_specta::Event` structs the frontend's
/// bindings are generated from, so the wire names (`system-theme-changed`,
/// `system-accent-changed`, …) do not move during the migration: the current
/// React shell keeps receiving exactly what it received before.
pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Host for TauriHost {
    fn emit(&self, event: Event) {
        // Exhaustive on purpose — see the enum's docs. A new variant must fail
        // to compile here rather than disappear on the way to the webview.
        match event {
            Event::SystemThemeChanged(dark) => {
                let _ = crate::system_theme::SystemThemeChanged(dark).emit(&self.app);
            }
            Event::SystemAccentChanged(accent) => {
                let _ = crate::system_accent::SystemAccentChanged(accent).emit(&self.app);
            }
            Event::FileOperationProgress {
                operation_id,
                operation,
                phase,
                completed,
                total,
                current_path,
            } => {
                // Field-by-field on purpose: a field added to the wire type stops
                // this compiling instead of silently staying at its default.
                let _ = crate::file_system::ipc_events::FileOperationProgress {
                    operation_id,
                    operation,
                    phase,
                    completed,
                    total,
                    current_path,
                }
                .emit(&self.app);
            }
            Event::DirectoryChanged { path, names } => {
                let _ = crate::file_system::ipc_events::DirectoryChanged { path, names }
                    .emit(&self.app);
            }
            Event::DirectoryEntries {
                stream_id,
                path,
                entries,
                done,
            } => {
                let _ = crate::file_system::ipc_events::DirectoryEntriesBatch {
                    stream_id,
                    path,
                    entries,
                    done,
                }
                .emit(&self.app);
            }
            Event::UndoRedoChanged { can_undo, can_redo } => {
                let _ = crate::file_system::ipc_events::UndoRedoChanged { can_undo, can_redo }
                    .emit(&self.app);
            }
            Event::DirectorySizeProgress {
                operation_id,
                path,
                size,
                completed,
            } => {
                let _ = crate::file_system::ipc_events::DirectorySizeProgress {
                    operation_id,
                    path,
                    size,
                    completed,
                }
                .emit(&self.app);
            }
            Event::FileHashProgress {
                operation_id,
                path,
                bytes_read,
                total_bytes,
                completed,
                digests,
                error,
            } => {
                let _ = crate::file_system::ipc_events::FileHashProgress {
                    operation_id,
                    path,
                    bytes_read,
                    total_bytes,
                    completed,
                    digests,
                    error,
                }
                .emit(&self.app);
            }
        }
    }
}

/// Test doubles live in the core so moved modules can keep using them; this
/// re-export keeps the shell's own tests on the same spelling.
#[cfg(test)]
pub(crate) mod testing {
    pub(crate) use dae_core::host::testing::RecordingHost;
}
