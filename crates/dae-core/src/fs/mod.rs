//! File system domain logic, shared by both shells.
//!
//! The layout mirrors the old `src-tauri/src/file_system/` tree so that each
//! `git mv` in the migration stays reviewable. Modules keep their names.
//!
//! `error` and `types` are re-exported from the crate root: they were the first
//! things to move, and the module-relative spelling (`super::types::…`) that the
//! rest of the tree already uses keeps working.
//!
//! `entry_codec` is a deliberate half-way house: [`entry_codec::ListingRow`],
//! `into_entry` and `base_of` are the enumerator's row model and stay here,
//! while the packet writer (`pack_with_header_flags`, `HEADER_FLAG_FINAL`) only
//! serves the Tauri webview. When the Qt shell lands, the packing half and the
//! shell's `listing_packets` channel target go away together.

pub use crate::{error, types};

pub mod archive;
pub mod atomic;
pub mod clipboard;
pub mod cloud;
pub mod connections;
pub mod directory_size;
pub mod entry_codec;
pub mod git;
pub mod hashing;
pub mod icons;
pub mod known_hosts;
pub mod listing;
pub mod local;
#[cfg(target_os = "macos")]
pub mod macos_trash;
pub mod media_meta;
pub mod open_with;
pub mod prefetch;
pub mod preview;
pub mod progress;
pub mod recents;
pub mod search;
pub mod service;
pub mod sftp;
pub mod shortcuts;
pub mod sidebar;
pub mod smb;
pub mod spaces;
pub mod thumbnails;
pub mod transfer;
pub mod trash_view;
pub mod undo;
pub mod vfs;
pub mod watch;
#[cfg(windows)]
pub mod windows_image;

#[cfg(test)]
pub(crate) mod test_support;
