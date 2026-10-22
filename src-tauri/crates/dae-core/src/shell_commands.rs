//! Shell-command formats shared with the icon and thumbnail backends.
//!
//! The Tauri shell hosts its own command menu here; what dae-core owns is the
//! parser for the desktop-entry dialect, because the Linux icon theme and
//! `.thumbnailer` lookups read the same `Key=value` files.

pub mod desktop_entry;
