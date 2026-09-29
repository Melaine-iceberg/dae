//! Linux' channel: GIO's `VolumeMonitor`, the same object GTK's file chooser
//! and nautilus listen to. It reports volumes (mountable things), mounts
//! (mounted filesystems) and drives (the physical device) as the desktop's
//! udisks2/gvfs stack sees them, which is exactly the set the sidebar lists —
//! it enumerates through the same GIO layer.
//!
//! The monitor is created once, on the GTK main thread during Tauri's setup
//! (`setup` runs inside the running main loop), and its signals are delivered
//! on that same main context from then on — which is why this backend attaches
//! where [`super::spawn`] runs instead of on a thread of its own: a thread
//! with no GLib main loop would never hear the signals.

use super::VolumesChanged;

pub(super) fn spawn(app: &tauri::AppHandle) {
    use gio::prelude::*;

    let monitor = gio::VolumeMonitor::get();

    // One closure per signal argument type, cloneable so one definition can
    // serve its pair of connected/disconnected (or added/removed) signals.
    //
    // The emit is spelt out as `tauri_specta::Event::emit`: `gio::prelude`
    // brings GLib's own `ObjectExt::emit` into scope, and a method call would
    // resolve against that instead.
    let on_volume = {
        let app = app.clone();
        move |_: &gio::VolumeMonitor, _: &gio::Volume| {
            let _ = tauri_specta::Event::emit(&VolumesChanged, &app);
        }
    };
    let on_mount = {
        let app = app.clone();
        move |_: &gio::VolumeMonitor, _: &gio::Mount| {
            let _ = tauri_specta::Event::emit(&VolumesChanged, &app);
        }
    };
    let on_drive = {
        let app = app.clone();
        move |_: &gio::VolumeMonitor, _: &gio::Drive| {
            let _ = tauri_specta::Event::emit(&VolumesChanged, &app);
        }
    };

    monitor.connect_volume_added(on_volume.clone());
    monitor.connect_volume_removed(on_volume);
    monitor.connect_mount_added(on_mount.clone());
    monitor.connect_mount_removed(on_mount);
    monitor.connect_drive_connected(on_drive.clone());
    monitor.connect_drive_disconnected(on_drive);
}
