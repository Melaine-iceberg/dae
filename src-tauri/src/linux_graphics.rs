//! The one Linux graphics quirk this app answers for itself.
//!
//! On a Wayland session whose compositor implements `linux-drm-syncobj-v1`
//! (Mutter 46.1 and later, so GNOME; KWin) and whose EGL context lands on the
//! NVIDIA driver, a WebKitGTK window dies on its *first* frame, before any of
//! this app's code has run:
//!
//! ```text
//! Gdk-Message: 18:11:32.053: Error 71 (Protocol error) dispatching to Wayland display.
//! ```
//!
//! `71` is `EPROTO`, and it says only that the compositor hung up the
//! connection — the complaint itself is in the protocol trace, where the
//! compositor drops the client with `explicit sync is used, but no acquire
//! point is set` (or `unsupported_buffer`). GTK decides *per frame* whether to
//! draw with GL or into a shared-memory buffer, from whether the window already
//! has a paint GL context; WebKitGTK creates one only from inside the draw, so
//! the first frame is a shared-memory frame whose surface, by the time it is
//! committed, already carries an NVIDIA-armed explicit-sync object. A
//! shared-memory buffer cannot carry a sync point, and the compositor is
//! entitled to treat that as a protocol violation.
//!
//! Why this is a session-time decision rather than an unconditional one: the
//! affected set follows the protocol, not the desktop name. KWin advertises
//! `linux-drm-syncobj-v1` from 6.1 on and fails exactly the way GNOME does — a
//! gate narrowed to GNOME by name was tried and withdrawn when KDE reproduced
//! the crash — while a compositor that does not advertise the protocol has no
//! sync object to arm, and the variable is inert there. An X11 session fails
//! differently if it fails at all (a blank white webview, from the GBM buffer
//! path), so a `GDK_BACKEND` that sends GTK to X11 ends the question before the
//! compositor is even asked.
//!
//! The gate is therefore the defect's own condition, asked of the compositor
//! directly: [`explicit_sync_is_advertised`] opens a second, short-lived
//! Wayland connection ahead of GTK's, walks the registry for the
//! [`EXPLICIT_SYNC_GLOBAL`] global, and hangs up. A session the probe
//! cannot ask — no compositor, a refusal, an error — is left alone, and its
//! user keeps the three variables [`already_answered`] honors.
//!
//! `__NV_DISABLE_EXPLICIT_SYNC=1` turns off the NVIDIA half of the pair and
//! leaves the DMA-BUF renderer — the fast path — in place, which is the trade
//! upstream recommends trying first. The blunter options exist and are left to
//! the user: `WEBKIT_DISABLE_DMABUF_RENDERER=1` gives up hardware-accelerated
//! compositing in every window, and `WEBKIT_DISABLE_COMPOSITING_MODE=1` is a
//! last resort that would additionally cost the terminal its WebGL renderer
//! (`@xterm/addon-webgl`). All three are recognized by
//! [`already_answered`], so a user who set any of them gets their choice.
//!
//! This is the same defect documented as tauri-apps/tauri#9394 and
//! webkit.org/b/280210, and it is upstream's to fix. What is ours is that a
//! file manager should not need its user to know any of the above.

use std::sync::atomic::{AtomicBool, Ordering};

/// The NVIDIA EGL platform switch. Undocumented but long-standing, and read by
/// the driver when the EGL display is initialized.
const NVIDIA_EXPLICIT_SYNC: &str = "__NV_DISABLE_EXPLICIT_SYNC";

/// PCI vendor id of the only vendor this workaround applies to, as
/// `/sys/class/drm/card*/device/vendor` spells it.
const NVIDIA_VENDOR_ID: &str = "0x10de";

/// The registry global that says the compositor speaks the protocol. The
/// protocol is named `linux-drm-syncobj-v1`, but a staging protocol carries the
/// `wp_` namespace, and it is the prefixed spelling the registry lists.
const EXPLICIT_SYNC_GLOBAL: &str = "wp_linux_drm_syncobj_manager_v1";

/// The routes a user, a packager, or a future version of this module may have
/// already taken. Any of them means the question is settled.
const ANSWERED_ELSEWHERE: [&str; 3] = [
    NVIDIA_EXPLICIT_SYNC,
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
];

/// Whether [`apply`] changed the environment, so the startup log line can say
/// so. Recorded here rather than logged here because the log file's target is
/// not open until the plugins' setup hooks run — see `lib.rs`.
static APPLIED: AtomicBool = AtomicBool::new(false);

/// Turns off the NVIDIA half of Wayland explicit sync for this process, if this
/// is a session that needs it.
///
/// Call this as the first statement of the entry point. The graphics stack
/// reads these variables once, when it initializes, and a variable set after
/// GTK has loaded is a variable that had no effect.
pub fn apply() {
    if !wayland_is_the_backend() || !nvidia_is_present() || already_answered() {
        return;
    }

    // A statement of its own, and host-gated: only the probe speaks Wayland,
    // while the rest of the gate stays host-independent so its tests run
    // everywhere the module does.
    #[cfg(target_os = "linux")]
    if !explicit_sync_is_advertised() {
        return;
    }

    // SAFETY: `set_var` is unsafe in edition 2024 because it mutates the
    // process environment while another thread could be reading it. This is the
    // first statement of `run`, which `main` calls directly and which has
    // spawned nothing yet, so there is exactly one thread. Moving the call any
    // later — into `run`'s setup hook, say, where the plugins have started
    // their watchers — would make it unsound, and setting a variable after GTK
    // has initialized would make it useless anyway.
    unsafe {
        std::env::set_var(NVIDIA_EXPLICIT_SYNC, "1");
    }
    APPLIED.store(true, Ordering::Relaxed);
}

/// Whether this process is running under [`apply`]'s workaround.
pub fn applied() -> bool {
    APPLIED.load(Ordering::Relaxed)
}

/// Whether GDK will talk to Wayland, which is what raises the error — a session
/// that has a `WAYLAND_DISPLAY` but was sent to X11 by `GDK_BACKEND` is not
/// affected, and `GDK_BACKEND=x11` is itself a documented workaround.
///
/// `pub(crate)` because the tab drag has the same question to answer: a Wayland
/// session is the one that exposes no global pointer or window position, which
/// is decided in `tab_windows::tab_drag_uses_frontend_bounds`.
pub(crate) fn wayland_is_the_backend() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return false;
    }
    match std::env::var("GDK_BACKEND") {
        Ok(backends) => names_wayland(&backends),
        Err(_) => true,
    }
}

/// Whether a `GDK_BACKEND` list names Wayland. GTK3 separates the list with
/// commas, GTK2 with colons, and both spellings turn up in the wild.
fn names_wayland(backends: &str) -> bool {
    backends.split([',', ':']).any(|name| name.trim() == "wayland")
}

/// Whether any DRM card on this machine is driven by NVIDIA.
///
/// Deliberately about the kernel module rather than the `boot_vga` flag: which
/// card owns the firmware's framebuffer is a hybrid laptop's MUX setting, while
/// what matters here is whether an NVIDIA EGL context is reachable at all. The
/// variable is inert on the Mesa cards it does not apply to, so a false
/// positive costs nothing and a false negative costs the crash.
fn nvidia_is_present() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            // `card0` and `card1` are the cards. `card0-DP-1` is a connector
            // and `renderD128` a render node; neither can change the answer,
            // and both would only re-read a card's vendor file.
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("card") && !name.contains('-')
        })
        .any(|entry| {
            std::fs::read_to_string(entry.path().join("device/vendor"))
                .is_ok_and(|vendor| vendor.trim() == NVIDIA_VENDOR_ID)
        })
}

/// Whether anything has already decided how to render this session.
fn already_answered() -> bool {
    ANSWERED_ELSEWHERE
        .iter()
        .any(|name| std::env::var_os(name).is_some())
}

/// Whether the compositor advertises `linux-drm-syncobj-v1`, the protocol that
/// carries the NVIDIA driver's sync objects — the half of the pair that can be
/// asked about this early, before an EGL context exists to hold the other.
///
/// A registry walk on a connection of the module's own: `apply` runs before
/// GTK opens the session's real connection, and a second connection to the
/// same compositor is the only way to have the answer in time. Nothing is
/// bound, and nothing is kept — the walk lasts one roundtrip, and the socket
/// is dropped on the way out.
#[cfg(target_os = "linux")]
fn explicit_sync_is_advertised() -> bool {
    use wayland_client::protocol::wl_registry;
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// The registry walk, reduced to its one question.
    struct Walk {
        advertised: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Walk {
        fn event(
            state: &mut Self,
            _registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _data: &(),
            _connection: &Connection,
            _queue: &QueueHandle<Self>,
        ) {
            let wl_registry::Event::Global { interface, .. } = event else {
                return;
            };
            if interface == EXPLICIT_SYNC_GLOBAL {
                state.advertised = true;
            }
        }
    }

    let Ok(connection) = Connection::connect_to_env() else {
        return false;
    };
    let mut queue = connection.new_event_queue();
    let _registry = connection.display().get_registry(&queue.handle(), ());
    let mut walk = Walk { advertised: false };
    let answered = queue.roundtrip(&mut walk).is_ok();
    walk.advertised && answered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of asking: `GDK_BACKEND=x11` under a Wayland session is
    /// a session this workaround must leave alone, because the shell that set
    /// it did so to be rid of exactly this class of problem.
    #[test]
    fn only_a_wayland_backend_counts() {
        assert!(names_wayland("wayland"));
        assert!(names_wayland("wayland,x11"));
        assert!(names_wayland("x11:wayland"));
        assert!(!names_wayland("x11"));
        assert!(!names_wayland(""));
    }

    /// Read by the driver as a string, so the spelling is load-bearing.
    #[test]
    fn the_vendor_id_has_the_shape_sysfs_prints() {
        assert_eq!(NVIDIA_VENDOR_ID, "0x10de");
    }
}
