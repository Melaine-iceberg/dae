//! Tab tear-off and cross-window merge support.
//!
//! The WebView stops delivering pointer events once the cursor leaves the
//! window, so the frontend cannot reliably decide whether an in-progress tab
//! drag is outside. These commands query the native window and create the
//! detached webview window while keeping the opaque tab snapshot in Rust until
//! the new frontend consumes it. Once the gesture crosses the window edge it
//! is handed to the platform's native drag loop — OLE `DoDragDrop` on Windows,
//! an `NSDraggingSession` on macOS, and a GTK drag on Linux — which owns the
//! drag image and mouse capture until the user releases the primary button or
//! presses Escape.
//!
//! When the drag is released over another of this app's windows, the tab is
//! merged into that window instead of spawning a new one: the source frontend
//! forwards the serialized handoff through the `TabMergedIntoWindow` event,
//! and the receiving window inserts the tab at the drop position.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use specta::Type;
use tauri::{EventTarget, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri_specta::Event;

const WINDOW_LABEL_PREFIX: &str = "tab-window-";
const DEFAULT_WIDTH: f64 = 960.0;
const DEFAULT_HEIGHT: f64 = 680.0;
const MIN_WIDTH: f64 = 640.0;
const MIN_HEIGHT: f64 = 480.0;
const MAX_WIDTH: f64 = 1280.0;
const MAX_HEIGHT: f64 = 900.0;
static TAB_DRAG_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const TAB_DRAG_FALLBACK_ICON: &[u8] = include_bytes!("../icons/32x32.png");
/// MIME type declared by the dummy data drag that carries the tab ghost. No
/// application accepts it, so the drag stays visual-only; GTK in particular
/// refuses to start a drag that advertises no target at all.
const TAB_DRAG_TYPE: &str = "application/x-dae-tab-drag";
/// How often the hover monitor re-reads the cursor while a native tab drag is
/// running; fast enough to feel live, slow enough to stay invisible on CPU.
const TAB_HOVER_POLL_INTERVAL: Duration = Duration::from_millis(33);
/// Set while a `TabDragHover` push has been handed to the receiving window but
/// not yet rendered by it. The monitor holds the next push back until the
/// acknowledgement arrives; see [`spawn_drag_hover_monitor`].
static HOVER_PUSH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// How long the monitor waits for that acknowledgement before pushing anyway.
/// A hidden or wedged receiver must not stall the indicator forever.
const HOVER_PUSH_ACK_TIMEOUT: Duration = Duration::from_millis(500);

/// Hover heartbeat for a window while a tab dragged from another window
/// crosses its bounds. Coordinates are in the receiving window's CSS pixels.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "tab-drag-hover")]
pub struct TabDragHover {
    pub x: f64,
    pub y: f64,
}

/// Tells a window that a dragged tab stopped hovering its bounds, either
/// because the cursor left or because the whole gesture ended.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "tab-drag-leave")]
pub struct TabDragLeave;

/// Sent window-to-window right after a native drop landed inside the
/// receiving window. Carries the serialized tab handoff plus the drop point
/// in the receiver's CSS pixels so it can pick an insertion index.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "tab-merged-into-window")]
pub struct TabMergedIntoWindow {
    pub payload: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TabDragOutcome {
    pub released: bool,
    pub outside: bool,
    pub cursor_x: i32,
    pub cursor_y: i32,
    /// Label of the app window under the release point, when the tab was
    /// dropped over another window of this app. The frontend then merges the
    /// tab into that window instead of tearing off a new one.
    pub target: Option<String>,
    /// Release point in the target window's CSS pixel space.
    pub target_x: f64,
    pub target_y: f64,
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
/// The frontend polls while a tab drag is in flight because the WebView may
/// stop delivering pointer events as soon as the cursor crosses the window
/// edge.
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

/// Hands an out-of-window tab gesture to the platform's native drag loop. The
/// shell owns the drag image and mouse capture until the user releases the
/// primary button or presses Escape, so the WebView does not need global mouse
/// hooks.
///
/// Windows blocks in `DoDragDrop` and returns through the main-thread closure;
/// macOS and GTK run asynchronous drag sessions, so the completion is reported
/// from the drag callback, possibly after this command has already finished.
///
/// While the native loop runs, a hover monitor broadcasts `TabDragHover` /
/// `TabDragLeave` to the window under the cursor so drop targets can show an
/// insertion indicator. The outcome also reports which app window received
/// the drop, if any, so the frontend can merge the tab instead of detaching.
#[tauri::command]
#[specta::specta]
pub async fn start_tab_drag(
    app: tauri::AppHandle,
    source: String,
    preview: Option<String>,
    offset_x: f64,
    offset_y: f64,
) -> Result<TabDragOutcome, String> {
    if !matches!(std::env::consts::OS, "windows" | "macos" | "linux") {
        let _ = (&app, &source, &preview, offset_x, offset_y);
        return Err("Native tab drag is not supported on this platform".into());
    }

    let hover_finished = Arc::new(AtomicBool::new(false));
    spawn_drag_hover_monitor(app.clone(), source.clone(), hover_finished.clone());

    let outcome = run_native_tab_drag(&app, source, preview, offset_x, offset_y).await;

    // Whatever happened — drop, cancel, or failure — the monitor must stop
    // and deliver its final leave event.
    hover_finished.store(true, Ordering::SeqCst);
    outcome
}

async fn run_native_tab_drag(
    app: &tauri::AppHandle,
    source: String,
    preview: Option<String>,
    offset_x: f64,
    offset_y: f64,
) -> Result<TabDragOutcome, String> {
    let window = app
        .get_webview_window(&source)
        .ok_or_else(|| format!("Source window '{source}' was not found"))?;
    let preview = decode_drag_preview(preview)?;
    let image_offset = drag::CursorPosition {
        x: finite_i32(offset_x),
        y: finite_i32(offset_y),
    };

    // The completion channel is consumed exactly once by whichever path
    // finishes first: the synchronous DoDragDrop loop on Windows, or the
    // asynchronous drag-session callback on macOS/GTK.
    let (sender, receiver) = mpsc::sync_channel::<Result<NativeDragSummary, String>>(1);

    #[cfg(windows)]
    {
        let drag_window = window.clone();
        let dispatch_sender = sender.clone();

        app.run_on_main_thread(move || {
            if TAB_DRAG_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                let _ = dispatch_sender.try_send(Err("Another tab drag is already active".into()));
                return;
            }

            let callback_sender = dispatch_sender.clone();
            let result = drag::start_drag(
                &drag_window,
                drag::DragItem::Data {
                    provider: Box::new(|_| None),
                    types: vec![TAB_DRAG_TYPE.into()],
                },
                drag::Image::Raw(preview),
                move |result, cursor| {
                    let released = matches!(result, drag::DragResult::Dropped);
                    let _ = callback_sender.try_send(Ok(NativeDragSummary { released, cursor }));
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
    }

    #[cfg(target_os = "macos")]
    {
        let drag_window = window.clone();
        let dispatch_sender = sender.clone();

        // The frontend's poll decided the gesture is outside, so the source
        // tab sits wherever the last in-window pointer event left it; the
        // ghost continues from the true cursor position instead.
        app.run_on_main_thread(move || {
            if TAB_DRAG_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                let _ = dispatch_sender.try_send(Err("Another tab drag is already active".into()));
                return;
            }

            let callback_sender = dispatch_sender.clone();
            let callback_window = drag_window.clone();
            let result = drag::start_drag(
                &drag_window,
                drag::DragItem::Data {
                    provider: Box::new(|_| None),
                    types: vec![TAB_DRAG_TYPE.into()],
                },
                drag::Image::Raw(preview),
                move |result, _| {
                    // tao's cursor_position shares the same (quirky but
                    // self-consistent) coordinate space as outer_position,
                    // which the outside test and the tear-off placement below
                    // both rely on, so query through the window rather than
                    // the drag callback's coordinates.
                    let cursor = callback_window
                        .cursor_position()
                        .map_err(|error| error.to_string())
                        .map(|position| drag::CursorPosition {
                            x: position.x.round() as i32,
                            y: position.y.round() as i32,
                        });
                    let summary = match cursor {
                        Ok(cursor) => Ok(NativeDragSummary {
                            released: matches!(result, drag::DragResult::Dropped),
                            cursor,
                        }),
                        Err(error) => Err(error),
                    };
                    let _ = callback_sender.try_send(summary);
                    TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
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
                TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
            }
        })
        .map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        let drag_window = window.clone();
        let dispatch_sender = sender.clone();

        app.run_on_main_thread(move || {
            if TAB_DRAG_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                let _ = dispatch_sender.try_send(Err("Another tab drag is already active".into()));
                return;
            }

            let gtk_window = match drag_window.gtk_window() {
                Ok(gtk_window) => gtk_window,
                Err(error) => {
                    let _ = dispatch_sender
                        .try_send(Err(format!("Unable to access the GTK window: {error}")));
                    TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let callback_sender = dispatch_sender;
            let result = drag::start_drag(
                &gtk_window,
                drag::DragItem::Data {
                    provider: Box::new(|_| None),
                    types: vec![TAB_DRAG_TYPE.into()],
                },
                drag::Image::Raw(preview),
                move |result, cursor| {
                    let _ = callback_sender.try_send(Ok(NativeDragSummary {
                        released: matches!(result, drag::DragResult::Dropped),
                        cursor,
                    }));
                    TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
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
                TAB_DRAG_IN_PROGRESS.store(false, Ordering::SeqCst);
            }
        })
        .map_err(|error| error.to_string())?;
    }

    drop(sender);

    let (released, cursor) =
        match tauri::async_runtime::spawn_blocking(move || receiver.recv()).await {
            Ok(Ok(Ok(summary))) => (summary.released, summary.cursor),
            Ok(Ok(Err(error))) => return Err(error),
            Ok(Err(_)) => return Err("The native tab drag ended without a result".into()),
            Err(error) => return Err(error.to_string()),
        };
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let cursor_x = cursor.x;
    let cursor_y = cursor.y;
    let outside = point_is_outside(
        f64::from(cursor_x),
        f64::from(cursor_y),
        position.x,
        position.y,
        size.width,
        size.height,
    );

    // A release outside the source window may still land on another window
    // of this app; that window receives the tab as a merge instead of a
    // tear-off. Hover tracking only reports one window per poll, so the
    // final check re-reads the cursor for the definitive answer.
    let drop_target = if released && outside {
        find_drop_target_at(app, &source, cursor_x, cursor_y)
    } else {
        None
    };

    Ok(TabDragOutcome {
        released,
        outside,
        cursor_x,
        cursor_y,
        target: drop_target.as_ref().map(|target| target.label.clone()),
        target_x: drop_target.as_ref().map_or(0.0, |target| target.local_x),
        target_y: drop_target.as_ref().map_or(0.0, |target| target.local_y),
    })
}

struct NativeDragSummary {
    released: bool,
    cursor: drag::CursorPosition,
}

/// Another application window under the cursor, with the cursor position
/// translated into that window's CSS pixel space.
#[derive(Clone)]
struct DropTarget {
    label: String,
    local_x: f64,
    local_y: f64,
}

/// Finds the visible application window (other than `source`) containing the
/// given desktop-space cursor position, if any. Minimized and hidden windows
/// are skipped; windows that merely sit below another one can still match,
/// since the desktop z-order is not queryable from here — in practice the
/// cursor only hovers windows the user can actually see.
fn find_drop_target_at(
    app: &tauri::AppHandle,
    source: &str,
    cursor_x: i32,
    cursor_y: i32,
) -> Option<DropTarget> {
    for (label, window) in app.webview_windows() {
        if label == source
            || !window.is_visible().unwrap_or(false)
            || window.is_minimized().unwrap_or(false)
        {
            continue;
        }
        let Ok(position) = window.outer_position() else {
            continue;
        };
        let Ok(size) = window.outer_size() else {
            continue;
        };
        if point_is_outside(
            f64::from(cursor_x),
            f64::from(cursor_y),
            position.x,
            position.y,
            size.width,
            size.height,
        ) {
            continue;
        }
        let scale = window.scale_factor().unwrap_or(1.0);
        if !scale.is_finite() || scale <= 0.0 {
            continue;
        }
        return Some(DropTarget {
            label,
            local_x: f64::from(cursor_x - position.x) / scale,
            local_y: f64::from(cursor_y - position.y) / scale,
        });
    }
    None
}

/// Reads the live cursor through the source window (every window shares the
/// same desktop coordinate space) and reports which other window, if any,
/// the dragged tab currently hovers.
fn find_hover_target(app: &tauri::AppHandle, source: &str) -> Option<DropTarget> {
    let source_window = app.get_webview_window(source)?;
    let cursor = source_window.cursor_position().ok()?;
    find_drop_target_at(
        app,
        source,
        cursor.x.round() as i32,
        cursor.y.round() as i32,
    )
}

/// Broadcasts `TabDragHover`/`TabDragLeave` while the native drag loop runs
/// so the window under the cursor can preview where the tab would land.
/// Purely cosmetic: hover state lives in the receiving frontend and every
/// path that ends the drag flips `finished`, which stops the loop after at
/// most one more poll and sends the final leave event.
fn spawn_drag_hover_monitor(app: tauri::AppHandle, source: String, finished: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let mut hovered: Option<DropTarget> = None;
        // What the receiving window is currently showing: pushing the same
        // window/point again would cost an eval and change nothing.
        let mut pushed: Option<(String, f64, f64)> = None;
        let mut pushed_at: Option<Instant> = None;
        loop {
            if finished.load(Ordering::SeqCst) {
                break;
            }

            let target = find_hover_target(&app, &source);
            let same_window = match (&hovered, &target) {
                (Some(previous), Some(current)) => previous.label == current.label,
                (None, None) => true,
                _ => false,
            };

            if !same_window && let Some(previous) = hovered.take() {
                pushed = None;
                let _ = TabDragLeave.emit_to(&app, EventTarget::labeled(previous.label));
            }

            match target.as_ref() {
                Some(current) => {
                    // One unacknowledged push at a time. Every push is a
                    // `wry::eval`, and wry keeps that eval's tracing span
                    // entered until WebView2's completion callback runs — so
                    // pushes issued while the main thread is parked in the
                    // OLE drag loop nest into a span parent chain that
                    // tracing-subscriber later closes with one stack frame per
                    // level. That chain overflowed the default 1 MB stack at
                    // ~2300 levels, and re-entering the registry's slab clear
                    // can also deadlock it; bounding the depth to one removes
                    // both failure modes without slowing the indicator down
                    // (the round trip is well inside the poll interval).
                    if HOVER_PUSH_IN_FLIGHT.load(Ordering::SeqCst)
                        && pushed_at.is_some_and(|sent| sent.elapsed() > HOVER_PUSH_ACK_TIMEOUT)
                    {
                        HOVER_PUSH_IN_FLIGHT.store(false, Ordering::SeqCst);
                    }

                    let position = (current.label.clone(), current.local_x, current.local_y);
                    if pushed.as_ref() != Some(&position)
                        && !HOVER_PUSH_IN_FLIGHT.swap(true, Ordering::SeqCst)
                    {
                        pushed = Some(position);
                        pushed_at = Some(Instant::now());
                        let _ = TabDragHover {
                            x: current.local_x,
                            y: current.local_y,
                        }
                        .emit_to(&app, EventTarget::labeled(current.label.clone()));
                    }
                }
                // Nothing hovered: the next push must go out even when it
                // lands on the same coordinates as the previous one.
                None => pushed = None,
            }
            hovered = target;

            tokio::time::sleep(TAB_HOVER_POLL_INTERVAL).await;
        }

        if let Some(previous) = hovered {
            let _ = TabDragLeave.emit_to(&app, EventTarget::labeled(previous.label));
        }
    });
}

/// Called by the window that has just rendered a `TabDragHover` indicator.
/// Releases the monitor's in-flight slot so the next position can go out.
///
/// This acknowledgement is what bounds the host-side cost of a cross-window
/// drag: see [`spawn_drag_hover_monitor`] for what happens without it.
#[tauri::command]
#[specta::specta]
pub fn tab_drag_hover_ack() {
    HOVER_PUSH_IN_FLIGHT.store(false, Ordering::SeqCst);
}

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
