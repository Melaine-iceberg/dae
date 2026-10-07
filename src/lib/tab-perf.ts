import { invoke } from "@tauri-apps/api/core";

import { getAppWindow } from "@/lib/app-window";

/**
 * Start-up instrumentation for a torn-off tab window.
 *
 * Every mark is an epoch timestamp, so the numbers line up on one timeline with
 * the marks the Rust side records around the same tear-off: the anchor is the
 * instant the native drag returned, and the last mark here is the first frame
 * the detached window paints. Reading the two halves apart is what makes this
 * worth measuring, because the busy half might have nothing to do with React.
 *
 * Inert unless the Rust side injected an anchor, which it only does when
 * `DAE_TAB_PERF=1` is set. A normal run executes the `tabPerfEnabled` check and
 * nothing else: no globals, no marks, no IPC.
 */

type Mark = [stage: string, at: number];

const globals = globalThis as typeof globalThis & {
  __DAE_TAB_PERF?: number;
  __DAE_TAB_PERF_ANCHOR?: number;
};

const marks: Mark[] = [];

/** Whether this window was started with profiling on. */
export function tabPerfEnabled(): boolean {
  return globals.__DAE_TAB_PERF === 1;
}

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
  if (!tabPerfEnabled() || marks.length === 0) return;
  const label = getAppWindow()?.label ?? "";
  const anchor = anchorAt ?? globals.__DAE_TAB_PERF_ANCHOR ?? 0;
  void invoke("tab_perf_report", {
    data: JSON.stringify({ label, anchor, marks }),
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
  const root = document.getElementById("root");
  const finish = () => {
    if (done) return;
    done = true;
    observer?.disconnect();
    requestAnimationFrame(() => {
      tabPerfMark("adopt-painted");
      // Zeroes on the split, so this block reads as what the adoption cost
      // rather than as the time this window happened to have been alive.
      tabPerfReport(split);
    });
  };
  const observer =
    root && typeof MutationObserver !== "undefined"
      ? new MutationObserver(finish)
      : undefined;

  if (observer && root) {
    observer.observe(root, { childList: true, subtree: true, characterData: true });
  }
  // A merge that changes nothing visible would otherwise never report, and a
  // probe that stays silent reads as a probe that was never switched on.
  setTimeout(finish, 2000);
}
