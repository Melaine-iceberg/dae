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
//! Each platform has its own channel, and each backend module carries the
//! argument for its choice: GIO's `VolumeMonitor` on Linux, `NSWorkspace`'s
//! mount notifications on macOS, and the `WM_DEVICECHANGE` broadcast on
//! Windows. All three end in the same [`VolumesChanged`], which the frontend
//! answers by re-reading the disks on a short debounce — a single plug
//! produces several signals (drive, volume, mount) in a burst.

use serde::Serialize;
use specta::Type;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(target_os = "windows")]
use windows as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod backend {
    /// No device channel is wired up for this platform. Saying so explicitly is
    /// the difference between an app that keeps its poll-and-focus refresh and
    /// one that fails to build.
    pub(super) fn spawn(_app: &tauri::AppHandle) {}
}

/// Emitted for every device/mount change the OS reports. Carries nothing: the
/// frontend answers by re-reading `list_disks`, and the events arrive in
/// bursts that a debounce collapses into one read.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "volumes-changed")]
pub struct VolumesChanged;

/// Starts listening for volume, mount and drive changes. Intended to be called
/// once, from the app's setup hook, on the main thread: that is where GIO's
/// main context delivers on Linux and where AppKit's run loop lives on macOS.
/// What that costs differs per backend — Windows moves the listen onto a
/// thread of its own — and each backend's module says why.
pub fn spawn(app: &tauri::AppHandle) {
    backend::spawn(app);
}
