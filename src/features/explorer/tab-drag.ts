/** The MIME type a tab handoff travels under, between this app's windows.
 *
 *  A drag whose data store carries this type is a tab being moved: the strip
 *  reorders it, another window of this app inserts it, and every other drop
 *  target — the desktop, another application — ignores it. The payload is
 *  opaque JSON (see {@link TabDragPayload}); nothing outside this app can do
 *  anything with it.
 *
 *  The gesture itself is the platform's: the browser starts the drag when a
 *  tab that is `draggable` is pulled off its strip, and the platform's drag
 *  loop carries it from there. That is the one mechanism that reaches a window
 *  the page holds no pointer events for, which on Wayland is every window but
 *  the one the drag started in — an implicit pointer grab is compositor state,
 *  and a drag session is the only way to cross it. */
export const TAB_DRAG_TYPE = "application/x-dae-tab";

/** What a tab drag carries: the window the tab is leaving, a token that
 *  identifies this one handoff, and the tab itself in the shape
 *  `serializeTabHandoff` produces. */
export type TabDragPayload = {
  source: string;
  token: string;
  payload: string;
};

/** Where a dropped tab would insert itself in this window's strip. */
export type DropIndicatorGeometry = {
  left: number;
  top: number;
  height: number;
};

/** Width of the strip's auto-scroll zones at each end, in CSS px. */
const SCROLL_EDGE = 28;
/** Auto-scroll speed once the cursor is fully in the zone, px per frame. */
const SCROLL_MAX_STEP = 20;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** Whether this drag is a tab from one of this app's windows.
 *
 *  Only the type list is readable here: the data store is in protected mode
 *  for the whole hover, so a drop target can recognise a tab without being
 *  allowed to read it — which is all the indicator needs, and all a foreign
 *  application would get to see anyway. */
export function isTabDrag(dataTransfer: DataTransfer | null): boolean {
  return dataTransfer ? Array.from(dataTransfer.types).includes(TAB_DRAG_TYPE) : false;
}

/** Stamps the tab this drag carries onto the platform's data store. Called
 *  from `dragstart`, the only moment the source may write to it. */
export function writeTabDrag(dataTransfer: DataTransfer, value: TabDragPayload): void {
  dataTransfer.setData(TAB_DRAG_TYPE, JSON.stringify(value));
}

/** The tab a drop is carrying, read at the drop, where the data store is
 *  unlocked. Returns null for anything malformed: a handoff crosses a process
 *  boundary (each window is its own renderer), so the receiver validates
 *  rather than trusts. */
export function readTabDrag(dataTransfer: DataTransfer | null): TabDragPayload | null {
  if (!dataTransfer) return null;

  let raw: string;
  try {
    raw = dataTransfer.getData(TAB_DRAG_TYPE);
  } catch {
    return null;
  }
  if (!raw) return null;

  try {
    const value: unknown = JSON.parse(raw);
    if (!isRecord(value)) return null;
    const { source, token, payload } = value;
    if (typeof source !== "string" || typeof token !== "string" || typeof payload !== "string") {
      return null;
    }
    return { source, token, payload };
  } catch {
    return null;
  }
}

/** A tab this window has put into a drag, waiting to hear who took it. */
type OutgoingHandoff = {
  tabId: string;
  claimed: boolean;
  settle: ((claimed: boolean) => void) | null;
};

/** Handoffs this window has sent out, keyed by the drag's token.
 *
 *  A drop target answers with the same token, and that answer is what tells
 *  this window its tab is gone — not the platform's drag result. A result is
 *  only a courtesy: Chromium derives it on Wayland from a drop action the
 *  compositor may never report, and reads a missing action as a cancelled
 *  drag, which would leave the tab in both windows (or tear off a second
 *  copy). The token also survives the gesture: the answer can arrive after
 *  `dragend`, once the source tab's own drag is already over. */
const outgoingHandoffs = new Map<string, OutgoingHandoff>();

/** How long an unanswered handoff is remembered. A claim crosses two renderers
 *  and the shell, so it is not instant, but it is never minutes late either. */
const OUTGOING_HANDOFF_TTL = 30_000;

/** How long a drag that ended without a drop waits to hear that another window
 *  took its tab, before deciding the tab was dropped on nothing. */
export const HANDOFF_CLAIM_GRACE_MS = 150;

export function trackOutgoingTabHandoff(token: string, tabId: string): void {
  outgoingHandoffs.set(token, { tabId, claimed: false, settle: null });
  window.setTimeout(() => outgoingHandoffs.delete(token), OUTGOING_HANDOFF_TTL);
}

/** Resolves the token a drop target claimed, reporting the tab it carried.
 *  Called from the window that *sent* the handoff, once the shell says who
 *  took it. */
export function takeClaimedTabHandoff(token: string): string | null {
  const handoff = outgoingHandoffs.get(token);
  if (!handoff) return null;
  handoff.claimed = true;
  outgoingHandoffs.delete(token);
  handoff.settle?.(true);
  return handoff.tabId;
}

/** Reports whether the given handoff was claimed, waiting up to `timeout` ms
 *  for an answer that is still in flight. */
export function waitForClaimedTabHandoff(
  token: string,
  timeout: number,
): Promise<boolean> {
  const handoff = outgoingHandoffs.get(token);
  if (!handoff) return Promise.resolve(false);
  if (handoff.claimed) return Promise.resolve(true);

  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      handoff.settle = null;
      resolve(false);
    }, timeout);
    handoff.settle = (claimed) => {
      window.clearTimeout(timer);
      resolve(claimed);
    };
  });
}

/** Spots the OS drag image on the point the tab was grabbed by.
 *
 *  The image is a clone parked exactly over the chip for the one frame the
 *  platform needs to rasterize it. Cloning rather than using the live chip is
 *  what keeps the image undimmed: the source chip fades to mark the tab as
 *  lifted (see `data-dragging`), and a snapshot taken after that fade would
 *  drag a greyed-out tab across the desktop. `position: fixed` at the chip's
 *  own box means no scroll offset or strip overflow can move the pixels, and
 *  the clone is removed as soon as the platform owns it. */
export function setTabDragImage(
  dataTransfer: DataTransfer,
  element: HTMLElement,
  grabX: number,
  grabY: number,
): void {
  const bounds = element.getBoundingClientRect();
  const clone = element.cloneNode(true) as HTMLElement;
  clone.removeAttribute("aria-grabbed");
  clone.removeAttribute("data-dragging");
  clone.setAttribute("aria-hidden", "true");
  clone.style.position = "fixed";
  clone.style.left = `${bounds.left}px`;
  clone.style.top = `${bounds.top}px`;
  clone.style.width = `${bounds.width}px`;
  clone.style.height = `${bounds.height}px`;
  clone.style.margin = "0";
  clone.style.transform = "none";
  clone.style.opacity = "1";
  clone.style.pointerEvents = "none";
  clone.style.zIndex = "2147483647";

  document.body.appendChild(clone);
  dataTransfer.setDragImage(clone, grabX, grabY);
  requestAnimationFrame(() => clone.remove());
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
