//! Shell-wide helpers QML cannot answer from its own event stream.
//!
//! Tearing a tab out is defined by the pointer crossing the window edge, and a
//! `MouseArea` stops hearing about it at exactly that point. The tab strip
//! therefore polls this singleton while a drag is in flight: where the physical
//! cursor is, which of this application's windows is under it, and whether the
//! primary button is still down. The native half lives in `cpp/window.h`; this
//! module is the typed seam and the token allocator.
//!
//! The registry itself (token → QML window) stays in QML (`Shell.js`): windows
//! are QML objects, and handing them through Rust would buy nothing.

use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt_lib::QString;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("window.h");
        #[namespace = "dae_qt"]
        #[rust_name = "window_at"]
        fn dae_qt_window_at(x: i32, y: i32) -> QString;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_physical_x"]
        fn dae_qt_cursor_physical_x() -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_physical_y"]
        fn dae_qt_cursor_physical_y() -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_logical_x"]
        fn dae_qt_cursor_logical_x() -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_logical_y"]
        fn dae_qt_cursor_logical_y() -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "primary_button_down"]
        fn dae_qt_primary_button_down() -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "focus_window"]
        fn dae_qt_focus_window(token: &QString) -> bool;
    }

    unsafe extern "RustQt" {
        /// Process-wide shell helpers, reachable from QML as `Shell`.
        ///
        /// QML cannot mint a token that is unique across windows, cannot read
        /// the physical cursor, and cannot see the window under the pointer.
        /// Keeping those three here means the tab strip's drag logic has one
        /// source of truth instead of one per window.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Shell = super::ShellRust;

        #[qinvokable]
        #[cxx_name = "newToken"]
        fn new_token(&self) -> QString;

        /// Physical screen coordinates, the space `WindowFromPoint` speaks.
        #[qinvokable]
        #[cxx_name = "cursorPhysicalX"]
        fn cursor_physical_x(&self) -> i32;

        #[qinvokable]
        #[cxx_name = "cursorPhysicalY"]
        fn cursor_physical_y(&self) -> i32;

        /// Logical screen coordinates, the space `Window.x/y` speaks.
        #[qinvokable]
        #[cxx_name = "cursorLogicalX"]
        fn cursor_logical_x(&self) -> i32;

        #[qinvokable]
        #[cxx_name = "cursorLogicalY"]
        fn cursor_logical_y(&self) -> i32;

        /// The token of this application's window under a physical point, or
        /// an empty string over anything else.
        #[qinvokable]
        #[cxx_name = "windowAt"]
        fn window_at(&self, x: i32, y: i32) -> QString;

        /// Whether the physical primary button is still down. The tab drag
        /// ends on this going false, because the release itself may happen
        /// outside every window QML can hear from.
        #[qinvokable]
        #[cxx_name = "mouseDown"]
        fn mouse_down(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "focusWindow"]
        fn focus_window(&self, token: &QString) -> bool;
    }
}

#[derive(Default)]
pub struct ShellRust {
    next_token: AtomicU64,
}

impl qobject::Shell {
    fn new_token(&self) -> QString {
        let serial = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
        QString::from(format!("dae-window-{serial}").as_str())
    }

    fn cursor_physical_x(&self) -> i32 {
        qobject::cursor_physical_x()
    }

    fn cursor_physical_y(&self) -> i32 {
        qobject::cursor_physical_y()
    }

    fn cursor_logical_x(&self) -> i32 {
        qobject::cursor_logical_x()
    }

    fn cursor_logical_y(&self) -> i32 {
        qobject::cursor_logical_y()
    }

    fn window_at(&self, x: i32, y: i32) -> QString {
        qobject::window_at(x, y)
    }

    fn mouse_down(&self) -> bool {
        qobject::primary_button_down()
    }

    fn focus_window(&self, token: &QString) -> bool {
        qobject::focus_window(token)
    }
}
