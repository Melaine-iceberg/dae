//! File system domain logic, shared by both shells.
//!
//! The layout mirrors the old `src-tauri/src/file_system/` tree so that each
//! `git mv` in the migration stays reviewable. Modules keep their names.
//!
//! `entry_codec` is a deliberate half-way house: [`entry_codec::ListingRow`],
//! `into_entry` and `base_of` are the enumerator's row model and stay here,
//! while the packet writer (`pack_with_header_flags`, `HEADER_FLAG_FINAL`) only
//! serves the Tauri webview. When the Qt shell lands, the packing half and the
//! shell's `listing_packets` channel target go away together.

pub mod atomic;
pub mod entry_codec;
pub mod listing;
