//! Tab tear-off support.
//!
//! WebView2 stops delivering pointer events after the cursor leaves a window,
//! so the frontend cannot reliably decide whether an in-progress tab drag is
//! outside. These commands query the native window and create the detached
//! webview window while keeping the opaque tab snapshot in Rust until the new
//! frontend consumes it.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use specta::Type;
use tauri::{Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

const WINDOW_LABEL_PREFIX: &str = "tab-window-";
const DEFAULT_WIDTH: f64 = 960.0;
const DEFAULT_HEIGHT: f64 = 680.0;
const MIN_WIDTH: f64 = 640.0;
const MIN_HEIGHT: f64 = 480.0;
const MAX_WIDTH: f64 = 1280.0;
const MAX_HEIGHT: f64 = 900.0;
#[cfg(windows)]
const TAB_DRAG_FALLBACK_ICON: &[u8] = include_bytes!("../icons/32x32.png");
#[cfg(windows)]
static TAB_DRAG_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TabDragOutcome {
    pub released: bool,
    pub outside: bool,
    pub cursor_x: i32,
    pub cursor_y: i32,
}

#[derive(Default)]
pub struct TabWindowState {
    handoffs: Mutex<HashMap<String, String>>,
    next_window_id: AtomicU64,
}

impl TabWindowState {
    fn insert(&self, label: String, payload: String) -> Result<(), String> {
        self.handoffs
            .lock()
            .map_err(|_| "Tab handoff state is unavailable".to_string())?
            .insert(label, payload);
        Ok(())
    }

    fn take(&self, label: &str) -> Result<Option<String>, String> {
        Ok(self
            .handoffs
            .lock()
            .map_err(|_| "Tab handoff state is unavailable".to_string())?
            .remove(label))
    }

    fn next_label(&self, app: &tauri::AppHandle) -> String {
        loop {
            let id = self.next_window_id.fetch_add(1, Ordering::Relaxed);
            let label = format!("{WINDOW_LABEL_PREFIX}{id}");
            if app.get_webview_window(&label).is_none() {
                return label;
            }
        }
    }
}

/// Whether the pointer has left the window a dragged tab came from.
///
/// The frontend polls while a tab drag is in flight because WebView2 may stop
/// delivering pointer events as soon as the cursor crosses the window edge.
#[tauri::command]
#[specta::specta]
pub fn tab_drag_outside(app: tauri::AppHandle, source: String) -> Result<bool, String> {
    let window = app
        .get_webview_window(&source)
        .ok_or_else(|| format!("Source window '{source}' was not found"))?;
    let cursor = window
        .cursor_position()
        .map_err(|error| error.to_string())?;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;

    Ok(point_is_outside(
        cursor.x,
        cursor.y,
        position.x,
        position.y,
        size.width,
        size.height,
    ))
}

/// Hands an out-of-window tab gesture to the Windows OLE drag loop. The shell
/// owns the drag image and mouse capture until the user releases the primary
/// button or presses Escape, so the WebView does not need global mouse hooks.
#[tauri::command]
#[specta::specta]
pub async fn start_tab_drag(
    app: tauri::AppHandle,
    source: String,
    preview: Option<String>,
    offset_x: f64,
    offset_y: f64,
) -> Result<TabDragOutcome, String> {
    #[cfg(not(windows))]
    {
        let _ = (app, source, preview, offset_x, offset_y);
        return Err("Native tab drag is currently only available on Windows".into());
    }

    #[cfg(windows)]
    {
        let window = app
            .get_webview_window(&source)
            .ok_or_else(|| format!("Source window '{source}' was not found"))?;
        let preview = decode_drag_preview(preview)?;
        let image_offset = drag::CursorPosition {
            x: finite_i32(offset_x),
            y: finite_i32(offset_y),
        };
        let drag_window = window.clone();
        let (sender, receiver) =
            mpsc::sync_channel::<Result<(bool, drag::CursorPosition), String>>(1);
        let dispatch_sender = sender.clone();

        app.run_on_main_thread(move || {
            if TAB_DRAG_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                let _ = dispatch_sender.send(Err("Another tab drag is already active".into()));
                return;
            }

            let callback_sender = dispatch_sender.clone();
            let result = drag::start_drag(
                &drag_window,
                drag::DragItem::Data {
                    provider: Box::new(|_| None),
                    types: Vec::new(),
                },
                drag::Image::Raw(preview),
                move |result, cursor| {
                    let released = matches!(result, drag::DragResult::Dropped);
                    let _ = callback_sender.send(Ok((released, cursor)));
                },
                drag::Options {
                    skip_animatation_on_cancel_or_failure: true,
                    mode: drag::DragMode::Move,
                    drag_image_offset: Some(image_offset),
                },
            );

            if let Err(error) = result {
                let _ = dispatch_sender
                    .try_send(Err(format!("Unable to start the native tab drag: {error}")));
            }
            TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
        })
        .map_err(|error| error.to_string())?;
        drop(sender);

        let (released, cursor) = tauri::async_runtime::spawn_blocking(move || receiver.recv())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|_| "The native tab drag ended without a result".to_string())??;
        let position = window.outer_position().map_err(|error| error.to_string())?;
        let size = window.outer_size().map_err(|error| error.to_string())?;
        let cursor_x = cursor.x;
        let cursor_y = cursor.y;

        Ok(TabDragOutcome {
            released,
            outside: point_is_outside(
                f64::from(cursor_x),
                f64::from(cursor_y),
                position.x,
                position.y,
                size.width,
                size.height,
            ),
            cursor_x,
            cursor_y,
        })
    }
}

#[cfg(windows)]
fn decode_drag_preview(preview: Option<String>) -> Result<Vec<u8>, String> {
    let Some(preview) = preview else {
        return Ok(TAB_DRAG_FALLBACK_ICON.to_vec());
    };
    let encoded = preview
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "The tab drag preview is not a PNG data URL".to_string())?;
    STANDARD
        .decode(encoded)
        .map_err(|error| format!("Unable to decode the tab drag preview: {error}"))
}

#[cfg(windows)]
fn finite_i32(value: f64) -> i32 {
    if value.is_finite() {
        value
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    } else {
        0
    }
}

/// Opens the serialized tab snapshot in a new, independent application window
/// and returns that window's label.
///
/// `source` lends the detached window its size and scale. `grab_x`/`grab_y` are
/// the original pointer coordinates inside the webview in CSS pixels, keeping
/// the same point of the window pinned beneath the release position. The
/// optional cursor coordinates preserve the exact native drop point.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn tear_off_tab(
    app: tauri::AppHandle,
    state: tauri::State<'_, TabWindowState>,
    source: String,
    payload: String,
    grab_x: Option<f64>,
    grab_y: Option<f64>,
    cursor_x: Option<f64>,
    cursor_y: Option<f64>,
) -> Result<String, String> {
    let source_window = app
        .get_webview_window(&source)
        .ok_or_else(|| format!("Source window '{source}' was not found"))?;
    let cursor = match (cursor_x, cursor_y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => PhysicalPosition::new(x, y),
        _ => source_window
            .cursor_position()
            .map_err(|error| error.to_string())?,
    };
    let scale = source_window
        .scale_factor()
        .map_err(|error| error.to_string())?;
    let source_size = source_window
        .inner_size()
        .map_err(|error| error.to_string())?;

    let width = (f64::from(source_size.width) / scale).clamp(MIN_WIDTH, MAX_WIDTH);
    let height = (f64::from(source_size.height) / scale).clamp(MIN_HEIGHT, MAX_HEIGHT);
    let grab_x = grab_x.unwrap_or(width / 2.0).clamp(0.0, width);
    let grab_y = grab_y.unwrap_or(20.0).clamp(0.0, height);
    let physical_x = (cursor.x - grab_x * scale).round() as i32;
    let physical_y = (cursor.y - grab_y * scale).round() as i32;
    let logical_x = f64::from(physical_x) / scale;
    let logical_y = f64::from(physical_y) / scale;
    let label = state.next_label(&app);

    state.insert(label.clone(), payload)?;

    let build_result =
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
            .title("dae")
            .inner_size(
                if width.is_finite() {
                    width
                } else {
                    DEFAULT_WIDTH
                },
                if height.is_finite() {
                    height
                } else {
                    DEFAULT_HEIGHT
                },
            )
            .position(logical_x, logical_y)
            .decorations(false)
            .visible(false)
            .focused(true)
            .build();

    let window = match build_result {
        Ok(window) => window,
        Err(error) => {
            let _ = state.take(&label);
            return Err(error.to_string());
        }
    };

    let cleanup_app = app.clone();
    let cleanup_label = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed)
            && let Some(state) = cleanup_app.try_state::<TabWindowState>()
        {
            let _ = state.take(&cleanup_label);
        }
    });

    // Builder positions are logical and use the source monitor's scale before
    // the new window has a monitor of its own. Correct the final placement in
    // physical desktop coordinates so mixed-DPI monitor layouts stay aligned.
    let _ = window.set_position(PhysicalPosition::new(physical_x, physical_y));

    if let Err(error) = window.show() {
        let _ = window.close();
        let _ = state.take(&label);
        return Err(error.to_string());
    }
    let _ = window.set_focus();

    Ok(label)
}

/// Pulls the snapshot parked for a newly created window and clears it. The new
/// frontend calls this once before its first render.
#[tauri::command]
#[specta::specta]
pub fn take_tab_handoff(
    state: tauri::State<'_, TabWindowState>,
    label: String,
) -> Result<Option<String>, String> {
    state.take(&label)
}

fn point_is_outside(
    cursor_x: f64,
    cursor_y: f64,
    window_x: i32,
    window_y: i32,
    window_width: u32,
    window_height: u32,
) -> bool {
    let left = f64::from(window_x);
    let top = f64::from(window_y);
    let right = left + f64::from(window_width);
    let bottom = top + f64::from(window_height);

    cursor_x < left || cursor_x >= right || cursor_y < top || cursor_y >= bottom
}

#[cfg(test)]
mod tests {
    use super::{TabWindowState, point_is_outside};

    #[test]
    fn detects_every_side_of_window_bounds() {
        assert!(!point_is_outside(100.0, 50.0, 100, 50, 800, 600));
        assert!(!point_is_outside(899.0, 649.0, 100, 50, 800, 600));
        assert!(point_is_outside(99.0, 300.0, 100, 50, 800, 600));
        assert!(point_is_outside(900.0, 300.0, 100, 50, 800, 600));
        assert!(point_is_outside(400.0, 49.0, 100, 50, 800, 600));
        assert!(point_is_outside(400.0, 650.0, 100, 50, 800, 600));
    }

    #[test]
    fn handoff_is_consumed_once() {
        let state = TabWindowState::default();
        state
            .insert("tab-window-test".into(), "snapshot".into())
            .unwrap();

        assert_eq!(
            state.take("tab-window-test").unwrap().as_deref(),
            Some("snapshot")
        );
        assert_eq!(state.take("tab-window-test").unwrap(), None);
    }
}
