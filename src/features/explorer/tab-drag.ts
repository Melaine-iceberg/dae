import { commands } from "@/bindings";

/** Distance the pointer travels before a press on a tab becomes a drag. */
export const TAB_DRAG_START_DISTANCE = 6;
/** Cushion the drag-image snapshot keeps around the ghost, in CSS pixels, on
 *  every side.
 *
 *  Antialiasing slack only, so it stays small and — deliberately — symmetric.
 *  Nothing of the tab is painted outside its border box, because the drag image
 *  carries no elevation; the two device pixels only cover the half-covered
 *  pixel straddling the border box, where the rounded corners' edge lands. */
export const TAB_DRAG_PREVIEW_PAD = 2;
/** Room past the strip's edges before a dragged tab counts as detached.
 *
 *  Asymmetric on purpose. Above the strip there is the window's own edge, so a
 *  few pixels of slack is all a drag can want. Below it, a pull of a whole strip
 *  height is what reads as "this tab is its own window now": a hand travelling
 *  horizontally dips and rises by a few dozen pixels as a matter of course, and
 *  a band that punished that would freeze the reorder halfway across the strip.
 *  Matches how far Chrome has to drag a tab down before it floats. */
const BAND_SLOP = 6;
/** Width of the strip's auto-scroll zones at each end, in CSS px. */
const SCROLL_EDGE = 28;
/** Auto-scroll speed once the cursor is fully in the zone, px per frame. */
const SCROLL_MAX_STEP = 20;
/** How often the native edge query runs on a platform whose WebView stops
 *  reporting the pointer past the window edge. */
const OUTSIDE_POLL_INTERVAL = 50;

/** The strip band: the header row, stretched across the whole window width.
 *
 *  A tab strip spans its window, so the chrome's horizontal edges are not
 *  detach boundaries. Clipping the band to the strip's own extent made the
 *  boundary asymmetric: its left edge landed on the window edge, unreachable
 *  for a leftward drag, while its right one fell *inside* the window at the
 *  caption buttons, so dragging right popped the tab out. Only leaving the band
 *  vertically, or leaving the window, detaches a tab. */
export type TabDragBand = {
  top: number;
  bottom: number;
};

/** Ghost geometry in viewport pixels, tracking the cursor by the grab point.
 *  Captured once when the ghost mounts; the frames after that position the
 *  ghost purely through its `transform`, never through React state. */
export type TabDragGhost = {
  x: number;
  y: number;
  width: number;
  height: number;
};

/** Where a tab that left the window is going, as reported by the platform's
 *  drag loop. */
export type NativeDragOutcome =
  /** Released over this window again, or cancelled: the tab stays put. */
  | { action: "keep" }
  /** Released over the desktop, or over something that takes no tabs. The
   *  cursor, when the platform could see it, is where the window lands. */
  | { action: "detach"; cursor: { x: number; y: number } | null }
  /** Released over another of this app's windows, at a point in that window's
   *  CSS pixels. */
  | { action: "merge"; target: string; x: number; y: number };

/** Where a drag released over another window would insert this window's tabs. */
export type DropIndicatorGeometry = {
  left: number;
  top: number;
  height: number;
};

/** The gesture's stages, in the order a cursor walks them.
 *
 *  `pending` is the pre-threshold press, `inStrip` reorders live, `popped` has
 *  left the band and detaches on release unless the cursor comes back, and
 *  `native` has handed the pointer to the platform drag loop because it left
 *  the window. Only the last one is one-way. */
type DragPhase = "pending" | "inStrip" | "popped" | "native";

/** Whether window-space coordinates lie outside the WebView's viewport.
 *
 *  The WebView keeps delivering pointer events past the window edge, and under
 *  Wayland those events are the only bound signal there is: the session exposes
 *  no global pointer position, and clamps the coordinates of a pointer that has
 *  left to exactly `innerWidth`/`innerHeight` rather than to one pixel short.
 *  The clamp is why the left and top edges are not detectable this way — they
 *  land on `0`, a legitimate position — and why losing pointer capture counts
 *  as an edge too. */
export function isOutsideViewport(x: number, y: number): boolean {
  return x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight;
}

export function measureTabBand(): TabDragBand | null {
  const header = document.querySelector<HTMLElement>("[data-tab-bar]");
  if (!header) return null;
  const chrome = header.getBoundingClientRect();
  return {
    top: chrome.top - BAND_SLOP,
    bottom: chrome.bottom + Math.max(BAND_SLOP, chrome.height),
  };
}

export function isInsideBand(band: TabDragBand, y: number): boolean {
  return y >= band.top && y <= band.bottom;
}

/** Index at which a tab dropped at window-space `x` belongs in the strip:
 *  before the first tab whose midpoint is right of the drop point. */
export function tabInsertionIndexAt(strip: HTMLElement, x: number): number {
  const tabs = Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]'));
  for (let index = 0; index < tabs.length; index++) {
    const rect = tabs[index].getBoundingClientRect();
    if (x < rect.left + rect.width / 2) return index;
  }
  return tabs.length;
}

/** Reorder target for the tab being dragged: its index among the *other* tabs,
 *  the convention `moveTab` and the drop indicator both use. Excluding the
 *  dragged element keeps the measured midpoints stable while the strip shifts
 *  under the cursor. */
export function tabReorderIndexAt(strip: HTMLElement, x: number, dragged: HTMLElement): number {
  const tabs = Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]')).filter(
    (element) => element !== dragged,
  );
  for (let index = 0; index < tabs.length; index++) {
    const rect = tabs[index].getBoundingClientRect();
    if (x < rect.left + rect.width / 2) return index;
  }
  return tabs.length;
}

/** Viewport-space placement of the drop indicator for a hover/drop at
 *  window-space `x`, aligned with the tab gap the insertion would occupy. */
export function dropIndicatorGeometryAt(x: number): DropIndicatorGeometry | null {
  const strip = document.querySelector<HTMLElement>('[role="tablist"]');
  if (!strip) return null;

  const tabs = Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]'));
  if (tabs.length === 0) {
    const stripRect = strip.getBoundingClientRect();
    return { left: stripRect.left + 10, top: stripRect.top + 8, height: stripRect.height - 16 };
  }

  const index = tabInsertionIndexAt(strip, x);
  const gap = 4; // The strip's gap-1 between neighbouring tabs.
  const left =
    index < tabs.length
      ? tabs[index].getBoundingClientRect().left - gap
      : tabs[tabs.length - 1].getBoundingClientRect().right + gap;
  const tabRect = tabs[0].getBoundingClientRect();
  return { left, top: tabRect.top, height: tabRect.height };
}

/** How far the strip has to scroll this frame to follow a drag whose cursor
 *  sits at `x`: zero outside the zones at either end, faster the deeper into
 *  the zone the cursor is. */
export function stripAutoScrollStep(strip: HTMLElement, x: number): number {
  const rect = strip.getBoundingClientRect();
  const depth = Math.max(rect.left + SCROLL_EDGE - x, x - (rect.right - SCROLL_EDGE));
  if (depth <= 0) return 0;
  const speed = Math.min(SCROLL_MAX_STEP, (SCROLL_MAX_STEP * depth) / SCROLL_EDGE);
  return x < rect.left + SCROLL_EDGE ? -speed : speed;
}

/** Whether this session hides the pointer's global position, leaving the
 *  WebView's own pointer events as the only way to see that a drag has left the
 *  window — in which case the platform drag loop must not be handed the pointer
 *  while it is still inside, since that takes the remaining events with it.
 *
 *  Asked once and cached: the answer is a property of the session, not of the
 *  drag, and it is warmed on pointer down so it is in hand by the time the drag
 *  threshold is crossed. See the Rust `tab_drag_uses_frontend_bounds` command
 *  for what Wayland denies the native path. */
let frontendBoundsAnswer: boolean | null = null;
let frontendBoundsRequest: Promise<boolean> | null = null;

function resolveFrontendBounds(): Promise<boolean> {
  frontendBoundsRequest ??= commands
    .tabDragUsesFrontendBounds()
    .then((value) => {
      frontendBoundsAnswer = value;
      return value;
    })
    .catch(() => {
      frontendBoundsAnswer = false;
      return false;
    });
  return frontendBoundsRequest;
}

/** The cached answer, defaulting to a session with global coordinates. */
function usesFrontendBounds(): boolean {
  return frontendBoundsAnswer ?? false;
}

export type TabDragDeps = {
  tabId: string;
  element: HTMLElement;
  /** The tab's slot when the press began, which Escape puts it back into. */
  originalIndex: number;
  pointerId: number;
  startX: number;
  startY: number;
  /** The press, relative to the tab's border box: the point that keeps
   *  following the cursor, and that the OS drag image is spotted by. */
  grabX: number;
  grabY: number;
  moveTab: (tabId: string, insertionIndex: number) => void;
  /** Mounts the ghost at this geometry, or unmounts it. Called once per
   *  gesture — never per frame; the per-frame work is `moveGhost`. */
  onGhost: (ghost: TabDragGhost | null) => void;
  /** Positions the mounted ghost for one frame. Writes the portal element's
   *  `transform` directly so a drag that tracks the cursor at rAF rate costs
   *  no re-render — a ghost steered through React state was redrawing the
   *  whole tab strip once per frame. */
  moveGhost: (x: number, y: number) => void;
  onDragging: (dragging: boolean) => void;
  /** Reports the press itself: held from the pointer down to the gesture's
   *  teardown, and released by a gesture that ends however it ends. The tab's
   *  pressed wash comes from this rather than from `:active`, because the
   *  WebView never sees the release of a drag that left the window — that one
   *  belongs to the platform's drag loop, and a `:active` left behind outlives
   *  the gesture it came from. */
  onPressed: (pressed: boolean) => void;
  /** Rasterizes the ghost for the OS drag image. Runs at the hand-off, where
   *  the ghost has been in the DOM since the drag began — starting it there
   *  instead of at the first move keeps the rasterization (a full-computed-
   *  style clone plus a canvas round-trip) off the drag's frames entirely. */
  capturePreview: () => Promise<string | null>;
  /** Gives the gesture to the platform drag loop and reports where it landed. */
  handOffToNative: (preview: string | null) => Promise<NativeDragOutcome>;
  /** Puts the tab in a window of its own, at `cursor` where one is known. */
  detach: (cursor: { x: number; y: number } | null) => Promise<void>;
  /** Hands the tab to another of this app's windows, inserting it at `x`. */
  merge: (target: string, x: number, y: number) => Promise<void>;
  /** Asks the window itself whether the cursor has left it, for a platform
   *  whose WebView stops reporting the pointer past that edge. */
  readNativeOutside: () => Promise<boolean>;
};

export type TabDragController = {
  /** Tears the gesture down. Idempotent, and safe to call from a dep. */
  end: () => void;
};

/** Runs one tab drag: the pointer tracking, the band state machine, the live
 *  reorder, and the two ways out of it — detach, or hand off to the platform.
 *
 *  Called from pointer down; it owns the gesture's listeners until it ends.
 *  Everything that decides *where* a tab goes lives here, while how it actually
 *  moves between windows is the caller's, through `TabDragDeps`. */
export function beginTabDragGesture(deps: TabDragDeps): TabDragController {
  const { element, pointerId, tabId } = deps;
  const strip = element.closest<HTMLElement>('[role="tablist"]');
  // The pointer is pinned to the strip rather than to the tab: the live reorder
  // moves the tab's node from slot to slot, and an engine that reads that move
  // as a capture loss would otherwise end the gesture on its first swap — which
  // is what makes a tab look as though it can only trade places with a
  // neighbour. The strip itself never moves, so a loss on it means what it is
  // supposed to mean: the WebView stopped reporting this pointer.
  const captureTarget = strip ?? element;
  // Read once for the whole gesture: the header does not move while a drag is
  // under way, and a mid-drag relayout (a tab closing behind the cursor) would
  // otherwise pop the tab on a boundary the user never crossed.
  const band = measureTabBand();

  let phase: DragPhase = "pending";
  let pointerX = deps.startX;
  let pointerY = deps.startY;
  let lastReorderIndex = -1;
  let frame: number | undefined;
  let pollTimer: number | undefined;
  let released = false;
  let ended = false;

  const end = () => {
    if (ended) return;
    ended = true;
    window.clearInterval(pollTimer);
    pollTimer = undefined;
    if (frame !== undefined) window.cancelAnimationFrame(frame);
    frame = undefined;
    window.removeEventListener("pointermove", handlePointerMove, true);
    window.removeEventListener("pointerup", handlePointerUp, true);
    window.removeEventListener("pointercancel", handlePointerGone, true);
    window.removeEventListener("keydown", handleKeyDown, true);
    captureTarget.removeEventListener("lostpointercapture", handleCaptureLost);
    if (captureTarget.hasPointerCapture(pointerId)) captureTarget.releasePointerCapture(pointerId);
    deps.onGhost(null);
    deps.onDragging(false);
    deps.onPressed(false);
  };

  const reorderTo = (x: number) => {
    if (!strip) return;
    const target = tabReorderIndexAt(strip, x, element);
    if (target === lastReorderIndex) return;
    lastReorderIndex = target;
    deps.moveTab(tabId, target);
  };

  /** One frame of the gesture. The ghost follows the cursor, and a cursor held
   *  near either end of the strip scrolls it — which moves the tabs under a
   *  cursor that has not moved at all, so the reorder slot is recomputed from
   *  the last known position and the frame keeps running. */
  const tick = () => {
    frame = undefined;
    if (ended || phase === "native") return;

    // The ghost is positioned by writing its transform, not by re-rendering:
    // a state-driven ghost meant the strip's React tree was redrawn once per
    // frame just to move one absolutely-positioned portal.
    deps.moveGhost(pointerX - deps.grabX, pointerY - deps.grabY);

    if (phase !== "inStrip" || !strip) return;
    const step = stripAutoScrollStep(strip, pointerX);
    if (step === 0) return;
    strip.scrollLeft += step;
    reorderTo(pointerX);
    frame ??= window.requestAnimationFrame(tick);
  };

  /** Hands the gesture to the platform drag loop — which happens exactly once
   *  the cursor has left the window. Before that the WebView is still reporting
   *  the pointer, and giving it away costs both the reorder and the ability to
   *  tell where the release happened. */
  const handOff = () => {
    if (ended || phase === "native") return;
    phase = "native";
    window.clearInterval(pollTimer);
    pollTimer = undefined;
    if (frame !== undefined) window.cancelAnimationFrame(frame);
    frame = undefined;
    void (async () => {
      // The rasterizer reads the live ghost, so the ghost — frozen at the
      // WebView edge since the ticks stopped — stays on screen until its
      // pixels are captured. The OS drag image then replaces it with the very
      // same pixels, and what would otherwise be a half-clipped leftover is
      // instead a seamless hand-over.
      const snapshot = await deps.capturePreview();
      if (ended) return;
      deps.onGhost(null);
      let outcome: NativeDragOutcome;
      try {
        outcome = await deps.handOffToNative(snapshot ?? null);
      } catch (error) {
        // The native loop is the only path that can report a merge, but failing
        // to start it — a compositor refusing a drag begun just past the window
        // edge, say — must not also cost the detach the cursor's own path
        // already established by leaving the window.
        console.error("Failed to start the native tab drag", error);
        end();
        await deps.detach(null);
        return;
      }
      if (ended) return;
      switch (outcome.action) {
        case "merge":
          end();
          await deps.merge(outcome.target, outcome.x, outcome.y);
          break;
        case "detach":
          end();
          await deps.detach(outcome.cursor);
          break;
        case "keep":
          end();
          break;
      }
    })().catch((error) => {
      console.error("Failed to settle a native tab drag", error);
      end();
    });
  };

  /** The edge query for platforms whose WebView stops reporting the pointer at
   *  the window boundary, where the move handler gets no event to run on. */
  const pollOutside = async () => {
    if (ended || phase === "native") return;
    try {
      if ((await deps.readNativeOutside()) && !ended) handOff();
    } catch (error) {
      console.error("Failed to track the tab drag", error);
      end();
    }
  };

  const begin = () => {
    phase = "inStrip";
    deps.onDragging(true);
    // One state change per gesture: the ghost mounts here, sized to the tab at
    // the moment it was grabbed, and every position after this is `moveGhost`.
    deps.onGhost({
      x: pointerX - deps.grabX,
      y: pointerY - deps.grabY,
      width: element.offsetWidth,
      height: element.offsetHeight,
    });
    // Wayland answers the native query with "inside" forever — it has no global
    // pointer position to measure against — while its clamped event coordinates
    // already carry the edge, so polling there only costs an IPC per tick.
    void resolveFrontendBounds().then((frontendBounds) => {
      if (!frontendBounds && !ended && phase !== "native") {
        pollTimer = window.setInterval(() => void pollOutside(), OUTSIDE_POLL_INTERVAL);
      }
    });
  };

  function handlePointerMove(moveEvent: PointerEvent) {
    if (moveEvent.pointerId !== pointerId || ended || released || phase === "native") return;

    if (phase === "pending") {
      const distance = Math.hypot(moveEvent.clientX - deps.startX, moveEvent.clientY - deps.startY);
      if (distance < TAB_DRAG_START_DISTANCE) return;
      begin();
    }

    pointerX = moveEvent.clientX;
    pointerY = moveEvent.clientY;

    // A pointer that has left the window has left the band too, so this covers
    // every edge the band check below cannot see.
    if (isOutsideViewport(pointerX, pointerY)) {
      handOff();
      return;
    }

    // Leaving the band pops the tab out of the strip; bringing it back docks it
    // and resumes the live reorder.
    phase = band && !isInsideBand(band, pointerY) ? "popped" : "inStrip";
    if (phase === "inStrip") reorderTo(pointerX);

    frame ??= window.requestAnimationFrame(tick);
  }

  function handlePointerUp(upEvent: PointerEvent) {
    if (upEvent.pointerId !== pointerId || ended || phase === "native") return;
    released = true;
    const detached = phase === "popped";
    // Releasing inside the window keeps the tab in the strip. Only a cursor
    // that left the band — where the tab already reads as its own window —
    // takes it away.
    end();
    if (detached) void deps.detach(null);
  }

  /** The WebView stopped reporting the pointer, either as `pointercancel` or as
   *  a loss of DOM capture: on a session with global coordinates that is a
   *  genuine cancellation, and on Wayland it is the compositor withdrawing the
   *  cursor from the window — i.e. the release itself. */
  const handlePointerGone = (event: PointerEvent) => {
    if (event.pointerId !== pointerId) return;
    // The platform drag loop owns the gesture past the window edge, so losing
    // capture there must not cancel the pending result.
    if (released || phase === "native" || ended) return;
    // A Wayland compositor sends no pointerup once the cursor leaves the
    // window, so the loss *is* the release: hand over, and let a drop on
    // another window merge rather than detach.
    if (usesFrontendBounds()) handOff();
    else end();
  };

  /** The capture loss for the node the pointer is pinned to, and only for it:
   *  the dragged tab's own node reports one every time the reorder moves it,
   *  bubbling up through the strip, and dismissing that is the whole point of
   *  pinning to the strip in the first place. */
  const handleCaptureLost = (event: PointerEvent) => {
    if (event.target !== captureTarget) return;
    handlePointerGone(event);
  };

  function handleKeyDown(keyEvent: KeyboardEvent) {
    if (keyEvent.key !== "Escape") return;
    keyEvent.preventDefault();
    if (phase !== "pending") deps.moveTab(tabId, deps.originalIndex);
    // Past the window edge the native loop cancels on its own Escape and
    // reports `keep`; restoring here is what makes that release land back in
    // the slot the drag started from.
    end();
  }

  void resolveFrontendBounds();
  deps.onPressed(true);
  captureTarget.setPointerCapture(pointerId);
  window.addEventListener("pointermove", handlePointerMove, true);
  window.addEventListener("pointerup", handlePointerUp, true);
  window.addEventListener("pointercancel", handlePointerGone, true);
  window.addEventListener("keydown", handleKeyDown, true);
  captureTarget.addEventListener("lostpointercapture", handleCaptureLost);

  return { end };
}
