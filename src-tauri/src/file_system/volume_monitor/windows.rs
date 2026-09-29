//! Windows' channel: the `WM_DEVICECHANGE` broadcast.
//!
//! Two Win32 facts shape this backend, and both cut against the shape the
//! other platforms suggest:
//!
//!   * Volume arrivals and removals are *broadcast* by the system to every
//!     top-level window — that is how Explorer's drive bar hears a USB stick —
//!     and `RegisterDeviceNotification` explicitly fails for a
//!     `DBT_DEVTYP_VOLUME` filter, because a registration would be redundant.
//!     So there is nothing to register; the only requirement is to own a
//!     top-level window.
//!   * That window cannot be a message-only one (`HWND_MESSAGE`): message-only
//!     windows are defined by not receiving broadcasts, which is the whole
//!     message here. A hidden ordinary window it is — created, never shown,
//!     and never destroyed until the process ends.
//!
//! Window creation and the message pump are thread-affine, so both live on a
//! dedicated thread that blocks in `GetMessageW` for the life of the app —
//! the same one-thread-per-watcher shape `system_accent` uses.

use super::VolumesChanged;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HWND, HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME,
    DEV_BROADCAST_HDR, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
    TranslateMessage, WM_DEVICECHANGE, WNDCLASSW, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::PCWSTR;

/// The window class this backend registers. The name only has to be unique
/// within the process.
const WINDOW_CLASS: &str = "DaeVolumeMonitor";

/// The handle the window procedure emits through. The monitor is process-wide
/// and started once, so a static is the whole hand-off.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Encodes a Rust string as a null-terminated UTF-16 vector for `PCWSTR`,
/// matching `system_accent::windows`'s helper.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(super) fn spawn(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());

    // The pump below blocks its thread for the life of the process, which is
    // why it gets a thread of its own rather than a turn on the async runtime.
    let spawned = std::thread::Builder::new()
        .name("volume-monitor".to_owned())
        .spawn(pump_messages);

    if let Err(error) = spawned {
        log::warn!("failed to start the volume-monitor thread: {error}");
    }
}

/// Creates the hidden window and pumps its messages. A failure at any step
/// only costs the live updates — the sidebar still re-lists on its poll — so
/// each step warns and returns rather than panicking.
fn pump_messages() {
    let class_name = wide(WINDOW_CLASS);

    // SAFETY: `GetModuleHandleW(None)` asks for the module this process was
    // started from — no pointers involved. The handle only associates the
    // window class with that module.
    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(module) => HINSTANCE(module.0),
        Err(error) => {
            log::warn!("failed to resolve the module handle: {error}");
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
    // `extern "system"` fn. A zero return means the class is taken, which can
    // only be a repeated `spawn`.
    if unsafe { RegisterClassW(&class) } == 0 {
        log::warn!("the volume-monitor window class is already registered");
        return;
    }

    // SAFETY: the class name is the one just registered, the absent parent
    // makes this a top-level window — the kind `WM_DEVICECHANGE` is broadcast
    // to — and the zero style means it is never shown and never takes focus.
    // The handle is not kept: everything this window has to say arrives
    // through its procedure, for the life of the process.
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
            log::warn!("failed to create the volume-monitor window: {error}");
            return;
        }
    };

    let mut message = MSG::default();
    // SAFETY: the thread owns a window from the call above, and `message` is a
    // valid out-parameter. Zero (`WM_QUIT`) or a negative return — neither of
    // which can happen while that window exists — ends the loop, and the
    // thread with it.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        // SAFETY: `GetMessageW` filled `message` completely.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Hears the broadcasts. Volume arrivals and removals are forwarded as
/// [`VolumesChanged`]; everything else is passed on untouched, so the window
/// stays inert for the device events that cannot move the disk list.
///
/// # Safety
///
/// Called by the system with a window handle belonging to this thread and the
/// message parameters that go with `message`; the only pointer read is the
/// `DEV_BROADCAST_HDR` that device messages carry in `lparam`.
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DEVICECHANGE && is_volume_change(wparam, lparam) {
        if let Some(app) = APP.get() {
            let _ = tauri_specta::Event::emit(&VolumesChanged, app);
        }
        return LRESULT(1);
    }

    // SAFETY: `window` is the handle this message arrived for, so the default
    // procedure is the right home for whatever this window does not handle.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Whether a `WM_DEVICECHANGE` is a volume arriving or leaving — the events
/// that move the sidebar's disk list.
///
/// `DBT_DEVNODES_CHANGED` (any device-tree change, a mouse included) and the
/// arrivals/removals of other device types (interfaces, ports) are not volumes
/// and must not trigger a re-list; arrival or removal with a
/// `DBT_DEVTYP_VOLUME` header is exactly "a drive letter appeared or
/// disappeared".
fn is_volume_change(wparam: WPARAM, lparam: LPARAM) -> bool {
    let arrival = wparam.0 == DBT_DEVICEARRIVAL as usize;
    let removal = wparam.0 == DBT_DEVICEREMOVECOMPLETE as usize;
    if (!arrival && !removal) || lparam.0 == 0 {
        return false;
    }

    // SAFETY: for the two events admitted above, `lparam` points to a
    // `DEV_BROADCAST_*` structure whose first field is the shared header, as
    // documented for `WM_DEVICECHANGE`. It is borrowed for this read only.
    let header = unsafe { &*(lparam.0 as *const DEV_BROADCAST_HDR) };
    header.dbch_devicetype == DBT_DEVTYP_VOLUME
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        DBT_DEVNODES_CHANGED, DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_HDR_DEVICE_TYPE,
    };

    /// A stand-in for the structure the system puts in `lparam`; only the
    /// header is ever read.
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
        // `DBT_DEVNODES_CHANGED` fires for every device on the machine — a
        // mouse, a webcam, a Bluetooth pair — and carries a volume header here
        // only to pin that the filter is the event, not the payload.
        let volume = broadcast(DBT_DEVTYP_VOLUME);
        assert!(!is_volume_change(
            WPARAM(DBT_DEVNODES_CHANGED as usize),
            as_lparam(&volume)
        ));

        // An interface arrival (a printer, a phone in MTP mode) is not a
        // mounted volume.
        let interface = broadcast(DBT_DEVTYP_DEVICEINTERFACE);
        assert!(!is_volume_change(
            WPARAM(DBT_DEVICEARRIVAL as usize),
            as_lparam(&interface)
        ));

        // And an arrival without a payload header must not be dereferenced.
        assert!(!is_volume_change(
            WPARAM(DBT_DEVICEARRIVAL as usize),
            LPARAM(0)
        ));
    }
}
