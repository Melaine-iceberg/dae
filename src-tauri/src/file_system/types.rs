//! Temporary shim.
//!
//! These types moved to `dae-core` so the incoming Qt Quick shell can share
//! them without linking Tauri. The module path stays so that the ~90 existing
//! `crate::file_system::types::…` call sites do not have to move in the same
//! commit; delete this file (and the `pub mod types;` line in
//! `file_system.rs`) once they are repointed at `dae_core::types`.
pub use dae_core::types::*;
