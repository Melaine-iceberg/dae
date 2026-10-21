//! Volume arrival/removal notifications for the sidebar's disk list.

use std::sync::atomic::{AtomicBool, Ordering};

const EVENT_NAME: &str = "volumes-changed";

pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    #[cfg(windows)]
    windows_backend::spawn();
}

#[cfg(windows)]
fn emit_volumes_changed() {
    crate::emit_event(EVENT_NAME, "null".to_owned());
}

#[cfg(windows)]
mod windows_backend {
    //! Windows' channel: the `WM_DEVICECHANGE` broadcast.
    //!
    //! Volume arrivals and removals are broadcast to every top-level window,
    //! so the monitor owns a hidden ordinary window (not a message-only one)
    //! and pumps its messages on a dedicated thread.

    use std::sync::OnceLock;

    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME,
        DEV_BROADCAST_HDR, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
        TranslateMessage, WM_DEVICECHANGE, WNDCLASSW, WINDOW_EX_STYLE, WINDOW_STYLE,
    };
    use windows::core::PCWSTR;

    const WINDOW_CLASS: &str = "DaeVolumeMonitor";

    static READY: OnceLock<()> = OnceLock::new();

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn spawn() {
        let _ = READY.set(());

        let spawned = std::thread::Builder::new()
            .name("volume-monitor".to_owned())
            .spawn(pump_messages);

        if let Err(error) = spawned {
            eprintln!("[dae-native] failed to start the volume-monitor thread: {error}");
        }
    }

    fn pump_messages() {
        let class_name = wide(WINDOW_CLASS);

        // SAFETY: `GetModuleHandleW(None)` asks for the module this process was
        // started from; no pointers involved.
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(module) => HINSTANCE(module.0),
            Err(error) => {
                eprintln!("[dae-native] failed to resolve the module handle: {error}");
                return;
            }
        };

        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        // SAFETY: `class` is fully initialised and the class name outlives the
        // call. A zero return means the class is already registered.
        if unsafe { RegisterClassW(&class) } == 0 {
            eprintln!("[dae-native] the volume-monitor window class is already registered");
            return;
        }

        // SAFETY: the class name is the one just registered, the absent parent
        // makes this a top-level window, and the zero style keeps it hidden.
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
                eprintln!("[dae-native] failed to create the volume-monitor window: {error}");
                return;
            }
        };

        let mut message = MSG::default();
        // SAFETY: the thread owns a window from the call above and `message`
        // is a valid out-parameter.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            // SAFETY: `GetMessageW` filled `message` completely.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// # Safety
    ///
    /// Called by the system with a window handle belonging to this thread and
    /// the message parameters that go with `message`; the only pointer read is
    /// the `DEV_BROADCAST_HDR` that device messages carry in `lparam`.
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DEVICECHANGE && is_volume_change(wparam, lparam) {
            super::emit_volumes_changed();
            return LRESULT(1);
        }

        // SAFETY: `window` is the handle this message arrived for.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn is_volume_change(wparam: WPARAM, lparam: LPARAM) -> bool {
        let arrival = wparam.0 == DBT_DEVICEARRIVAL as usize;
        let removal = wparam.0 == DBT_DEVICEREMOVECOMPLETE as usize;
        if (!arrival && !removal) || lparam.0 == 0 {
            return false;
        }

        // SAFETY: for the two events admitted above, `lparam` points to a
        // `DEV_BROADCAST_*` structure whose first field is the shared header.
        let header = unsafe { &*(lparam.0 as *const DEV_BROADCAST_HDR) };
        header.dbch_devicetype == DBT_DEVTYP_VOLUME
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::UI::WindowsAndMessaging::{
            DBT_DEVNODES_CHANGED, DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_HDR_DEVICE_TYPE,
        };

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
