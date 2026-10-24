//! Instrumentation for the "drag released → new window shows content" path of a
//! torn-off tab.
//!
//! Tear-off crosses a process boundary: the drag ends in the source window's
//! Rust side, and the content appears in a webview that does not exist yet. A
//! single timeline therefore needs a shared clock, which is what this module
//! provides: [`mark_release`] stamps the moment the drag was released, the new
//! window reads that stamp through an injected `initialization_script`, and
//! reports its own marks back as offsets from it.
//!
//! Inert unless `DAE_TAB_PERF=1`. Nothing here changes behaviour when off: every
//! entry point returns immediately, and no anchor is injected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Epoch millis at which the most recent native tab drag was released.
///
/// Atomic because the release is observed on the drag worker while the window
/// born from it reads the value on the main thread. A single slot is enough: the
/// UI allows one tab drag at a time.
static LAST_RELEASE_MS: AtomicU64 = AtomicU64::new(0);

/// A release stamp older than this is a leftover from a run that never
/// reported, and is replaced instead of reused.
const RELEASE_STALE_MS: f64 = 10_000.0;

thread_local! {
    /// Set on the window this process created, so marks are filed under the
    /// label the frontend will report with.
    static CURRENT_LABEL: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

fn store() -> &'static Mutex<HashMap<String, Vec<(String, f64)>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<(String, f64)>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether this run was asked for a profile.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| match std::env::var("DAE_TAB_PERF") {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    })
}

/// Epoch millis, the unit every mark is expressed in.
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Stamps the drag release. Returns the stamp so the caller can log it.
pub fn mark_release() -> f64 {
    let now = now_ms();
    // Keep the first stamp of a tear-off that is still in flight: the native
    // drag ends a hair before `tear_off_tab` runs, and that earlier instant is
    // the one the user actually released on. Re-stamping would silently shift
    // the zero point forward and make everything look faster.
    let previous = LAST_RELEASE_MS.load(Ordering::SeqCst) as f64;
    if previous > 0.0 && now - previous <= RELEASE_STALE_MS {
        return previous;
    }
    LAST_RELEASE_MS.store(now as u64, Ordering::SeqCst);
    now
}

/// Sets the release stamp for a tear-off that has just been resolved.
///
/// Unlike [`mark_release`] this always writes: the caller holds the drag's own
/// end instant, and a stamp left behind by an earlier drag must not win over it.
/// It is not gated on [`enabled`] either - one atomic store per drag is not
/// worth a branch, and an untestable function is worth less than that.
pub fn stamp_release(at: f64) {
    LAST_RELEASE_MS.store(at as u64, Ordering::SeqCst);
}

/// Forgets the release stamp, for a drag that ended as anything but a tear-off.
pub fn clear_release() {
    LAST_RELEASE_MS.store(0, Ordering::SeqCst);
}

/// Whether a resolved drag ends in a tear-off, and so starts a timeline.
///
/// Mirrors the frontend's three-way split in `explorer-tabs.tsx`: a drag not
/// released outside is a reorder, one with a drop target is a soak, and only
/// the remainder detaches a tab into a window of its own.
pub fn starts_tear_off(released: bool, outside: bool, has_target: bool) -> bool {
    released && outside && !has_target
}

/// The release stamp, if a tear-off is in flight. Used to anchor the new window.
pub fn release_ms() -> Option<f64> {
    match LAST_RELEASE_MS.load(Ordering::SeqCst) {
        0 => None,
        stamp => Some(stamp as f64),
    }
}

/// The JavaScript this module injects into a new tab window: the anchor the
/// frontend measures its own marks against. Empty when profiling is off, so a
/// normal run gets no injected globals at all.
pub fn anchor_script() -> String {
    match (enabled(), release_ms()) {
        (true, Some(anchor)) => {
            format!("window.__DAE_TAB_PERF_ANCHOR={anchor};window.__DAE_TAB_PERF=1;")
        }
        _ => String::new(),
    }
}

/// Whether this run was asked for a cold-start profile.
///
/// A separate switch from [`enabled`] because the two probes answer different
/// questions and are anchored on different instants: a tear-off is measured from
/// the drag release and only means anything in a window born from one, while a
/// cold start is measured from this process and wants the main window untouched
/// by any drag state.
///
/// Inert unless `DAE_STARTUP_PERF=1`.
pub fn startup_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| match std::env::var("DAE_STARTUP_PERF") {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    })
}

/// Epoch millis at which this process entered `run()`, stamped on first ask.
///
/// The one instant earlier than any window or webview exists, and therefore the
/// only honest zero point for a cold-start timeline. It is the entry to this
/// crate's `run`, not the OS `CreateProcess`: the gap between them holds the
/// loader and the runtime's own init, which neither side can mark from here.
pub fn process_start() -> f64 {
    static START: OnceLock<f64> = OnceLock::new();
    *START.get_or_init(now_ms)
}

/// The cold-start half of [`anchor_script`]: turns the frontend probe on and
/// hands it the process start to measure from.
pub fn startup_script() -> String {
    if !startup_enabled() {
        return String::new();
    }
    format!(
        "window.__DAE_STARTUP_PERF=1;window.__DAE_STARTUP_ANCHOR={};",
        process_start()
    )
}

/// The enabling half of [`anchor_script`], for a window whose release is not
/// known when it is built.
///
/// A pooled window is built before anyone has dragged anything, so there is no
/// instant to anchor on and `anchor_script` returns empty - which switches the
/// whole frontend probe off, including the timeline it would report later.
/// The anchor can wait; the switch cannot.
pub fn enable_script() -> String {
    if enabled() {
        "window.__DAE_TAB_PERF=1;".to_string()
    } else {
        String::new()
    }
}

/// Files this window's label so [`mark`] knows where to record.
pub fn set_current_label(label: &str) {
    if !enabled() {
        return;
    }
    CURRENT_LABEL.with(|slot| *slot.borrow_mut() = Some(label.to_string()));
}

/// Records a mark for the current window. Repeats are ignored so a second run of
/// the same stage cannot overwrite the first, and therefore meaningful, timing.
pub fn mark(stage: &str) {
    if !enabled() {
        return;
    }
    let Some(label) = CURRENT_LABEL.with(|slot| slot.borrow().clone()) else {
        return;
    };
    record(&label, stage, now_ms());
}

/// Records a mark with a timestamp the caller already holds.
///
/// [`mark`] stamps on arrival, which cannot express an instant that happened
/// before the label was known - process start, for one. Used by the cold-start
/// probe, whose Rust-side marks all carry a timestamp observed elsewhere.
pub fn mark_at(label: &str, stage: &str, at: f64) {
    if !startup_enabled() {
        return;
    }
    record(label, stage, at);
}

fn record(label: &str, stage: &str, at: f64) {
    let mut store = store().lock().expect("tab_perf store poisoned");
    let marks = store.entry(label.to_string()).or_default();
    if marks.iter().any(|(name, _)| name == stage) {
        return;
    }
    marks.push((stage.to_string(), at));
}

/// Folds the marks the new window reported into this process's own marks and
/// prints one aligned timeline.
///
/// `release_ms` has no stored release to fall back on for a window that was not
/// born from a drag, so the earliest mark is used as the zero point instead.
pub fn report(label: &str, anchor: f64, from_frontend: Vec<(String, f64)>) {
    let ours = store()
        .lock()
        .expect("tab_perf store poisoned")
        .remove(label)
        .unwrap_or_default();

    let rows = offset_rows(anchor, ours, from_frontend);
    // `RELEASE` is the tear-off zero point, not a stage of a cold start (which
    // is anchored on `process-start` and says so itself).
    let rows: Vec<(String, f64)> = if startup_enabled() {
        rows.into_iter()
            .filter(|(stage, _)| stage != "RELEASE")
            .collect()
    } else {
        rows
    };

    // A cold start's total is the first frame, not the last row: the timeline is
    // held open for `first-data`, which lands after the frame and is not part of
    // what the user waited for. A tear-off has no such row and ends at its last
    // mark.
    let first_frame = rows
        .iter()
        .find(|(stage, _)| startup_enabled() && stage == "first-frame")
        .map(|(_, offset)| *offset);
    let last = first_frame
        .or_else(|| rows.last().map(|(_, offset)| *offset))
        .unwrap_or(0.0);
    let title = if startup_enabled() {
        "cold start timeline"
    } else {
        "tab tear-off timeline"
    };
    let mut out = format!("\n{title} [{label}]\n");
    for (stage, offset) in &rows {
        out.push_str(&format!("  {stage:<28} {offset:>8.1} ms\n"));
    }
    out.push_str(&format!("  {:<28} {last:>8.1} ms\n", "TOTAL"));
    eprintln!("{out}");
    log::info!("{out}");

    // A release build is a `windows_subsystem = "windows"` binary, so its
    // stderr goes nowhere and the log file is rotated. A harness needs one file
    // it can poll, with one block per run, so the path is opt-in.
    if let Ok(path) = std::env::var("DAE_STARTUP_PERF_OUT") {
        if !path.is_empty() {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut file| std::io::Write::write_all(&mut file, out.as_bytes()));
        }
    }

    // The run is done, so the next tear-off anchors on its own release.
    LAST_RELEASE_MS.store(0, Ordering::SeqCst);
}

/// Serves `tab_perf_report`.
///
/// Kept out of the specta registry on purpose: a probe must not disturb
/// `src/bindings.ts`.
#[tauri::command]
fn tab_perf_report(data: String) {
    #[derive(serde::Deserialize)]
    struct Payload {
        label: String,
        #[serde(default)]
        anchor: f64,
        #[serde(default)]
        marks: Vec<(String, f64)>,
    }

    match serde_json::from_str::<Payload>(&data) {
        Ok(payload) => report(&payload.label, payload.anchor, payload.marks),
        Err(error) => eprintln!("tab_perf_report: unusable payload: {error}"),
    }
}

/// Handles this module's commands, returning `false` for everything else so the
/// caller can fall through to the generated handler.
pub fn handle_invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    match invoke.message.command() {
        "tab_perf_report" => __cmd__tab_perf_report!(tab_perf_report, invoke),
        _ => false,
    }
}

/// Records a mark under an explicit label, for the source window which never had
/// [`set_current_label`] called on it.
#[allow(dead_code)] // kept as the source-window entry point; see NOTES.md
pub fn mark_for(label: &str, stage: &str) {
    if !enabled() {
        return;
    }
    record(label, stage, now_ms());
}

/// Turns raw marks into `(stage, ms since the drag was released)` rows, earliest
/// first.
///
/// Split out of [`report`] because this is the part that can be quietly wrong: a
/// shifted zero point moves every number at once without anything looking broken,
/// which is how a probe ends up arguing for the wrong optimization.
fn offset_rows(
    anchor: f64,
    ours: Vec<(String, f64)>,
    from_frontend: Vec<(String, f64)>,
) -> Vec<(String, f64)> {
    // A window not born from a drag has no release to measure from, so its own
    // earliest mark becomes the zero point.
    let zero = if anchor > 0.0 {
        anchor
    } else {
        ours.iter()
            .chain(from_frontend.iter())
            .map(|(_, at)| *at)
            .fold(f64::INFINITY, f64::min)
    };

    let mut rows: Vec<(String, f64)> = ours
        .into_iter()
        .chain(from_frontend)
        .map(|(stage, at)| (stage, at - zero))
        .collect();

    // The release is stored as a bare timestamp, not as a recorded mark, so it
    // would otherwise be the invisible zero the whole table hangs off. Show it
    // when it was a real release rather than a fall-back to the first mark.
    if anchor > 0.0 {
        rows.push(("RELEASE".to_string(), 0.0));
    }

    rows.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::{clear_release, offset_rows, release_ms, stamp_release, starts_tear_off};

    fn marks(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs
            .iter()
            .map(|(name, at)| (name.to_string(), *at))
            .collect()
    }

    #[test]
    fn offsets_are_measured_from_the_release() {
        let rows = offset_rows(
            1_000.0,
            marks(&[("BUILT", 1_060.0), ("SHOWN", 1_090.0)]),
            marks(&[("PAINT", 1_200.0)]),
        );

        assert_eq!(
            rows,
            marks(&[
                ("RELEASE", 0.0),
                ("BUILT", 60.0),
                ("SHOWN", 90.0),
                ("PAINT", 200.0)
            ])
        );
    }

    #[test]
    fn a_window_without_a_release_measures_from_its_own_first_mark() {
        let rows = offset_rows(
            0.0,
            Vec::new(),
            marks(&[("ANCHOR", 500.0), ("PAINT", 540.0)]),
        );

        assert_eq!(rows, marks(&[("ANCHOR", 0.0), ("PAINT", 40.0)]));
    }

    #[test]
    fn rows_are_ordered_across_the_two_sources() {
        let rows = offset_rows(0.0, marks(&[("LATE", 300.0)]), marks(&[("EARLY", 100.0)]));

        assert_eq!(rows.first().map(|(stage, _)| stage.as_str()), Some("EARLY"));
        assert_eq!(rows.last().map(|(stage, _)| stage.as_str()), Some("LATE"));
    }

    #[test]
    fn only_a_tear_off_starts_a_timeline() {
        // Released inside the source window: a reorder.
        assert!(!starts_tear_off(false, false, false));
        // Released outside, over another window's strip: a soak.
        assert!(!starts_tear_off(true, true, true));
        // Released outside, over nothing: the tab detaches.
        assert!(starts_tear_off(true, true, false));
        // A drag the platform never reported as released stays a reorder no
        // matter how far it travelled.
        assert!(!starts_tear_off(false, true, false));
    }

    #[test]
    fn a_soak_between_two_tear_offs_does_not_anchor_the_second() {
        // The reported bug: a reorder or soak near a tear-off used to leave its
        // release in the slot, so the next timeline started at the soak and
        // counted the user's dragging time as latency.
        let soak = 1_000.0;
        let tear_off = 1_356.0;

        stamp_release(soak);
        assert_eq!(release_ms(), Some(soak));
        clear_release();
        assert_eq!(release_ms(), None);

        stamp_release(tear_off);
        assert_eq!(release_ms(), Some(tear_off));

        clear_release();
    }
}
