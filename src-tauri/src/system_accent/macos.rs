//! macOS' accent: `NSColor.controlAccentColor`, the same colour AppKit draws
//! a focused checkbox in. Reading the colour rather than `AppleAccentColor`
//! from NSUserDefaults is deliberate: that key is an *index* into Apple's
//! preset swatches, says nothing when the user is on "Multicolor", and has no
//! value at all for the custom accent the General pane allows. The resolved
//! colour is the right answer whatever the user picked.
//!
//! There is no change detection, only polling. macOS posts nothing public when
//! the accent moves — `AppleInterfaceThemeChangedNotification` covers the
//! light/dark switch and nothing else, and KVO on the global NSUserDefaults
//! domain is not supported. Two seconds is short enough that the retint lands
//! while the Settings window is still on screen, and `controlAccentColor` is a
//! cached object lookup rather than IPC, so the read is free.

use std::time::Duration;

use objc2_app_kit::{NSColor, NSColorSpace};

use super::SystemAccentChanged;

/// `NSColor` components are `CGFloat` in 0…1; the seam wants 8-bit sRGB.
fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(super) fn read() -> Option<String> {
    // `controlAccentColor` is a dynamic colour: it means "the accent" until it
    // is asked for concrete components, and only answers those in a real
    // colour space. Without the conversion every component reads 0 and the
    // accent silently becomes black.
    let accent = NSColor::controlAccentColor();
    let rgb = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(super::srgb_hex(
        channel(rgb.redComponent() as f64),
        channel(rgb.greenComponent() as f64),
        channel(rgb.blueComponent() as f64),
    ))
}

pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, read());

    let mut last = read();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let now = read();
        if now != last {
            last = now.clone();
            super::report(app, now);
        }
    }
}
