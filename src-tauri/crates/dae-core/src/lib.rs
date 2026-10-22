//! Platform-neutral file system core.
//!
//! This crate holds the backends, path handling, transfer engine, and the
//! account stores. It deliberately has no dependency on Tauri: the Tauri shell
//! and the Electron N-API addon both wrap it.

pub mod file_system;
pub mod file_icons;
// The Linux thumbnailers are compiled under `test` too, so their
// host-independent pieces type-check on the Windows machine the addon is
// developed on. Only Linux builds reach them from `thumbnail::render`.
#[cfg(any(target_os = "linux", test))]
pub mod desktop_thumbnails;
pub mod shell_commands;
pub mod xdg;
