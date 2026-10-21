//! Platform-neutral file system core.
//!
//! This crate holds the backends, path handling, transfer engine, and the
//! account stores. It deliberately has no dependency on Tauri: the Tauri shell
//! and the Electron N-API addon both wrap it.

pub mod file_system;
