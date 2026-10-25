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
