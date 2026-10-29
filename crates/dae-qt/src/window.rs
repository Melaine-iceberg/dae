//! QML-facing window controller.
//!
//! One instance per window. It only carries the window's token; on Windows the
//! native state lives in `cpp/window.h`, keyed by HWND, so a second window uses
//! the same style/backdrop/hit-test path as the first (gate 4) instead of a
//! parallel one. Linux and macOS keep the same entry points as portable
//! fallbacks: there is no DWM or caption hit test to feed there.
//!
//! QML usage:
//!
//! ```qml
//! Window {
//!     objectName: "dae-main"
//!     WindowController { id: chrome; token: parent.objectName }
//!     Component.onCompleted: {
//!         chrome.attach()
//!         chrome.applyMaterial(Appearance.windowMaterial, Appearance.blurEnabled, true)
//!     }
//! }
//! ```

use core::pin::Pin;

use cxx_qt_lib::QString;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("window.h");
        #[namespace = "dae_qt"]
        #[rust_name = "attach_window"]
        fn dae_qt_attach(token: &QString) -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "apply_window_backdrop"]
        fn dae_qt_apply_backdrop(token: &QString, material: i32, dark: bool) -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "set_window_caption_client"]
        fn dae_qt_set_caption_client(token: &QString, left: f64, right: f64) -> bool;

        #[namespace = "dae_qt"]
        fn dae_qt_enable_alpha();

        /// True when the compositor advertises `ext-background-effect-v1` and
        /// the platform plugin is Wayland. Always false on Windows and macOS.
        #[namespace = "dae_qt"]
        fn dae_qt_wayland_blur_supported() -> bool;

        /// Selects the platform's UI face for the whole application. Only the
        /// Windows variant does anything; the other platforms already default
        /// to their system font.
        #[namespace = "dae_qt"]
        fn dae_qt_apply_system_font();

        #[namespace = "dae_qt"]
        #[rust_name = "install_message_handler"]
        fn dae_qt_install_message_handler();
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, token)]
        #[qproperty(bool, attached)]
        /// The material the platform actually drew last: `mica`, `acrylic`,
        /// `blur` or `none`. QML paints translucent chrome for anything but
        /// `none`, so an unavailable choice must report `none` here even
        /// though the requested value stays in settings.
        #[qproperty(QString, applied_material, cxx_name = "appliedMaterial")]
        /// Whether this session can blur a window at all (Wayland only; the
        /// Windows materials are resolved separately). The settings panel
        /// reads it to say why blur is unavailable.
        #[qproperty(bool, blur_supported, cxx_name = "blurSupported")]
        type WindowController = super::WindowControllerRust;

        /// Applies the native frame, installs the IME focus guard and records
        /// the window (and, on Windows, its HWND). The QML window must be
        /// hidden until this returns.
        #[qinvokable]
        fn attach(self: Pin<&mut WindowController>) -> bool;

        /// Resolves the user's settings against this machine and applies the
        /// result. `requested` is "mica", "acrylic" or "none" (Windows only);
        /// `blur` is the background-effects switch.
        #[qinvokable]
        #[cxx_name = "applyMaterial"]
        fn apply_material(
            self: Pin<&mut WindowController>,
            requested: &QString,
            blur: bool,
            dark: bool,
        ) -> bool;

        /// The horizontal slice of the chrome bar that carries the tabs, in dp
        /// from the client's left edge. The rest of the bar drags the window.
        #[qinvokable]
        #[cxx_name = "setCaptionClient"]
        fn set_caption_client(self: Pin<&mut WindowController>, left: f64, right: f64) -> bool;
    }
}

pub struct WindowControllerRust {
    token: QString,
    attached: bool,
    applied_material: QString,
    blur_supported: bool,
}

impl Default for WindowControllerRust {
    fn default() -> Self {
        Self {
            token: QString::default(),
            attached: false,
            applied_material: QString::from("none"),
            blur_supported: false,
        }
    }
}

impl qobject::WindowController {
    pub fn attach(mut self: Pin<&mut Self>) -> bool {
        let token = self.token.clone();
        if token.as_slice().is_empty() {
            return false;
        }
        let attached =
            qobject::attach_window(&token) && crate::ime::install_focus_cancel_guard(&token);
        self.as_mut().set_attached(attached);
        self.as_mut()
            .set_blur_supported(wayland_blur_supported());
        attached
    }

    pub fn apply_material(
        mut self: Pin<&mut Self>,
        requested: &QString,
        blur: bool,
        dark: bool,
    ) -> bool {
        let material = crate::material::resolve(blur, &requested.to_string());
        let token = self.token.clone();
        let applied = qobject::apply_window_backdrop(&token, material.code(), dark);
        // An apply call for a window that is not attached yet is not a
        // material answer; keep the previous value rather than paint chrome
        // solid and then translucent again on the next call.
        if applied {
            let drawn = if material == crate::material::Material::None {
                "none"
            } else {
                material.as_str()
            };
            self.as_mut().set_applied_material(QString::from(drawn));
        } else if material != crate::material::Material::None {
            self.as_mut().set_applied_material(QString::from("none"));
        }
        applied
    }

    pub fn set_caption_client(self: Pin<&mut Self>, left: f64, right: f64) -> bool {
        let token = self.token.clone();
        qobject::set_window_caption_client(&token, left, right)
    }
}

pub fn enable_alpha() {
    qobject::dae_qt_enable_alpha();
}

/// Whether this session can blur a window behind it. Only Wayland compositors
/// that advertise `ext-background-effect-v1` answer true.
pub fn wayland_blur_supported() -> bool {
    qobject::dae_qt_wayland_blur_supported()
}

pub fn apply_system_font() {
    qobject::dae_qt_apply_system_font();
}

pub fn enable_diagnostics() {
    qobject::install_message_handler();
}
