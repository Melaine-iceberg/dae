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

pub mod config;
pub mod format;
pub mod ime;
pub mod material;
pub mod model;
pub mod window;

/// Process-wide setup that must run before the QML engine creates a window.
pub fn init() {
    window::enable_alpha();
}
