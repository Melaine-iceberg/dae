//! The window's backdrop: whatever the operating system paints behind a window,
//! seen through the parts of this shell that paint nothing.
//!
//! Steps one and two of this effort moved the file glyphs and the canvas
//! luminance to what the platforms actually do; this one stops imitating them.
//! Windows 11 composites Mica — a wallpaper-derived, *non-live* tint — behind
//! every application window, and macOS keeps a live blurred copy of the desktop
//! inside the window frame. Both are reachable through Tauri's own
//! [`tauri::WebviewWindow::set_effects`], which already depends on
//! `window-vibrancy` on those two platforms, so this module adds no dependency:
//! it decides *whether* a backdrop is available, tells the window to be
//! transparent so the backdrop can be seen, and reports the answer to the
//! frontend's `data-window-material` seam (see `src/lib/window-material.ts` and
//! the material block in `src/App.css`).
//!
//! The three decisions this module owns, and why each is where it is:
//!
//!   * **The backdrop, not the surface.** Only the window chrome goes
//!     translucent — the tab strip and the sidebar, which is where Explorer and
//!     Finder both let their window backdrop through. The content plane stays
//!     opaque: a file list over a live blur is unreadable, and neither platform
//!     does it. Deciding that is CSS's job; this module only says which backdrop
//!     is live.
//!   * **Degradation is automatic and per-platform.** No Mica below Windows 11,
//!     nothing on Linux (no compositor exposes a client-requestable backdrop —
//!     see `linux.rs`), and an OS "show fewer effects" setting wins over the
//!     effect. `Material::None` is the answer that means "paint your own canvas,
//!     as before", and it is also what an unsupported platform gets.
//!   * **The transparency flag is decided once, before any window exists.**
//!     A window that was built opaque cannot be made to show a backdrop later,
//!     and a window built transparent cannot be made opaque again, so
//!     [`startup`] is frozen the first time it is asked and every later answer
//!     is clamped to it ([`clamp`]). That is why [`set_window_material_appearance`]
//!     and the watcher can follow a mid-session toggle but a Windows 10 machine
//!     never suddenly gains one.
//!
//! Two things cross the IPC boundary and nothing else, in the same shape as
//! [`crate::system_accent`]: the material as a short string, pulled once by
//! `get_window_material()` and pushed by `WindowMaterialChanged` whenever it
//! moves — plus one write-only call, [`set_window_material_appearance`], because
//! both backdrops are tinted by the *system's* theme while this shell can be
//! pinned to the other one, and a dark window over light Mica is the exact
//! mismatch this whole effort is about.
//!
//! Unlike `system_accent`, the answer is needed *synchronously* by the very
//! first frame: `index.html` paints a canvas before React exists, and an opaque
//! one there would hide the backdrop until the app hydrated. So the decision
//! also travels as an initialization script ([`boot_script`]) that runs before
//! any page script, and `index.html` reads it from `__DAE_WINDOW_MATERIAL__`.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI8, AtomicU8, Ordering};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Manager, Runtime, WebviewWindow};
use tauri_specta::Event;

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
use windows as backend;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod backend {
    //! No backdrop is written for this platform. Saying so explicitly is the
    //! difference between an app that keeps its own canvas and one that fails to
    //! build — and unlike an accent, a backdrop with nowhere to come from must
    //! not leave a transparent window behind it either, which is what
    //! [`super::startup`] and [`super::wants_transparency`] conspire to prevent.
    pub(super) fn read() -> Material {
        Material::None
    }

    pub(super) fn apply(_window: &WebviewWindow, _material: Material, _dark: Option<bool>) {}

    pub(super) fn watch(app: &tauri::AppHandle) {
        super::report(app, super::startup());
    }
}

/// The backdrop currently composited behind this window, as the CSS seam names
/// it. Serialized to `"mica"`, `"vibrancy"` or `"none"`; the frontend mirrors it
/// onto `<html data-window-material>` and App.css decides what each one paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Material {
    /// Windows 11's Mica: the desktop wallpaper, blurred hard and tinted by the
    /// window's own light/dark attribute. Derived once per window rather than
    /// tracked live, which is what makes it usable behind a file list.
    Mica,
    /// macOS' sidebar vibrancy: a live blur of what is behind the window, using
    /// the material AppKit gives to a source list.
    Vibrancy,
    /// Nothing usable. The shell paints its own canvas, exactly as it did before
    /// this module existed.
    None,
}

impl Material {
    /// The string the frontend and `index.html` both key off. A fixed set of
    /// literals rather than `format!("{material:?}")` because it is spliced into
    /// an injected script.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mica => "mica",
            Self::Vibrancy => "vibrancy",
            Self::None => "none",
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Mica => 0,
            Self::Vibrancy => 1,
            Self::None => 2,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Mica),
            1 => Some(Self::Vibrancy),
            2 => Some(Self::None),
            _ => None,
        }
    }

    /// Whether a window has to be built transparent for this backdrop to be
    /// visible at all. `set_effects` documents the same requirement; the
    /// difference is that here it is decided before the window exists.
    pub fn wants_transparency(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The backdrop this process was built for, frozen the first time it is asked —
/// see the module docs for why it cannot be re-read per window.
static STARTUP: OnceLock<Material> = OnceLock::new();

/// The backdrop currently composited. Starts at [`STARTUP`] and can only move
/// down from it, which is what [`clamp`] enforces.
static CURRENT: AtomicU8 = AtomicU8::new(u8::MAX);

/// The theme the frontend last reported, or `-1` while it has reported nothing.
/// Kept so a mid-session backdrop change re-tints the way the window already
/// looks rather than snapping to the system theme.
static DARK: AtomicI8 = AtomicI8::new(-1);

/// Emitted when the usable backdrop changes. `Material::None` means "there is
/// nothing behind this window, paint your own canvas".
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "window-material-changed")]
pub struct WindowMaterialChanged(pub Material);

/// The backdrop in force. Frontend: `commands.getWindowMaterial()`.
#[tauri::command]
#[specta::specta]
pub fn get_window_material() -> Material {
    current()
}

/// Reports which theme the shell is drawn in, so the backdrop matches it.
///
/// Both platforms tint their material from the *system* appearance, and this
/// app can be pinned to the other one — Mica then comes out light under a dark
/// tab strip. Windows answers by setting the window's immersive-dark attribute
/// (which is what Mica's tint follows) and macOS by setting the window's
/// appearance (which is what `NSVisualEffectView` follows). Each of those also
/// fixes the 1px DWM border and the native popups for free.
///
/// Windows that have not asked keep the system tint, so a shell that never calls
/// this is left exactly as the platform would draw any other window.
#[tauri::command]
#[specta::specta]
pub fn set_window_material_appearance(window: WebviewWindow, dark: bool) {
    DARK.store(i8::from(dark), Ordering::Relaxed);
    if current() == Material::None {
        // Nothing is tinted, so there is nothing to match. Deliberately not
        // touching the window's theme for the same reason: without a backdrop
        // this app draws every pixel itself.
        return;
    }
    backend::apply(&window, current(), Some(dark));
}

/// Decorates a window builder with what the backdrop needs. Call this on every
/// window before `build()`; [`attach`] finishes the job afterwards.
pub fn configure<R: Runtime, M: Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'_, R, M>,
) -> tauri::WebviewWindowBuilder<'_, R, M> {
    builder
        .transparent(startup().wants_transparency())
        .initialization_script(boot_script())
}

/// Composites the backdrop onto a window that has just been built.
pub fn attach<R: Runtime>(window: &WebviewWindow<R>) {
    backend::apply(window, current(), dark_hint());
}

/// The script that hands the startup decision to the page before it paints.
///
/// An initialization script rather than a `get_window_material()` call because
/// the pre-React splash has to agree with it: a splash that paints the canvas
/// opaque covers the backdrop for as long as the bundle takes to load, and every
/// later pull is too late to un-paint the frame that is already on screen.
pub fn boot_script() -> String {
    format!(
        "window.__DAE_WINDOW_MATERIAL__ = \"{}\";\n",
        startup().as_str()
    )
}

/// Starts the platform watcher on its own thread, in the shape
/// [`crate::system_accent`] uses: it reports once and then per change, and
/// blocks its thread for the life of the app.
pub fn init(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("window-material".to_owned())
        .spawn(move || backend::watch(&app))
        .expect("failed to spawn the window-material watcher");
}

/// The frozen startup decision. Reads the platform once; every later call is a
/// lock-free hit on the `OnceLock`.
pub fn startup() -> Material {
    *STARTUP.get_or_init(backend::read)
}

/// The backdrop in force right now. Seeded from [`startup`] on first use so a
/// platform with no watcher still answers, and never above it — see [`clamp`].
pub fn current() -> Material {
    if let Some(material) = Material::from_code(CURRENT.load(Ordering::Relaxed)) {
        return material;
    }
    let initial = clamp(backend::read());
    CURRENT.store(initial.code(), Ordering::Relaxed);
    initial
}

/// A mid-session change cannot outrank what the windows were built for: the
/// transparent flag is fixed at creation, so a backdrop that only became
/// available after launch has to wait for the next one. Clamping here rather
/// than in each backend keeps that true for all three.
fn clamp(material: Material) -> Material {
    if startup().wants_transparency() {
        material
    } else {
        Material::None
    }
}

/// The theme the backends should tint to, if the frontend has named one.
fn dark_hint() -> Option<bool> {
    theme_from_code(DARK.load(Ordering::Relaxed))
}

/// `-1` is "the shell has not said", which must leave the platform's own answer
/// standing rather than defaulting to either appearance.
const fn theme_from_code(stored: i8) -> Option<bool> {
    match stored {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// Hands one reading to every live window and to the frontend. Every change
/// goes through here so a backend cannot apply a backdrop the shell cannot see,
/// or repaint a window it just made transparent.
fn report(app: &tauri::AppHandle, material: Material) {
    let material = clamp(material);
    if current() == material {
        // Nothing moved, so nothing is re-applied. Applying twice is not
        // idempotent on macOS — `window-vibrancy` adds an `NSVisualEffectView`
        // below the content view rather than replacing the tagged one it put
        // there last time — and on every platform it would wake each webview
        // with a value it already has. This is also why the opening report each
        // watcher makes is free: `attach` already applied that answer.
        return;
    }
    CURRENT.store(material.code(), Ordering::Relaxed);
    for window in app.webview_windows().values() {
        backend::apply(window, material, dark_hint());
    }
    let _ = WindowMaterialChanged(material).emit(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the seam: an app that never reaches a platform
    /// backdrop must ask for an opaque window, because a transparent one with
    /// nothing behind it shows the desktop through the tab strip.
    #[test]
    fn only_a_real_backdrop_asks_for_transparency() {
        assert!(Material::Mica.wants_transparency());
        assert!(Material::Vibrancy.wants_transparency());
        assert!(!Material::None.wants_transparency());
    }

    /// The three spellings are what `App.css` and `index.html` both match on,
    /// and `boot_script` splices one into injected JavaScript unescaped.
    #[test]
    fn names_are_the_lowercase_words_css_matches() {
        assert_eq!(Material::Mica.as_str(), "mica");
        assert_eq!(Material::Vibrancy.as_str(), "vibrancy");
        assert_eq!(Material::None.as_str(), "none");
    }

    #[test]
    fn round_trips_through_the_atomic_code() {
        for material in [Material::Mica, Material::Vibrancy, Material::None] {
            assert_eq!(Material::from_code(material.code()), Some(material));
        }
        // `u8::MAX` is how CURRENT starts out: "nobody has answered yet", which
        // must not decode to a backdrop.
        assert_eq!(Material::from_code(u8::MAX), None);
    }

    /// The startup decision is what the transparency flag is built from, so a
    /// second read that disagreed with the first would silently repaint windows
    /// that cannot show it.
    #[test]
    fn the_startup_answer_never_moves() {
        let first = startup();
        assert_eq!(first, startup());
        assert_eq!(first, current());
    }

    #[test]
    fn a_window_built_opaque_never_gains_a_backdrop() {
        let built_for = startup();
        let stronger = Material::Mica;
        if built_for.wants_transparency() {
            assert_eq!(clamp(stronger), stronger);
        } else {
            assert_eq!(clamp(stronger), Material::None);
            assert_eq!(clamp(Material::None), Material::None);
        }
    }

    /// `-1` means the shell has not said which theme it is drawn in, and the
    /// platform's own answer must stand until it does — defaulting to either
    /// appearance would fight a user who pinned the other one.
    #[test]
    fn an_unreported_theme_tints_to_the_system() {
        assert_eq!(theme_from_code(-1), None);
        assert_eq!(theme_from_code(0), Some(false));
        assert_eq!(theme_from_code(1), Some(true));
        // Anything else is a value nothing stores; reading it as "dark" or
        // "light" would be a guess.
        assert_eq!(theme_from_code(i8::MIN), None);
    }

    /// The injected script is the one place this module writes JavaScript source,
    /// so it has to stay one line and one of three literals whatever the platform
    /// answers — a name that drifted would break the splash silently.
    #[test]
    fn the_boot_script_is_one_line_and_one_of_three_literals() {
        let script = boot_script();
        assert_eq!(script.lines().count(), 1);
        assert!(
            [
                "window.__DAE_WINDOW_MATERIAL__ = \"mica\";",
                "window.__DAE_WINDOW_MATERIAL__ = \"vibrancy\";",
                "window.__DAE_WINDOW_MATERIAL__ = \"none\";",
            ]
            .contains(&script.trim_end()),
            "unexpected boot script {script:?}"
        );
    }
}
