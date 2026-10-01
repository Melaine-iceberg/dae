//! Linux' backdrop: whatever the compositor decides to blur behind a
//! transparent window, which is a fact about the session rather than a request
//! this process can make.
//!
//! Both platforms this seam imitates get their material *from the compositor*:
//! DWM draws Mica behind the window, the WindowServer blurs the desktop into
//! `NSVisualEffectView`. On Linux that job also belongs to the compositor, but
//! no compositor this module can rely on exposes it as a request — the best
//! available move is a window that paints nothing where the backdrop should
//! show, and a compositor that happens to blur those pixels:
//!
//!   * **GNOME** — mutter has no protocol for it, on Wayland or X11. libadwaita's
//!     translucent-looking sidebars are drawn by the toolkit inside the surface,
//!     which is the thing this effort exists to stop doing.
//!   * **KDE Plasma 6 / KWin** — `_KDE_NET_WM_BLUR_BEHIND_REGION` and the
//!     `org_kde_kwin_blur` protocol do ask for a live blur and KWin honors both,
//!     but answering means protocol code this module cannot test from the
//!     Windows machine it is written on. `_KDE_NET_WM_BACKGROUND_CONTRAST_REGION`
//!     is the one that is not a backdrop at all: KWin answers a *contrast* region
//!     with a solid derived colour, not the wallpaper.
//!   * **tiling compositors** (niri, Hyprland, picom) — blur is configured per
//!     window by the user, outside the application. niri is the one with a
//!     session-wide rule this app can be the target of (`background-effect
//!     { blur true }` in a `window-rule`), so it is the one worth building a
//!     transparent window for.
//!
//! That is the whole asymmetry with Windows and macOS, and the risk that comes
//! with it: a compositor either blurs this window or it does not, and nothing
//! here can ask which. [`choose`] therefore answers [`Material::Blur`] only for a
//! session it can name, leaving every other Linux desktop with
//! [`Material::None`] and the opaque canvas the CSS seam falls back to — because
//! a transparent window in front of a compositor that does not blur shows the
//! raw desktop through the tab strip, which is the failure mode this module
//! exists to avoid. The cost of narrowing it that far is a user whose niri has
//! blur switched off, and there is no way to tell that from inside the app.

use tauri::{Runtime, WebviewWindow};

use super::Material;

/// The compositor this session runs under, named by whatever it leaves in the
/// process environment. Nothing in the Wayland protocol answers this question —
/// a registry walk reports which *interfaces* are advertised, not who advertises
/// them — so these are the two markers niri is known by, in the order they
/// appear.
fn compositor(desktop: &str, niri_socket: &str) -> Option<&'static str> {
    // niri exports its control socket path into the session, which only niri
    // does; `XDG_CURRENT_DESKTOP` is the fallback for a session that has the
    // socket stripped (a sandboxed launch) but still declares the desktop.
    if !niri_socket.is_empty()
        || desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("niri"))
    {
        Some("niri")
    } else {
        None
    }
}

/// The decision, apart from the environment reads, so a machine that is neither
/// of these things can still check what a session name is worth.
fn choose(wayland: bool, desktop: &str, niri_socket: &str) -> Material {
    // X11 is left alone entirely: the blur-capable X11 compositors (KWin, xfwm4,
    // picom) each need a property write of their own and none of them is this
    // session's, and a transparent X11 window without a redirecting compositor
    // shows the root window.
    if wayland && compositor(desktop, niri_socket) == Some("niri") {
        Material::Blur
    } else {
        Material::None
    }
}

fn env(name: &str) -> String {
    std::env::var_os(name).map_or_else(String::new, |value| value.to_string_lossy().into_owned())
}

pub(super) fn read() -> Material {
    choose(
        !env("WAYLAND_DISPLAY").is_empty(),
        &env("XDG_CURRENT_DESKTOP"),
        &env("NIRI_SOCKET"),
    )
}

/// Nothing to composite: the blur is the compositor's own, drawn behind the
/// transparent chrome [`super::configure`] asked for, and there is no client-side
/// tint to match [`super::dark_hint`] to. `tauri::WebviewWindow::set_effects`
/// documents Linux as unsupported and drops the request on the floor, so asking
/// would only look like it worked.
pub(super) fn apply<R: Runtime>(
    _window: &WebviewWindow<R>,
    _material: Material,
    _dark: Option<bool>,
) {
}

pub(super) fn watch(app: &tauri::AppHandle) {
    // A niri config reload is not observable from a client, so the startup
    // answer is the answer for the life of the process.
    super::report(app, super::startup());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session this is developed against: niri on Wayland gets the blur, and
    /// with it the transparent window [`super::startup`] builds the backdrop into.
    #[test]
    fn a_wayland_niri_session_gets_the_blur() {
        assert_eq!(choose(true, "niri", ""), Material::Blur);
        assert_eq!(choose(true, "Niri:GNOME", ""), Material::Blur);
        assert_eq!(choose(true, "", "/run/user/1000/niri.sock"), Material::Blur);
    }

    /// The failure this guards: naming a compositor that does not blur, or
    /// answering on a session that has no such rule, leaves a see-through tab
    /// strip over the user's desktop.
    #[test]
    fn nothing_else_is_named() {
        for desktop in ["GNOME", "KDE", "Hyprland", "i3", "", "x-cinnamon"] {
            assert_eq!(choose(true, desktop, ""), Material::None, "{desktop}");
        }
        // Not even the name makes the answer yes without the session type: an X11
        // launch inside a niri-configured environment is not a niri window.
        assert_eq!(choose(false, "niri", ""), Material::None);
    }

    /// The socket is the stronger marker, but a desktop name alone is enough —
    /// a sandboxed launch loses the socket, not the declaration.
    #[test]
    fn either_marker_names_niri() {
        assert_eq!(compositor("niri", ""), Some("niri"));
        assert_eq!(
            compositor("", "/run/user/1000/niri.wayland-1.1575.sock"),
            Some("niri")
        );
        assert_eq!(compositor("", ""), None);
    }

    /// Compiled on every host — this module is built under `cfg(test)` too, so
    /// on a Windows machine this is the check that a session with no known blur
    /// is the one the CSS seam treats as "paint your own canvas".
    #[test]
    fn an_unnamed_session_asks_for_no_transparency() {
        assert!(!Material::None.wants_transparency());
        assert!(Material::Blur.wants_transparency());
    }
}
