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
        fn dae_qt_set_caption_client(
            token: &QString,
            left: f64,
            right: f64,
            plus_left: f64,
            plus_right: f64,
        ) -> bool;

        /// The system caption-icon family (Segoe Fluent Icons / Segoe MDL2
        /// Assets on Windows, empty elsewhere). The bar draws the window buttons
        /// with the same glyphs DWM uses.
        #[namespace = "dae_qt"]
        fn dae_qt_caption_icon_family() -> QString;

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
        fn dae_qt_set_ui_font(family: &QString);

        #[namespace = "dae_qt"]
        fn dae_qt_warm_font_database();

        #[namespace = "dae_qt"]
        fn dae_qt_startup_begin();

        #[namespace = "dae_qt"]
        fn dae_qt_startup_mark(stage: &QString);

        /// The native window handle as a number, or 0. Only the Windows share
        /// sheet consumes it; QML's `Window` has no `winId` of its own.
        #[namespace = "dae_qt"]
        #[rust_name = "native_window_handle"]
        fn dae_qt_native_handle(token: &QString) -> f64;

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
        fn dae_qt_load_after_shell(engine: usize, url: &QString);

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
        /// The caption button the pointer is over, as a column index (0 minimise,
        /// 1 maximise, 2 close) or -1. Windows only: the three columns are native
        /// non-client zones, so the native filter pushes this from the NC mouse
        /// state and the bar paints the system hover wash from it. -1 elsewhere.
        #[qproperty(i32, nc_button_hover, cxx_name = "ncButtonHover")]
        /// The caption button being held, same encoding as `ncButtonHover`.
        #[qproperty(i32, nc_button_pressed, cxx_name = "ncButtonPressed")]
        /// The system caption-icon family the bar draws its window-button glyphs
        /// with. Empty off Windows, where the drawn glyphs stay.
        #[qproperty(QString, caption_icon_family, cxx_name = "captionIconFamily")]
        type WindowController = super::WindowControllerRust;

        /// Applies the native frame, installs the IME focus guard and records
        /// the window (and, on Windows, its HWND). The QML window must be
        /// hidden until this returns.
        #[qinvokable]
        fn attach(self: Pin<&mut WindowController>) -> bool;

        /// Resolves the user's settings against this machine and applies the
        /// result. `requested` is "mica", "mica-alt", "acrylic" or "none"
        /// (Windows only); `blur` is the background-effects switch.
        #[qinvokable]
        #[cxx_name = "applyMaterial"]
        fn apply_material(
            self: Pin<&mut WindowController>,
            requested: &QString,
            blur: bool,
            dark: bool,
        ) -> bool;

        /// The horizontal slices of the chrome bar that must stay client
        /// areas, in dp from the client's left edge: first the tab chips, then
        /// the pinned `+` column. The rest of the bar drags the window.
        #[qinvokable]
        #[cxx_name = "setCaptionClient"]
        fn set_caption_client(
            self: Pin<&mut WindowController>,
            left: f64,
            right: f64,
            plus_left: f64,
            plus_right: f64,
        ) -> bool;

        /// What a double-click on the chrome bar should do here: `"zoom"`,
        /// `"minimize"` or `"none"`.
        ///
        /// The bar is content on Linux and macOS, so the platform never sees the
        /// second click; on macOS the user's own preference therefore has to be
        /// read and applied by QML. Read per call rather than cached, so a
        /// preference changed while dae runs applies on the next double-click.
        #[qinvokable]
        #[cxx_name = "titlebarDoubleAction"]
        fn titlebar_double_action(self: Pin<&mut WindowController>) -> QString;

        /// The HWND for this window, or 0 before it is attached. The Windows
        /// share sheet needs it; other platforms return 0 and never ask.
        #[qinvokable]
        #[cxx_name = "nativeHandle"]
        fn native_handle(self: Pin<&mut WindowController>) -> f64;

        /// Mirrors the shell's operation progress onto this window's Windows
        /// taskbar button. `state` is `none`, `indeterminate`, `normal` or
        /// `paused`; `progress` is a 0–1 fraction for the determinate states.
        /// Other platforms accept the call without drawing anything.
        #[qinvokable]
        #[cxx_name = "setTaskbarProgress"]
        fn set_taskbar_progress(
            self: Pin<&mut WindowController>,
            state: &QString,
            progress: f64,
        ) -> bool;
    }
}

pub struct WindowControllerRust {
    token: QString,
    attached: bool,
    applied_material: QString,
    blur_supported: bool,
    nc_button_hover: i32,
    nc_button_pressed: i32,
    caption_icon_family: QString,
}

impl Default for WindowControllerRust {
    fn default() -> Self {
        Self {
            token: QString::default(),
            attached: false,
            applied_material: QString::from("none"),
            blur_supported: false,
            nc_button_hover: -1,
            nc_button_pressed: -1,
            caption_icon_family: QString::default(),
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
        self.as_mut().set_blur_supported(blur_supported());
        self.as_mut()
            .set_caption_icon_family(qobject::dae_qt_caption_icon_family());
        startup_mark(&format!(
            "caption-icons [{}]",
            self.caption_icon_family.to_string()
        ));
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

    pub fn set_caption_client(
        self: Pin<&mut Self>,
        left: f64,
        right: f64,
        plus_left: f64,
        plus_right: f64,
    ) -> bool {
        let token = self.token.clone();
        qobject::set_window_caption_client(&token, left, right, plus_left, plus_right)
    }

    pub fn titlebar_double_action(self: Pin<&mut Self>) -> QString {
        QString::from(crate::platform::titlebar_double_click())
    }

    pub fn native_handle(self: Pin<&mut Self>) -> f64 {
        qobject::native_window_handle(&self.token)
    }

    pub fn set_taskbar_progress(self: Pin<&mut Self>, state: &QString, progress: f64) -> bool {
        #[cfg(windows)]
        {
            return windows_taskbar::set_progress(
                qobject::native_window_handle(&self.token),
                state.to_string().as_str(),
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
    qobject::dae_qt_enable_alpha();
}

/// Whether this session can blur a window behind it: Wayland with
/// `ext-background-effect-v1`, or macOS with `NSVisualEffectView`.
pub fn blur_supported() -> bool {
    qobject::dae_qt_blur_supported()
}

pub fn apply_fonts() {
    qobject::dae_qt_apply_fonts();
}

/// Applies the user's UI family before the first QML document exists. Empty
/// keeps the bundled Google Sans Flex; a later change applies on the next
/// launch.
pub fn set_ui_font(family: &str) {
    qobject::dae_qt_set_ui_font(&QString::from(family));
}

/// Starts the process-relative clock used by `DAE_STARTUP_PERF=1`.
pub fn startup_begin() {
    qobject::dae_qt_startup_begin();
}

/// Prints one `DAE_STARTUP_PERF=1` mark on stderr.
pub fn startup_mark(stage: &str) {
    qobject::dae_qt_startup_mark(&QString::from(stage));
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
    qobject::dae_qt_warm_font_database();
}

/// Hooks the main window's first `frameSwapped` for the startup timeline.
pub fn watch_first_frame(engine_ptr: usize) {
    qobject::dae_qt_watch_first_frame(engine_ptr);
}

/// Shows the stand-in window (`DAE_SHELL_FIRST=1`).
///
/// It is a bare `QQuickWindow` with no content item, so it never touches the
/// font database: its first frame can land while the QML tree is still being
/// built and while the one-time font cost is being paid behind it. The real
/// window's first frame drops it (`dae_qt_watch_first_frame`).
pub fn show_placeholder() {
    qobject::dae_qt_show_placeholder();
}

/// Drops the stand-in window if it is still up.
pub fn hide_placeholder() {
    qobject::dae_qt_hide_placeholder();
}

/// Loads `url` from the event loop, after the stand-in window's first frame.
///
/// Rust must not mark `qml-loaded` itself in this mode: the load happens inside
/// the C++ slot, which marks it.
pub fn load_after_shell(engine_ptr: usize, url: &str) {
    qobject::dae_qt_load_after_shell(engine_ptr, &QString::from(url));
}

pub fn enable_diagnostics() {
    qobject::install_message_handler();
}
