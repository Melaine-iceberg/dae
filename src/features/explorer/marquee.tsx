import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";

/**
 * Rubber-band (marquee) selection for the file views (SKILL.md §19 drag
 * selection). The hook owns the pointer lifecycle, rAF-throttled hit tests
 * and edge auto-scroll; views only supply a hit test mapping a viewport-space
 * rect to matching entry paths.
 */

export interface MarqueeRect {
  bottom: number;
  height: number;
  left: number;
  right: number;
  top: number;
  width: number;
}

interface MarqueeStartState {
  additive: boolean;
  baseSelection: string[];
  pointerId: number;
  startX: number;
  startY: number;
}

export interface MarqueeSelection {
  /** Viewport-space rect for rendering; null while idle. */
  rect: MarqueeRect | null;
  /**
   * Attempts to begin a marquee from a container background pointerdown.
   * Returns true when the gesture was claimed.
   */
  beginMarquee: (event: ReactPointerEvent) => boolean;
}

const MARQUEE_MIN_SIZE_PX = 4;
const AUTOSCROLL_EDGE_PX = 28;
const AUTOSCROLL_STEP_PX = 12;

export function useMarqueeSelection({
  enabled,
  getBaseSelection,
  hitTest,
  onSelectionChange,
  scrollElementRef,
}: {
  enabled: boolean;
  getBaseSelection: () => string[];
  /** Maps a viewport-space rect to the entry paths it intersects. */
  hitTest: (rect: MarqueeRect) => string[];
  onSelectionChange: (paths: string[]) => void;
  scrollElementRef: RefObject<HTMLElement | null>;
}): MarqueeSelection {
  const [rect, setRect] = useState<MarqueeRect | null>(null);
  const [isActive, setIsActive] = useState(false);
  const hitTestRef = useRef(hitTest);
  hitTestRef.current = hitTest;
  const onSelectionChangeRef = useRef(onSelectionChange);
  onSelectionChangeRef.current = onSelectionChange;
  const getBaseSelectionRef = useRef(getBaseSelection);
  getBaseSelectionRef.current = getBaseSelection;
  const startRef = useRef<MarqueeStartState | null>(null);
  const currentRectRef = useRef<MarqueeRect | null>(null);
  const rafRef = useRef<number | null>(null);

  const applySelection = useCallback(() => {
    rafRef.current = null;
    const currentRect = currentRectRef.current;
    const start = startRef.current;
    if (!currentRect || !start) return;

    const matched = hitTestRef.current(currentRect);
    onSelectionChangeRef.current(
      start.additive ? [...new Set([...start.baseSelection, ...matched])] : matched,
    );
  }, []);

  const scheduleApply = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = requestAnimationFrame(applySelection);
  }, [applySelection]);

  const stopMarquee = useCallback(() => {
    startRef.current = null;
    currentRectRef.current = null;
    setIsActive(false);
    setRect(null);
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  }, []);

  useEffect(
    () => () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  const beginMarquee = useCallback(
    (event: ReactPointerEvent) => {
      if (!enabled || event.button !== 0) return false;

      // React propagates portal events (e.g. context menu items) through the
      // React tree, so a pointerdown on a floating menu can reach this
      // container's handler. Only claim gestures that start inside the
      // container's own DOM subtree.
      const container = event.currentTarget;
      if (
        !(container instanceof Node) ||
        !(event.target instanceof Node) ||
        !container.contains(event.target)
      ) {
        return false;
      }

      const additive = event.ctrlKey || event.metaKey;
      startRef.current = {
        additive,
        baseSelection: additive ? getBaseSelectionRef.current() : [],
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
      };
      // Prevent native text/image selection during the gesture.
      event.preventDefault();
      setIsActive(true);
      return true;
    },
    [enabled],
  );

  useEffect(() => {
    if (!isActive) return;

    const handlePointerMove = (event: PointerEvent) => {
      const start = startRef.current;
      if (!start || start.pointerId !== event.pointerId) return;

      const nextRect: MarqueeRect = {
        bottom: Math.max(start.startY, event.clientY),
        height: Math.abs(event.clientY - start.startY),
        left: Math.min(start.startX, event.clientX),
        right: Math.max(start.startX, event.clientX),
        top: Math.min(start.startY, event.clientY),
        width: Math.abs(event.clientX - start.startX),
      };
      currentRectRef.current = nextRect;
      setRect(nextRect);
      scheduleApply();

      const scrollElement = scrollElementRef.current;
      if (scrollElement) {
        const bounds = scrollElement.getBoundingClientRect();
        if (event.clientY < bounds.top + AUTOSCROLL_EDGE_PX) {
          scrollElement.scrollTop -= AUTOSCROLL_STEP_PX;
          scheduleApply();
        } else if (event.clientY > bounds.bottom - AUTOSCROLL_EDGE_PX) {
          scrollElement.scrollTop += AUTOSCROLL_STEP_PX;
          scheduleApply();
        }
      }
    };

    const handlePointerEnd = () => {
      const start = startRef.current;
      if (!start) return;

      const currentRect = currentRectRef.current;
      const isClick =
        !currentRect ||
        (currentRect.width < MARQUEE_MIN_SIZE_PX && currentRect.height < MARQUEE_MIN_SIZE_PX);
      stopMarquee();

      // A plain background click clears the selection; ctrl-click keeps it.
      if (isClick && !start.additive) {
        onSelectionChangeRef.current([]);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        stopMarquee();
      }
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerEnd);
    window.addEventListener("pointercancel", handlePointerEnd);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerEnd);
      window.removeEventListener("pointercancel", handlePointerEnd);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [isActive, scheduleApply, scrollElementRef, stopMarquee]);

  return { beginMarquee, rect };
}

/** Renders the live marquee rectangle in viewport space. */
export function MarqueeOverlay({ rect }: { rect: MarqueeRect | null }) {
  if (!rect) return null;

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed z-40 rounded-sm border border-primary/60 bg-primary/10"
      style={{ height: rect.height, left: rect.left, top: rect.top, width: rect.width }}
    />
  );
}

/** Intersect helpers shared by the marquee hit tests. */
export function rectsIntersect(
  rect: { bottom: number; left: number; right: number; top: number },
  other: { bottom: number; left: number; right: number; top: number },
): boolean {
  return (
    rect.left < other.right && rect.right > other.left && rect.top < other.bottom && rect.bottom > other.top
  );
}
