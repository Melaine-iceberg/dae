//! The fixed-size pool that renders thumbnails and extracts icons.
//!
//! These jobs are blocking by nature — an image decode, a shell COM round-trip,
//! a `.thumbnailer` spawn — and must not run on the Node event loop. They do not
//! use `tokio::task::spawn_blocking` either: directory listings share that pool,
//! and a scrolling photo folder would fill it with decodes and delay the
//! listing the user is waiting on. Three threads with a plain channel bound the
//! concurrency exactly as the Tauri shell's render pool did.
//!
//! On Windows each worker enters a single-threaded apartment and pumps its
//! message queue between jobs. Shell thumbnail providers can call back into an
//! object living in our apartment; an STA that never dispatches those messages
//! deadlocks the provider.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

/// Renders allowed at once. A decode holds up to
/// `THUMBNAIL_MAX_DECODED_PIXELS * 4` bytes, so this bounds peak pixel memory
/// while keeping a fast scroll fed.
const WORKERS: usize = 3;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The one queue, drained by [`WORKERS`] threads.
///
/// The receiver is shared behind a mutex because `mpsc` has no multi-consumer
/// receiver; the guard is held only for the blocking `recv`, so a busy worker
/// never keeps a peer from picking up work.
fn queue() -> &'static Sender<Job> {
    static QUEUE: OnceLock<Sender<Job>> = OnceLock::new();

    QUEUE.get_or_init(|| {
        let (sender, receiver) = channel::<Job>();
        let receiver = std::sync::Arc::new(Mutex::new(receiver));

        for index in 0..WORKERS {
            let receiver = std::sync::Arc::clone(&receiver);
            let worker = std::thread::Builder::new()
                .name(format!("dae-render-{index}"))
                .spawn(move || work(&receiver));

            if let Err(error) = worker {
                log::warn!("Unable to start render worker {index}: {error}");
            }
        }

        sender
    })
}

/// Queues one job. The returned receiver is awaited by the N-API promise.
pub fn submit(job: Job) {
    if let Err(error) = queue().send(job) {
        // The pool is gone (shutdown, or every worker failed to start). Running
        // inline keeps the request from being answered with silence; it is the
        // only path where a caller pays for the render on its own thread.
        run_inline(error.0);
    }
}

fn run_inline(job: Job) {
    enter_apartment();
    run_job(job);
    pump_apartment();
}

fn work(receiver: &Mutex<Receiver<Job>>) {
    enter_apartment();

    loop {
        // Scoped so the queue lock is not held while a render runs.
        let job = receiver
            .lock()
            .expect("the render queue receiver is never poisoned")
            .recv();

        let Ok(job) = job else {
            // Every sender is gone: the process is shutting down.
            return;
        };

        run_job(job);
        pump_apartment();
    }
}

fn run_job(job: Job) {
    // A panicking decoder must not take a pool thread down with it. Release
    // builds abort on panic anyway; this is what keeps the failure contained
    // in dev and in tests.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
}

/// Puts a worker into a COM single-threaded apartment.
///
/// The shell paths reach `IShellItemImageFactory` and the registered thumbnail
/// providers behind it, and those are apartment-bound: a fresh thread has no
/// apartment to pin them to. The apartment belongs to the thread, which lives as
/// long as the process does, so the matching `CoUninitialize` would never be
/// reached anyway.
#[cfg(windows)]
fn enter_apartment() {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::core::HRESULT;

    let result: HRESULT = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.0 < 0 {
        log::warn!(
            "Unable to initialize COM on a render worker: HRESULT {:#010x}",
            result.0
        );
    }
}

#[cfg(not(windows))]
fn enter_apartment() {}

/// Dispatches messages queued for a worker's apartment.
#[cfg(windows)]
fn pump_apartment() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(not(windows))]
fn pump_apartment() {}
