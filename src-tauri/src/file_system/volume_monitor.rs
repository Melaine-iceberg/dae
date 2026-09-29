//! OS-level device observation for the sidebar's disk list.
//!
//! `list_disks` answers "what is mounted right now", but nothing tells the
//! sidebar *when* that answer changed while it is on screen: a USB stick
//! plugged in, a phone connected, a share unmounted. Polling alone is what the
//! OS file managers avoid — Explorer, nautilus and Dolphin all sit on a device
//! notification channel and re-read their volume list when it speaks — and it
//! is what makes a 60-second poll visible as "the drive appeared a minute
//! late".
//!
//! On Linux that channel is GIO's `VolumeMonitor`, the same object GTK's file
//! chooser and nautilus listen to: it reports volumes (mountable things),
//! mounts (mounted filesystems) and drives (the physical device) as the
//! desktop's udisks2/gvfs stack sees them, which is exactly the set the sidebar
//! lists — it enumerates through the same GIO layer.
//!
//! The monitor is created once, on the GTK main thread during Tauri's setup
//! (`setup` runs inside the running main loop), and its signals are delivered
//! on that same main context from then on. Each signal emits [`VolumesChanged`]
//! to the frontend, which re-lists the disks on a short debounce — a single
//! plug produces several signals (drive, volume, mount) in a burst.

use serde::Serialize;
use specta::Type;

/// Emitted for every device/mount change the OS reports. Carries nothing: the
/// frontend answers by re-reading `list_disks`, and the events arrive in
/// bursts that a debounce collapses into one read.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "volumes-changed")]
pub struct VolumesChanged;

/// Starts listening for volume, mount and drive changes. Idempotent at the
/// OS level (GIO hands out one shared monitor), but intended to be called once
/// from the app's setup hook.
#[cfg(target_os = "linux")]
pub fn spawn(app: &tauri::AppHandle) {
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

/// Other platforms still learn about device changes on their existing poll /
/// focus-refresh schedule; the event seam is in place for their native
/// channels, which are a follow-up rather than a rewrite:
///
/// - Windows: `WM_DEVICECHANGE` (`DBT_DEVICEARRIVAL`/`DBT_DEVICEREMOVECOMPLETE`)
///   on the window proc, or `RegisterDeviceNotification`.
/// - macOS: `NSWorkspace` mount/unmount notifications (`didMountNotification`,
///   `didUnmountNotification`).
#[cfg(not(target_os = "linux"))]
pub fn spawn(_app: &tauri::AppHandle) {}
