//! macOS' channel: `NSWorkspace`'s mount notifications.
//!
//! NSWorkspace is the AppKit object that owns the desktop's view of the
//! filesystem — the Finder itself is one of its clients — and it posts three
//! notifications when the mounted set changes: a volume was mounted (a USB
//! stick, a disk image, an install volume), a volume was unmounted (eject), or
//! a volume was renamed. Those are the three ways the sidebar's list can go
//! stale between polls.
//!
//! Two details of the API shape the code below:
//!
//!   * The notifications go to the workspace's *own* notification center
//!     (`NSWorkspace.notificationCenter`), not to `NotificationCenter.default`.
//!     Subscribing to the default center is the mistake that compiles, runs
//!     and silently never fires.
//!   * Observers are registered with a block rather than a selector, because a
//!     Rust closure is the block: `block2::RcBlock` wraps it, the center copies
//!     it, and this module never hears about it again — the observers are
//!     deliberately never removed, since the process exit is what ends them,
//!     matching the other backends' watchers.

use super::VolumesChanged;
use block2::{DynBlock, RcBlock};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidMountNotification, NSWorkspaceDidRenameVolumeNotification,
    NSWorkspaceDidUnmountNotification,
};
use objc2_foundation::{NSNotification, NSNotificationName};
use std::ptr::NonNull;

pub(super) fn spawn(app: &tauri::AppHandle) {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();

    // SAFETY: the three names are the framework's own notification name
    // constants, live for the process by construction.
    let names: [&NSNotificationName; 3] = unsafe {
        [
            NSWorkspaceDidMountNotification,
            NSWorkspaceDidUnmountNotification,
            NSWorkspaceDidRenameVolumeNotification,
        ]
    };

    for name in names {
        let app = app.clone();
        let on_change = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            let _ = tauri_specta::Event::emit(&VolumesChanged, &app);
        });
        // The annotation is what pins the closure's signature into the
        // `dyn Fn` type the method takes; inference alone leaves the block's
        // dynamic type open, and the call then does not resolve.
        let on_change: &DynBlock<dyn Fn(NonNull<NSNotification>) + 'static> = &on_change;

        // SAFETY: `name` is a valid notification name and the block is a valid
        // block for its `dyn Fn` shape; `None` for object and queue is the
        // documented "every object, delivered on the posting thread" — which
        // for a workspace notification is the main thread. The token is only
        // needed to remove the observer and is dropped on purpose: nothing here
        // ever removes it, and the center holds the registration itself.
        let _ = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, on_change)
        };
    }
}
