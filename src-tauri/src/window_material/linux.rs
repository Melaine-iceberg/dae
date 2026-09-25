//! Linux has no window backdrop to ask for, and this module says so rather
//! than approximating one.
//!
//! Both platforms this seam imitates get their material *from the compositor*:
//! DWM draws Mica behind the window, the WindowServer blurs the desktop into
//! `NSVisualEffectView`. On Linux that job belongs to the compositor, and it
//! exposes no client-requestable version of it that is worth requesting:
//!
//!   * **GNOME** — mutter has no protocol for it, on Wayland or X11. libadwaita's
//!     translucent-looking sidebars are drawn by the toolkit inside the surface,
//!     which is the thing this effort exists to stop doing.
//!   * **KDE Plasma 6 / KWin** — `_KDE_NET_WM_BACKGROUND_CONTRAST_REGION` asks
//!     the compositor for a *contrast* region, which KWin answers with a solid
//!     derived colour rather than the wallpaper, and which is honored by KWin
//!     alone. A hint that one compositor turns into a flat fill is not a
//!     backdrop, and `window-vibrancy` has no Linux backend to lean on, so it
//!     would be X11 property code written and merged without ever being run.
//!   * **tiling compositors** (Hyprland, picom) — blur is configured per window
//!     by the user, outside the application.
//!
//! So the answer is [`Material::None`] and the window stays opaque, which is the
//! state the CSS seam was built to fall back to: the chrome paints its own
//! canvas, the same as every other Linux build of this app has always done. What
//! a real implementation would hook is already in place — a `read` that can
//! answer per session (the `XDG_CURRENT_DESKTOP` / Wayland display checks belong
//! here, and `crate::xdg` already resolves the desktop directories), one
//! `apply` call per window, and the `data-window-material` seam that lets a new
//! material choose which surfaces go translucent without touching any of them.

use tauri::{Runtime, WebviewWindow};

use super::Material;

pub(super) fn read() -> Material {
    Material::None
}

/// Nothing to composite: `tauri::WebviewWindow::set_effects` documents Linux as
/// unsupported and drops the request on the floor, so asking would only look
/// like it worked.
pub(super) fn apply<R: Runtime>(
    _window: &WebviewWindow<R>,
    _material: Material,
    _dark: Option<bool>,
) {
}

pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, super::startup());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compiled on every host — this module is built under `cfg(test)` too, so
    /// on a Windows machine this is the check that the Linux answer is the one
    /// the CSS seam treats as "paint your own canvas".
    #[test]
    fn linux_asks_for_no_transparency() {
        assert_eq!(read(), Material::None);
        assert!(!read().wants_transparency());
    }
}
