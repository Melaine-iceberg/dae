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

use std::cell::{Cell, RefCell};

use qtbridge::{QmlElement, QmlObject, qobject};

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("window.h");
        #[namespace = "dae_qt"]
        #[rust_name = "attach_window"]
        fn dae_qt_attach_rs(token: &str) -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "apply_window_backdrop"]
        fn dae_qt_apply_backdrop_rs(token: &str, material: i32, dark: bool) -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "set_window_caption_client"]
        fn dae_qt_set_caption_client_rs(
            token: &str,
            left: f64,
            right: f64,
            plus_left: f64,
            plus_right: f64,
        ) -> bool;

        /// The system caption-icon family (Segoe Fluent Icons / Segoe MDL2
        /// Assets on Windows, empty elsewhere). The bar draws the window buttons
        /// with the same glyphs DWM uses.
        #[namespace = "dae_qt"]
        #[rust_name = "dae_qt_caption_icon_family"]
        fn dae_qt_caption_icon_family_rs() -> String;

        #[namespace = "dae_qt"]
        fn dae_qt_enable_alpha();

        /// True when this platform can blur a window behind it: Wayland with
        /// `ext-background-effect-v1`, or macOS with `NSVisualEffectView`.
        /// Always false on Windows, where Mica/Acrylic are resolved separately.
        #[namespace = "dae_qt"]
        fn dae_qt_blur_supported() -> bool;

        /// Loads the bundled UI faces and selects the application font: Google
        /// Sans Flex for Latin and numbers with the platform face behind it for
        /// Chinese, Google Sans Code for fixed pitch, and the platform's own
        /// faces behind them (proposal §6.2.1, §9.2).
        #[namespace = "dae_qt"]
        fn dae_qt_apply_fonts();

        /// Replaces the application font's first family with the user's choice
        /// from settings.toml. Empty keeps the bundled Google Sans Flex.
        /// Existing QML `Text` items keep their font; the shell applies this
        /// before the first document is created.
        #[namespace = "dae_qt"]
        #[rust_name = "dae_qt_set_ui_font"]
        fn dae_qt_set_ui_font_rs(family: &str);

        #[namespace = "dae_qt"]
        fn dae_qt_warm_font_database();

        #[namespace = "dae_qt"]
        fn dae_qt_startup_begin();

        #[namespace = "dae_qt"]
        #[rust_name = "dae_qt_startup_mark"]
        fn dae_qt_startup_mark_rs(stage: &str);

        /// The native window handle as a number, or 0. Only the Windows share
        /// sheet consumes it; QML's `Window` has no `winId` of its own.
        #[namespace = "dae_qt"]
        #[rust_name = "native_window_handle"]
        fn dae_qt_native_handle_rs(token: &str) -> f64;

        #[namespace = "dae_qt"]
        fn dae_qt_watch_first_frame(engine: usize);

        /// Puts a solid, text-free window on screen while the QML tree is still
        /// being built (`DAE_SHELL_FIRST=1`). Dropped automatically on the real
        /// window's first frame.
        #[namespace = "dae_qt"]
        fn dae_qt_show_placeholder();

        #[namespace = "dae_qt"]
        fn dae_qt_hide_placeholder();

        /// Loads the QML root only after the stand-in window has presented its
        /// first frame.
        #[namespace = "dae_qt"]
        #[rust_name = "dae_qt_load_after_shell"]
        fn dae_qt_load_after_shell_rs(engine: usize, url: &str);

        #[namespace = "dae_qt"]
        #[rust_name = "install_message_handler"]
        fn dae_qt_install_message_handler();
    }
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
mod window_impl {
    use super::*;

    pub struct WindowController {
        token: RefCell<String>,
        attached: Cell<bool>,
        applied_material: RefCell<String>,
        blur_supported: Cell<bool>,
        nc_button_hover: Cell<i32>,
        nc_button_pressed: Cell<i32>,
        caption_icon_family: RefCell<String>,
    }

    impl Default for WindowController {
        fn default() -> Self {
            Self {
                token: RefCell::new(String::new()),
                attached: Cell::new(false),
                applied_material: RefCell::new("none".to_owned()),
                blur_supported: Cell::new(false),
                nc_button_hover: Cell::new(-1),
                nc_button_pressed: Cell::new(-1),
                caption_icon_family: RefCell::new(String::new()),
            }
        }
    }

    impl WindowController {
        qproperty!("token", Read = get_token, Write = set_token, Notify = token_changed);
        qproperty!("attached", Read = get_attached, Notify = attached_changed);
        qproperty!("appliedMaterial", Read = get_applied_material, Notify = applied_material_changed);
        qproperty!("blurSupported", Read = get_blur_supported, Notify = blur_supported_changed);
        qproperty!("ncButtonHover", Read = get_nc_button_hover, Write = set_nc_button_hover, Notify = nc_button_hover_changed);
        qproperty!("ncButtonPressed", Read = get_nc_button_pressed, Write = set_nc_button_pressed, Notify = nc_button_pressed_changed);
        qproperty!("captionIconFamily", Read = get_caption_icon_family, Notify = caption_icon_family_changed);

        #[qsignal]
        fn token_changed(&mut self);
        #[qsignal]
        fn attached_changed(&mut self);
        #[qsignal]
        fn applied_material_changed(&mut self);
        #[qsignal]
        fn blur_supported_changed(&mut self);
        #[qsignal]
        fn nc_button_hover_changed(&mut self);
        #[qsignal]
        fn nc_button_pressed_changed(&mut self);
        #[qsignal]
        fn caption_icon_family_changed(&mut self);

        fn get_token(&self) -> String {
            self.token.borrow().clone()
        }

        fn set_token(&mut self, value: String) {
            if *self.token.borrow() != value {
                *self.token.borrow_mut() = value;
                self.token_changed();
            }
        }

        fn get_attached(&self) -> bool {
            self.attached.get()
        }

        fn get_applied_material(&self) -> String {
            self.applied_material.borrow().clone()
        }

        fn get_blur_supported(&self) -> bool {
            self.blur_supported.get()
        }

        fn get_nc_button_hover(&self) -> i32 {
            self.nc_button_hover.get()
        }

        fn set_nc_button_hover(&mut self, value: i32) {
            if self.nc_button_hover.get() != value {
                self.nc_button_hover.set(value);
                self.nc_button_hover_changed();
            }
        }

        fn get_nc_button_pressed(&self) -> i32 {
            self.nc_button_pressed.get()
        }

        fn set_nc_button_pressed(&mut self, value: i32) {
            if self.nc_button_pressed.get() != value {
                self.nc_button_pressed.set(value);
                self.nc_button_pressed_changed();
            }
        }

        fn get_caption_icon_family(&self) -> String {
            self.caption_icon_family.borrow().clone()
        }

        #[qslot]

        pub fn attach(&self) -> bool {
            let token = self.token.borrow().clone();
            if token.is_empty() {
                return false;
            }
            let attached =
                ffi::attach_window(&token) && crate::ime::install_focus_cancel_guard(&token);
            self.attached.set(attached);
            self.blur_supported.set(blur_supported());
            let family = ffi::dae_qt_caption_icon_family();
            *self.caption_icon_family.borrow_mut() = family.clone();
            startup_mark(&format!("caption-icons [{family}]"));
            self.get_thread_handle().invoke_mut(|controller| {
                controller.attached_changed();
                controller.blur_supported_changed();
                controller.caption_icon_family_changed();
            });
            attached
        }

        #[qslot]

        pub fn apply_material(
            &self,
            requested: String,
            blur: bool,
            dark: bool,
        ) -> bool {
            let material = crate::material::resolve(blur, &requested);
            let token = self.token.borrow().clone();
            let applied = ffi::apply_window_backdrop(&token, material.code(), dark);
            // An apply call for a window that is not attached yet is not a
            // material answer; keep the previous value rather than paint chrome
            // solid and then translucent again on the next call.
            let next = if applied {
                Some(
                    if material == crate::material::Material::None {
                        "none"
                    } else {
                        material.as_str()
                    }
                    .to_owned(),
                )
            } else if material != crate::material::Material::None {
                Some("none".to_owned())
            } else {
                None
            };
            if let Some(drawn) = next {
                if *self.applied_material.borrow() != drawn {
                    *self.applied_material.borrow_mut() = drawn;
                    self.get_thread_handle().invoke_mut(|controller| {
                        controller.applied_material_changed();
                    });
                }
            }
            applied
        }

        #[qslot]

        pub fn set_caption_client(
            &self,
            left: f64,
            right: f64,
            plus_left: f64,
            plus_right: f64,
        ) -> bool {
            let token = self.token.borrow().clone();
            ffi::set_window_caption_client(&token, left, right, plus_left, plus_right)
        }

        #[qslot]

        pub fn titlebar_double_action(&self) -> String {
            crate::platform::titlebar_double_click().to_owned()
        }

        #[qslot]

        pub fn native_handle(&self) -> f64 {
            ffi::native_window_handle(&self.token.borrow())
        }

        #[qslot]

        pub fn set_taskbar_progress(&self, state: String, progress: f64) -> bool {
            #[cfg(windows)]
            {
                return windows_taskbar::set_progress(
                    ffi::native_window_handle(&self.token.borrow()),
                    state.as_str(),
                    progress,
                );
            }
            #[cfg(not(windows))]
            {
                let _ = (state, progress);
                false
            }
        }
    }
}

pub use window_impl::WindowController;

impl QmlElement for WindowController {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "WindowController";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}

/// The taskbar COM object is apartment-bound. It is therefore cached on the
/// calling (GUI) thread rather than behind a process-wide mutex, and every
/// window still gets its own HWND-specific progress value.
#[cfg(windows)]
mod windows_taskbar {
    use std::cell::RefCell;

    use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        ITaskbarList3, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED, TaskbarList,
    };
    use windows::core::Result;

    struct TaskbarCom {
        taskbar: Option<ITaskbarList3>,
        uninitialize: bool,
    }

    impl TaskbarCom {
        fn new() -> Self {
            // Qt normally initializes OLE for drag-and-drop, but taskbar
            // progress can be the first shell COM call in a session. Balance
            // only the reference this call actually acquires; if Qt already
            // chose an MTA, do not try to change its apartment.
            let apartment = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let uninitialize = apartment.is_ok();
            if apartment.is_err() && apartment != RPC_E_CHANGED_MODE {
                log::warn!("dae-taskbar COM initialization failed: {apartment:?}");
                return Self {
                    taskbar: None,
                    uninitialize: false,
                };
            }

            let created: Result<ITaskbarList3> =
                unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) };
            let taskbar = match created {
                Ok(taskbar) => match unsafe { taskbar.HrInit() } {
                    Ok(()) => Some(taskbar),
                    Err(error) => {
                        log::warn!("dae-taskbar initialization failed: {error}");
                        None
                    }
                },
                Err(error) => {
                    log::warn!("dae-taskbar creation failed: {error}");
                    None
                }
            };
            Self {
                taskbar,
                uninitialize,
            }
        }
    }

    impl Drop for TaskbarCom {
        fn drop(&mut self) {
            // Release the apartment-bound interface before balancing our COM
            // initialization reference.
            self.taskbar = None;
            if self.uninitialize {
                unsafe { CoUninitialize() };
            }
        }
    }

    thread_local! {
        static TASKBAR: RefCell<Option<TaskbarCom>> = const { RefCell::new(None) };
    }

    pub(super) fn set_progress(handle: f64, state: &str, progress: f64) -> bool {
        if !handle.is_finite() || handle <= 0.0 {
            return false;
        }
        let hwnd = HWND(handle as usize as *mut core::ffi::c_void);
        let flag = match state {
            "none" => TBPF_NOPROGRESS,
            "indeterminate" => TBPF_INDETERMINATE,
            "normal" => TBPF_NORMAL,
            "paused" => TBPF_PAUSED,
            _ => return false,
        };
        TASKBAR.with(|slot| {
            let mut slot = slot.borrow_mut();
            // Starting or closing a quiet shell must not pay for COM. Once an
            // operation did create the object, the same call clears its bar.
            if slot.is_none() && flag == TBPF_NOPROGRESS {
                return true;
            }
            let com = slot.get_or_insert_with(TaskbarCom::new);
            let Some(taskbar) = com.taskbar.as_ref() else {
                return false;
            };
            // SAFETY: the HWND comes from this GUI thread's live QQuickWindow;
            // the interface was initialized on that same thread.
            if unsafe { taskbar.SetProgressState(hwnd, flag) }.is_err() {
                return false;
            }
            if flag == TBPF_NORMAL || flag == TBPF_PAUSED {
                let fraction = if progress.is_finite() {
                    progress.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let completed = (fraction * 1000.0).round() as u64;
                return unsafe { taskbar.SetProgressValue(hwnd, completed, 1000) }.is_ok();
            }
            true
        })
    }
}

pub fn enable_alpha() {
    ffi::dae_qt_enable_alpha();
}

/// Whether this session can blur a window behind it: Wayland with
/// `ext-background-effect-v1`, or macOS with `NSVisualEffectView`.
pub fn blur_supported() -> bool {
    ffi::dae_qt_blur_supported()
}

pub fn apply_fonts() {
    ffi::dae_qt_apply_fonts();
}

/// Applies the user's UI family before the first QML document exists. Empty
/// keeps the bundled Google Sans Flex; a later change applies on the next
/// launch.
pub fn set_ui_font(family: &str) {
    ffi::dae_qt_set_ui_font(family);
}

/// Starts the process-relative clock used by `DAE_STARTUP_PERF=1`.
pub fn startup_begin() {
    ffi::dae_qt_startup_begin();
}

/// Prints one `DAE_STARTUP_PERF=1` mark on stderr.
pub fn startup_mark(stage: &str) {
    ffi::dae_qt_startup_mark(stage);
}

/// Prints a mark only the first time this stage is reported.
///
/// QML calls this from every view that can reach a startup moment (the
/// overview and the explorer list both observe their models, and the hidden
/// one can publish rows too). The first report is the one that belongs on the
/// timeline; later ones would be navigation, not startup.
pub fn startup_mark_once(stage: &str) {
    static SEEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::LazyLock::new(Default::default);
    let mut seen = match SEEN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if seen.insert(stage.to_owned()) {
        startup_mark(stage);
    }
}

/// Populates the font database off the GUI thread (Windows only; Qt documents
/// the database as thread-safe).
pub fn warm_font_database() {
    ffi::dae_qt_warm_font_database();
}

/// Hooks the main window's first `frameSwapped` for the startup timeline.
pub fn watch_first_frame(engine_ptr: usize) {
    ffi::dae_qt_watch_first_frame(engine_ptr);
}

/// Shows the stand-in window (`DAE_SHELL_FIRST=1`).
///
/// It is a bare `QQuickWindow` with no content item, so it never touches the
/// font database: its first frame can land while the QML tree is still being
/// built and while the one-time font cost is being paid behind it. The real
/// window's first frame drops it (`dae_qt_watch_first_frame`).
pub fn show_placeholder() {
    ffi::dae_qt_show_placeholder();
}

/// Drops the stand-in window if it is still up.
pub fn hide_placeholder() {
    ffi::dae_qt_hide_placeholder();
}

/// Loads `url` from the event loop, after the stand-in window's first frame.
///
/// Rust must not mark `qml-loaded` itself in this mode: the load happens inside
/// the C++ slot, which marks it.
pub fn load_after_shell(engine_ptr: usize, url: &str) {
    ffi::dae_qt_load_after_shell(engine_ptr, url);
}

pub fn enable_diagnostics() {
    ffi::install_message_handler();
}
