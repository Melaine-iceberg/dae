//! Windows 11 Snap Layout for the renderer-drawn maximize button.
//!
//! Windows shows the snap flyout for a window only when its hit test answers
//! `HTMAXBUTTON` in that region. A frameless Electron window hit-tests its
//! whole client area as client space, so the renderer's button never gets the
//! flyout. The proven workaround (also used by `tauri-plugin-snap-layout`) is a
//! transparent `STATIC` child window laid over the button that answers
//! `HTMAXBUTTON` for its whole area and toggles the parent on click.
//!
//! The renderer reports the button's rect in CSS pixels; this module scales it
//! by the window's DPI and keeps the child in sync on every resize. Hover
//! feedback is driven from the main process with a pointer poll, because once
//! the child covers the button the DOM stops seeing `:hover`.

#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForWindow;
#[cfg(windows)]
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetPropW, IsWindow, IsZoomed, LoadCursorW, RemovePropW,
    SendMessageW, SetCursor, SetPropW, SetWindowPos, HTMAXBUTTON, IDC_ARROW, SC_MAXIMIZE,
    SC_RESTORE, SWP_NOACTIVATE, SWP_SHOWWINDOW, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_SETCURSOR, WM_SYSCOMMAND, WINDOW_EX_STYLE, WS_CHILD, WS_VISIBLE,
};

#[cfg(windows)]
const CHILD_PROP: windows::core::PCWSTR = w!("dae.snap.child");
#[cfg(windows)]
const SUBCLASS_ID: usize = 0x6461_6573_6e61_7000;

/// Positions the snap-layout child over `(x, y, width, height)`, given in CSS
/// pixels relative to the window's client area, creating it on first use.
/// Returns false where the platform has no snap layouts (or on a dead HWND).
pub fn install(hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> bool {
    #[cfg(windows)]
    {
        unsafe {
            if hwnd == 0 {
                return false;
            }
            let parent = HWND(hwnd as *mut core::ffi::c_void);
            if !IsWindow(Some(parent)).as_bool() {
                return false;
            }

            let dpi = match GetDpiForWindow(parent) {
                0 => 96,
                value => value,
            };
            let (x, y, width, height) = (
                scaled(x, dpi),
                scaled(y, dpi),
                scaled(width.max(1), dpi),
                scaled(height.max(1), dpi),
            );

            let existing = GetPropW(parent, CHILD_PROP);
            let child = if existing.0.is_null() {
                let created = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!(""),
                    WS_CHILD | WS_VISIBLE,
                    x,
                    y,
                    width,
                    height,
                    Some(parent),
                    None,
                    None,
                    None,
                );
                let Ok(child) = created else {
                    return false;
                };
                if SetPropW(parent, CHILD_PROP, Some(HANDLE(child.0))).is_err() {
                    let _ = DestroyWindow(child);
                    return false;
                }
                let _ = SetWindowSubclass(child, Some(snap_child_proc), SUBCLASS_ID, hwnd as usize);
                child
            } else {
                HWND(existing.0)
            };

            if !IsWindow(Some(child)).as_bool() {
                return false;
            }
            SetWindowPos(
                child,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOP),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .is_ok()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, x, y, width, height);
        false
    }
}

/// Removes the child, if one was installed, before its window goes away.
pub fn destroy(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        unsafe {
            if hwnd == 0 {
                return false;
            }
            let parent = HWND(hwnd as *mut core::ffi::c_void);
            if !IsWindow(Some(parent)).as_bool() {
                return false;
            }
            let existing = GetPropW(parent, CHILD_PROP);
            if existing.0.is_null() {
                return false;
            }
            let _ = RemovePropW(parent, CHILD_PROP);
            let child = HWND(existing.0);
            if IsWindow(Some(child)).as_bool() {
                let _ = DestroyWindow(child);
            }
            true
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

#[cfg(windows)]
fn scaled(value: i32, dpi: u32) -> i32 {
    // CSS pixels are DIPs; child-window coordinates are physical pixels.
    let sign = if value < 0 { -1i64 } else { 1i64 };
    (sign * (i64::from(value.unsigned_abs()) * i64::from(dpi) + 48) / 96) as i32
}

#[cfg(windows)]
unsafe extern "system" fn snap_child_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    parent: usize,
) -> LRESULT {
    let parent = HWND(parent as *mut core::ffi::c_void);

    unsafe {
        match message {
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(snap_child_proc), subclass_id);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_NCHITTEST => LRESULT(HTMAXBUTTON as isize),
            WM_NCLBUTTONDOWN if wparam.0 == HTMAXBUTTON as usize => LRESULT(0),
            WM_NCLBUTTONUP if wparam.0 == HTMAXBUTTON as usize => {
                let command = if IsZoomed(parent).as_bool() {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                SendMessageW(
                    parent,
                    WM_SYSCOMMAND,
                    Some(WPARAM(command as usize)),
                    Some(LPARAM(0)),
                );
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
                    SetCursor(Some(cursor));
                }
                LRESULT(1)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }
}
