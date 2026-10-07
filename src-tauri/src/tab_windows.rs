//! Tab tear-off and cross-window merge support.
//!
//! A tab drag stays the frontend's own affair — live reorder, its own ghost —
//! until the cursor leaves the *window*, and only then is it handed to the
//! platform's native drag loop as soon as it does: OLE `DoDragDrop` on Windows,
//! an `NSDraggingSession` on macOS, a GTK drag on Linux, each of which owns the
//! drag image and mouse capture until the user releases the primary button or
//! presses Escape. The distinction matters because the loop can report only
//! what it can see: handing the pointer over while it is still inside this
//! window leaves it with nothing to say about a release *here*, which then reads
//! as a drop on the desktop and tears off a window. From past the window edge,
//! the absence of a drop target is finally evidence of something.
//! [`tab_drag_outside`] answers for the platforms whose WebView stops reporting
//! the pointer at that edge, which leaves no position to measure; Wayland has
//! neither, so the frontend measures the edge from its own events.
//!
//! The same commands create the detached webview window while keeping the
//! opaque tab snapshot in Rust until the new frontend consumes it.
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

const WINDOW_LABEL_PREFIX: &str = "tab-window-";
const DEFAULT_WIDTH: f64 = 960.0;
const DEFAULT_HEIGHT: f64 = 680.0;
const MIN_WIDTH: f64 = 640.0;
const MIN_HEIGHT: f64 = 480.0;
const MAX_WIDTH: f64 = 1280.0;
const MAX_HEIGHT: f64 = 900.0;
static TAB_DRAG_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const TAB_DRAG_FALLBACK_ICON: &[u8] = include_bytes!("../icons/32x32.png");
/// MIME type this application advertises while a tab is dragged. Nothing
/// outside the application understands it, so a drop anywhere else is offered
/// the tab and refuses it — but the application's own windows accept it, which
/// is how the drag learns where it landed (see [`attach_tab_drop_target`]).
/// GTK in particular refuses to start a drag that advertises no target at all.
const TAB_DRAG_TYPE: &str = "application/x-dae-tab-drag";

/// Where a finished native drag landed, as reported by the window that accepted
/// the drop.
#[derive(Clone)]
struct TabDropTarget {
    label: String,
    x: f64,
    y: f64,
}

/// Set while a native tab drag is in flight, by the window whose WebView
/// accepted the drop. Read once by the drag that started it.
///
/// This is the Linux answer to a question the other platforms answer with
/// desktop coordinates: a Wayland session exposes neither a global pointer
/// position nor a window position, so the coordinate hit test can answer
/// nothing there. The platform's own drag routing, by contrast, does still
/// deliver the drop to the window under the pointer — so the window that
/// handled it *is* the drop target. Only one native drag can be in flight at a
/// time ([`TAB_DRAG_IN_PROGRESS`]), so one slot is enough.
#[cfg(target_os = "linux")]
static TAB_DROP_TARGET: Mutex<Option<TabDropTarget>> = Mutex::new(None);

/// Forgets the previous drag's drop, so a stale one cannot decide a later drag.
#[cfg(target_os = "linux")]
fn clear_tab_drop_target() {
    if let Ok(mut slot) = TAB_DROP_TARGET.lock() {
        *slot = None;
    }
}

/// Consumes the drop the drag that just finished produced, if any.
#[cfg(target_os = "linux")]
fn take_tab_drop_target() -> Option<TabDropTarget> {
    TAB_DROP_TARGET.lock().ok().and_then(|mut slot| slot.take())
}

/// Whether the drag in progress offers [`TAB_DRAG_TYPE`], so a window's handlers
/// only speak for tab drags and leave every other drag to WebKit.
#[cfg(target_os = "linux")]
fn offers_tab_drag(context: &gtk::gdk::DragContext) -> bool {
    context
        .list_targets()
        .into_iter()
        .any(|atom| atom.name().as_str() == TAB_DRAG_TYPE)
}

/// Answers the drag while it is over this window.
///
/// A destination that never calls `drag_status` is one the drag is never
/// offered to: GDK reads the silence as a refusal, and the compositor ends the
/// session as soon as the button comes up without ever delivering a drop — which
/// from the source is indistinguishable from releasing over the desktop.
#[cfg(target_os = "linux")]
fn accept_tab_drag(context: &gtk::gdk::DragContext, time: u32) -> bool {
    if !offers_tab_drag(context) {
        return false;
    }
    context.drag_status(gtk::gdk::DragAction::MOVE, time);
    true
}

/// Records a tab dropped on this window and accepts it.
///
/// Accepting is what tells the source its tab found a home; the tab itself then
/// travels as a window-to-window handoff rather than as drag data, so nothing
/// has to be marshalled through the clipboard here — and equally important, the
/// WebView's own target list is left alone, so the page keeps its HTML5 drag
/// and drop.
#[cfg(target_os = "linux")]
fn handle_tab_drop(
    label: &str,
    context: &gtk::gdk::DragContext,
    x: i32,
    y: i32,
    time: u32,
) -> bool {
    use gtk::prelude::*;

    if !offers_tab_drag(context) {
        return false;
    }
    if let Ok(mut slot) = TAB_DROP_TARGET.lock() {
        *slot = Some(TabDropTarget {
            label: label.to_owned(),
            x: f64::from(x),
            y: f64::from(y),
        });
    }
    context.drag_finish(true, false, time);
    true
}

/// Makes `window` report a dropped tab back to the drag that started it.
///
/// Must be called on the main thread: every GTK call below asserts it.
///
/// The destination has to be declared on the toplevel: GTK engages with a drag
/// — dispatching motion, and delivering a drop at all — only for a window that
/// claims to accept it, and with no claim the compositor refuses the drag
/// outright and nothing but `drag-leave` arrives anywhere. The drop itself is
/// still delivered to the WebView under the pointer, which is where it is
/// caught.
///
/// Accepting the drop is what tells the source its tab found a home. The tab
/// then travels as a window-to-window handoff rather than as drag data, so
/// nothing has to be marshalled through the clipboard here — and equally
/// important, the WebView's own target list is left completely alone, so the
/// page keeps its HTML5 drag and drop.
#[cfg(target_os = "linux")]
pub fn attach_tab_drop_target(window: &tauri::WebviewWindow) -> Result<(), String> {
    use gtk::{gdk, prelude::*};

    let gtk_window = window.gtk_window().map_err(|error| error.to_string())?;
    let entries = [gtk::TargetEntry::new(
        TAB_DRAG_TYPE,
        gtk::TargetFlags::empty(),
        0,
    )];
    gtk_window.drag_dest_set(gtk::DestDefaults::MOTION, &entries, gdk::DragAction::MOVE);
    gtk_window.drag_dest_set_track_motion(true);

    // The window declares the destination and answers the drag; the WebView is
    // where the drop itself lands.
    gtk_window.connect_drag_motion(move |_, context, _x, _y, time| {
        accept_tab_drag(context, time)
    });
    let window_label = window.label().to_owned();
    gtk_window.connect_drag_drop(move |_, context, x, y, time| {
        handle_tab_drop(&window_label, context, x, y, time)
    });

    let label = window.label().to_owned();

    window
        .with_webview(move |webview| {
            let widget: gtk::Widget = webview.inner().upcast();
            widget.connect_drag_motion(move |_, context, _x, _y, time| {
                accept_tab_drag(context, time)
            });
            widget.connect_drag_drop(move |_, context, x, y, time| {
                handle_tab_drop(&label, context, x, y, time)
            });
        })
        .map_err(|error| error.to_string())
}
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

/// Hands a pooled window the tab it is to become.
///
/// Separate from [`TabMergedIntoWindow`] because it means the opposite thing:
/// a merge adds a tab beside the ones already there, while this *replaces*
/// them. A pooled window booted onto a default surface nobody has seen, so
/// there is nothing of its own worth keeping.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "tab-adopted-into-window")]
pub struct TabAdoptedIntoWindow {
    pub payload: String,
}

/// Delivers an event from the platform's main thread.
///
/// Emitting from any other thread can freeze the whole application on Linux:
/// `emit` holds tauri's `webviews` lock for the whole loop in which it
/// round-trips each webview's `eval` back to the main thread, and the main
/// thread wants that same lock inside WebKitGTK's `ipc://` callback to resolve
/// the webview an invoke arrived on. The two then wait on each other, and the
/// lock is the only thing left running. On the main thread the `eval` runs
/// inline rather than being waited for, so the lock is never held across it.
fn emit_to_window<E>(app: &tauri::AppHandle, label: String, event: E)
where
    E: tauri_specta::Event + Serialize + Clone + Send + 'static,
{
    let emitter = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let _ = event.emit_to(&emitter, EventTarget::labeled(label));
    }) {
        log::warn!("Unable to deliver a tab drag event: {error}");
    }
}

/// Hands a merged tab to the window the native drop landed on.
///
/// The frontend cannot emit [`TabMergedIntoWindow`] itself, however convenient
/// `emitTo` would be: that is an invoke, and its handler emits on the async
/// runtime — the one thread [`emit_to_window`] exists to keep emitting off. As
/// a non-`async` command the emit happens to run on the main thread anyway.
#[tauri::command]
#[specta::specta]
pub fn merge_tab_into_window(
    app: tauri::AppHandle,
    target: String,
    payload: String,
    x: f64,
    y: f64,
) {
    emit_to_window(&app, target, TabMergedIntoWindow { payload, x, y });
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
    pool: Mutex<PoolSlot>,
}

/// The warm pool's single slot: the window still booting, and the window that
/// has finished booting and may be adopted.
///
/// Both are tracked because a window is not adoptable the moment it exists. Its
/// frontend installs the listener that receives the tab, and a hand-off sent
/// before that is simply lost.
#[derive(Default)]
struct PoolSlot {
    /// A window exists and is still loading. Holds its label.
    booting: Option<String>,
    /// A build was authorised but the window does not exist yet, so there is no
    /// label to record. Without this a second prime could slip in between the
    /// check and the build and the pool would cost twice what it was budgeted.
    building: bool,
    ready: Option<String>,
}

impl PoolSlot {
    /// Claims the right to build, or refuses because one is already primed or
    /// on its way.
    fn reserve(&mut self) -> bool {
        if self.building || self.booting.is_some() || self.ready.is_some() {
            return false;
        }
        self.building = true;
        true
    }

    /// Records the window a reservation produced.
    fn built(&mut self, label: &str) {
        self.building = false;
        self.booting = Some(label.to_string());
    }

    /// Gives up a reservation whose build failed, so the pool can try again.
    fn abandoned(&mut self) {
        self.building = false;
    }

    /// Promotes a booting window to adoptable. Returns whether the label was
    /// the pool's own, so a stray report cannot vouch for a window the pool
    /// never built.
    fn became_ready(&mut self, label: &str) -> bool {
        if self.booting.as_deref() != Some(label) {
            return false;
        }
        self.booting = None;
        self.ready = Some(label.to_string());
        true
    }

    fn take(&mut self) -> Option<String> {
        self.ready.take()
    }

    /// Forgets a window that went away, at whichever end of its life it was.
    fn forget(&mut self, label: &str) {
        if self.booting.as_deref() == Some(label) {
            self.booting = None;
        }
        if self.ready.as_deref() == Some(label) {
            self.ready = None;
        }
    }
    /// Empties the slot, returning every window it was holding so the caller can
    /// close them. Every one, not just the adoptable one: a booting window is a
    /// real window holding real memory, and the point of switching the pool off
    /// is to stop paying for it.
    fn take_all(&mut self) -> Vec<String> {
        self.building = false;
        [self.ready.take(), self.booting.take()]
            .into_iter()
            .flatten()
            .collect()
    }
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

    /// Whether a tear-off may adopt a primed window instead of building one.
    ///
    /// This is the user's setting, less an escape hatch: `DAE_TAB_POOL=0` forces
    /// it off, which is what comparing the two paths by hand needs - a run that
    /// does not have to edit settings and restart.
    fn pool_enabled() -> bool {
        if std::env::var("DAE_TAB_POOL").is_ok_and(|value| value == "0") {
            return false;
        }
        crate::settings::tab_pool_enabled()
    }

    /// Starts one hidden window booting, if the pool is empty.
    ///
    /// It is given no hand-off, so it opens on the Overview surface the way any
    /// window without one does. Everything it does from here - module load,
    /// mount, its own queries - is what a tear-off would otherwise have to wait
    /// for, and it happens while the user is doing something else.
    fn prime_pool(&self, app: &tauri::AppHandle) {
        if !Self::pool_enabled() {
            return;
        }
        // `PoolSlot::reserve` cannot run in a match guard: a guard's bindings
        // are immutable until it passes.
        match self.pool.lock() {
            Ok(mut slot) => {
                if !slot.reserve() {
                    return;
                }
            }
            Err(_) => return,
        }

        let label = self.next_label(app);
        let build = crate::window_material::configure(
            WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
                .title("dae")
                .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
                .decorations(false)
                .visible(false)
                // Tells the page it is the pool, so only the pool reports back.
                .initialization_script("window.__DAE_POOL_WINDOW = 1;"),
        )
        .build();

        match build {
            Ok(window) => {
                let cleanup_app = app.clone();
                let cleanup_label = label.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed)
                        && let Some(state) = cleanup_app.try_state::<TabWindowState>()
                    {
                        state.forget_pooled(&cleanup_label);
                    }
                });
                if let Ok(mut slot) = self.pool.lock() {
                    slot.built(&label);
                }
            }
            Err(error) => {
                if let Ok(mut slot) = self.pool.lock() {
                    slot.abandoned();
                }
                log::warn!("Unable to prime a pooled window: {error}");
            }
        }
    }

    /// Promotes a booting window to adoptable once its frontend reports in.
    fn mark_pool_ready(&self, label: &str) {
        if let Ok(mut slot) = self.pool.lock() {
            slot.became_ready(label);
        }
    }

    /// Claims the ready window, if any. Returns `None` when the pool is empty
    /// or disabled, which sends the caller down the cold path.
    fn take_pooled(&self) -> Option<String> {
        // The setting wins over a window that already exists: switching the pool
        // off has to stop hand-offs, and `sync_tab_pool` closes what is left.
        if !Self::pool_enabled() {
            return None;
        }
        self.pool.lock().ok().and_then(|mut slot| slot.take())
    }

    fn forget_pooled(&self, label: &str) {
        if let Ok(mut slot) = self.pool.lock() {
            slot.forget(label);
        }
    }

    /// Closes whatever the pool holds, for the setting being turned off.
    fn dispose_pool(&self, app: &tauri::AppHandle) {
        let labels = match self.pool.lock() {
            Ok(mut slot) => slot.take_all(),
            Err(_) => return,
        };
        for label in labels {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.close();
            }
        }
    }
}

/// Whether the pointer has left the window a dragged tab came from.
///
/// The bar hand-off the frontend measures from its own layout covers a pointer
/// it still hears about, so this is the fallback for a platform whose WebView
/// stops delivering pointer events at the window edge: polled while a tab drag
/// is in flight, it reports the gesture outside the moment nobody can say
/// otherwise. A position outside the window is outside the tab bar in any case.
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

/// Whether a tab drag has to be settled from the frontend's own pointer bounds
/// instead of by the native drag loop.
///
/// A Wayland session denies clients every global coordinate: `tao`'s
/// `cursor_position` answers `(0, 0)` there and a toplevel has no
/// `outer_position`, so [`tab_drag_outside`] can only ever report "inside" and
/// [`start_tab_drag`] can never observe a release outside the window either.
/// The WebView's own pointer events do keep carrying coordinates past the
/// window edge under Wayland, so the frontend measures the edge itself — and
/// must not hand the pointer to the GTK drag loop, which would take the
/// remaining events with it.
///
/// Always `false` off Linux, where the native path is authoritative.
#[tauri::command]
#[specta::specta]
pub fn tab_drag_uses_frontend_bounds() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::linux_graphics::wayland_is_the_backend()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Hands a tab gesture that has left the window to the platform's native drag
/// loop. The shell owns the drag image and mouse capture until the user releases
/// the primary button or presses Escape, so the WebView does not need global
/// mouse hooks.
///
/// `offset_x`/`offset_y` spot the drag image under the cursor; `press_x` and
/// `press_y` are where the gesture's `pointerdown` happened in the window's CSS
/// pixels, which a Wayland session needs back — see [`hand_button_back_to_webview`].
///
/// Windows blocks in `DoDragDrop` and returns through the main-thread closure;
/// macOS and GTK run asynchronous drag sessions, so the completion is reported
/// from the drag callback, possibly after this command has already finished.
///
/// While the native loop runs, a hover monitor broadcasts `TabDragHover` /
/// `TabDragLeave` to the window under the cursor so drop targets can show an
/// insertion indicator — everywhere a session reveals the cursor's desktop
/// position to begin with. The outcome also reports which app window received
/// the drop, if any, so the frontend can merge the tab instead of detaching.
#[tauri::command]
#[specta::specta]
pub async fn start_tab_drag(
    app: tauri::AppHandle,
    source: String,
    preview: Option<String>,
    offset_x: f64,
    offset_y: f64,
    press_x: f64,
    press_y: f64,
) -> Result<TabDragOutcome, String> {
    if !matches!(std::env::consts::OS, "windows" | "macos" | "linux") {
        let _ = (
            &app, &source, &preview, offset_x, offset_y, press_x, press_y,
        );
        return Err("Native tab drag is not supported on this platform".into());
    }

    // A drop left over from an earlier drag must not decide this one.
    #[cfg(target_os = "linux")]
    clear_tab_drop_target();

    // The monitor hit-tests desktop coordinates, which a Wayland session
    // exposes to nobody: polling there matches whichever window sits at (0, 0)
    // and lights up its insertion indicator for a pointer no one can see. The
    // drop still arrives with the receiver's own coordinates, so a merge has
    // everything it needs without the preview.
    let hover_finished = Arc::new(AtomicBool::new(false));
    if !tab_drag_uses_frontend_bounds() {
        spawn_drag_hover_monitor(app.clone(), source.clone(), hover_finished.clone());
    }

    let outcome =
        run_native_tab_drag(&app, source, preview, offset_x, offset_y, press_x, press_y).await;

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
    press_x: f64,
    press_y: f64,
) -> Result<TabDragOutcome, String> {
    // The release this session owes the WebView goes back at the point the
    // press happened, which is a position only the frontend knows.
    #[cfg(not(target_os = "linux"))]
    let _ = (press_x, press_y);

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

        // The frontend decides when to hand over from its own pointer events, so
        // the source tab sits wherever the last of them left it; the ghost
        // continues from the true cursor position instead.
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

            let callback_sender = dispatch_sender.clone();
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

    // The instant the native drag handed control back, before any window work
    // starts. It becomes a timeline's zero point only if this drag turns out
    // to be a tear-off, which the drop resolution below decides, so it is held
    // here rather than stamped.
    let drag_ended = crate::tab_perf::now_ms();

    // The drag is over, and the button that started it went out of this
    // application's reach the moment the drag took it.
    #[cfg(target_os = "linux")]
    if tab_drag_uses_frontend_bounds() {
        hand_button_back_to_webview(&window, press_x, press_y);
    }

    let cursor_x = cursor.x;
    let cursor_y = cursor.y;
    let (outside, drop_target) = resolve_native_drop(app, &source, &window, released, cursor)?;

    // Only a tear-off gets a timeline; every other drag clears the slot.
    // Stamping each drag instead left a soak's release behind, and the next
    // tear-off then counted the user's own dragging time as latency - 1356ms of
    // it, for a drag that took that long to perform.
    if crate::tab_perf::starts_tear_off(released, outside, drop_target.is_some()) {
        crate::tab_perf::stamp_release(drag_ended);
    } else {
        crate::tab_perf::clear_release();
    }

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

/// Gives the WebView back the primary button the Wayland drag took from it.
///
/// An xdg drag is grabbed by the compositor, so the release that ends it goes
/// into the drag session and this client never sees a `GDK_BUTTON_RELEASE` for
/// the press that started it. WebKit is left counting a button released long
/// ago: `:active` keeps matching, and the *next* real press pairs with the
/// stale one instead of itself, so its release reports the common ancestor of
/// the two rather than what the user clicked — a click the window appears to
/// have swallowed, until another click re-pairs the stream. Handing back the
/// release the compositor owes settles that state without disturbing the drag,
/// whose own routing has already finished by the time this runs.
#[cfg(target_os = "linux")]
fn hand_button_back_to_webview(window: &tauri::WebviewWindow, press_x: f64, press_y: f64) {
    use gtk::prelude::*;

    // The press point is frontend state, so it arrives unsanitized; a wild
    // position would only aim the release at a different widget.
    let x = if press_x.is_finite() { press_x } else { 0.0 };
    let y = if press_y.is_finite() { press_y } else { 0.0 };

    if let Err(error) = window.with_webview(move |webview| {
        let widget: gtk::Widget = webview.inner().upcast();
        release_primary_button(&widget, x, y);
    }) {
        log::warn!("Unable to return the tab drag's button to the WebView: {error}");
    }
}

/// Queues a primary-button release over `widget` at `x`/`y`, in its own
/// window's CSS pixels.
///
/// The WebView is a no-window widget, so `widget.window()` answers with the
/// surface WebKit paints into — the one whose origin the page's viewport
/// shares, which is the space these coordinates are in.
#[cfg(target_os = "linux")]
fn release_primary_button(widget: &gtk::Widget, x: f64, y: f64) {
    use gtk::{
        gdk,
        glib::{self, translate::ToGlibPtr},
        prelude::*,
    };

    let Some(gdk_window) = widget.window() else {
        return;
    };
    let display = gdk_window.display();
    let Some(pointer) = display.default_seat().and_then(|seat| seat.pointer()) else {
        return;
    };

    // GDK walks an event's axis map for as many entries as the device claims,
    // and releases the array with the event, so it is allocated rather than
    // left null. Zeroed entries read as `GDK_AXIS_IGNORE` and match no query.
    let axis_count = pointer.n_axes().max(0) as usize;
    let axes = if axis_count == 0 {
        std::ptr::null_mut()
    } else {
        unsafe { glib::ffi::g_malloc0(axis_count * std::mem::size_of::<f64>()) as *mut f64 }
    };

    unsafe {
        let event = gdk::ffi::gdk_event_new(gdk::ffi::GDK_BUTTON_RELEASE);
        if event.is_null() {
            if !axes.is_null() {
                glib::ffi::g_free(axes as *mut _);
            }
            return;
        }

        // An event owns the reference it holds on its window, and
        // `gdk_event_put` dispatches a copy of its own, so this one is the
        // caller's to release.
        (*event).button.window = gdk_window.to_glib_full();
        (*event).button.axes = axes;
        (*event).button.time = gtk::current_event_time();
        (*event).button.x = x;
        (*event).button.y = y;
        let (root_x, root_y) = gdk_window.root_coords(x as i32, y as i32);
        (*event).button.x_root = root_x as f64;
        (*event).button.y_root = root_y as f64;
        (*event).button.button = 1;
        gdk::ffi::gdk_event_set_device(event, pointer.to_glib_none().0);
        gdk::ffi::gdk_event_put(event);
        gdk::ffi::gdk_event_free(event);
    }
}

/// Where a finished native drag ended, as far as this platform can tell.
///
/// Linux answers from the drag routing itself: whichever window's WebView
/// accepted the drop is the window the pointer was over, which is the one piece
/// of placement evidence a Wayland session offers — it has neither a global
/// pointer position nor a window position to measure against. Everywhere else
/// the desktop coordinates still decide it, which leaves the platform's own
/// drag loop, its drag image and its cross-window hit test untouched.
fn resolve_native_drop(
    app: &tauri::AppHandle,
    source: &str,
    window: &tauri::WebviewWindow,
    released: bool,
    cursor: drag::CursorPosition,
) -> Result<(bool, Option<DropTarget>), String> {
    #[cfg(target_os = "linux")]
    {
        let _ = (app, window, cursor);
        if !released {
            return Ok((false, None));
        }
        return Ok(match take_tab_drop_target() {
            // Dropped back on the window it came from: neither a tear-off nor
            // anything to merge.
            Some(drop) if drop.label == source => (false, None),
            Some(drop) => (
                true,
                Some(DropTarget {
                    label: drop.label,
                    local_x: drop.x,
                    local_y: drop.y,
                }),
            ),
            // No window accepted it: released over the desktop, or over an
            // application that does not take tabs.
            None => (true, None),
        });
    }

    #[cfg(not(target_os = "linux"))]
    {
        let position = window.outer_position().map_err(|error| error.to_string())?;
        let size = window.outer_size().map_err(|error| error.to_string())?;
        let outside = point_is_outside(
            f64::from(cursor.x),
            f64::from(cursor.y),
            position.x,
            position.y,
            size.width,
            size.height,
        );

        // A release outside the source window may still land on another window
        // of this app; that window receives the tab as a merge instead of a
        // tear-off.
        let drop_target = if released && outside {
            find_drop_target_at(app, source, cursor.x, cursor.y)
        } else {
            None
        };

        Ok((outside, drop_target))
    }
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
                emit_to_window(&app, previous.label, TabDragLeave);
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
                        emit_to_window(
                            &app,
                            current.label.clone(),
                            TabDragHover {
                                x: current.local_x,
                                y: current.local_y,
                            },
                        );
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
            emit_to_window(&app, previous.label, TabDragLeave);
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
///
/// Deliberately not `async`: building a webview window ends with tauri's
/// `webview-created` broadcast, which holds the `webviews` lock across an
/// `eval` round trip — see [`emit_to_window`] for what that costs on a worker
/// thread. A non-`async` command runs inline on the thread that received the
/// invoke, which on every platform this app builds windows on is the one that
/// owns the event loop, so the eval completes where it is issued.
/// The two things every window that can hold a tab owes, whether it was built
/// for the tear-off or adopted from the pool.
///
/// Kept together because forgetting the second is invisible until a drag is
/// refused: GTK dispatches motion and delivers a drop at all only for a window
/// that claims to accept it, so a window missing this quietly turns the merge
/// the user meant into a tear-off. Must run on the main thread, which is the
/// only thread GTK may be touched from.
fn finish_tab_window(window: &tauri::WebviewWindow) {
    crate::window_material::attach(window);
    #[cfg(target_os = "linux")]
    if let Err(error) = attach_tab_drop_target(window) {
        log::warn!("Unable to register the tab drop target: {error}");
    }
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub fn tear_off_tab(
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
    // Adopt the primed window when there is one. It has already paid the boot
    // cost this function is otherwise about to pay again, so all that is left
    // is to place it, say what it has become, and show it. Everything between
    // here and `SHOWN` is the whole of what a pooled tear-off costs.
    if let Some(pooled) = state.take_pooled() {
        if let Some(window) = app.get_webview_window(&pooled) {
            // Keeps the drag's own stamp, so the timeline still starts at the
            // instant the user let go rather than at this call.
            let _ = crate::tab_perf::mark_release();
            crate::tab_perf::set_current_label(&pooled);
            crate::tab_perf::mark("ADOPTED");
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
            let _ = window.set_position(PhysicalPosition::new(physical_x, physical_y));
            finish_tab_window(&window);
            emit_to_window(&app, pooled.clone(), TabAdoptedIntoWindow { payload });
            if let Err(error) = window.show() {
                log::warn!("Unable to show a pooled window: {error}");
            }
            let _ = window.set_focus();
            crate::tab_perf::mark("SHOWN");
            // Replaces the slot once this tear-off is over, so priming does not
            // compete with the window the user is now looking at.
            let replenish_app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if let Some(state) = replenish_app.try_state::<TabWindowState>() {
                    state.prime_pool(&replenish_app);
                }
            });
            return Ok(pooled);
        }
        // It went away between the claim and here. The cold path below is
        // always allowed to be the answer, so nothing is lost but the prime.
    }

    let label = state.next_label(&app);

    state.insert(label.clone(), payload)?;

    // The drag path already stamped the release and this keeps that first
    // stamp, so the anchor is the instant the user let go rather than the
    // instant this function happened to run. A tear-off that did not come from
    // a drag has nothing stamped yet and gets one here.
    let _ = crate::tab_perf::mark_release();
    crate::tab_perf::set_current_label(&label);
    // The window's own first mark. Its distance from the release is the JS round
    // trip plus this command's IPC — the cost a hand-off done through
    // `initialization_script` would remove.
    crate::tab_perf::mark("TEAR_OFF_ENTER");

    let build_result = crate::window_material::configure(
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
            // Hands the new window its anchor before any page script runs, and
            // nothing at all when profiling is off.
            .initialization_script(crate::tab_perf::anchor_script()),
    )
    .build();

    let window = match build_result {
        Ok(window) => window,
        Err(error) => {
            let _ = state.take(&label);
            return Err(error.to_string());
        }
    };

    // First paint of the placeholder the user actually sees as "the window".
    crate::tab_perf::mark("BUILT");

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

    finish_tab_window(&window);

    // Probe for the warm-pool question (NOTES.md): with `DAE_TAB_PERF_HIDE_MS`
    // set, the show is deferred, so this window's own timeline says whether it
    // reached `first-frame` without ever having been shown. That is the
    // difference between a pool that banks the cold start and one that only
    // defers it to the moment the window appears.
    let defer_ms = std::env::var("DAE_TAB_PERF_HIDE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    if defer_ms > 0 {
        crate::tab_perf::mark("SHOW_DEFERRED");
        let deferred = window.clone();
        let deferred_label = label.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(defer_ms)).await;
            crate::tab_perf::mark_for(&deferred_label, "SHOWING");
            if let Err(error) = deferred.show() {
                log::warn!("Unable to show a deferred probe window: {error}");
            }
            let _ = deferred.set_focus();
            crate::tab_perf::mark_for(&deferred_label, "SHOWN");
        });
        return Ok(label);
    }

    if let Err(error) = window.show() {
        let _ = window.close();
        let _ = state.take(&label);
        return Err(error.to_string());
    }
    let _ = window.set_focus();
    crate::tab_perf::mark("SHOWN");

    Ok(label)
}

/// Primes the warm pool, for the startup path which has no other reason to
/// touch it. Delayed by the caller so the app's own launch does not compete
/// with a window nobody has asked for yet.
pub fn prime_pool_at_startup(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<TabWindowState>() {
        state.prime_pool(app);
    }
}

/// Brings the pool in line with the setting, which the frontend calls after
/// saving one.
///
/// Both directions need this. Switched on, a pool that was never primed would
/// stay empty until a tear-off happened to refill it, so the first tear-off
/// after the change would still be cold. Switched off, the hidden window keeps
/// its memory for nothing.
#[tauri::command]
#[specta::specta]
pub fn sync_tab_pool(app: tauri::AppHandle, state: tauri::State<'_, TabWindowState>) {
    if TabWindowState::pool_enabled() {
        state.prime_pool(&app);
    } else {
        state.dispose_pool(&app);
    }
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

/// Called by a pooled window once its listener is installed.
///
/// Until this arrives the pool will not hand it a tab. A hand-off delivered to
/// a page that is not listening is simply lost, and the pool would then hold a
/// window it believed was primed while the tear-off that adopted it showed
/// nothing.
#[tauri::command]
#[specta::specta]
pub fn pool_window_ready(state: tauri::State<'_, TabWindowState>, label: String) {
    state.mark_pool_ready(&label);
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

    mod pool {
        use super::super::PoolSlot;

        #[test]
        fn a_second_prime_is_refused_while_one_is_in_flight() {
            let mut slot = PoolSlot::default();
            assert!(slot.reserve());
            assert!(
                !slot.reserve(),
                "a racing prime would build a second window"
            );
            slot.built("pool-1");
            assert!(!slot.reserve(), "priming again would hold two windows");
        }

        #[test]
        fn a_second_prime_is_refused_once_one_is_waiting() {
            let mut slot = PoolSlot::default();
            assert!(slot.reserve());
            slot.built("pool-1");
            assert!(slot.became_ready("pool-1"));
            assert!(!slot.reserve());
        }

        #[test]
        fn a_failed_build_lets_the_pool_try_again() {
            let mut slot = PoolSlot::default();
            assert!(slot.reserve());
            slot.abandoned();
            assert!(
                slot.reserve(),
                "a build that failed must not wedge the pool"
            );
        }

        #[test]
        fn only_the_pools_own_window_can_report_ready() {
            let mut slot = PoolSlot::default();
            assert!(slot.reserve());
            slot.built("pool-1");
            assert!(!slot.became_ready("someone-else"));
            assert_eq!(slot.take(), None, "a stray report must not be adoptable");
            assert!(slot.became_ready("pool-1"));
            assert_eq!(slot.take().as_deref(), Some("pool-1"));
        }

        #[test]
        fn taking_empties_the_slot_so_the_next_tear_off_primes_again() {
            let mut slot = PoolSlot::default();
            slot.built("pool-1");
            slot.became_ready("pool-1");
            assert_eq!(slot.take().as_deref(), Some("pool-1"));
            assert_eq!(slot.take(), None, "claimed twice");
        }

        #[test]
        fn a_window_that_died_is_forgotten_from_either_end() {
            let mut booting = PoolSlot::default();
            booting.built("pool-1");
            booting.forget("pool-1");
            assert!(!booting.became_ready("pool-1"), "it is gone");

            let mut ready = PoolSlot::default();
            ready.built("pool-2");
            ready.became_ready("pool-2");
            ready.forget("pool-2");
            assert_eq!(ready.take(), None, "a destroyed window is not handed over");
        }

        #[test]
        fn forgetting_another_window_leaves_the_pool_alone() {
            let mut slot = PoolSlot::default();
            slot.built("pool-1");
            slot.became_ready("pool-1");
            slot.forget("tab-window-9");
            assert_eq!(slot.take().as_deref(), Some("pool-1"));
        }
    }
}
