//! Engine-agnostic core of dae.
//!
//! Two shells link this crate: the Tauri shell in `src-tauri` (still shipped)
//! and the Qt Quick shell that replaces it (`docs/decisions/engine-qt-quick.md`).
//! That is the whole rule — **nothing here may know how the UI is drawn**. A
//! `tauri::` import, a webview handle, a QML type or a serialized IPC packet in
//! this crate would silently re-decide the engine, which is exactly the thing
//! the migration is trying to undo.
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
pub mod hashing;
pub mod host;
pub mod paths;
pub mod progress;
pub mod pty;
pub mod settings;
pub mod types;
pub mod xdg;

pub use error::FileSystemError;
