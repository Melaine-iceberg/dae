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
//! Two things cross the IPC boundary and nothing else:
//!
//!   * `get_system_theme()` — `Some(true)` dark, `Some(false)` light, or `None`
//!     where the platform publishes no preference (the frontend then keeps its
//!     `prefers-color-scheme` reading, which is the right answer everywhere
//!     this module reads nothing);
//!   * `SystemThemeChanged` — the same payload, every time it moves.
//!
//! Each backend answers two questions (`read`, `watch`) and `watch` blocks its
//! thread for the life of the app, emitting once on startup and again per
//! change — the same shape as `system_accent`, for the same reason: a watcher
//! that has to be polled, joined or woken is a watcher nobody shuts down
//! cleanly, and the process exit already reaps the thread.

use serde::Serialize;
use specta::Type;
use tauri_specta::Event;

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
    pub(super) fn read() -> Option<bool> {
        None
    }

    pub(super) fn watch(_app: &tauri::AppHandle) {}
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
pub fn init(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("system-theme".to_owned())
        .spawn(move || backend::watch(&app))
        .expect("failed to spawn the system-theme watcher");
}

/// Hands one reading to the frontend. Every emission goes through here so a
/// backend cannot forget the event and leave the shell on a stale appearance.
fn report(app: &tauri::AppHandle, dark: Option<bool>) {
    let _ = SystemThemeChanged(dark).emit(app);
}
