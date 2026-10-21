//! The Tauri side of the file system: command adapters, watchers, and the
//! UI-facing state. The platform-neutral backends live in `dae-core` and are
//! re-exported here so existing paths keep working.

pub mod archive;
pub mod commands;
pub mod directory_size;
pub mod git;
pub mod hashing;
pub mod listing;
pub mod media_meta;
pub mod network;
pub mod open_with;
pub mod prefetch;
pub mod preview;
pub mod progress;
pub mod recents;
pub mod search;
pub mod sidebar;
pub mod spaces;
pub mod system_files;
pub mod trash_view;
pub mod undo;
pub mod volume_monitor;
pub mod watch;

pub use dae_core::file_system::{
    cloud, connections, entry_codec, error, local, smb, transfer, types, vfs,
};
#[cfg(target_os = "macos")]
pub use dae_core::file_system::macos_trash;

#[cfg(test)]
pub(crate) mod test_support;

pub use search::FileSearchState;
pub use undo::UndoRedoState;
pub use watch::{DirectoryChanged, DirectoryWatcher};
