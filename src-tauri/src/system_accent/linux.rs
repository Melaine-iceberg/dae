//! Linux's accent: `org.freedesktop.appearance accent-color`, read through
//! the freedesktop settings portal.
//!
//! The portal is the only accent source that is right on every desktop. It is
//! what GNOME 47+, KDE and the XDG-compliant shells all publish, and it is
//! behind a well-known bus name, so it works on a desktop that has no GNOME
//! settings daemon and no GTK settings schema. Reading `org.gnome.desktop.interface`
//! or a GTK setting instead would work on one desktop and quietly fall back on
//! the others.
//!
//! `accent-color` is a colour, not an index: the Settings portal publishes it
//! as `(ddd)`, three sRGB channels in `0…1`, and a channel outside that range
//! means "no accent". [`rgb_to_hex`] is what turns that into the `#rrggbb` the
//! seam takes; [`accent_to_hex`] is a fallback for a portal that answers with
//! the 1-based palette index some early documentation described instead.
//!
//! Change detection polls. The portal does broadcast `SettingChanged`, but
//! consuming it needs a stream from `Proxy::receive_signal` and this backend is
//! the one place in the app that cannot be compiled or run on the machine the
//! rest is written on — so it is held to the three calls its read already
//! needs. Two seconds costs one bus round trip and lands the retint while the
//! settings window is still open. The signal is the obvious upgrade.

/// The fallback palette. The spec's `accent-color` is `(ddd)`, but a portal
/// that answers with a `u` is publishing a 1-based index into these swatches
/// (`0` is "no preference"), so such a desktop still gets its colour rather
/// than nothing.
const XDG_ACCENT_COLORS: [&str; 9] = [
    "#3584e4", // 1 blue
    "#2190a4", // 2 teal
    "#3a944a", // 3 green
    "#c88800", // 4 yellow
    "#ed5b00", // 5 orange
    "#e62d42", // 6 red
    "#d56199", // 7 pink
    "#9141ac", // 8 purple
    "#986a44", // 9 brown
];

/// Maps a palette-index `accent-color` to `#rrggbb`, or `None` for "no
/// preference" and for anything outside the palette (a newer desktop may add
/// one, and guessing a hue for it would be worse than leaving the shell on its
/// own default).
///
/// Lives outside the `cfg(target_os = "linux")` half so it is compiled and
/// tested on every host: a one-off index is exactly the kind of thing that
/// goes wrong by one and turns a blue desktop orange.
pub(super) fn accent_to_hex(value: u32) -> Option<String> {
    XDG_ACCENT_COLORS
        .get(value.checked_sub(1)? as usize)
        .map(|hex| (*hex).to_owned())
}

/// Converts the portal's spec `(ddd)` accent to `#rrggbb`.
///
/// The three channels are sRGB in `0…1`; a channel outside that range means the
/// desktop has no accent set, so this answers `None` rather than a clamped
/// colour, exactly as the spec says to treat out-of-range values. Compiled and
/// tested on every host for the same reason as [`accent_to_hex`].
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

/// One `0…1` sRGB channel as 8-bit, rounded to nearest.
fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(target_os = "linux")]
mod portal {
    use zbus::zvariant::{OwnedValue, Value};

    use super::{accent_to_hex, rgb_to_hex};

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "accent-color";

    /// `org.freedesktop.portal.Settings`, the portal interface that publishes
    /// desktop-wide appearance values. `default_service` / `default_path` are
    /// fixed by the spec, so a proxy is one call.
    #[zbus::proxy(
        interface = "org.freedesktop.portal.Settings",
        default_service = "org.freedesktop.portal.Desktop",
        default_path = "/org/freedesktop/portal/desktop"
    )]
    trait Settings {
        /// `Read(namespace, key) -> v`, the variant that carries the setting.
        fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
    }

    /// Unwraps the one variant level the portal's `Read` reply carries.
    ///
    /// `Read` returns `v`, so zbus hands that wrapper through as the body: an
    /// accent read arrives as `Value::Value(U32(…))`, not `Value::U32(…)`, and
    /// `u32::try_from` on the raw reply fails with "incorrect type" — which is
    /// exactly what it did, leaving the shell on its shipped accent on every
    /// Linux desktop. A value that is not a variant is passed through, so the
    /// helper is a no-op if a future zbus strips the wrapper itself. The same
    /// unwrap now lives in `system_theme::linux`, which reads the neighbouring
    /// `color-scheme` from the same interface.
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
            // The spec's `(ddd)`: three sRGB channels in `0…1`.
            Value::Structure(_) => {
                let (r, g, b) = <(f64, f64, f64)>::try_from(&value).ok()?;
                rgb_to_hex([r, g, b])
            }
            // A palette index, for a portal that reports the value that way.
            Value::U32(index) => accent_to_hex(*index),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unwraps_the_portal_variant() {
            let wrapped = Value::Value(Box::new(Value::U32(5)));
            assert!(matches!(unwrap_variant(wrapped), Value::U32(5)));
        }

        #[test]
        fn passes_a_bare_value_through() {
            assert!(matches!(unwrap_variant(Value::U32(1)), Value::U32(1)));
        }

        /// The `(ddd)` structure that the portal actually answers with has to
        /// come back through the tuple conversion as the same three channels —
        /// otherwise `read_accent` picks the `Structure` arm and then drops the
        /// value on the floor.
        #[test]
        fn extracts_the_rgb_tuple_from_a_structure() {
            let gnome_blue = [
                0.207_843_139_767_646_8,
                0.517_647_087_574_005_1,
                0.894_117_653_369_903_6,
            ];
            let value = Value::Structure(zbus::zvariant::Structure::from((
                gnome_blue[0],
                gnome_blue[1],
                gnome_blue[2],
            )));
            let (r, g, b) = <(f64, f64, f64)>::try_from(&value).unwrap();
            assert_eq!(rgb_to_hex([r, g, b]).as_deref(), Some("#3584e4"));
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<String> {
    tauri::async_runtime::block_on(portal::read_accent())
}

#[cfg(target_os = "linux")]
pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, read());

    let mut last = read();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = read();
        if now != last {
            last = now.clone();
            super::report(app, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal palette is 1-based and `0` means "no preference", so the
    /// whole map is shifted one off what a Rust index would be.
    #[test]
    fn maps_the_one_based_palette() {
        assert_eq!(accent_to_hex(1).as_deref(), Some("#3584e4")); // blue
        assert_eq!(accent_to_hex(5).as_deref(), Some("#ed5b00")); // orange
        assert_eq!(accent_to_hex(9).as_deref(), Some("#986a44")); // brown
    }

    #[test]
    fn no_preference_is_none() {
        assert_eq!(accent_to_hex(0), None);
    }

    #[test]
    fn a_palette_this_build_doesnt_know_falls_back() {
        // A desktop that adds a tenth accent must not get a guessed hue.
        assert_eq!(accent_to_hex(10), None);
        assert_eq!(accent_to_hex(u32::MAX), None);
    }

    #[test]
    fn every_entry_is_a_six_digit_hex() {
        for hex in XDG_ACCENT_COLORS {
            assert_eq!(hex.len(), 7);
            assert!(hex.starts_with('#'));
        }
    }

    /// The spec's `(ddd)` accent, at the default GNOME blue.
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

    /// The spec says an out-of-range channel is "no accent", not a clamped one.
    #[test]
    fn out_of_range_channels_are_no_accent() {
        assert_eq!(rgb_to_hex([-0.01, 0.5, 0.5]), None);
        assert_eq!(rgb_to_hex([0.5, 1.01, 0.5]), None);
    }
}
