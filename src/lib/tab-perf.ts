import { invoke } from "@tauri-apps/api/core";

import { getAppWindow } from "@/lib/app-window";

/**
 * Start-up instrumentation for a torn-off tab window and for a cold start.
 *
 * Every mark is an epoch timestamp, so the numbers line up on one timeline with
 * the marks the Rust side records around the same event. A tear-off anchors on
 * the instant the native drag returned; a cold start anchors on `run()`, so the
 * head of the table is the webview being created and the tail is the first frame
 * it paints. Reading the halves apart is what makes this worth measuring,
 * because `render -> commit` might have nothing to do with the bundle load.
 *
 * Inert unless the Rust side injected an anchor, which it only does when
 * `DAE_TAB_PERF=1` or `DAE_STARTUP_PERF=1` is set. A normal run executes the
 * `tabPerfEnabled` check and nothing else: no globals, no marks, no IPC.
 */

type Mark = [stage: string, at: number];

const globals = globalThis as typeof globalThis & {
  __DAE_TAB_PERF?: number;
  __DAE_TAB_PERF_ANCHOR?: number;
  __DAE_STARTUP_PERF?: number;
  __DAE_STARTUP_ANCHOR?: number;
  __DAE_STARTUP_MARKS?: Mark[];
};

const marks: Mark[] = [];

/**
 * Whether this window was started with a probe on.
 *
 * Two probes share this timeline because they share a clock and a report:
 * `__DAE_TAB_PERF` for a tear-off (anchored on the drag release) and
 * `__DAE_STARTUP_PERF` for a cold start (anchored on `run()`), whose zero point
 * is the main window's process rather than a drag. Which one is on decides only
 * what the zero point is; the marks are the same shape.
 */
export function tabPerfEnabled(): boolean {
  return globals.__DAE_TAB_PERF === 1 || globals.__DAE_STARTUP_PERF === 1;
}

/** Whether this window is reporting a cold start rather than a tear-off. */
export function startupPerfEnabled(): boolean {
  return globals.__DAE_STARTUP_PERF === 1;
}

// The cold-start probe records its first marks in `index.html`, before any
// module exists — the HTML itself is earlier than the bundle that would record
// it, which is the whole point of the split. The array is adopted by
// **reference**, not copied: the frame probe in `index.html` keeps appending to
// it (a frame can land while React is mounting, after this module has already
// been evaluated), and a copy taken here would silently drop exactly the rows
// §2's A/B is asking about.
const pageMarks: Mark[] = globals.__DAE_STARTUP_MARKS ?? [];

/** Stages already recorded, for the probe that must not repeat a first. */
const once = new Set<string>();

/** Continues a cold-start report once the first payload has landed. */
let onFirstData: (() => void) | null = null;

/**
 * Records one stage of this window's start-up, in epoch milliseconds so it can
 * be compared against the Rust-side marks directly.
 *
 * `at` is for callers that already hold a timeline timestamp — React's Profiler
 * reports `commitTime` relative to `timeOrigin` — so a subtree's row lands at
 * the instant its work finished rather than when this function was called.
 * It must be **epoch milliseconds**, the unit the rest of the timeline uses.
 */
export function tabPerfMark(stage: string, at?: number): void {
  if (!tabPerfEnabled()) return;
  marks.push([stage, at ?? performance.timeOrigin + performance.now()]);
}

/**
 * Marks the first payload a startup surface receives, and lets the cold-start
 * reporter know it can close the timeline.
 *
 * The instant that matters is arrival, not the render it causes: whether the
 * first frame waited for this is exactly the question the waterfall answers, so
 * the mark has to sit on the response side of that boundary.
 */
export function tabPerfFirstData(): void {
  if (!tabPerfEnabled() || once.has("first-data")) return;
  once.add("first-data");
  tabPerfMark("first-data");
  onFirstData?.();
}

/**
 * Registers what to do when the first payload lands. One window has one cold
 * start, so this is a slot rather than a listener set.
 */
export function tabPerfOnFirstData(callback: () => void): void {
  onFirstData = callback;
}

/**
 * Hands the collected marks back for the Rust side to fold into one timeline
 * and print. Failure is swallowed: a probe must never take down the window it
 * is measuring.
 *
 * `anchorAt` overrides the injected anchor for one report. A pooled window is
 * built before anyone drags anything, so its injected anchor is zero and its
 * marks carry times from its own boot; the adopt report passes the instant the
 * split happened instead, so its block measures the split rather than the wait
 * for one.
 */
export function tabPerfReport(anchorAt?: number): void {
  if (!tabPerfEnabled()) return;
  const all = pageMarks.length > 0 ? [...pageMarks, ...marks] : marks;
  if (all.length === 0) return;
  const label = getAppWindow()?.label ?? "";
  const anchor = anchorAt ?? globals.__DAE_TAB_PERF_ANCHOR ?? globals.__DAE_STARTUP_ANCHOR ?? 0;
  void invoke("tab_perf_report", {
    data: JSON.stringify({ label, anchor, marks: all }),
  }).catch(() => {});
}

/**
 * Measures a tab being merged into this window, which is the work a warm pool
 * would have to do to adopt it: re-point an already-booted React tree at
 * different content.
 *
 * The soak path already carries a tab across windows, so the number can come
 * from the real interaction instead of from a pool built to measure it. The
 * paint is taken from a mutation of the root rather than from the call, for the
 * reason given in `main.tsx`: a frame requested before React commits paints the
 * old content and reports a number that is too small.
 */
export function tabPerfAdoptProbe(): void {
  if (!tabPerfEnabled()) return;
  // The split, taken before anything else runs, so the report can zero here.
  // Anchoring on the report instead put the paint at zero and every cause of it
  // at a negative offset.
  const split = performance.timeOrigin + performance.now();
  // Boot happened minutes ago and was reported then; only what follows belongs
  // to this block, or every row above it reads as a negative offset.
  marks.length = 0;
  tabPerfMark("adopt");

  let done = false;
  // Whether the root actually changed. A merge that paints nothing would
  // otherwise reach the fallback below, and the fallback's own delay would be
  // reported as the adoption having taken that long.
  let committed = false;
  const root = document.getElementById("root");
  const finish = () => {
    if (done) return;
    done = true;
    observer?.disconnect();
    requestAnimationFrame(() => {
      if (!committed) tabPerfMark("adopt-no-commit");
      tabPerfMark("adopt-painted");
      // Zeroes on the split, so this block reads as what the adoption cost
      // rather than as the time this window happened to have been alive.
      tabPerfReport(split);
    });
  };
  const observer =
    root && typeof MutationObserver !== "undefined"
      ? new MutationObserver(() => {
          committed = true;
          finish();
        })
      : undefined;

  if (observer && root) {
    observer.observe(root, { childList: true, subtree: true, characterData: true });
  }
  // A merge that changes nothing visible would otherwise never report, and a
  // probe that stays silent reads as a probe that was never switched on.
  setTimeout(finish, 2000);
}
