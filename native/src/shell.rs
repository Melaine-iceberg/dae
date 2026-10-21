//! Small shell-level answers the renderer needs before it can paint.
//!
//! These are the commands still owned by the Electron shell rather than
//! `dae-core`: system places are an OS lookup, and the workspace lists are
//! empty until the shell's stores are ported. They are served here so the real
//! renderer has a coherent first paint instead of four failed queries.

use serde::Serialize;

#[derive(Serialize)]
pub struct SystemPlace {
    kind: &'static str,
    path: String,
}

pub fn system_places() -> Vec<SystemPlace> {
    let candidates = [
        ("home", dirs::home_dir()),
        ("desktop", dirs::desktop_dir()),
        ("documents", dirs::document_dir()),
        ("downloads", dirs::download_dir()),
        ("pictures", dirs::picture_dir()),
        ("music", dirs::audio_dir()),
        ("videos", dirs::video_dir()),
    ];

    candidates
        .into_iter()
        .filter_map(|(kind, resolved)| {
            resolved.map(|path| SystemPlace {
                kind,
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

/// Applies the renderer's light/dark choice to the Electron window's system
/// backdrop. Electron exposes no runtime appearance API, so this reaches the
/// window's HWND directly. Only Windows has a documented per-window switch.
pub fn set_window_appearance(hwnd: isize, dark: bool) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{
            DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
        };

        let value = windows::core::BOOL(dark as i32);
        // SAFETY: the HWND comes from Electron's `getNativeWindowHandle` and is
        // valid for this process; the attribute pointer is a live BOOL.
        unsafe {
            DwmSetWindowAttribute(
                HWND(hwnd as *mut core::ffi::c_void),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &value as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<windows::core::BOOL>() as u32,
            )
            .is_ok()
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (hwnd, dark);
        false
    }
}
