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

use qtbridge::{QmlElement, qobject};

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("drag.h");
        #[namespace = "dae_qt"]
        #[rust_name = "start_tab_drag"]
        fn dae_qt_start_tab_drag_rs(
            source_token: &str,
            title: &str,
            tab_index: i32,
            grab_x: i32,
            grab_y: i32,
            width: i32,
            height: i32,
            background: &str,
            border: &str,
            text: &str,
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
        fn dae_qt_focus_window_rs(token: &str) -> bool;
    }
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
mod shell_impl {
    use super::*;

    #[derive(Default)]
    pub struct Shell {
        next_token: AtomicU64,
    }

    impl Shell {

        #[qslot]

        fn new_token(&self) -> String {
            let serial = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
            format!("dae-window-{serial}")
        }

        #[qslot]

        fn cursor_logical_x(&self) -> i32 {
            ffi::cursor_logical_x()
        }

        #[qslot]

        fn cursor_logical_y(&self) -> i32 {
            ffi::cursor_logical_y()
        }

        #[qslot]

        fn focus_window(&self, token: String) -> bool {
            ffi::focus_window(&token)
        }

        #[qslot]

        fn start_tab_drag(
            &self,
            source_token: String,
            title: String,
            tab_index: i32,
            grab_x: i32,
            grab_y: i32,
            width: i32,
            height: i32,
            background: String,
            border: String,
            text: String,
        ) -> i32 {
            ffi::start_tab_drag(
                &source_token,
                &title,
                tab_index,
                grab_x,
                grab_y,
                width,
                height,
                &background,
                &border,
                &text,
            )
        }
    }
}

pub use shell_impl::Shell;

impl QmlElement for Shell {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Shell";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}
