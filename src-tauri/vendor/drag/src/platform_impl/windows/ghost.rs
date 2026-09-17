// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! A preview window that follows the cursor for `DragItem::Data` drags.
//!
//! The Shell's drag image — `IDragSourceHelper::InitializeFromBitmap` — is
//! always composited at 75% alpha: an image handed over with alpha 255 comes
//! back as alpha 191 in the `DragImageBits` blob the drag manager stores, for
//! both `CreateBitmap` and `CreateDIBSection` sources and regardless of
//! `crColorKey`. A dragged tab card therefore looks see-through, with the
//! desktop showing through its fill. There is no flag to opt out, so drags that
//! need an opaque preview own the window instead.
//!
//! The window lives on its own thread because it has to keep moving while the
//! caller is parked inside the modal `DoDragDrop` loop, and that loop holds the
//! mouse capture — no pointer messages reach the preview, so its position can
//! only be polled.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, PeekMessageW,
    RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, HTTRANSPARENT,
    HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SW_SHOWNA,
    ULW_ALPHA, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::image::{self, PremultipliedBitmap};
use crate::CursorPosition;

/// How often the preview catches up with the cursor. Comfortably above the
/// display refresh rate while staying cheap enough for a whole drag gesture.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(8);

/// Waits for the first frame so the caller can retire its own in-window ghost
/// without a visible gap in between.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_millis(250);

/// A cursor-following preview window, alive until dropped.
pub(crate) struct DragGhost {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DragGhost {
    /// Shows `item` with the grab point `offset` pixels into the image pinned
    /// under the cursor.
    ///
    /// Returns `None` when the image cannot be decoded, leaving the caller free
    /// to run the drag without a preview.
    pub(crate) fn start(item: &crate::Image, offset: CursorPosition) -> Option<Self> {
        let bitmap = image::read_image(item).ok()?;

        let stop = Arc::new(AtomicBool::new(false));
        let (ready, first_frame) = mpsc::channel();
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("drag-ghost".into())
            .spawn(move || run(bitmap, offset, thread_stop, ready))
            .ok()?;

        // The caller retires its in-window ghost right after this returns.
        let _ = first_frame.recv_timeout(FIRST_FRAME_TIMEOUT);

        Some(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for DragGhost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    bitmap: PremultipliedBitmap,
    offset: CursorPosition,
    stop: Arc<AtomicBool>,
    ready: Sender<()>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let ghost = unsafe { LayeredGhost::create(&bitmap, offset) };
    let Some(ghost) = ghost else {
        let _ = ready.send(());
        return;
    };

    let _ = ready.send(());

    // `SetWindowPos` posts window messages, so only move when the cursor really
    // moved instead of every tick.
    let mut placed: Option<POINT> = None;
    while !stop.load(Ordering::SeqCst) {
        unsafe { pump_messages() };

        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_ok()
            && placed.is_none_or(|previous| previous != cursor)
        {
            ghost.follow(cursor, offset);
            placed = Some(cursor);
        }

        thread::sleep(FOLLOW_INTERVAL);
    }
}

/// The layered window and the device contexts its bitmap is bound to. Dropping
/// it takes the window off screen.
struct LayeredGhost {
    window: HWND,
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl LayeredGhost {
    /// Paints `bitmap` and shows it with the grab point under the cursor.
    unsafe fn create(bitmap: &PremultipliedBitmap, offset: CursorPosition) -> Option<Self> {
        let class = w!("dae-drag-ghost");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ghost_window_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: class,
            ..Default::default()
        });

        let window = CreateWindowExW(
            // Layered for per-pixel alpha; tool window so it stays out of the
            // taskbar and Alt+Tab; no activate so focus never moves; topmost so
            // it floats over whatever the cursor crosses; transparent so the
            // OLE drag loop keeps hit-testing the window underneath.
            WS_EX_LAYERED
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_TOPMOST
                | WS_EX_TRANSPARENT,
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            bitmap.width,
            bitmap.height,
            None,
            None,
            None,
            None,
        );
        if window.0 == 0 {
            return None;
        }

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let screen = GetDC(None);
        let memory = CreateCompatibleDC(screen);
        let bitmap_handle = match create_dib_section(screen, bitmap) {
            Some(bitmap_handle) => bitmap_handle,
            None => {
                let _ = DestroyWindow(window);
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return None;
            }
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap_handle.0));

        let size = SIZE {
            cx: bitmap.width,
            cy: bitmap.height,
        };
        let source = POINT { x: 0, y: 0 };
        let destination = POINT {
            x: cursor.x - offset.x,
            y: cursor.y - offset.y,
        };
        // The preview already carries premultiplied alpha and full opacity;
        // `UpdateLayeredWindow` copies it verbatim, so the Shell's 75% blend
        // never enters the picture.
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let painted = UpdateLayeredWindow(
            window,
            screen,
            Some(&destination),
            Some(&size),
            memory,
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        if painted.is_err() {
            SelectObject(memory, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap_handle.0));
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            let _ = DestroyWindow(window);
            return None;
        }

        // Only worth showing once there is something to show; a layered window
        // with no content would be invisible anyway.
        let _ = ShowWindow(window, SW_SHOWNA);

        Some(Self {
            window,
            screen,
            memory,
            bitmap: bitmap_handle,
            previous,
        })
    }

    /// Keeps the grab point under the cursor by placing the window's top-left
    /// corner `offset` pixels up and to the left of it.
    fn follow(&self, cursor: POINT, offset: CursorPosition) {
        unsafe {
            let _ = SetWindowPos(
                self.window,
                HWND_TOPMOST,
                cursor.x - offset.x,
                cursor.y - offset.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }
}

impl Drop for LayeredGhost {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.window);
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

/// A top-down 32bpp DIB section holding the preview pixels; `UpdateLayeredWindow`
/// reads alpha from it and needs the rows in the same order the decoder produced.
unsafe fn create_dib_section(screen: HDC, bitmap: &PremultipliedBitmap) -> Option<HBITMAP> {
    let mut header = BITMAPINFO::default();
    header.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    header.bmiHeader.biWidth = bitmap.width;
    header.bmiHeader.biHeight = -bitmap.height;
    header.bmiHeader.biPlanes = 1;
    header.bmiHeader.biBitCount = 32;
    header.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut c_void = std::ptr::null_mut();
    let section = CreateDIBSection(screen, &header, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    std::ptr::copy_nonoverlapping(bitmap.pixels.as_ptr(), bits as *mut u8, bitmap.pixels.len());
    Some(section)
}

/// Hit-testing this window would hide the real drop target from the drag loop,
/// so every point of it belongs to the window underneath.
unsafe extern "system" fn ghost_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    DefWindowProcW(window, message, wparam, lparam)
}

/// Drains this thread's queue so the preview window stays responsive; it owns no
/// other window, so every queued message belongs to the ghost.
unsafe fn pump_messages() {
    let mut message = MSG::default();
    while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}
