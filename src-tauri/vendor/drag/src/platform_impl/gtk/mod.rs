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
        DeviceExt, DragContextExtManual, PixbufLoaderExt, SeatExt, WidgetExt, WidgetExtManual,
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
            // drag-failed and drop-performed are mutually exclusive for one
            // drag, but guard against a double delivery anyway: callers hand
            // the outcome to a one-shot channel.
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

            log::debug!("Setting up drag icon");
            let icon_pixbuf: Option<gdk_pixbuf::Pixbuf> = match &image {
                Image::Raw(data) => image_binary_to_pixbuf(data),
                Image::File(path) => match std::fs::read(path) {
                    Ok(bytes) => image_binary_to_pixbuf(&bytes),
                    Err(_) => None,
                },
            };
            if let Some(icon) = icon_pixbuf {
                // GDK positions the drag icon in logical pixels while preview
                // images are rendered at the device pixel ratio, so both the
                // icon and the grab offset shrink by the window's scale.
                let scale = f64::from(window.scale_factor().max(1));
                let icon = if scale != 1.0 {
                    icon.scale_simple(
                        (f64::from(icon.width()) / scale).round() as i32,
                        (f64::from(icon.height()) / scale).round() as i32,
                        gdk_pixbuf::InterpType::Bilinear,
                    )
                    .unwrap_or(icon)
                } else {
                    icon
                };
                let (hot_x, hot_y) = match options.drag_image_offset {
                    Some(offset) => (
                        (f64::from(offset.x) / scale).round() as i32,
                        (f64::from(offset.y) / scale).round() as i32,
                    ),
                    None => (0, 0),
                };
                drag_context.drag_set_icon_pixbuf(&icon, hot_x, hot_y);
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
            // A dummy data drag has no targets, so a plain mouse release also
            // ends as GTK_DRAG_RESULT_NO_TARGET; only an Escape keypress (or
            // another genuine failure) is a real cancel.
            let released_by_mouse =
                count_release_as_drop && drag_result == gtk::DragResult::NoTarget;
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
