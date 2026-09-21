import { useLayoutEffect, useState, type RefObject } from "react";

/**
 * Observes the content width of an element the caller already owns a ref to.
 *
 * The listing's columns have to answer to the width of the *pane* they are in,
 * not to the window: in the dual-pane layout each pane is roughly half the
 * window, so a viewport media query would leave both panes claiming room they
 * do not have. That makes this a container measurement, done in JS rather than
 * with container queries because the same number has to reach a grid template
 * *and* decide which cells exist at all — conditional rendering is not
 * something a CSS variant can do, and rendering four cells into a two-column
 * template leaves the name stranded in the wrong track.
 *
 * `remeasureKey` re-attaches the observer when the measured node changes
 * identity — the listing's scroller is unmounted in the grid and column views,
 * so a measurement taken once on mount would report 0 forever after.
 *
 * Returns `clientWidth` (the width the columns actually lay out in, scrollbar
 * gutter excluded), or `0` when there is nothing to measure. Callers should
 * read `0` as "not measured yet" and pick the widest layout, so an unmeasured
 * pane renders the full column set and then narrows.
 */
export function useMeasuredWidth<T extends HTMLElement>(
  ref: RefObject<T | null>,
  remeasureKey?: unknown,
): number {
  const [width, setWidth] = useState(0);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) {
      setWidth(0);
      return;
    }

    const measure = () => setWidth(element.clientWidth);
    measure();

    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, remeasureKey]);

  return width;
}
