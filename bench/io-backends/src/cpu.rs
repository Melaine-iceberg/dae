//! Process CPU time.
//!
//! Windows charges CPU time in whole clock ticks (15.625 ms by default), and
//! the accounting per *thread* can be attributed a tick that began before the
//! window being measured. A pass shorter than `ticks * threads` therefore
//! reports a CPU delta that is quantised at best and larger than the wall time
//! at worst, so callers must not publish it — see `CPU_REPORT_FLOOR_TICKS`.
//!
//! Wall time alone cannot tell "the completion runtime is faster" from "the
//! completion runtime is doing the same work on another core". A backend that
//! trades a lower wall time for several extra cores of spinning has not won
//! anything on a laptop, so every measurement carries the process's own
//! user+kernel time next to it.

/// The default Windows clock tick, in milliseconds. `GetProcessTimes` advances
/// in these units.
pub const TICK_MS: f64 = 15.625;

/// A pass must last at least this many ticks *per concurrent thread* before its
/// CPU delta means anything.
pub const CPU_REPORT_FLOOR_TICKS: f64 = 8.0;

/// Whether a pass of this duration, run at this concurrency, has a CPU delta
/// worth printing.
pub fn worth_reporting(wall_ms: f64, threads: usize) -> bool {
    wall_ms >= TICK_MS * CPU_REPORT_FLOOR_TICKS * threads.max(1) as f64
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuTime {
    pub user_ms: f64,
    pub kernel_ms: f64,
}

impl CpuTime {
    pub fn total_ms(&self) -> f64 {
        self.user_ms + self.kernel_ms
    }

    pub fn since(self, earlier: Self) -> Self {
        Self {
            user_ms: self.user_ms - earlier.user_ms,
            kernel_ms: self.kernel_ms - earlier.kernel_ms,
        }
    }
}

#[cfg(windows)]
pub fn process_cpu() -> Option<CpuTime> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ticks(value: FILETIME) -> f64 {
        let raw = ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64;
        // FILETIME counts 100 ns intervals.
        raw as f64 / 10_000.0
    }

    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: four valid writable FILETIME values and the pseudo-handle of the
    // current process. GetProcessTimes never fails for that handle.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }

    Some(CpuTime {
        user_ms: ticks(user),
        kernel_ms: ticks(kernel),
    })
}

#[cfg(not(windows))]
pub fn process_cpu() -> Option<CpuTime> {
    // `getrusage`/`clock_gettime` would both need a dependency or unsafe libc
    // calls that the report does not need: the numbers this harness is built to
    // produce are wall-clock comparisons.
    None
}
