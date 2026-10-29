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
//!         chrome.applyMaterial(chrome.startupMaterial(), true)
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
        type WindowController = super::WindowControllerRust;

        /// Applies the native frame, installs the IME focus guard and records
        /// the window (and, on Windows, its HWND). The QML window must be
        /// hidden until this returns.
        #[qinvokable]
        fn attach(self: Pin<&mut WindowController>) -> bool;

        /// `material` is "mica", "acrylic" or "none".
        #[qinvokable]
        #[cxx_name = "applyMaterial"]
        fn apply_material(self: Pin<&mut WindowController>, material: &QString, dark: bool)
        -> bool;

        /// The horizontal slice of the chrome bar that carries the tabs, in dp
        /// from the client's left edge. The rest of the bar drags the window.
        #[qinvokable]
        #[cxx_name = "setCaptionClient"]
        fn set_caption_client(self: Pin<&mut WindowController>, left: f64, right: f64) -> bool;

        #[qinvokable]
        #[cxx_name = "startupMaterial"]
        fn startup_material(&self) -> QString;
    }
}

#[derive(Default)]
pub struct WindowControllerRust {
    token: QString,
    attached: bool,
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
        attached
    }

    pub fn apply_material(self: Pin<&mut Self>, material: &QString, dark: bool) -> bool {
        let Some(material) = crate::material::Material::from_name(&material.to_string()) else {
            return false;
        };
        let token = self.token.clone();
        qobject::apply_window_backdrop(&token, material.code(), dark)
    }

    pub fn startup_material(&self) -> QString {
        QString::from(crate::material::startup().as_str())
    }

    pub fn set_caption_client(self: Pin<&mut Self>, left: f64, right: f64) -> bool {
        let token = self.token.clone();
        qobject::set_window_caption_client(&token, left, right)
    }
}

pub fn enable_alpha() {
    qobject::dae_qt_enable_alpha();
}

pub fn apply_system_font() {
    qobject::dae_qt_apply_system_font();
}

pub fn enable_diagnostics() {
    qobject::install_message_handler();
}
