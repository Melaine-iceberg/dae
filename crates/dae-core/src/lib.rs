//! Engine-agnostic core of dae.
//!
//! One shell links this crate: the Qt Quick shell in `apps/dae` + `crates/dae-qt`
//! (`docs/decisions/engine-qt-quick.md`). The Tauri shell that preceded it was
//! deleted on 2026-11-01, which is why the rule below survived the migration
//! that produced it. **Nothing here may know how the UI is drawn**: a `tauri::`
//! import, a webview handle, a QML type or a serialized IPC packet in this crate
//! would put the engine back in the domain layer.
//!
//! # How code gets here
//!
//! By strangling, one module at a time, in an order that never leaves the tree
//! unbuildable:
//!
//! 1. `git mv` the module into this crate.
//! 2. Leave a `pub use dae_core::<module>::*;` shim at the old path, so the
//!    ~43k lines of call sites do not have to change in the same commit.
//! 3. Delete the shim once the last caller is gone.
//!
//! The order matters: the *contract* (types, errors) comes first, then the
//! modules with no Tauri references beyond injected handles, then the ones that
//! actually emit events — those need [`host`] to exist before they can move,
//! because emitting is the one thing that is genuinely engine-specific.
pub mod error;
pub mod fs;
mod ghostty_vt;
pub mod hashing;
pub mod host;
pub mod package;
pub mod paths;
pub mod progress;
pub mod pty;
pub mod settings;
pub mod types;
pub mod update;
pub mod xdg;

pub use error::FileSystemError;
