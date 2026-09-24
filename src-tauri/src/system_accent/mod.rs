//! The OS accent colour, feeding the frontend's `--system-accent` seam
//! (see `src/lib/system-accent.ts` and the accent block in `src/App.css`).
//!
//! One colour, chosen by the user in their own system settings, is what makes
//! a cross-platform app read as native on Windows, macOS and Linux without
//! imitating any of them. Reading it is the platform half of that seam; the
//! frontend owns everything derived from it (ink, selection, focus rings).
//!
//! Two things cross the IPC boundary and nothing else:
//!
//!   * `get_system_accent()` — the accent as `#rrggbb`, or `None` where the
//!     platform has no accent setting (the frontend then keeps its default);
//!   * `SystemAccentChanged` — the same payload, every time it moves.
//!
//! Each backend answers two questions (`read`, `watch`) and `watch` blocks its
//! thread for the life of the app, emitting once on startup and again per
//! change. Blocking is deliberate: a watcher that has to be polled, joined or
//! woken is a watcher nobody shuts down cleanly, and the process exit already
//! reaps the thread.

use serde::Serialize;
use specta::Type;
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
    //! No accent reader is written for this platform. Saying so explicitly is
    //! the difference between an app that falls back to its default accent and
    //! one that fails to build.
    pub(super) fn read() -> Option<String> {
        None
    }

    pub(super) fn watch(_app: &tauri::AppHandle) {}
}

/// Emitted when the OS accent changes. `None` means the platform has no accent
/// setting and the frontend should hold its own default.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "system-accent-changed")]
pub struct SystemAccentChanged(pub Option<String>);

/// The OS accent as `#rrggbb`, or `None` where the platform has no accent
/// setting (or it could not be read). Frontend: `commands.getSystemAccent()`.
#[tauri::command]
#[specta::specta]
pub fn get_system_accent() -> Option<String> {
    backend::read()
}

/// Starts the platform watcher on its own thread. It reports the current
/// accent immediately and then every change as `SystemAccentChanged`, so a
/// listener that mounts late still converges.
pub fn init(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("system-accent".to_owned())
        .spawn(move || backend::watch(&app))
        .expect("failed to spawn the system-accent watcher");
}

/// Formats 8-bit sRGB channels as the `#rrggbb` the CSS seam takes.
fn srgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Hands one reading to the frontend. Every emission goes through here so a
/// backend cannot forget the event and leave the shell on a stale tint.
fn report(app: &tauri::AppHandle, accent: Option<String>) {
    let _ = SystemAccentChanged(accent).emit(app);
}

/// Unpacks `0xAARRGGBB`, the layout WinRT's `UIColorType::Accent` and
/// `HKCU\...\DWM!ColorizationColor` use. In practice this is the fallback for
/// a Windows 10 build whose DWM key has no `AccentColor` yet.
fn from_argb(packed: u32) -> Option<String> {
    if packed == 0 {
        return None;
    }
    Some(srgb_hex(
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
    ))
}

/// Unpacks `0xAABBGGRR`, a Windows `COLORREF` (`0x00BBGGRR`, red in the low
/// byte) with alpha added to the top one. `HKCU\...\DWM!AccentColor` is stored
/// this way.
///
/// The two layouts are easy to swap and the failure is invisible — a blue
/// accent renders orange — so they are kept as two named functions rather than
/// one that takes a flag. Measured on a real machine to be sure: with the
/// default Windows blue accent, `AccentColor` reads `0xFFD47800` and
/// `ColorizationColor` reads `0xC40078D4`, and these two functions are what
/// make both of them `#0078d4`.
fn from_abgr(packed: u32) -> Option<String> {
    if packed == 0 {
        return None;
    }
    Some(srgb_hex(
        packed as u8,
        (packed >> 8) as u8,
        (packed >> 16) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement the byte-order comments above rest on: the same accent,
    /// read from the two neighbouring registry values, has to agree.
    #[test]
    fn the_two_windows_layouts_agree_on_one_accent() {
        assert_eq!(from_abgr(0xFF_D4_78_00), Some("#0078d4".to_owned()));
        assert_eq!(from_argb(0xC4_00_78_D4), Some("#0078d4".to_owned()));
    }

    /// The same 24 bits decode to mirrored colours in the two layouts, which
    /// is exactly the failure a swapped byte order looks like.
    #[test]
    fn picks_each_channel_from_its_own_byte() {
        assert_eq!(from_abgr(0x0001_0203), Some("#030201".to_owned()));
        assert_eq!(from_argb(0x0001_0203), Some("#010203".to_owned()));
    }

    #[test]
    fn an_absent_colour_is_none_not_black() {
        // 0 would otherwise decode to `#000000`, which the frontend would take
        // as "the user chose black" and derive a white ink for.
        assert_eq!(from_abgr(0), None);
        assert_eq!(from_argb(0), None);
    }

    #[test]
    fn formats_channels_with_leading_zeroes() {
        assert_eq!(srgb_hex(0, 7, 255), "#0007ff");
    }
}
