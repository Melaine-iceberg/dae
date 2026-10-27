//! Shell-wide helpers QML cannot answer from its own event stream.
//!
//! Tearing a tab out is decided by the platform's own drag loop: QML hands the
//! gesture over with `startTabDrag`, and the native half (`cpp/drag.cpp`) runs
//! it until the drop lands or the gesture is cancelled. That loop is what makes
//! the preview follow the grab point and what lets the pointer leave the window
//! on OLE, NSDraggingSession and X11/Wayland alike. This module is the typed
//! seam: the drag entry point, the logical cursor position the tear-off window
//! is placed at, and the token allocator.
//!
//! The registry itself (token → QML window) stays in QML (`Shell.js`): windows
//! are QML objects, and handing them through Rust would buy nothing.

#![allow(clippy::too_many_arguments)]

use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt_lib::{QColor, QString};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("drag.h");
        #[namespace = "dae_qt"]
        #[rust_name = "start_tab_drag"]
        fn dae_qt_start_tab_drag(
            source_token: &QString,
            title: &QString,
            tab_index: i32,
            grab_x: i32,
            grab_y: i32,
            width: i32,
            height: i32,
            background: &QColor,
            border: &QColor,
            text: &QColor,
        ) -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_logical_x"]
        fn dae_qt_cursor_logical_x() -> i32;

        #[namespace = "dae_qt"]
        #[rust_name = "cursor_logical_y"]
        fn dae_qt_cursor_logical_y() -> i32;

        include!("window.h");
        #[namespace = "dae_qt"]
        #[rust_name = "focus_window"]
        fn dae_qt_focus_window(token: &QString) -> bool;
    }

    unsafe extern "RustQt" {
        /// Process-wide shell helpers, reachable from QML as `Shell`.
        ///
        /// QML cannot mint a token that is unique across windows, cannot run
        /// the platform's drag loop, and cannot read the logical global cursor
        /// when a drag ends outside the window. Keeping those here means the
        /// tab strip's drag logic has one source of truth instead of one per
        /// window.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Shell = super::ShellRust;

        #[qinvokable]
        #[cxx_name = "newToken"]
        fn new_token(&self) -> QString;

        /// Logical screen coordinates, the space QML `Window.x/y` speaks.
        #[qinvokable]
        #[cxx_name = "cursorLogicalX"]
        fn cursor_logical_x(&self) -> i32;

        #[qinvokable]
        #[cxx_name = "cursorLogicalY"]
        fn cursor_logical_y(&self) -> i32;

        #[qinvokable]
        #[cxx_name = "focusWindow"]
        fn focus_window(&self, token: &QString) -> bool;

        /// Runs the platform drag loop for a tab, blocking until it ends. The
        /// preview is the tab chip spotted at `grabX`/`grabY`, so it follows
        /// the point the user grabbed, not the cursor-centred convention of a
        /// plain drag image. Returns the resulting `Qt::DropAction` as an int;
        /// `Qt::IgnoreAction` (0) means nothing took the drop.
        #[qinvokable]
        #[cxx_name = "startTabDrag"]
        fn start_tab_drag(
            &self,
            source_token: &QString,
            title: &QString,
            tab_index: i32,
            grab_x: i32,
            grab_y: i32,
            width: i32,
            height: i32,
            background: &QColor,
            border: &QColor,
            text: &QColor,
        ) -> i32;
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

    fn cursor_logical_x(&self) -> i32 {
        qobject::cursor_logical_x()
    }

    fn cursor_logical_y(&self) -> i32 {
        qobject::cursor_logical_y()
    }

    fn focus_window(&self, token: &QString) -> bool {
        qobject::focus_window(token)
    }

    fn start_tab_drag(
        &self,
        source_token: &QString,
        title: &QString,
        tab_index: i32,
        grab_x: i32,
        grab_y: i32,
        width: i32,
        height: i32,
        background: &QColor,
        border: &QColor,
        text: &QColor,
    ) -> i32 {
        qobject::start_tab_drag(
            source_token,
            title,
            tab_index,
            grab_x,
            grab_y,
            width,
            height,
            background,
            border,
            text,
        )
    }
}
