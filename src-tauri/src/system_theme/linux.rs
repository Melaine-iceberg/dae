//! Linux's light/dark preference: `org.freedesktop.appearance color-scheme`,
//! read through the freedesktop settings portal.
//!
//! The portal is the only source that is right on every desktop. It is what
//! GNOME 42+, KDE and the XDG-compliant shells all publish, and it is the same
//! value the desktop's own Light/Dark toggle writes — behind a well-known bus
//! name, so it works on a desktop that has no GNOME settings daemon and no GTK
//! settings schema. Reading a GTK setting instead would work on one desktop and
//! quietly fall back on the others, and it is exactly the GTK-theme inference
//! this module exists to correct.
//!
//! Values are `0` no preference, `1` prefer-dark, `2` prefer-light. Anything
//! else — a newer desktop adding a value — reads as no preference rather than a
//! guess, which leaves the frontend on its `prefers-color-scheme` reading.
//!
//! Change detection polls. The portal does broadcast `SettingChanged`, but
//! consuming it needs a stream from `Proxy::receive_signal` and this backend is
//! the one place in the app that cannot be compiled or run on the machine the
//! rest is written on — so it is held to the three calls its read already
//! needs. Two seconds costs one bus round trip and lands the repaint while the
//! settings window is still open. The signal is the obvious upgrade. This is
//! the same trade `system_accent::linux` makes for the accent, for the same
//! reason.

/// Maps the portal's `color-scheme` value to the seam's `Option<bool>`.
/// `0` ("no preference") and anything outside the spec are `None`, so the
/// frontend keeps its media-query reading rather than snapping to a guess.
///
/// Lives outside the `cfg(target_os = "linux")` half so it is compiled and
/// tested on every host: the value is the one place a portal read can go wrong
/// by one, and this is what pins it.
pub(super) fn color_scheme(value: u32) -> Option<bool> {
    match value {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use zbus::zvariant::{OwnedValue, Value};

    use super::color_scheme;

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "color-scheme";

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
    /// `Read` returns `v`, so zbus hands that wrapper through as the body: a
    /// colour-scheme read arrives as `Value::Value(U32(2))`, not `Value::U32(2)`,
    /// and `u32::try_from` on the raw reply fails with "incorrect type". (This
    /// is the same unwrap `system_accent::linux` needs and, as of zbus 5.19,
    /// is missing.) A value that is not a variant is passed through, so the
    /// helper is a no-op if a future zbus strips the wrapper itself.
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
        // `zvariant` implements `TryFrom<Value> for u32`, which is what unwraps
        // the integer the variant carries.
        color_scheme(u32::try_from(unwrap_variant(Value::from(raw))).ok()?)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unwraps_the_portal_variant() {
            let wrapped = Value::Value(Box::new(Value::U32(2)));
            assert!(matches!(unwrap_variant(wrapped), Value::U32(2)));
        }

        #[test]
        fn passes_a_bare_value_through() {
            assert!(matches!(unwrap_variant(Value::U32(1)), Value::U32(1)));
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<bool> {
    tauri::async_runtime::block_on(portal::read_theme())
}

#[cfg(target_os = "linux")]
pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, read());

    let mut last = read();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = read();
        if now != last {
            last = now;
            super::report(app, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal's values, pinned so a read cannot silently invert the shell's
    /// appearance on every desktop at once.
    #[test]
    fn maps_the_portal_values() {
        assert_eq!(color_scheme(0), None); // no preference
        assert_eq!(color_scheme(1), Some(true)); // prefer-dark
        assert_eq!(color_scheme(2), Some(false)); // prefer-light
    }

    /// A desktop that adds a value must not get a guessed appearance.
    #[test]
    fn a_value_this_build_doesnt_know_falls_back() {
        assert_eq!(color_scheme(3), None);
        assert_eq!(color_scheme(u32::MAX), None);
    }
}
