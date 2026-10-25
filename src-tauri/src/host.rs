//! The Tauri implementation of [`dae_core::host::Host`] — the one place a
//! engine-agnostic event is turned back into a webview event.
//!
//! This file is meant to stay this small. Everything that grows here is a sign
//! that an engine detail is leaking into the core instead of being adapted at
//! the boundary.

use dae_core::host::{Event, Host};
use tauri::AppHandle;
use tauri_specta::Event as _;

/// Reports through the same `tauri_specta::Event` structs the frontend's
/// bindings are generated from, so the wire names (`system-theme-changed`,
/// `system-accent-changed`, …) do not move during the migration: the current
/// React shell keeps receiving exactly what it received before.
pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Host for TauriHost {
    fn emit(&self, event: Event) {
        // Exhaustive on purpose — see the enum's docs. A new variant must fail
        // to compile here rather than disappear on the way to the webview.
        match event {
            Event::SystemThemeChanged(dark) => {
                let _ = crate::system_theme::SystemThemeChanged(dark).emit(&self.app);
            }
            Event::SystemAccentChanged(accent) => {
                let _ = crate::system_accent::SystemAccentChanged(accent).emit(&self.app);
            }
        }
    }
}

/// Test doubles for modules that report through the seam.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Mutex;

    /// Records instead of delivering, so a test can assert that a module goes
    /// through the seam and hands its reading over unchanged.
    #[derive(Default)]
    pub(crate) struct RecordingHost {
        events: Mutex<Vec<Event>>,
    }

    impl RecordingHost {
        pub(crate) fn take(&self) -> Vec<Event> {
            std::mem::take(&mut *self.events.lock().expect("poisoned"))
        }
    }

    impl Host for RecordingHost {
        fn emit(&self, event: Event) {
            self.events.lock().expect("poisoned").push(event);
        }
    }
}
