//! Pause, cancellation, and rate limiting for the long transfers.
//!
//! A copy owns one [`TransferControl`] for its whole life, registered under the
//! operation id the shell already hands it. The engine consults it at the two
//! places it is safe to stop — between stream chunks and between tree entries —
//! which is what makes "pause" mean "stop moving bytes" rather than "ask the
//! operation to look at a flag one day".
//!
//! # Why the gate is a condition variable and not an atomic
//!
//! A paused transfer must not burn a core in a spin loop, and it must still
//! answer `cancel` immediately: with four network streams paused, polling every
//! few milliseconds would be four wakeups per interval for no reason. The
//! condvar wakes exactly the threads that are waiting, and `resume`/`cancel`
//! notify all of them.
//!
//! # Why the limit is a bucket and not a per-chunk sleep
//!
//! Sleeping `chunk / rate` after every chunk turns a 20 MB/s limit into
//! `chunk_size / latency` whenever the backend answers slowly, because the sleep
//! is paid on top of the transfer instead of against it. The bucket measures
//! bytes against wall-clock time, so a slow link simply never reaches the
//! limit, and a fast one is held exactly at it.

use crate::error::FileSystemError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Longest a throttled copy sleeps before it re-checks pause and cancellation.
/// A pause must feel immediate even while a limit is holding the transfer back.
const THROTTLE_SLICE: Duration = Duration::from_millis(50);

/// How much of the limit may be spent in one burst, in seconds. A tenth of a
/// second keeps a bucket from turning a stalled read into a visible spike while
/// still letting a 256 KiB chunk through without an extra round of sleeping.
const BURST_SECONDS: f64 = 0.1;

/// The live rate limit, in bytes per second; `0` is "no limit".
///
/// It is a process-wide value rather than a per-transfer argument because the
/// user sets it once ("my uplink is 10 Mbit") and expects every transfer to obey
/// it, including the ones already running. Changing it wakes nothing: the next
/// chunk simply measures against the new number.
static GLOBAL_RATE: AtomicU64 = AtomicU64::new(0);

/// One entry per operation currently moving bytes.
static CONTROLS: LazyLock<Mutex<HashMap<String, Arc<TransferControl>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Sets the process-wide limit. `None` or `Some(0)` clears it.
///
/// Running transfers pick the change up on their next chunk, which is what makes
/// this usable from a menu without restarting anything.
pub fn set_global_rate_limit(bytes_per_second: Option<u64>) {
    let rate = bytes_per_second.unwrap_or(0);
    GLOBAL_RATE.store(rate, Ordering::Relaxed);

    for control in CONTROLS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
    {
        control.set_rate(if rate == 0 { None } else { Some(rate) });
    }
}

/// The process-wide limit, `None` when unlimited.
pub fn global_rate_limit() -> Option<u64> {
    match GLOBAL_RATE.load(Ordering::Relaxed) {
        0 => None,
        rate => Some(rate),
    }
}

/// Registers a new control for `operation_id`, replacing any stale entry with
/// the same id (operation ids are monotonic, so a collision means a bug, and
/// replacing keeps the newer transfer controllable either way).
pub fn register(operation_id: &str) -> Arc<TransferControl> {
    let control = Arc::new(TransferControl::new(global_rate_limit()));
    CONTROLS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(operation_id.to_owned(), Arc::clone(&control));
    control
}

/// The control for `operation_id`, if the operation is still running.
pub fn control_for(operation_id: &str) -> Option<Arc<TransferControl>> {
    CONTROLS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(operation_id)
        .cloned()
}

/// Drops the registry entry. Called when the operation returns, whatever it
/// returned: a finished transfer cannot be paused.
pub fn finish(operation_id: &str) {
    CONTROLS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(operation_id);
}

/// Removes `operation_id` from the registry when it goes out of scope, so an
/// early `?` inside a worker cannot leave a control behind for the UI to find.
pub struct ControlRegistration {
    operation_id: String,
}

impl ControlRegistration {
    pub fn new(operation_id: &str) -> (Self, Arc<TransferControl>) {
        let control = register(operation_id);
        (
            Self {
                operation_id: operation_id.to_owned(),
            },
            control,
        )
    }
}

impl Drop for ControlRegistration {
    fn drop(&mut self) {
        finish(&self.operation_id);
    }
}

struct Inner {
    paused: bool,
    cancelled: bool,
    rate: Option<u64>,
    /// Tokens available right now, in bytes.
    tokens: f64,
    /// When `tokens` was last refilled.
    last_refill: Instant,
}

/// Pause / cancellation / rate-limit state for one long operation.
pub struct TransferControl {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl TransferControl {
    pub fn new(rate: Option<u64>) -> Self {
        let rate = rate.filter(|rate| *rate > 0);
        Self {
            inner: Mutex::new(Inner {
                paused: false,
                cancelled: false,
                rate,
                // Start empty: the first chunk is paid for rather than granted,
                // so a limit applies from the first byte instead of the second.
                tokens: 0.0,
                last_refill: Instant::now(),
            }),
            wake: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn pause(&self) {
        self.lock().paused = true;
    }

    pub fn resume(&self) {
        {
            let mut inner = self.lock();
            inner.paused = false;
            // A pause is not free time: the bucket is refilled from now, so
            // resuming does not release a burst worth of the whole pause.
            inner.last_refill = Instant::now();
        }
        self.wake.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    /// Stops the operation at its next checkpoint. Idempotent.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
        // A cancelled transfer must not stay blocked on a pause it can never
        // leave: the gate re-checks cancellation after every wakeup.
        self.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    pub fn set_rate(&self, rate: Option<u64>) {
        let mut inner = self.lock();
        let rate = rate.filter(|rate| *rate > 0);
        if inner.rate != rate {
            inner.rate = rate;
            inner.tokens = 0.0;
            inner.last_refill = Instant::now();
        }
    }

    /// The current limit, `None` when unlimited.
    pub fn rate_limit(&self) -> Option<u64> {
        self.lock().rate
    }

    /// Blocks while the operation is paused and fails once it is cancelled.
    ///
    /// Every checkpoint in the engine goes through this, so a paused transfer
    /// holds its threads here and moves no bytes.
    pub fn gate(&self) -> Result<(), FileSystemError> {
        let mut inner = self.lock();

        loop {
            if inner.cancelled {
                return Err(FileSystemError::Cancelled(
                    "The transfer was cancelled".to_owned(),
                ));
            }
            if !inner.paused {
                return Ok(());
            }

            inner = self
                .wake
                .wait(inner)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Records `bytes` as moved and sleeps out whatever the limit has not paid
    /// for yet. Returns early — with an error — when the operation is cancelled
    /// while it waits.
    pub fn advance(&self, bytes: u64) -> Result<(), FileSystemError> {
        self.gate()?;

        let delay = {
            let mut inner = self.lock();
            let Some(rate) = inner.rate else {
                return Ok(());
            };

            inner.refill(rate, Instant::now());

            if inner.tokens >= bytes as f64 {
                inner.tokens -= bytes as f64;
                return Ok(());
            }

            let owed = bytes as f64 - inner.tokens;
            inner.tokens = 0.0;
            // A rate low enough to make one chunk take minutes is legitimate
            // (dial-up over a metered link); the wait is sliced by `wait_for`.
            Some(Duration::from_secs_f64(owed / rate as f64))
        };

        if let Some(delay) = delay {
            self.wait_for(delay)?;

            // The wait refilled the bucket; take the bytes that were owed out
            // of it so the next chunk starts from zero again.
            let mut inner = self.lock();
            if let Some(rate) = inner.rate {
                inner.refill(rate, Instant::now());
                inner.tokens = (inner.tokens - bytes as f64).max(0.0);
            }
        }

        Ok(())
    }

    /// Sleeps in slices so a pause or a cancel lands within [`THROTTLE_SLICE`]
    /// instead of at the end of a long wait.
    fn wait_for(&self, delay: Duration) -> Result<(), FileSystemError> {
        let deadline = Instant::now() + delay;

        loop {
            self.gate()?;

            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }

            std::thread::sleep((deadline - now).min(THROTTLE_SLICE));
        }
    }
}

impl Inner {
    /// Adds the tokens earned since the last call, capped at one burst.
    fn refill(&mut self, rate: u64, now: Instant) {
        let earned = now.saturating_duration_since(self.last_refill).as_secs_f64() * rate as f64;
        let capacity = (rate as f64 * BURST_SECONDS).max(1.0);
        self.tokens = (self.tokens + earned).min(capacity);
        self.last_refill = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elapsed_since(start: Instant) -> Duration {
        start.elapsed()
    }

    #[test]
    fn a_gate_blocks_while_paused_and_opens_on_resume() {
        let control = Arc::new(TransferControl::new(None));
        control.pause();

        let worker = Arc::clone(&control);
        let handle = std::thread::spawn(move || worker.gate().is_ok());

        // The thread cannot pass the gate while the pause holds.
        std::thread::sleep(Duration::from_millis(60));
        assert!(!handle.is_finished(), "the gate let a paused transfer through");

        control.resume();
        assert!(handle.join().expect("gate thread"), "resume did not open it");
    }

    #[test]
    fn cancelling_a_paused_gate_returns_an_error_instead_of_hanging() {
        let control = Arc::new(TransferControl::new(None));
        control.pause();

        let worker = Arc::clone(&control);
        let handle = std::thread::spawn(move || worker.gate());

        std::thread::sleep(Duration::from_millis(30));
        control.cancel();

        let outcome = handle.join().expect("gate thread");
        assert!(
            matches!(outcome, Err(FileSystemError::Cancelled(_))),
            "expected a cancellation, got {outcome:?}"
        );
    }

    #[test]
    fn an_unlimited_transfer_pays_no_sleep() {
        let control = TransferControl::new(None);
        let start = Instant::now();

        for _ in 0..64 {
            control.advance(256 * 1024).expect("unlimited advance");
        }

        assert!(
            elapsed_since(start) < Duration::from_millis(50),
            "an unlimited transfer waited: {:?}",
            elapsed_since(start)
        );
    }

    #[test]
    fn a_limit_holds_the_transfer_to_the_budget() {
        // 1 MB/s. One chunk beyond the burst allowance must cost about a
        // second's worth of waiting at that rate, measured generously.
        let control = TransferControl::new(Some(1024 * 1024));
        let chunk = 512 * 1024;

        let start = Instant::now();
        // The first chunk fits inside the burst (100 ms at 1 MB/s is 102 KiB,
        // so it does not) — either way the pair below must cost at least the
        // time the limit owes for one chunk.
        control.advance(chunk).expect("first chunk");
        control.advance(chunk).expect("second chunk");

        let elapsed = elapsed_since(start);
        assert!(
            elapsed >= Duration::from_millis(350),
            "two 512 KiB chunks at 1 MB/s finished in {elapsed:?}"
        );
    }

    #[test]
    fn a_cancelled_transfer_stops_advancing() {
        let control = TransferControl::new(None);
        control.cancel();

        let outcome = control.advance(1);
        assert!(
            matches!(outcome, Err(FileSystemError::Cancelled(_))),
            "expected a cancellation, got {outcome:?}"
        );
    }

    #[test]
    fn the_registry_hands_out_one_control_per_operation() {
        let (registration, control) = ControlRegistration::new("op-registry-test");
        assert!(control_for("op-registry-test").is_some());
        assert!(control_for("op-registry-test").is_some_and(|found| Arc::ptr_eq(&found, &control)));

        drop(registration);
        assert!(
            control_for("op-registry-test").is_none(),
            "the control outlived its operation"
        );
    }

    #[test]
    fn a_global_limit_reaches_the_running_transfer() {
        let (registration, control) = ControlRegistration::new("op-global-rate-test");
        assert_eq!(control.rate_limit(), None);

        set_global_rate_limit(Some(4 * 1024 * 1024));
        assert_eq!(control.rate_limit(), Some(4 * 1024 * 1024));

        set_global_rate_limit(None);
        assert_eq!(control.rate_limit(), None);

        drop(registration);
    }
}
