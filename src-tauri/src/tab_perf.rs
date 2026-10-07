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

    let last = rows.last().map_or(0.0, |(_, offset)| *offset);
    let mut out = format!("\ntab tear-off timeline [{label}]\n");
    for (stage, offset) in &rows {
        out.push_str(&format!("  {stage:<14} {offset:>8.1} ms\n"));
    }
    out.push_str(&format!("  {:<14} {last:>8.1} ms\n", "TOTAL"));
    eprintln!("{out}");

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
    use super::offset_rows;

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
}
