//! Progress for the long file operations: copy, move, delete, compress, extract,
//! and the properties dialog's size walk.
//!
//! This is the *shape* of what the UI is told, not how it is told. The throttling
//! (one event per 60 ms unless something finished) and the event name belong to
//! the shell — a webview event and a Qt queued signal want different rates, and
//! neither belongs in a crate that also runs under `cargo test` with no window.
//!
//! # Why the enums carry `serde`/`specta` here
//!
//! [`crate::types`] is already the IPC contract (the Tauri shell still exports
//! its TypeScript bindings from these derives), and these two enums are part of
//! it. [`crate::host::Event`] itself stays derive-free — the distinction is
//! "a value that happens to be serialized" versus "the transport".

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationKind {
    Copy,
    Move,
    Delete,
    Compress,
    Extract,
    Properties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationPhase {
    Preparing,
    Running,
    // Only synthesized by the frontend; kept so the exported union covers all states.
    #[allow(dead_code)]
    Completed,
}
