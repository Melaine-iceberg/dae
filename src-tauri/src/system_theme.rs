//! The OS light/dark preference, feeding the frontend's theme seam
//! (see `src/lib/theme.ts`).
//!
//! "Follow system" is one boolean — is the OS drawn dark? — and on Windows and
//! macOS the WebView already answers it correctly through
//! `prefers-color-scheme` (WebView2 and WKWebView both follow the system
//! appearance), so there is nothing to add there. On Linux there is: this
//! app's WebView is WebKitGTK 4.1, which is GTK 3, and a GTK 3 WebView infers
//! `prefers-color-scheme` from the *GTK theme* rather than from the desktop's
//! colour-scheme preference. A user can leave the GTK theme pinned to a dark
//! variant (a very common `gtk-theme-name=…-dark` setup) while the desktop's
//! own Light/Dark toggle says light, and this shell then follows the theme
//! while every GTK 4 app follows the toggle. The freedesktop settings portal
//! publishes the same value the desktop's toggle writes, so reading it is what
//! makes "follow system" agree with the desktop.
//!
//! Two things reach the UI and nothing else:
//!
//!   * `get_system_theme()` — `Some(true)` dark, `Some(false)` light, or `None`
//!     where the platform publishes no preference (the frontend then keeps its
//!     `prefers-color-scheme` reading, which is the right answer everywhere
//!     this module reads nothing);
//!   * `SystemThemeChanged` — the same payload, every time it moves.
//!
//! The second one travels through [`dae_core::host::Host`], not through
//! `tauri::AppHandle`: the watcher thread knows nothing about a webview, which
//! is what lets this module move to the Qt shell unchanged.
//!
//! Each backend answers two questions (`read`, `watch`) and `watch` blocks its
//! thread for the life of the app, emitting once on startup and again per
//! change — the same shape as `system_accent`, for the same reason: a watcher
//! that has to be polled, joined or woken is a watcher nobody shuts down
//! cleanly, and the process exit already reaps the thread.

use dae_core::host::{Event as UiEvent, Host};
use serde::Serialize;
use specta::Type;
use std::sync::Arc;

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;

#[cfg(target_os = "linux")]
use linux as backend;

/// No reader is written for this platform, and none is needed: WebView2 and
/// WKWebView report the system appearance through `prefers-color-scheme`
/// faithfully, so the frontend's fallback already follows the OS. Saying so
/// explicitly is the difference between an app that keeps following the system
/// and one that fails to build.
#[cfg(not(target_os = "linux"))]
mod backend {
    use dae_core::host::Host;

    pub(super) fn read() -> Option<bool> {
        None
    }

    pub(super) fn watch(_host: &dyn Host) {}
}

/// Emitted when the OS light/dark preference changes. `None` means the platform
/// publishes no preference and the frontend should keep its own media-query
/// reading.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "system-theme-changed")]
pub struct SystemThemeChanged(pub Option<bool>);

/// The OS light/dark preference as `Some(true)` for dark and `Some(false)` for
/// light, or `None` where the platform publishes no preference (or it could not
/// be read). Frontend: `commands.getSystemTheme()`.
#[tauri::command]
#[specta::specta]
pub fn get_system_theme() -> Option<bool> {
    backend::read()
}

/// Starts the platform watcher on its own thread. It reports the current
/// preference immediately and then every change as `SystemThemeChanged`, so a
/// listener that mounts late still converges.
///
/// Takes the seam rather than an `AppHandle`: the thread outlives every window
/// and has no business holding one.
pub fn init(host: Arc<dyn Host>) {
    std::thread::Builder::new()
        .name("system-theme".to_owned())
        .spawn(move || backend::watch(host.as_ref()))
        .expect("failed to spawn the system-theme watcher");
}

/// Hands one reading to the UI. Every emission goes through here so a backend
/// cannot forget the event and leave the shell on a stale appearance.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn report(host: &dyn Host, dark: Option<bool>) {
    host.emit(UiEvent::SystemThemeChanged(dark));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testing::RecordingHost;

    /// The watcher's reading reaches the seam unchanged. The type is also the
    /// guard: `report` has no `AppHandle` to fall back to, so a future edit
    /// cannot quietly bypass the seam and still compile.
    #[test]
    fn a_reading_reaches_the_seam_unchanged() {
        let host = RecordingHost::default();

        report(&host, Some(true));
        report(&host, None);

        assert_eq!(
            host.take(),
            vec![
                UiEvent::SystemThemeChanged(Some(true)),
                UiEvent::SystemThemeChanged(None),
            ]
        );
    }
}
