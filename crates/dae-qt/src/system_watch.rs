//! Native OS watchers for the things Qt Quick does not report on its own.
//!
//! Three questions move while the shell is on screen:
//!
//! * **The accent colour.** Read once at startup by [`crate::platform`], it
//!   changes when the user picks another colour in Settings. The watcher
//!   blocks on the DWM key and re-reads it through the same decoder.
//! * **The light/dark preference.** The native backdrop and every QML colour
//!   derive from it; a registry notification on the Personalize key is the
//!   same source Qt's platform style hints poll.
//! * **The mounted volumes.** A USB stick plugged in, a phone connected or a
//!   share unmounted does not change anything the listing watcher sees. The
//!   `WM_DEVICECHANGE` broadcast is what Explorer's drive bar answers, and it
//!   is what the sidebar answers by calling `SidebarModel.refresh()`.
//!
//! Every watcher blocks a dedicated thread for the life of the process, the
//! same shape as `dae-core`'s event seams: one thread per OS channel, no poll,
//! no shutdown to get wrong because process exit reaps them. Readings are
//! queued onto the Qt thread before touching `Workspace`, so the watcher
//! threads never see a QObject directly.
//!
//! The native half is Windows-first (Win32 + the registry), matching
//! `window.h` and `platform.rs`. Other platforms keep the single startup
//! reading and are wired up with their own measured gate rather than an
//! untested guess.

#[cfg(not(windows))]
use cxx_qt::CxxQtThread;

#[cfg(not(windows))]
use crate::workspace::qobject::Workspace;

/// Starts the platform watchers once for the process. Calling it again from a
/// later window is a no-op.
#[cfg(not(windows))]
pub(crate) fn start(_thread: CxxQtThread<Workspace>) {}

#[cfg(windows)]
pub(crate) use windows_backend::start;

#[cfg(windows)]
mod windows_backend {
    use std::io::Write;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cxx_qt::CxxQtThread;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_DWORD, REG_NOTIFY_CHANGE_LAST_SET,
        REG_SAM_FLAGS, RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME,
        DEV_BROADCAST_HDR, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, MSG,
        RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DEVICECHANGE,
        WM_TIMER, WNDCLASSW,
    };
    use windows::core::PCWSTR;

    use crate::workspace::qobject::Workspace;

    const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";
    const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    /// `1` when apps use the light theme, `0` when they use the dark one.
    const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme";

    const WINDOW_CLASS: &str = "DaeQtVolumeMonitor";
    const VOLUME_REFRESH_TIMER: usize = 1;
    /// One plug produces a burst of drive, volume and mount events; collapse
    /// them into one re-list so the sidebar does not run several overlapping
    /// disk probes whose results can land out of order.
    const VOLUME_DEBOUNCE_MS: u32 = 350;

    /// The window procedure has no user data, so the process-wide sink is the
    /// hand-off. The monitor is started once.
    static SINK: OnceLock<Sink> = OnceLock::new();
    static VOLUME_REFRESH_PENDING: AtomicBool = AtomicBool::new(false);

    #[derive(Clone)]
    struct Sink {
        thread: CxxQtThread<Workspace>,
    }

    impl Sink {
        fn accent(&self, accent: Option<String>) {
            let _ = self.thread.queue(move |mut workspace| {
                workspace.as_mut().apply_system_accent(accent);
            });
        }

        fn theme(&self, dark: Option<bool>) {
            let _ = self.thread.queue(move |mut workspace| {
                workspace.as_mut().apply_system_theme(dark);
            });
        }

        fn volumes(&self) {
            let _ = self.thread.queue(|mut workspace| {
                workspace.as_mut().notify_volumes_changed();
            });
        }
    }

    pub(crate) fn start(thread: CxxQtThread<Workspace>) {
        static STARTED: OnceLock<()> = OnceLock::new();

        let sink = Sink { thread };
        STARTED.get_or_init(move || {
            let accent = sink.clone();
            spawn_thread("dae-accent-watch", move || watch_accent(accent));

            let theme = sink.clone();
            spawn_thread("dae-theme-watch", move || watch_theme(theme));

            spawn_thread("dae-volume-watch", move || watch_volumes(sink));
        });
    }

    fn spawn_thread(name: &str, body: impl FnOnce() + Send + 'static) {
        if let Err(error) = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(body)
        {
            log(format!("dae: failed to start {name}: {error}"));
        }
    }

    /// Encodes a Rust string as a null-terminated UTF-16 vector for `PCWSTR`.
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_key(path: &str, access: REG_SAM_FLAGS) -> Option<HKEY> {
        let path = wide(path);
        let mut key = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `key` is a
        // valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                access,
                &mut key,
            )
        };
        opened.is_ok().then_some(key)
    }

    fn read_dword(path: &str, name: &str) -> Option<u32> {
        let key = open_key(path, KEY_READ)?;
        let name = wide(name);
        let mut kind = REG_DWORD;
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `name` outlives the call; the out-pointers are valid and the
        // buffer is exactly one DWORD, which `size` announces.
        let read = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(&mut value as *mut u32 as *mut u8),
                Some(&mut size),
            )
        };
        // SAFETY: `key` came from `open_key` and is closed exactly once.
        let _ = unsafe { RegCloseKey(key) };
        (read.is_ok() && kind == REG_DWORD && size >= 4).then_some(value)
    }

    fn read_dark() -> Option<bool> {
        read_dword(PERSONALIZE_KEY, APPS_USE_LIGHT_THEME).map(|light| light == 0)
    }

    /// Reports the current accent, then waits for the DWM key to move.
    ///
    /// `RegNotifyChangeKeyValue` in its synchronous form blocks until a value
    /// under the key is written and needs no event handle. Watching the key
    /// rather than the `WM_DWMCOLORIZATIONCOLORCHANGED` broadcast avoids a
    /// second hidden window and reports the *accent* rather than the title-bar
    /// colorization colour.
    fn watch_accent(sink: Sink) {
        sink.accent(crate::platform::accent());

        let Some(key) = open_key(DWM_KEY, KEY_NOTIFY) else {
            // Nothing to watch; the reading above is still correct.
            return;
        };

        loop {
            // SAFETY: `key` is an open key. With `fasynchronous` false and no
            // event this call blocks the thread until the key is written,
            // which is the wait wanted; `None` is a valid absent `HANDLE`.
            let changed = unsafe {
                RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false)
            };
            if changed.is_err() {
                break;
            }
            sink.accent(crate::platform::accent());
        }

        // SAFETY: `key` is closed exactly once, on the way out of the loop.
        let _ = unsafe { RegCloseKey(key) };
    }

    /// Reports the current light/dark preference, then follows the same
    /// Personalize key Qt's style hints read.
    fn watch_theme(sink: Sink) {
        sink.theme(read_dark());

        let Some(key) = open_key(PERSONALIZE_KEY, KEY_NOTIFY) else {
            return;
        };

        loop {
            // SAFETY: identical to `watch_accent`'s blocking registry wait.
            let changed = unsafe {
                RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false)
            };
            if changed.is_err() {
                break;
            }
            sink.theme(read_dark());
        }

        // SAFETY: `key` is closed exactly once.
        let _ = unsafe { RegCloseKey(key) };
    }

    /// Owns the hidden top-level window that receives `WM_DEVICECHANGE`.
    ///
    /// Volume arrivals and removals are broadcast to every top-level window;
    /// a message-only window (`HWND_MESSAGE`) is defined by not receiving
    /// broadcasts, so the window is ordinary, never shown and never destroyed
    /// until the process ends.
    fn watch_volumes(sink: Sink) {
        if SINK.set(sink).is_err() {
            return;
        }

        let class_name = wide(WINDOW_CLASS);

        // SAFETY: `GetModuleHandleW(None)` asks for this process's module; the
        // handle only associates the window class with it.
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(module) => HINSTANCE(module.0),
            Err(error) => {
                log(format!("dae: volume watch has no module handle: {error}"));
                return;
            }
        };

        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        // SAFETY: `class` is fully initialised, `lpszClassName` points into
        // `class_name` which outlives the call, and the procedure is a plain
        // `extern "system"` fn. Zero means the class is already taken.
        if unsafe { RegisterClassW(&class) } == 0 {
            log("dae: volume watch window class is already registered".to_owned());
            return;
        }

        // SAFETY: the class name is the one just registered, the absent parent
        // makes this a top-level window — the kind device changes are
        // broadcast to — and style 0 means it is never shown or focused.
        let _window = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
        } {
            Ok(window) => window,
            Err(error) => {
                log(format!(
                    "dae: could not create the volume watch window: {error}"
                ));
                return;
            }
        };

        let mut message = MSG::default();
        // SAFETY: this thread owns the window above, and `message` is a valid
        // out-parameter. Zero (`WM_QUIT`) or a negative return ends the pump.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            // SAFETY: `GetMessageW` filled `message` completely.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// Hears the broadcasts, debounces the burst and re-lists the volumes.
    ///
    /// # Safety
    ///
    /// Called by the system with a window handle belonging to this thread and
    /// the message parameters that go with `message`; the only pointer read is
    /// the `DEV_BROADCAST_HDR` a device message carries in `lparam`.
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DEVICECHANGE && is_volume_change(wparam, lparam) {
            if !VOLUME_REFRESH_PENDING.swap(true, Ordering::SeqCst) {
                // SAFETY: `window` is this thread's window and the timer
                // procedure is the absent one, so the tick arrives as
                // `WM_TIMER` to the same procedure.
                let armed = unsafe {
                    SetTimer(Some(window), VOLUME_REFRESH_TIMER, VOLUME_DEBOUNCE_MS, None)
                };
                if armed == 0 {
                    // Without a timer the debounce would swallow every later
                    // event, so answer this one immediately.
                    VOLUME_REFRESH_PENDING.store(false, Ordering::SeqCst);
                    report_volumes();
                }
            }
            return LRESULT(0);
        }

        if message == WM_TIMER && wparam.0 == VOLUME_REFRESH_TIMER {
            // SAFETY: the timer belongs to this window and is killed once.
            let _ = unsafe { KillTimer(Some(window), VOLUME_REFRESH_TIMER) };
            if VOLUME_REFRESH_PENDING.swap(false, Ordering::SeqCst) {
                report_volumes();
            }
            return LRESULT(0);
        }

        // SAFETY: `window` is the handle this message arrived for, so the
        // default procedure is the right home for anything unhandled.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn report_volumes() {
        if let Some(sink) = SINK.get() {
            sink.volumes();
        }
    }

    /// Whether a `WM_DEVICECHANGE` is a volume arriving or leaving.
    ///
    /// `DBT_DEVNODES_CHANGED` fires for every device-tree change — a mouse, a
    /// webcam, a Bluetooth pair — and other device types (interfaces, ports)
    /// are not mounted volumes, so only an arrival/removal with a
    /// `DBT_DEVTYP_VOLUME` header moves the sidebar's disk list.
    fn is_volume_change(wparam: WPARAM, lparam: LPARAM) -> bool {
        let arrival = wparam.0 == DBT_DEVICEARRIVAL as usize;
        let removal = wparam.0 == DBT_DEVICEREMOVECOMPLETE as usize;
        if (!arrival && !removal) || lparam.0 == 0 {
            return false;
        }

        // SAFETY: for the two events admitted above, `lparam` points to a
        // `DEV_BROADCAST_*` structure whose first field is the shared header,
        // as documented for `WM_DEVICECHANGE`. It is borrowed for this read.
        let header = unsafe { &*(lparam.0 as *const DEV_BROADCAST_HDR) };
        header.dbch_devicetype == DBT_DEVTYP_VOLUME
    }

    fn log(message: String) {
        let _ = writeln!(std::io::stderr(), "{message}");
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::UI::WindowsAndMessaging::{
            DBT_DEVNODES_CHANGED, DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_HDR_DEVICE_TYPE,
        };

        /// Only the shared header is ever read, so the tests build that part.
        fn broadcast(device_type: DEV_BROADCAST_HDR_DEVICE_TYPE) -> DEV_BROADCAST_HDR {
            DEV_BROADCAST_HDR {
                dbch_size: std::mem::size_of::<DEV_BROADCAST_HDR>() as u32,
                dbch_devicetype: device_type,
                dbch_reserved: 0,
            }
        }

        fn as_lparam(header: &DEV_BROADCAST_HDR) -> LPARAM {
            LPARAM(header as *const DEV_BROADCAST_HDR as isize)
        }

        #[test]
        fn forwards_volume_arrivals_and_removals() {
            let volume = broadcast(DBT_DEVTYP_VOLUME);

            assert!(is_volume_change(
                WPARAM(DBT_DEVICEARRIVAL as usize),
                as_lparam(&volume)
            ));
            assert!(is_volume_change(
                WPARAM(DBT_DEVICEREMOVECOMPLETE as usize),
                as_lparam(&volume)
            ));
        }

        #[test]
        fn ignores_every_other_device_event() {
            let volume = broadcast(DBT_DEVTYP_VOLUME);
            assert!(!is_volume_change(
                WPARAM(DBT_DEVNODES_CHANGED as usize),
                as_lparam(&volume)
            ));

            let interface = broadcast(DBT_DEVTYP_DEVICEINTERFACE);
            assert!(!is_volume_change(
                WPARAM(DBT_DEVICEARRIVAL as usize),
                as_lparam(&interface)
            ));

            assert!(!is_volume_change(
                WPARAM(DBT_DEVICEARRIVAL as usize),
                LPARAM(0)
            ));
        }
    }
}
