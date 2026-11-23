//! Progress for the long file operations: copy, move, delete, compress, extract,
//! and the properties dialog's size walk.
//!
//! This is the *shape* of what the UI is told, not how it is told. The throttling
//! (one event per 60 ms unless something finished) and the event name belong to
//! the shell — a webview event and a Qt queued signal want different rates, and
//! neither belongs in a crate that also runs under `cargo test` with no window.
//!
//! # Why the enums carry `serde` here
//!
//! They are part of the value vocabulary [`crate::types`] establishes, so they
//! keep the same derives as the rest of it. [`crate::host::Event`] itself stays
//! derive-free — the distinction is "a value that happens to be serialized"
//! versus "the transport", and the transport is the shell's business.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationKind {
    Copy,
    Move,
    Delete,
    Compress,
    Extract,
    Properties,
}

impl FileOperationKind {
    /// Stable code naming the operation, e.g. `"delete"`.
    ///
    /// The shells need it because the *unit* of the reported progress depends
    /// on the operation and nothing in the numbers says which: a delete counts
    /// entries, a compression counts bytes. A UI that prints "12/40" for the
    /// first must print a percentage for the second, so the kind travels with
    /// the progress rather than being inferred from its magnitude.
    pub fn code(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Delete => "delete",
            Self::Compress => "compress",
            Self::Extract => "extract",
            Self::Properties => "properties",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationPhase {
    Preparing,
    Running,
    // Only synthesized by the frontend; kept so the exported union covers all states.
    #[allow(dead_code)]
    Completed,
}

#[cfg(test)]
mod tests {
    use super::FileOperationKind;
    use std::collections::HashSet;

    /// The codes are the key the shells switch on to pick a unit and a
    /// wording, so two operations sharing one would make a delete print a
    /// compression's bytes. Every variant is listed here on purpose: adding
    /// one has to be a deliberate decision about what it means.
    #[test]
    fn every_kind_has_its_own_code() {
        let kinds = [
            FileOperationKind::Copy,
            FileOperationKind::Move,
            FileOperationKind::Delete,
            FileOperationKind::Compress,
            FileOperationKind::Extract,
            FileOperationKind::Properties,
        ];

        let codes: HashSet<&str> = kinds.iter().map(|kind| kind.code()).collect();
        assert_eq!(codes.len(), kinds.len(), "codes must be distinct: {codes:?}");
        assert_eq!(FileOperationKind::Delete.code(), "delete");
        assert_eq!(FileOperationKind::Properties.code(), "properties");
    }
}
