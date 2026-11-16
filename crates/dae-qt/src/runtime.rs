//! One Tokio runtime for the shell.
//!
//! `dae_core` hands its long operations to `tokio::task::spawn_blocking`, so a
//! caller has to be inside a reactor. The Tauri shell inherits Tauri's; the Qt
//! shell starts its own once and shares it between the commands and the
//! sidebar, which keeps the thread pool bounded by one configuration rather
//! than one per QObject.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

pub(crate) fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("dae-io")
            .build()
            .expect("failed to create the dae I/O runtime")
    })
}

/// Runs `future` on the shared runtime. Errors are the future's business; this
/// helper exists so no caller has to unwrap the runtime handle.
pub(crate) fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}

/// The lane speculative work runs on: preview preloads today. Its own worker
/// and blocking pool, because `spawn_blocking` inside a future lands on the
/// *current* runtime's pool — sharing the interactive one would let a
/// speculative read take a thread a selection or a listing is waiting on.
/// One worker serializes the speculation; two blocking threads let a slow
/// read overlap with the next file's stat without ever scaling up.
pub(crate) fn spawn_preload<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    static PRELOAD: OnceLock<Runtime> = OnceLock::new();
    PRELOAD
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(1)
                .max_blocking_threads(2)
                .enable_all()
                .thread_name("dae-preload")
                .build()
                .expect("failed to create the dae preload runtime")
        })
        .spawn(future);
}
