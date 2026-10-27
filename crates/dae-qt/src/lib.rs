//! Qt Quick shell bridge.
//!
//! This crate is the Qt half of the engine seam described in
//! `docs/decisions/engine-qt-quick.md` §4. It carries only the code that was
//! proven by gates 1–4 and is now product infrastructure:
//!
//! * [`model::DirectoryListModel`] — a `QAbstractListModel` fed by the real
//!   `dae_core::fs::listing` stream (gate 1).
//! * [`window`] — frameless chrome, Mica/Acrylic and native hit testing, with
//!   one state record per HWND so a second window is not a special case
//!   (gates 3 and 4).
//! * [`ime`] — the `CPS_CANCEL` guard that runs before Qt moves focus, so a
//!   composition is cancelled instead of being committed as raw pinyin
//!   (gate 2).
//!
//! Nothing here is a benchmark harness: the spike clocks, JSON result writers,
//! synthetic scroll workload and scripted input live in `bench/qt-*` only.
//!
//! The native half is Windows-first (Win32, DWM, IMM32), because all four gates
//! were measured there. The Linux/macOS chrome and input backends land with
//! their own measured gates rather than as an untested guess.

pub mod activation;
pub mod config;
mod errors;
pub mod format;
pub mod icons;
pub mod ime;
pub mod material;
pub mod model;
pub mod open_with_model;
pub mod platform;
pub mod preferences;
pub mod preview;
mod runtime;
pub mod search_model;
pub mod shell;
pub mod sidebar_model;
mod system_watch;
pub mod terminal;
pub mod trash_model;
pub mod updater;
pub mod window;
pub mod workspace;

/// Process-wide setup that must run before the QML engine creates a window.
pub fn init() {
    window::enable_diagnostics();
    window::enable_alpha();
    terminal::register_item();
}

/// Loads the file-backed connection and cloud-account registries once. Both
/// live in `dae-core` and are shared with the Tauri shell, so the Qt shell
/// reads the same `%APPDATA%` documents instead of keeping its own list.
pub(crate) fn init_stores() {
    use std::sync::Once;

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Err(error) = dae_core::fs::connections::init() {
            eprintln!("dae: connection store initialization failed: {error}");
        }
        if let Err(error) = dae_core::fs::cloud::accounts::init() {
            eprintln!("dae: cloud account store initialization failed: {error}");
        }
    });
}
