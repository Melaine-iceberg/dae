import { Profiler, type ReactNode } from "react";

import { tabPerfEnabled, tabPerfMark } from "./tab-perf";

/**
 * Times one subtree's render with React's own Profiler and puts the result on
 * the tear-off timeline.
 *
 * `render -> commit` measured ~157ms of fixed cost that did not move when the
 * folder being shown went from a handful of entries to thousands, which the
 * virtualised listing explains: only the rows near the viewport ever mount. So
 * that number says nothing yet about *which* subtree is spending it, and
 * React's Profiler is the only thing that does.
 *
 * Two reasons this is a component and not DevTools. React ships the Profiler in
 * production builds, where DevTools cannot attach at all, and a release build
 * is the only place these numbers mean anything. And a subtree wrapped here
 * still commits normally — this adds measurement, never structure.
 *
 * `onRender` reports `commitTime` on the same clock `performance.now()` uses,
 * so it needs `timeOrigin` added to become the epoch milliseconds the rest of
 * the timeline is kept in. `actualDuration` goes into the stage name, which is
 * how the duration reaches the table without teaching the Rust side a second
 * shape of mark.
 *
 * With `DAE_TAB_PERF` unset this returns the children untouched: no Profiler,
 * no callback, no overhead.
 */
export function TabPerfProfiler({ id, children }: { id: string; children: ReactNode }) {
  if (!tabPerfEnabled()) return <>{children}</>;
  return (
    <Profiler
      id={id}
      onRender={(_id, phase, actualDuration, _baseDuration, _startTime, commitTime) => {
        tabPerfMark(
          `${id}[${phase}] ${actualDuration.toFixed(1)}ms`,
          performance.timeOrigin + commitTime,
        );
      }}
    >
      {children}
    </Profiler>
  );
}
