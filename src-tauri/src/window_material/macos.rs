//! macOS' backdrop: the same vibrancy the Finder puts behind its source list.
//!
//! AppKit's `NSVisualEffectView` with the `sidebar` material is what a system
//! application's nav column is made of — a live blur of the desktop behind it,
//! rendered from the material the OS picks for that role, which is why it stays
//! readable over any wallpaper in either appearance. `window-vibrancy` inserts
//! one below the webview's own view, so everything this shell leaves unpainted
//! shows that instead of the desktop: which is exactly the point, and exactly
//! why `tauri.conf.json` has to enable `macOSPrivateApi` for this to work at all
//! (a transparent window is a private API on macOS, and the trade is that the
//! resulting build is not eligible for the Mac App Store — see the note in
//! `window_material::mod`).
//!
//! The answer is unconditional, and that is the considered part. macOS does have
//! an accessibility setting for this — System Settings › Accessibility › Display
//! › Reduce transparency — but `NSVisualEffectView` already honors it *itself*,
//! substituting the solid window background for the blur while keeping the
//! correct appearance and the same text contrast. That is what a native sidebar
//! does with the setting on, so the shell's transparent chrome stays as it is and
//! what shows through simply stops moving; opting out on the app's side would
//! replace an OS-drawn fallback with an app-drawn one, and reading the setting
//! means asking `NSWorkspace` on the main thread.
//!
//! There is nothing to watch, which is why [`watch`] reports once and returns.

use tauri::window::{Effect, EffectsBuilder};
use tauri::{Runtime, Theme, WebviewWindow};

use super::Material;

pub(super) fn read() -> Material {
    Material::Vibrancy
}

/// Sidebar material, following the window's active state — the state
/// `window-vibrancy` defaults to and the one a real sidebar uses, so an
/// inactive window dims its backdrop instead of holding a bright blur.
///
/// `dark` is answered by the window's appearance rather than by the material,
/// because on macOS the material *is* a function of the appearance: setting it
/// is both what tints the vibrancy and what the shell has to do for a pinned
/// appearance to reach the frame. `Window::set_theme` drives it, and a `None`
/// hands the window back to the system, which is the same as never having
/// called it.
pub(super) fn apply<R: Runtime>(window: &WebviewWindow<R>, material: Material, dark: Option<bool>) {
    if material == Material::Vibrancy {
        let _ = window.set_effects(EffectsBuilder::new().effect(Effect::Sidebar).build());
    }

    if let Some(dark) = dark {
        let _ = window.set_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
    }
}

pub(super) fn watch(app: &tauri::AppHandle) {
    // Nothing moves here that the platform does not handle on its own; see the
    // module docs. The report keeps this watcher shaped like
    // `system_accent`'s — and `report` drops it, because by the time this thread
    // starts every window has already been attached with the same answer.
    super::report(app, super::startup());
}
