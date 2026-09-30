// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use crate::{CursorPosition, DragItem, DragMode, DragResult, Error, Image, Options};
use gdkx11::{
    gdk,
    glib::{ObjectExt, Propagation, SignalHandlerId},
};
use gtk::{
    gdk_pixbuf,
    prelude::{
        DeviceExt, DragContextExtManual, GdkPixbufExt, PixbufLoaderExt, SeatExt, WidgetExt,
        WidgetExtManual,
    },
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, Mutex},
};

pub fn start_drag<F: Fn(DragResult, CursorPosition) + Send + 'static>(
    window: &gtk::ApplicationWindow,
    item: DragItem,
    image: Image,
    on_drop_callback: F,
    options: Options,
) -> crate::Result<()> {
    log::debug!("Starting drag operation with mode: {:?}", options.mode);
    let handler_ids: Arc<Mutex<Vec<SignalHandlerId>>> = Arc::new(Mutex::new(vec![]));
    let drag_action = match options.mode {
        DragMode::Copy => gdk::DragAction::COPY,
        DragMode::Move => gdk::DragAction::MOVE,
        DragMode::Link => gdk::DragAction::LINK,
        // Same "let the drop target choose" contract as the Windows build.
        DragMode::Any => gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK,
    };

    // GTK refuses to run a drag without at least one advertised target, so
    // data drags publish their type names (private to the dragging app) as
    // drag targets.
    let count_release_as_drop = matches!(item, DragItem::Data { .. });
    match item {
        DragItem::Files(paths) => {
            log::debug!("Setting up file drag with {} paths", paths.len());
            window.drag_source_set(gdk::ModifierType::BUTTON1_MASK, &[], drag_action);
            window.drag_source_add_uri_targets();
            handler_ids
                .lock()
                .unwrap()
                .push(window.connect_drag_data_get(move |_, _, data, _, _| {
                    log::debug!("Preparing URIs for drag data");
                    let uris: Vec<String> = paths
                        .iter()
                        .map(|path| format!("file://{}", path.display()))
                        .collect();
                    let uris: Vec<&str> = uris.iter().map(|s| s.as_str()).collect();
                    log::debug!("Setting URIs: {:?}", uris);
                    data.set_uris(&uris);
                }));
        }
        DragItem::Data { types, .. } => {
            let entries: Vec<gtk::TargetEntry> = types
                .iter()
                .enumerate()
                .map(|(index, target)| {
                    gtk::TargetEntry::new(target, gtk::TargetFlags::empty(), index as u32)
                })
                .collect();
            window.drag_source_set(gdk::ModifierType::BUTTON1_MASK, &entries, drag_action);
            // No drag_data_get wiring: the types are private to the dragging
            // application, so no drop target will ever request their data.
        }
    }

    if let Some(target_list) = &window.drag_source_get_target_list() {
        log::debug!("Got target list, initiating drag");
        if let Some(drag_context) =
            window.drag_begin_with_coordinates(target_list, drag_action, 1, None, -1, -1)
        {
            log::debug!("Drag context created successfully");
            let callback = Rc::new(on_drop_callback);
            // drag-failed, drop-performed and drag-end are meant to be mutually
            // exclusive reports of one ending, but guard against a double
            // delivery anyway: callers hand the outcome to a one-shot channel.
            let fired = Rc::new(Cell::new(false));
            on_drop_failed(
                callback.clone(),
                fired.clone(),
                window,
                &handler_ids,
                &options,
                count_release_as_drop,
            );
            on_drop_performed(
                callback.clone(),
                fired.clone(),
                window,
                &handler_ids,
                &drag_context,
            );
            on_drag_end(
                callback.clone(),
                fired.clone(),
                window,
                &handler_ids,
                count_release_as_drop,
            );

            log::debug!("Setting up drag icon");
            let icon_pixbuf: Option<gdk_pixbuf::Pixbuf> = match &image {
                Image::Raw(data) => image_binary_to_pixbuf(data),
                Image::File(path) => match std::fs::read(path) {
                    Ok(bytes) => image_binary_to_pixbuf(&bytes),
                    Err(_) => None,
                },
            };
            if let Some(icon) = icon_pixbuf {
                // A cairo surface is the only way to hand GDK a bitmap *and* its
                // pixel density. `gtk_drag_set_icon_pixbuf` registers the pixbuf
                // as a scale-1 image — one image pixel per logical pixel — so a
                // preview rendered at the device pixel ratio would come out at
                // twice its intended size, and shrinking it back to logical
                // pixels to compensate is what made the drag image softer than
                // the in-window ghost: the compositor then scales those fewer
                // pixels back up. A surface that carries the scale keeps every
                // device pixel the frontend rendered: GDK sizes the icon window
                // from the surface's logical extents, which the device scale
                // divides, and paints the pattern one to one.
                let scale = window.scale_factor().max(1);
                if let Some(surface) =
                    GdkPixbufExt::create_surface(&icon, scale, window.window().as_ref())
                {
                    // The cursor lands on the surface's (0,0), so shifting the
                    // surface by the grab point — in device pixels, matching
                    // the bitmap's own pixel grid — puts the grabbed pixel, and
                    // only that pixel, under the pointer.
                    let (grab_x, grab_y) = match options.drag_image_offset {
                        Some(offset) => (offset.x, offset.y),
                        None => (0, 0),
                    };
                    surface.set_device_offset(-f64::from(grab_x), -f64::from(grab_y));
                    drag_context.drag_set_icon_surface(&surface);
                }
            }

            Ok(())
        } else {
            Err(crate::Error::FailedToStartDrag)
        }
    } else {
        Err(crate::Error::EmptyTargetList)
    }
}

fn image_binary_to_pixbuf(data: &[u8]) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader
        .write(data)
        .and_then(|_| loader.close())
        .map_err(|_| ())
        .and_then(|_| loader.pixbuf().ok_or(()))
        .ok()
}

fn clear_signal_handlers(window: &gtk::ApplicationWindow, handler_ids: &mut Vec<SignalHandlerId>) {
    for handler_id in handler_ids.drain(..) {
        window.disconnect(handler_id);
    }
}

fn on_drop_failed<F: Fn(DragResult, CursorPosition) + Send + 'static>(
    callback: Rc<F>,
    fired: Rc<Cell<bool>>,
    window: &gtk::ApplicationWindow,
    handler_ids: &Arc<Mutex<Vec<SignalHandlerId>>>,
    options: &Options,
    count_release_as_drop: bool,
) {
    log::debug!("Setting up drop failed handler");
    let window_clone = window.clone();
    let handler_ids_clone = handler_ids.clone();

    let skip_animatation_on_cancel_or_failure = options.skip_animatation_on_cancel_or_failure;

    handler_ids
        .lock()
        .unwrap()
        .push(window.connect_drag_failed(move |_, _, drag_result| {
            if fired.replace(true) {
                return Propagation::Proceed;
            }
            log::debug!("Drag failed or cancelled ({drag_result:?})");
            // A data drag has no target outside this application, so a plain
            // mouse release ends as GTK_DRAG_RESULT_NO_TARGET — and under
            // Wayland as GTK_DRAG_RESULT_ERROR, because the compositor reports
            // "the drag ended with nothing to receive it" as a protocol error
            // rather than as a missing target. An Escape keypress is the one
            // outcome that really is a cancel, and it arrives as
            // GTK_DRAG_RESULT_USER_CANCELLED.
            let released_by_mouse = count_release_as_drop
                && matches!(
                    drag_result,
                    gtk::DragResult::NoTarget | gtk::DragResult::Error
                );
            let result = if released_by_mouse {
                DragResult::Dropped
            } else {
                DragResult::Cancel
            };
            callback(result, get_cursor_position(&window_clone).unwrap());

            cleanup_signal_handlers(&handler_ids_clone, &window_clone);
            if skip_animatation_on_cancel_or_failure {
                Propagation::Stop
            } else {
                Propagation::Proceed
            }
        }));
}

fn on_drag_end<F: Fn(DragResult, CursorPosition) + Send + 'static>(
    callback: Rc<F>,
    fired: Rc<Cell<bool>>,
    window: &gtk::ApplicationWindow,
    handler_ids: &Arc<Mutex<Vec<SignalHandlerId>>>,
    count_release_as_drop: bool,
) {
    log::debug!("Setting up drag end handler");
    let window_clone = window.clone();
    let handler_ids_clone = handler_ids.clone();

    // `drag-end` is the one ending GTK guarantees the source widget, and for a
    // drop that never left the process it is the *only* one: `gtk_drag_finish`
    // reports a local drag straight back to the source, so no Xdnd traffic ever
    // arrives to raise `drop-performed`, and nothing raises `drag-failed` either
    // because the drop succeeded. Any release over a window of the dragging
    // application's own process — the window the drag came from above all —
    // would otherwise never end as far as the caller is concerned.
    //
    // Reaching this handler at all means neither of the two more specific
    // signals spoke first, and a cancel always speaks first (Escape arrives as
    // `drag-failed` with USER_CANCELLED). What is left is a mouse release, which
    // is exactly what `drag-failed` above counts a data drag as.
    handler_ids
        .lock()
        .unwrap()
        .push(window.connect_drag_end(move |_, context| {
            if fired.replace(true) {
                return;
            }
            log::debug!("Drag ended ({:?})", context.selected_action());
            let result = if count_release_as_drop {
                DragResult::Dropped
            } else {
                DragResult::Cancel
            };
            callback(result, get_cursor_position(&window_clone).unwrap());

            cleanup_signal_handlers(&handler_ids_clone, &window_clone);
        }));
}

fn cleanup_signal_handlers(
    handler_ids: &Arc<Mutex<Vec<SignalHandlerId>>>,
    window: &gtk::ApplicationWindow,
) {
    log::debug!("Cleaning up signal handlers");
    let handler_ids = &mut handler_ids.lock().unwrap();
    clear_signal_handlers(window, handler_ids);
    window.drag_source_unset();
    log::debug!("Signal handlers cleaned up");
}

fn on_drop_performed<F: Fn(DragResult, CursorPosition) + Send + 'static>(
    callback: Rc<F>,
    fired: Rc<Cell<bool>>,
    window: &gtk::ApplicationWindow,
    handler_ids: &Arc<Mutex<Vec<SignalHandlerId>>>,
    drag_context: &gdk::DragContext,
) {
    log::debug!("Setting up drop performed handler");
    let window = window.clone();
    let handler_ids = handler_ids.clone();

    drag_context.connect_drop_performed(move |context, _| {
        if fired.replace(true) {
            return;
        }
        log::debug!("Drop performed successfully");
        log::trace!("Selected action: {:?}", context.selected_action());
        log::trace!("Suggested action: {:?}", context.suggested_action());
        cleanup_signal_handlers(&handler_ids, &window);
        callback(DragResult::Dropped, get_cursor_position(&window).unwrap());
    });
}

fn get_cursor_position(window: &gtk::ApplicationWindow) -> Result<CursorPosition, Error> {
    if let Some(cursor) = window
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())
    {
        let (_, x, y) = cursor.position();
        Ok(CursorPosition { x, y })
    } else {
        Err(Error::FailedToGetCursorPosition)
    }
}
