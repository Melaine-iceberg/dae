//! OS appearance seam: accent colour and light/dark preference.
//!
//! The platform backends below are the same readers the Tauri shell used. The
//! only change is where reports go: instead of a Tauri event handle they are
//! serialized into the core process event pump, which the Electron shell
//! forwards to the renderer.

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;

use crate::emit_event;

const ACCENT_EVENT: &str = "system-accent-changed";
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const THEME_EVENT: &str = "system-theme-changed";

/// Formats 8-bit sRGB channels as the `#rrggbb` the CSS seam takes.
pub(crate) fn srgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Unpacks `0xAARRGGBB`, the layout WinRT's `UIColorType::Accent` and
/// `HKCU\...\DWM!ColorizationColor` use.
pub(crate) fn from_argb(packed: u32) -> Option<String> {
    if packed == 0 {
        return None;
    }
    Some(srgb_hex(
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
    ))
}

/// Unpacks `0xAABBGGRR`, a Windows `COLORREF` with alpha added to the top
/// byte. `HKCU\...\DWM!AccentColor` is stored this way.
pub(crate) fn from_abgr(packed: u32) -> Option<String> {
    if packed == 0 {
        return None;
    }
    Some(srgb_hex(
        packed as u8,
        (packed >> 8) as u8,
        (packed >> 16) as u8,
    ))
}

/// The current OS accent as `#rrggbb`, or `None` where the platform has no
/// accent setting (or it could not be read).
pub fn accent() -> Option<String> {
    accent_backend::read()
}

/// The current OS light/dark preference: `Some(true)` is dark.
pub fn theme() -> Option<bool> {
    theme_backend::read()
}

/// Starts both platform observers once. Each reports the current value on
/// startup and then every change; a renderer that mounts late still converges.
pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    std::thread::Builder::new()
        .name("system-accent".to_owned())
        .spawn(accent_backend::watch)
        .expect("failed to spawn the system-accent watcher");
    std::thread::Builder::new()
        .name("system-theme".to_owned())
        .spawn(theme_backend::watch)
        .expect("failed to spawn the system-theme watcher");
}

fn report_accent(accent: Option<String>) {
    if let Ok(payload) = serde_json::to_string(&accent) {
        emit_event(ACCENT_EVENT, payload);
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn report_theme(dark: Option<bool>) {
    if let Ok(payload) = serde_json::to_string(&dark) {
        emit_event(THEME_EVENT, payload);
    }
}

// ---------------------------------------------------------------------------
// Accent backends
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod accent_backend {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_DWORD, REG_NOTIFY_CHANGE_LAST_SET,
        RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";
    const ACCENT_VALUE: &str = "AccentColor";
    const COLORIZATION_VALUE: &str = "ColorizationColor";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_key(sam: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
        let path = wide(DWM_KEY);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                sam,
                &mut handle,
            )
        };
        opened.is_ok().then_some(handle)
    }

    pub(super) fn read() -> Option<String> {
        read_dword(ACCENT_VALUE)
            .and_then(super::from_abgr)
            .or_else(|| read_dword(COLORIZATION_VALUE).and_then(super::from_argb))
    }

    fn read_dword(name: &str) -> Option<u32> {
        let handle = open_key(KEY_READ)?;
        let name = wide(name);

        let read_value = (|| {
            let mut kind = REG_DWORD;
            let mut size: u32 = 0;
            // SAFETY: `name` outlives the call; `kind`/`size` are valid
            // out-pointers; `lpdata` is null on the size probe.
            let probe = unsafe {
                RegQueryValueExW(
                    handle,
                    PCWSTR(name.as_ptr()),
                    None,
                    Some(&mut kind),
                    None,
                    Some(&mut size),
                )
            };
            if probe.is_err() {
                return None;
            }

            let mut buffer: Vec<u8> = vec![0; size.max(4) as usize];
            // SAFETY: `buffer` has at least `size` bytes of capacity.
            let read = unsafe {
                RegQueryValueExW(
                    handle,
                    PCWSTR(name.as_ptr()),
                    None,
                    Some(&mut kind),
                    Some(buffer.as_mut_ptr()),
                    Some(&mut size),
                )
            };
            if read.is_err() {
                return None;
            }
            Some(u32::from_ne_bytes(buffer[..4].try_into().ok()?))
        })();

        // SAFETY: `handle` came from open_key and is closed exactly once.
        let _ = unsafe { RegCloseKey(handle) };
        read_value
    }

    pub(super) fn watch() {
        super::report_accent(read());

        let Some(handle) = open_key(KEY_NOTIFY) else {
            return;
        };

        loop {
            // SAFETY: `handle` is an open key; the synchronous form blocks
            // until a value under the key is written.
            let changed = unsafe {
                RegNotifyChangeKeyValue(
                    handle,
                    false,
                    REG_NOTIFY_CHANGE_LAST_SET,
                    None,
                    false,
                )
            };
            if changed.is_err() {
                break;
            }
            super::report_accent(read());
        }

        // SAFETY: `handle` is closed exactly once, on the way out of the loop.
        let _ = unsafe { RegCloseKey(handle) };
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Reads this machine's real DWM key: a smoke test of the FFI rather
        /// than of a value.
        #[test]
        fn reads_this_machines_accent_out_of_the_dwm_key() {
            let packed = read_dword(ACCENT_VALUE).expect("this machine has a DWM AccentColor");
            let decoded = super::super::from_abgr(packed).expect("a nonzero accent decodes");
            assert_eq!(decoded.len(), 7);
            assert!(decoded.starts_with('#'));
            assert_eq!(read().as_deref(), Some(decoded.as_str()));
        }
    }
}

#[cfg(target_os = "macos")]
mod accent_backend {
    use std::time::Duration;

    use objc2_app_kit::{NSColor, NSColorSpace};

    fn channel(value: f64) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    pub(super) fn read() -> Option<String> {
        let accent = NSColor::controlAccentColor();
        let rgb = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        Some(super::srgb_hex(
            channel(rgb.redComponent() as f64),
            channel(rgb.greenComponent() as f64),
            channel(rgb.blueComponent() as f64),
        ))
    }

    pub(super) fn watch() {
        super::report_accent(read());

        let mut last = read();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = read();
            if now != last {
                last = now.clone();
                super::report_accent(now);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod accent_backend {
    use std::time::Duration;

    /// The fallback palette. `accent-color` is `(ddd)`, but a portal that
    /// answers with a `u` is publishing a 1-based index into these swatches.
    const XDG_ACCENT_COLORS: [&str; 9] = [
        "#3584e4", "#2190a4", "#3a944a", "#c88800", "#ed5b00", "#e62d42", "#d56199",
        "#9141ac", "#986a44",
    ];

    pub(super) fn accent_to_hex(value: u32) -> Option<String> {
        XDG_ACCENT_COLORS
            .get(value.checked_sub(1)? as usize)
            .map(|hex| (*hex).to_owned())
    }

    pub(super) fn rgb_to_hex(rgb: [f64; 3]) -> Option<String> {
        if rgb.iter().any(|channel| !(0.0..=1.0).contains(channel)) {
            return None;
        }
        Some(super::srgb_hex(
            channel(rgb[0]),
            channel(rgb[1]),
            channel(rgb[2]),
        ))
    }

    fn channel(value: f64) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    mod portal {
        use zbus::zvariant::{OwnedValue, Value};

        use super::{accent_to_hex, rgb_to_hex};

        const NAMESPACE: &str = "org.freedesktop.appearance";
        const KEY: &str = "accent-color";

        #[zbus::proxy(
            interface = "org.freedesktop.portal.Settings",
            default_service = "org.freedesktop.portal.Desktop",
            default_path = "/org/freedesktop/portal/desktop"
        )]
        trait Settings {
            fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
        }

        fn unwrap_variant(value: Value<'static>) -> Value<'static> {
            match value {
                Value::Value(inner) => *inner,
                value => value,
            }
        }

        pub(super) async fn read_accent() -> Option<String> {
            let connection = zbus::Connection::session().await.ok()?;
            let proxy = SettingsProxy::new(&connection).await.ok()?;
            let raw = proxy.read(NAMESPACE, KEY).await.ok()?;
            let value = unwrap_variant(Value::from(raw));
            match &value {
                Value::Structure(_) => {
                    let (r, g, b) = <(f64, f64, f64)>::try_from(&value).ok()?;
                    rgb_to_hex([r, g, b])
                }
                Value::U32(index) => accent_to_hex(*index),
                _ => None,
            }
        }
    }

    pub(super) fn read() -> Option<String> {
        crate::system::block_on(portal::read_accent())
    }

    pub(super) fn watch() {
        super::report_accent(read());

        let mut last = read();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = read();
            if now != last {
                last = now.clone();
                super::report_accent(now);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_the_one_based_palette() {
            assert_eq!(accent_to_hex(1).as_deref(), Some("#3584e4"));
            assert_eq!(accent_to_hex(5).as_deref(), Some("#ed5b00"));
            assert_eq!(accent_to_hex(9).as_deref(), Some("#986a44"));
            assert_eq!(accent_to_hex(0), None);
            assert_eq!(accent_to_hex(10), None);
        }

        #[test]
        fn reads_the_spec_rgb_tuple() {
            let gnome_blue = [
                0.207_843_139_767_646_8,
                0.517_647_087_574_005_1,
                0.894_117_653_369_903_6,
            ];
            assert_eq!(rgb_to_hex(gnome_blue).as_deref(), Some("#3584e4"));
        }

        #[test]
        fn rounds_channels_to_the_nearest_byte() {
            assert_eq!(rgb_to_hex([0.0, 0.5, 1.0]).as_deref(), Some("#0080ff"));
        }

        #[test]
        fn out_of_range_channels_are_no_accent() {
            assert_eq!(rgb_to_hex([-0.01, 0.5, 0.5]), None);
            assert_eq!(rgb_to_hex([0.5, 1.01, 0.5]), None);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod accent_backend {
    pub(super) fn read() -> Option<String> {
        None
    }

    pub(super) fn watch() {}
}

// ---------------------------------------------------------------------------
// Theme backends
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod theme_backend {
    use std::time::Duration;

    pub(super) fn color_scheme(value: u32) -> Option<bool> {
        match value {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    mod portal {
        use zbus::zvariant::{OwnedValue, Value};

        use super::color_scheme;

        const NAMESPACE: &str = "org.freedesktop.appearance";
        const KEY: &str = "color-scheme";

        #[zbus::proxy(
            interface = "org.freedesktop.portal.Settings",
            default_service = "org.freedesktop.portal.Desktop",
            default_path = "/org/freedesktop/portal/desktop"
        )]
        trait Settings {
            fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
        }

        fn unwrap_variant(value: Value<'static>) -> Value<'static> {
            match value {
                Value::Value(inner) => *inner,
                value => value,
            }
        }

        pub(super) async fn read_theme() -> Option<bool> {
            let connection = zbus::Connection::session().await.ok()?;
            let proxy = SettingsProxy::new(&connection).await.ok()?;
            let raw = proxy.read(NAMESPACE, KEY).await.ok()?;
            color_scheme(u32::try_from(unwrap_variant(Value::from(raw))).ok()?)
        }
    }

    pub(super) fn read() -> Option<bool> {
        crate::system::block_on(portal::read_theme())
    }

    pub(super) fn watch() {
        super::report_theme(read());

        let mut last = read();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = read();
            if now != last {
                last = now;
                super::report_theme(now);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_the_portal_values() {
            assert_eq!(color_scheme(0), None);
            assert_eq!(color_scheme(1), Some(true));
            assert_eq!(color_scheme(2), Some(false));
            assert_eq!(color_scheme(3), None);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod theme_backend {
    /// WebView2 / WKWebView already follow the OS appearance through
    /// `prefers-color-scheme`, so no reader is needed here.
    pub(super) fn read() -> Option<bool> {
        None
    }

    pub(super) fn watch() {}
}

#[cfg(target_os = "linux")]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the portal runtime")
        })
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_two_windows_byte_orders() {
        // Measured on a real machine: both packed values are the same accent.
        assert_eq!(from_abgr(0xFFD4_7800).as_deref(), Some("#0078d4"));
        assert_eq!(from_argb(0xC400_78D4).as_deref(), Some("#0078d4"));
        assert_eq!(from_abgr(0), None);
        assert_eq!(from_argb(0), None);
    }

    #[test]
    fn formats_six_digit_hex() {
        assert_eq!(srgb_hex(0x12, 0x34, 0x56), "#123456");
    }
}
