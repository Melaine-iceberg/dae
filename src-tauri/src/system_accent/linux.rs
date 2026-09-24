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
//! `accent-color` is not a colour: it is a 1-based index into the portal's
//! palette, which [`accent_to_hex`] maps to the swatch the desktop is drawing.
//!
//! Change detection polls. The portal does broadcast `SettingChanged`, but
//! consuming it needs a stream from `Proxy::receive_signal` and this backend is
//! the one place in the app that cannot be compiled or run on the machine the
//! rest is written on — so it is held to the three calls its read already
//! needs. Two seconds costs one bus round trip and lands the retint while the
//! settings window is still open. The signal is the obvious upgrade.

/// `org.freedesktop.appearance accent-color` values: `0` is "no preference",
/// then `1` blue … `9` brown. These are the swatches the GNOME and KDE accent
/// pickers draw, so the shell matches the desktop rather than approximating it
/// from a hue of its own.
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

/// Maps the portal's `accent-color` value to `#rrggbb`, or `None` for "no
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

#[cfg(target_os = "linux")]
mod portal {
    use zbus::zvariant::OwnedValue;

    use super::accent_to_hex;

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
        /// `Read(namespace, key) -> value`, with the value wrapped in a
        /// variant — hence `OwnedValue` rather than `u32`.
        fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
    }

    pub(super) async fn read_accent() -> Option<String> {
        let connection = zbus::Connection::session().await.ok()?;
        let proxy = SettingsProxy::new(&connection).await.ok()?;
        let raw = proxy.read(NAMESPACE, KEY).await.ok()?;
        // `zvariant` implements `TryFrom<OwnedValue> for u32`, which is what
        // unwraps the variant the reply carries.
        accent_to_hex(u32::try_from(raw).ok()?)
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
}
