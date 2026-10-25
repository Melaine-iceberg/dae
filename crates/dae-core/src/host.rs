//! The seam between the code that *knows* things and the code that *draws* them.
//!
//! A module that has to tell the UI something takes `&dyn Host`. It never takes
//! a `tauri::AppHandle`, a webview, a window or a QML object. That is the whole
//! contract — one trait, one event enum — and it is the only thing in this
//! crate that is allowed to be shaped by the fact that a UI exists at all.
//!
//! # Why the event carries no `Serialize` and no `specta::Type`
//!
//! The wire format belongs to the shell. The Tauri shell derives
//! `tauri_specta::Event` on its own struct and keeps generating
//! `src/bindings.ts` from it; the Qt shell will turn the same variant into a
//! queued signal. Deriving serialization here would let one shell's transport
//! leak back into the domain — which is exactly how this tree ended up with a
//! columnar IPC codec sitting inside the directory-listing pipeline
//! (`file_system/entry_codec.rs`, 1,510 lines, tuned for a JS consumer).
//!
//! # Why [`Host::emit`] is synchronous and returns nothing
//!
//! Its callers are OS watcher threads: `system_accent` and `system_theme` block
//! on a registry notification or a portal read for the life of the process. An
//! `async fn` would force those threads to become an executor, and a
//! `Result` would make every watcher invent a policy for "the window is gone".
//! The shell is expected to post and return — Tauri's `Emitter` already does,
//! and Qt's `qt_thread().queue(...)` will.
//!
//! # Why the enum is exhaustive
//!
//! Adding a variant must break every shell adapter at compile time. Marking
//! this `#[non_exhaustive]` would let the Tauri shell keep compiling while
//! silently dropping the new event — the failure mode this migration exists to
//! leave behind.

/// One thing the UI has to be told.
///
/// Deliberately flat and payload-only: an event says what moved, never how it
/// should look. Everything derived from a value (ink, hover, selection) is the
/// shell's business, as `src/lib/system-accent.ts` already assumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The OS light/dark preference moved. `Some(true)` dark, `Some(false)`
    /// light, `None` where the platform publishes no preference.
    SystemThemeChanged(Option<bool>),
    /// The OS accent colour moved, as the `#rrggbb` the CSS seam takes. `None`
    /// where the platform has no accent setting.
    SystemAccentChanged(Option<String>),
}

/// What a module needs in order to report. Implemented once per shell.
pub trait Host: Send + Sync + 'static {
    /// Deliver `event` to the UI. Must not block: called from OS watcher
    /// threads and, later, from file-system worker threads.
    fn emit(&self, event: Event);
}
