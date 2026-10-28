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
//! # Platforms
//!
//! The crate compiles on Windows, Linux and macOS. The *implementation* of the
//! native chrome is still Windows-only, because all four gates were measured
//! there, but the C++ seam no longer is: `cpp/window.h` and `cpp/ime.h` keep
//! the same entry points on every platform and do only the portable part
//! elsewhere.
//!
//! | | Windows | Linux / macOS |
//! |---|---|---|
//! | frame, move, resize | Win32 hit test (`WM_NCHITTEST`) | frameless Qt window; QML `startSystemMove` / `startSystemResize` |
//! | backdrop | DWM Mica/Acrylic | `Material::None` (solid chrome) |
//! | IME focus cancel | `CPS_CANCEL` guard (gate 2) | not installed; no measured equivalent |
//! | `dae://` registration | registry write | not implemented yet; the single-instance socket still works |
//!
//! A Linux/macOS release artifact is therefore a working shell with the
//! platform's own chrome, not a pixel-for-pixel copy of the Windows one. Each
//! native behaviour above lands on those platforms with its own measured gate
//! rather than as an untested guess.

pub mod activation;
pub mod appearance;
pub mod config;
mod default_manager;
mod errors;
pub mod format;
pub mod i18n;
pub mod icons;
pub mod ime;
pub mod material;
pub mod model;
pub mod open_with_model;
pub mod overview_model;
pub mod platform;
pub mod preferences;
pub mod preview;
mod preview_markup;
pub mod properties;
mod runtime;
pub mod search_model;
pub mod shell;
pub mod shell_command_model;
pub mod sidebar_model;
pub mod sidebar_tree_model;
pub mod spaces_model;
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
    if let Err(error) = dae_core::settings::init() {
        eprintln!("dae: settings initialization failed: {error}");
    } else {
        appearance::migrate_legacy();
    }
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
