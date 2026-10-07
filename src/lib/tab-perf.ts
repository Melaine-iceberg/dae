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
 */
export function tabPerfMark(stage: string): void {
  if (!tabPerfEnabled()) return;
  marks.push([stage, performance.timeOrigin + performance.now()]);
}

/**
 * Hands the collected marks back for the Rust side to fold into one timeline
 * and print. Failure is swallowed: a probe must never take down the window it
 * is measuring.
 */
export function tabPerfReport(): void {
  if (!tabPerfEnabled() || marks.length === 0) return;
  const label = getAppWindow()?.label ?? "";
  const anchor = globals.__DAE_TAB_PERF_ANCHOR ?? 0;
  void invoke("tab_perf_report", {
    data: JSON.stringify({ label, anchor, marks }),
  }).catch(() => {});
}
