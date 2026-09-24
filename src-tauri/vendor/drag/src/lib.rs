// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//!Start a drag operation out of a window on macOS, Windows and Linux (via GTK).
//!
//! Tested for [tao](https://github.com/tauri-apps/tao) (latest),
//! [winit](https://github.com/rust-windowing/winit) (latest),
//! [wry](https://github.com/tauri-apps/wry) (v0.24) and
//! [tauri](https://github.com/tauri-apps/tauri) (v1) windows.
//!
//! Due to the GTK-based implementation, winit currently cannot leverage this crate on Linux yet.
//!
//! - Add the `drag` dependency:
//!
//! `$ cargo add drag`
//!
//! - Use the `drag::start_drag` function. It takes a `&T: raw_window_handle::HasWindowHandle` type on macOS and Windows, and a `&gtk::ApplicationWindow` on Linux:
//!
//! - tao:
//!   ```rust,no_run
//!   let event_loop = tao::event_loop::EventLoop::new();
//!   let window = tao::window::WindowBuilder::new().build(&event_loop).unwrap();
//!
//!   let item = drag::DragItem::Files(vec![std::fs::canonicalize("./examples/icon.png").unwrap()]);
//!   let preview_icon = drag::Image::File("./examples/icon.png".into());
//!
//!   drag::start_drag(
//!     #[cfg(target_os = "linux")]
//!     {
//!       use tao::platform::unix::WindowExtUnix;
//!       window.gtk_window()
//!     },
//!     #[cfg(not(target_os = "linux"))]
//!     &window,
//!     item,
//!     preview_icon,
//!     |result, cursor_position| {
//!       println!("drag result: {result:?}");
//!     },
//!     drag::Options::default(),
//!   );
//!   ```
//!
//!   - wry:
//!   ```rust,no_run
//!   let event_loop = tao::event_loop::EventLoop::new();
//!   let window = tao::window::WindowBuilder::new().build(&event_loop).unwrap();
//!   let webview = wry::WebViewBuilder::new().build(&window).unwrap();
//!
//!   let item = drag::DragItem::Files(vec![std::fs::canonicalize("./examples/icon.png").unwrap()]);
//!   let preview_icon = drag::Image::File("./examples/icon.png".into());
//!
//!   drag::start_drag(
//!     #[cfg(target_os = "linux")]
//!     {
//!       use tao::platform::unix::WindowExtUnix;
//!       window.gtk_window()
//!     },
//!     #[cfg(not(target_os = "linux"))]
//!     &window,
//!     item,
//!     preview_icon,
//!     |result, cursor_position| {
//!       println!("drag result: {result:?}");
//!     },
//!     drag::Options::default(),
//!   );
//!   ```
//!
//!   - winit:
//!   ```rust,ignore
//!   let window = ...winit window;
//!
//!   let item = drag::DragItem::Files(vec![std::fs::canonicalize("./examples/icon.png").unwrap()]);
//!   let preview_icon = drag::Image::File("./examples/icon.png".into());
//!
//!   # #[cfg(not(target_os = "linux"))]
//!   let _ = drag::start_drag(&window, item, preview_icon, |result, cursor_position| {
//!     println!("drag result: {result:?}");
//!   }, Default::default());
//!   ```

use std::path::PathBuf;

mod platform_impl;
pub use platform_impl::start_drag;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(windows)]
    #[error("{0}")]
    WindowsError(#[from] windows::core::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unsupported window handle")]
    UnsupportedWindowHandle,
    #[error("failed to start drag")]
    FailedToStartDrag,
    #[error("drag image not found")]
    ImageNotFound,
    #[cfg(target_os = "linux")]
    #[error("empty drag target list")]
    EmptyTargetList,
    #[error("failed to drop items")]
    FailedToDrop,
    #[error("failed to get cursor position")]
    FailedToGetCursorPosition,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum DragResult {
    Dropped,
    Cancel,
}

pub type DataProvider = Box<dyn Fn(&str) -> Option<Vec<u8>>>;

/// Item to be dragged.
pub enum DragItem {
    /// A list of files to be dragged.
    ///
    /// The paths must be absolute.
    Files(Vec<PathBuf>),
    /// Data to share with another app.
    ///
    /// - **Windows**: Not supported. Will result in a dummy drag operation of current folder that will be cancelled upon dropping.
    /// - **Linux (gtk)**: Not supported. Will result in a dummy drag operation that contains nothing to drop.
    Data {
        provider: DataProvider,
        types: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(u64)]
pub enum DragMode {
    #[default]
    Copy = 1, // NSDragOperationCopy
    Move = 16, // NSDragOperationMove
    // Local patch: link/shortcut effect (DROPEFFECT_LINK, GDK_ACTION_LINK,
    // NSDragOperationLink).
    Link = 4,
    // Local patch: advertise copy, move and link at once and let the drop
    // target pick the effect — Windows Explorer's plain drag. The Shell folder
    // target is what decides move vs copy (same volume moves, across volumes
    // copies), and it can only do so when every effect is on offer; the forced
    // variants above stay for the modifier keys (Ctrl copies, Shift moves, Alt
    // links). The value is the union of the three effects above.
    Any = 21,
}

#[cfg(target_os = "macos")]
impl From<DragMode> for objc2_app_kit::NSDragOperation {
    fn from(value: DragMode) -> Self {
        match value {
            DragMode::Copy => objc2_app_kit::NSDragOperation::Copy,
            DragMode::Move => objc2_app_kit::NSDragOperation::Move,
            DragMode::Link => objc2_app_kit::NSDragOperation::Link,
            DragMode::Any => {
                objc2_app_kit::NSDragOperation::Copy
                    | objc2_app_kit::NSDragOperation::Move
                    | objc2_app_kit::NSDragOperation::Link
            }
        }
    }
}

#[derive(Default)]
pub struct Options {
    // TODO: Fix typo in v3
    pub skip_animatation_on_cancel_or_failure: bool,
    pub mode: DragMode,
    /// Cursor offset from the drag image's top-left corner, in physical pixels.
    /// Applied by the Windows drag implementations: the preview window used for
    /// data drags, and the Shell's drag image used for file drags.
    pub drag_image_offset: Option<CursorPosition>,
}

/// An image definition.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Image {
    /// A path to a image.
    File(PathBuf),
    /// Raw bytes of the image.
    Raw(Vec<u8>),
}

/// Logical position of the cursor.
///
/// - **Windows**: Currently the win32 API for logical position reports physical position as well, due to the complicated nature of potential multiple monitor with different scaling there's no trivial solution to be incorporated.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}
