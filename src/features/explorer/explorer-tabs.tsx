import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Window as TauriWindow, claimTabHandoff, getBridge, type EventCallback, type UnlistenFn } from "@/lib/ipc";
import {
  AddIcon,
  AltArrowLeftIcon,
  AltArrowRightIcon,
  CloseIcon,
  HistoryIcon,
  HomeIcon,
  MagnifierIcon,
  StarIcon,
  TrashBinTrashIcon,
  WidgetIcon,
} from "@solar-icons/react/linear";

import { Kbd } from "@/components/ui/kbd";
import { WindowControls } from "@/components/window-controls";
import { commands, events } from "@/bindings";
import { FocusedPaneStatusBar } from "@/features/explorer/explorer-chrome";
import { InspectorPanel } from "@/features/explorer/inspector-panel";
import { Sidebar } from "@/features/sidebar/sidebar";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import {
  commandBarModeAtom,
  commandBarOpenAtom,
} from "@/features/workspace/command-bar-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import { WorkspaceSurfaceView } from "@/features/workspace/workspace-surface";
import { TabPerfProfiler } from "@/lib/tab-perf-profiler";
import type { WorkspaceSurface } from "@/features/workspace/types";
import { MOD_KEY } from "@/lib/platform";
import { getAppWindow } from "@/lib/app-window";
import { cn } from "@/lib/utils";

// xterm and its renderer addons are only needed once the terminal panel is
// first revealed, so they load as a separate chunk instead of delaying the
// first frame.
const TerminalPanel = lazy(() =>
  import("@/features/terminal/terminal-panel").then((m) => ({ default: m.TerminalPanel })),
);

import { getFolderPresentation } from "./file-icons";
import {
  activeTabIdAtom,
  activateTabAtom,
  closeTabAtom,
  createTabAtom,
  getSplitNavigator,
  getTabNavigator,
  mergeTabFromHandoff,
  adoptTabFromHandoff,
  moveTab,
  serializeTabHandoff,
  splitEnabledFamily,
  tabsAtom,
  type ExplorerTab,
} from "./tabs";
import {
  HANDOFF_CLAIM_GRACE_MS,
  dropIndicatorGeometryAt,
  isTabDrag,
  readTabDrag,
  setTabDragImage,
  stripAutoScrollStep,
  tabInsertionIndexAt,
  tabReorderIndexAt,
  takeClaimedTabHandoff,
  trackOutgoingTabHandoff,
  waitForClaimedTabHandoff,
  writeTabDrag,
  type DropIndicatorGeometry,
} from "./tab-drag";

const TAB_STRIP_SCROLL_AMOUNT = 512;

/** How soon after the drag left this window a release still counts as having
 *  happened inside it.
 *
 *  Wayland sends no `drop` when the compositor reports no drag action; the
 *  leave that ends the session is what a source window has to go on. A teardown
 *  leave lands on top of `dragend`, while a real departure is followed by
 *  travel before the release. Getting this wrong only keeps a tab the user
 *  meant to cast out — a cross-window claim still wins. */
const IN_WINDOW_RELEASE_GRACE_MS = 300;

/** The tab the pointer is holding in *this* window, for the lifetime of one
 *  HTML5 drag.
 *
 *  Every field is event-time data, so it lives in a ref rather than in state:
 *  only `draggingTabId` and the drop indicator are painted, and everything here
 *  would otherwise re-render the strip on every `dragover`. */
type TabDragSession = {
  tabId: string;
  /** This handoff's identity, which is how a window that takes the tab answers
   *  this one. See `tab-drag.ts`. */
  token: string;
  /** Where inside the chip the drag was grabbed, which is the point a tear-off
   *  window keeps under the cursor. */
  grabX: number;
  grabY: number;
  /** The last pointer position seen inside this window, in its CSS pixels. */
  lastX: number;
  lastY: number;
  /** When this window last lost the drag (the dragleave that brought the
   *  hover count to zero), in `performance.now()` milliseconds, or `null`
   *  while the drag is still over it.
   *
   *  This — not the drag result — is what says whether the release happened
   *  here: Wayland's compositor reports no drop action, Chromium reads that as
   *  a cancellation, and a drop inside the source window arrives as
   *  `dragleave` + `dragend` with no `drop` at all. A pointer that left has to
   *  travel somewhere before it is released, so a leave that is still fresh
   *  when the drag ends was part of the teardown, not a departure. */
  leftAt: number | null;
  /** Set once the drag has left this window: a release that nothing accepted
   *  is then a tear-off rather than a cancelled gesture. */
  leftWindow: boolean;
  /** The drag was released over this window, which keeps the tab here even
   *  though a drop in the strip still reorders it. Only a platform that
   *  reports the drop sets this; Wayland is read from {@link leftAt}. */
  landed: boolean;
  /** Another window claimed this handoff. Remembered on the session rather
   *  than acted on at once: the claim can cross the drag it belongs to, and
   *  closing the chip while it is still the platform's drag source would pull
   *  the gesture out from under the browser. */
  claimed: boolean;
  /** Last reorder slot pushed to the store, so a hover that changes nothing
   *  costs no write. */
  index: number;
};

/** The shell's answer that a window took a tab this one was dragging. */
type TabHandoffClaim = {
  token: string;
  target: string | null;
};

/** Logs one stage of a tab drag when `DAE_TAB_DRAG_DEBUG=1`.
 *
 *  The gesture belongs to the renderer and, past the window edge, to the
 *  platform, so the two ways it can fail — a drag the platform never started,
 *  and a drop that arrived somewhere else — are indistinguishable from
 *  inside the page unless the stages are printed. Paired with
 *  `ELECTRON_ENABLE_LOGGING=1` these lines land next to Chromium's own. */
function logTabDrag(stage: string, detail: Record<string, unknown> = {}): void {
  if (!getBridge()?.tabDragDebug) return;
  console.info(`[tab-drag ${getAppWindow()?.label ?? "?"}] ${stage}`, detail);
}

/** The drag's carried types, readable even in protected mode. */
function dragTypes(dataTransfer: DataTransfer | null): string[] {
  return dataTransfer ? Array.from(dataTransfer.types) : [];
}

/** A generated event, optionally callable with a target to scope its binding. */
type WindowScopedEvent<T> = {
  (target: TauriWindow): { listen: (callback: EventCallback<T>) => Promise<UnlistenFn> };
  listen: (callback: EventCallback<T>) => Promise<UnlistenFn>;
};

/**
 * Binds a window event to the window this webview belongs to.
 *
 * Tauri matches JS listeners by the target they registered with, and the
 * generated `events.x.listen()` helper registers as `EventTarget::Any` — which
 * the backend hands every emit, including one addressed to a different window.
 * Unscoped, a window consumes handoffs meant for its siblings: a pooled window
 * would adopt a tab another pool window was given, and the strip's preview
 * would answer traffic that belongs elsewhere. Listening through the window
 * object keeps this window's traffic to itself. Note this is no defence
 * against a broadcast — that path applies no filter at all — so the handoff
 * has to be addressed on the sending side too.
 */
function listenInThisWindow<T>(
  event: WindowScopedEvent<T>,
  handler: EventCallback<T>,
): Promise<UnlistenFn> {
  const appWindow = getAppWindow();
  // The browser preview bridge has no native window to scope to, so the dev
  // plumbing falls back to the global listener.
  return (appWindow ? event(appWindow) : event).listen(handler);
}

const WORKSPACE_TAB_ICONS = {
  overview: HomeIcon,
  recents: HistoryIcon,
  favorites: StarIcon,
  trash: TrashBinTrashIcon,
  space: WidgetIcon,
} as const;

/** The plate that tracks the current tab, and the reason a tab switch is a
 *  TRAVEL rather than a swap.
 *
 *  The pill is positioned from the active chip's own box rather than derived
 *  from its index. The strip reorders live under a drag, scrolls, and gains and
 *  loses tabs; every one of those would need re-deriving to place the plate by
 *  arithmetic, and none of them needs a thought when you measure the element
 *  that is actually on screen. One `getBoundingClientRect` per change.
 *
 *  Written straight to the node instead of through state because a re-render
 *  of the strip on every measurement would re-render every chip's title, icon
 *  and close button to move one decoration — the same budget the drag loop
 *  spends its reorders from.
 *
 *  `animate` is the difference between the plate sliding to a tab and
 *  teleporting. A cold strip and a resize want the second — an entrance that
 *  sweeps in from the strip's origin reads as a bug the user has to watch once
 *  per window — so those paths flush the new geometry with the transition held
 *  off, and the flush (`void pill.offsetWidth`) is what makes the hold land
 *  before the transition goes back on. Only a real change of active tab is
 *  worth animating. */
function useTabPill({
  activeTabId,
  pillRef,
  stripRef,
  tabs,
}: {
  activeTabId: string;
  pillRef: RefObject<HTMLSpanElement | null>;
  stripRef: RefObject<HTMLDivElement | null>;
  tabs: ExplorerTab[];
}) {
  const measuredRef = useRef(false);

  const measure = useCallback(
    (animate: boolean) => {
      const strip = stripRef.current;
      const pill = pillRef.current;
      if (!strip || !pill) return;

      const active = strip.querySelector<HTMLElement>("[data-tab-active='true']");
      if (!active) {
        // Every tab closed: the plate has nothing to sit under.
        pill.dataset.ready = "false";
        return;
      }

      if (!animate) pill.dataset.hold = "true";
      const tabBox = active.getBoundingClientRect();
      const stripBox = strip.getBoundingClientRect();
      // `left: 0` on an absolutely positioned child of a scroller resolves
      // against the scroller's PADDING box, and the child scrolls with the
      // content — so the offset is taken from that same edge (`clientLeft` and
      // `clientTop` skip the border) and put back by however far the strip
      // currently is scrolled. The result is scroll-invariant, which is why
      // scrolling the strip needs no remeasure.
      pill.style.width = `${tabBox.width}px`;
      pill.style.height = `${tabBox.height}px`;
      pill.style.transform = `translate3d(${
        tabBox.left - stripBox.left - strip.clientLeft + strip.scrollLeft
      }px, ${tabBox.top - stripBox.top - strip.clientTop + strip.scrollTop}px, 0)`;
      pill.dataset.ready = "true";
      if (!animate) {
        void pill.offsetWidth;
        pill.dataset.hold = "false";
      }
    },
    [pillRef, stripRef],
  );

  useLayoutEffect(() => {
    const strip = stripRef.current;
    if (!strip) return;

    const first = !measuredRef.current;
    measuredRef.current = true;
    measure(!first);

    // A ResizeObserver rather than a window listener: what moves the plate is
    // the strip changing size — a scroll button appearing, the window
    // resizing — and the observer is rebuilt here, so it always watches the
    // chip that is current now rather than the one that was on mount.
    const observer = new ResizeObserver(() => measure(false));
    observer.observe(strip);
    const active = strip.querySelector<HTMLElement>("[data-tab-active='true']");
    if (active) observer.observe(active);
    return () => observer.disconnect();
  }, [measure, stripRef, tabs, activeTabId]);
}

export function ExplorerTabs() {
  const { t } = useTranslation("explorer");
  const tabs = useAtomValue(tabsAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const createTab = useSetAtom(createTabAtom);
  const activateTab = useSetAtom(activateTabAtom);
  const closeTab = useSetAtom(closeTabAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const setCommandBarOpen = useSetAtom(commandBarOpenAtom);
  const setCommandBarMode = useSetAtom(commandBarModeAtom);
  const commandBarBinding = formatBinding(useBinding("app.commandBar"));
  const terminalVisible = useAtomValue(terminalVisibleAtom);
  const stripRef = useRef<HTMLDivElement>(null);
  const pillRef = useRef<HTMLSpanElement>(null);
  const [canScroll, setCanScroll] = useState({ left: false, right: false });
  const [draggingTabId, setDraggingTabId] = useState<string | null>(null);
  const [dropGeometry, setDropGeometry] = useState<DropIndicatorGeometry | null>(null);
  const dragSession = useRef<TabDragSession | null>(null);
  const autoScrollFrame = useRef<number | null>(null);
  // Keep the terminal mounted after its first reveal so the PTY session
  // survives being hidden; before that there is nothing to keep alive.
  const [terminalMounted, setTerminalMounted] = useState(terminalVisible);

  useEffect(() => {
    if (terminalVisible) setTerminalMounted(true);
  }, [terminalVisible]);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  // A tab dropped from another window is inserted by the drop handler itself
  // (see the drag and drop effect below): the drop carries the whole tab, so
  // the window that receives it is the one that installs it, and the sending
  // window closes its copy when the claim comes back. The old
  // `tab-merged-into-window` event has no listener left to feed; the shell
  // command behind it stays as part of the command surface.

  // A pooled window adopts the torn-off tab instead of merging it: it has been
  // running with its own default tab, which the tear-off replaces rather than
  // joins. Skips the insertion-index work below, which only positions a tab
  // among others.
  useEffect(() => {
    let cancelled = false;
    const unlisten = listenInThisWindow(events.tabAdoptedIntoWindow, (event) => {
      try {
        adoptTabFromHandoff(event.payload.payload);
        void getAppWindow()?.setFocus();
      } catch (error) {
        console.error("Failed to adopt a tab into a pooled window", error);
      }
    });
    // Readiness is reported only once the listener above exists, because Rust
    // will not hand a tab over before this call and an adoption emitted at a
    // page that is not listening is simply lost. Passive effects run after the
    // first paint, so this deliberately does not live in the bootstrap frame.
    void unlisten.then(() => {
      if (cancelled) return;
      const isPool = (globalThis as typeof globalThis & { __DAE_POOL_WINDOW?: number })
        .__DAE_POOL_WINDOW;
      if (isPool === 1) {
        void commands.poolWindowReady(getAppWindow()?.label ?? "");
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((unlisten) => unlisten());
    };
  }, []);

  // The tab strip's drag and drop, in one place. The browser owns the gesture
  // itself — its drag loop is the only one that reaches a window this page has
  // no pointer events for, which on Wayland is every window but the source — so
  // what is assembled here is the window's end of it: the live reorder, the
  // insertion preview for a tab arriving from another window, and the two ways
  // a drag can end. The wire format and the measuring live in `tab-drag.ts`.
  useEffect(() => {
    // How many elements of this window the drag is currently over. Chrome
    // reports a null `relatedTarget` on every `dragleave`, so the count is the
    // only reliable way to tell "moved to another child" from "left the
    // window" — the same depth counter the preload keeps for file drags.
    let dragDepth = 0;
    // One `over` line per drag is enough to prove dragover is arriving; the
    // event itself fires many times a second.
    let overLogged = false;

    const cancelAutoScroll = () => {
      if (autoScrollFrame.current === null) return;
      cancelAnimationFrame(autoScrollFrame.current);
      autoScrollFrame.current = null;
    };

    /** The chip being dragged in this window, if the drag started here. The
     *  `data-dragging` mark is what other windows lack, so it is also the
     *  test for "this drag is ours"; the id is the fallback for the first
     *  frame, before the chip's own state has been committed. */
    const draggedTab = () => {
      const strip = stripRef.current;
      const tabId = dragSession.current?.tabId;
      if (!strip || !tabId) return null;
      return (
        strip.querySelector<HTMLElement>('[data-dragging="true"]') ??
        strip.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(tabId)}"]`)
      );
    };

    const reorderToPointer = (session: TabDragSession) => {
      const strip = stripRef.current;
      const dragged = draggedTab();
      if (!strip || !dragged) return;
      const index = tabReorderIndexAt(strip, session.lastX, dragged);
      if (index === session.index) return;
      session.index = index;
      moveTab(session.tabId, index);
    };

    /** One frame of edge scrolling. The tabs move under a cursor that is not
     *  moving at all, so the reorder slot is recomputed from the last known
     *  position instead of from a new event. */
    const autoScroll = () => {
      autoScrollFrame.current = null;
      const session = dragSession.current;
      const strip = stripRef.current;
      if (!session || !strip) return;
      const box = strip.getBoundingClientRect();
      // Scrolling is the strip's, not the window's: a cursor parked at the same
      // `x` in the listing is over no scroller that should follow it.
      if (session.lastY < box.top || session.lastY > box.bottom) return;
      const step = stripAutoScrollStep(strip, session.lastX);
      if (step === 0) return;
      strip.scrollLeft += step;
      reorderToPointer(session);
      autoScrollFrame.current = requestAnimationFrame(autoScroll);
    };

    const follow = (session: TabDragSession, x: number, y: number) => {
      session.lastX = x;
      session.lastY = y;
      const strip = stripRef.current;
      if (!strip) return;
      const box = strip.getBoundingClientRect();
      const inStrip = y >= box.top && y <= box.bottom && x >= box.left && x <= box.right;
      if (inStrip) reorderToPointer(session);
      const scrolling = inStrip && stripAutoScrollStep(strip, x) !== 0;
      if (scrolling) {
        autoScrollFrame.current ??= requestAnimationFrame(autoScroll);
      } else {
        cancelAutoScroll();
      }
    };

    const begin = (event: DragEvent) => {
      dragDepth = 0;
      overLogged = false;
      const element =
        event.target instanceof Element
          ? event.target.closest<HTMLElement>('[role="tab"][data-tab-id]')
          : null;
      const tabId = element?.dataset.tabId;
      if (!element || !tabId || !event.dataTransfer) {
        logTabDrag("start-ignored", { types: dragTypes(event.dataTransfer) });
        return;
      }

      // A press that starts on the close button is a click on it, not a tab
      // being lifted: the button sits inside the chip, so without this the
      // browser would hand the gesture to the chip and swallow the click.
      if (event.target instanceof Element && event.target.closest("button")) {
        event.preventDefault();
        logTabDrag("start-cancelled", { reason: "close-button" });
        return;
      }

      const bounds = element.getBoundingClientRect();
      const token = crypto.randomUUID();
      const session: TabDragSession = {
        tabId,
        token,
        grabX: event.clientX - bounds.left,
        grabY: event.clientY - bounds.top,
        lastX: event.clientX,
        lastY: event.clientY,
        leftAt: null,
        leftWindow: false,
        landed: false,
        claimed: false,
        index: -1,
      };
      dragSession.current = session;
      trackOutgoingTabHandoff(token, tabId);
      setDraggingTabId(tabId);
      setDropGeometry(null);
      activateTab(tabId);
      event.dataTransfer.effectAllowed = "move";
      writeTabDrag(event.dataTransfer, {
        source: getAppWindow()?.label ?? "",
        token,
        payload: serializeTabHandoff(tabId),
      });
      setTabDragImage(event.dataTransfer, element, session.grabX, session.grabY);
      logTabDrag("start", { tabId, types: dragTypes(event.dataTransfer) });
    };

    const over = (event: DragEvent) => {
      if (!isTabDrag(event.dataTransfer)) return;
      // A tab is welcome anywhere in this window: a drop that misses the strip
      // keeps it, it does not cancel the gesture. Only `drop` decides what it
      // means, so the whole window is a valid target for the platform.
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";

      const session = dragSession.current;
      if (!overLogged) {
        overLogged = true;
        logTabDrag("over", {
          own: Boolean(session),
          types: dragTypes(event.dataTransfer),
          x: Math.round(event.clientX),
        });
      }
      if (session) {
        follow(session, event.clientX, event.clientY);
        return;
      }

      const geometry = dropIndicatorGeometryAt(event.clientX);
      if (!geometry) return;
      setDropGeometry((previous) =>
        previous && Math.abs(previous.left - geometry.left) < 0.5 ? previous : geometry,
      );
    };

    const enter = (event: DragEvent) => {
      if (!isTabDrag(event.dataTransfer)) return;
      dragDepth += 1;
      overLogged = false;
      logTabDrag("enter", { depth: dragDepth, types: dragTypes(event.dataTransfer) });
      const session = dragSession.current;
      // A drag back in from outside is no longer a tear-off candidate.
      if (session) {
        session.leftWindow = false;
        session.leftAt = null;
      }
    };

    const leave = (event: DragEvent) => {
      if (!isTabDrag(event.dataTransfer)) return;
      dragDepth = Math.max(0, dragDepth - 1);
      if (dragDepth > 0) return;
      logTabDrag("leave-window", { own: Boolean(dragSession.current) });

      const session = dragSession.current;
      if (session) {
        session.leftWindow = true;
        session.leftAt = performance.now();
        cancelAutoScroll();
        return;
      }
      setDropGeometry(null);
    };

    const drop = (event: DragEvent) => {
      if (!isTabDrag(event.dataTransfer)) return;
      dragDepth = 0;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";

      const session = dragSession.current;
      if (session) {
        session.landed = true;
        follow(session, event.clientX, event.clientY);
        cancelAutoScroll();
        logTabDrag("drop-own", { x: Math.round(event.clientX) });
        return;
      }

      setDropGeometry(null);
      const handoff = readTabDrag(event.dataTransfer);
      logTabDrag("drop-foreign", {
        parsed: Boolean(handoff),
        types: dragTypes(event.dataTransfer),
        x: Math.round(event.clientX),
      });
      const appWindow = getAppWindow();
      const strip = stripRef.current;
      if (!handoff || !appWindow) return;

      // The tab is inserted here, and only then claimed: the sending window
      // gives its copy up when it hears the claim, so a handoff that cannot be
      // inserted has to land somewhere on this side first. A window of its own
      // keeps the tab reachable instead of losing it.
      const index = strip ? tabInsertionIndexAt(strip, event.clientX) : Number.MAX_SAFE_INTEGER;
      try {
        mergeTabFromHandoff(handoff.payload, index);
        void appWindow.setFocus();
      } catch (error) {
        console.error("Failed to insert a dropped tab", error);
        void commands
          .tearOffTab(appWindow.label, handoff.payload, 0, 0, null, null)
          .catch((recovery) => console.error("Failed to recover a dropped tab", recovery));
      }
      void claimTabHandoff(handoff.source, handoff.token);
    };

    /** Where a drag ended, once every answer that can still arrive is in. */
    const settle = async (session: TabDragSession) => {
      // A claim that beat the drag's own end is the answer already.
      if (session.claimed) {
        closeTab(session.tabId);
        return;
      }

      // Did the release happen over this window? A platform that reports the
      // drop says so outright. Wayland does not — a drop inside the source
      // window arrives as `dragleave` + `dragend`, with no `drop` and no
      // effect — so the last departure is the evidence: a teardown leave is
      // still fresh when `dragend` runs, a real one is followed by travel.
      const leftSince = session.leftAt === null ? null : performance.now() - session.leftAt;
      if (session.landed || leftSince === null || leftSince <= IN_WINDOW_RELEASE_GRACE_MS) {
        // The live reorder has already moved the tab; only the final slot was
        // missed when no `drop` announced the release.
        reorderToPointer(session);
        logTabDrag("settle-in-window", {
          landed: session.landed,
          leftAgoMs: leftSince === null ? null : Math.round(leftSince),
        });
        return;
      }

      // Another window may still be claiming the tab, and that claim crosses
      // two renderers and the shell: wait for it before deciding the tab was
      // dropped on nothing.
      if (await waitForClaimedTabHandoff(session.token, HANDOFF_CLAIM_GRACE_MS)) return;
      // A pointer that never left this window ended on a cancelled drag
      // (Escape, on a session whose platform owns the keyboard): cancelling
      // puts the tab back rather than casting it out.
      if (!session.leftWindow) return;
      logTabDrag("settle-tear-off", { leftAgoMs: Math.round(leftSince) });
      void detachTab(getAppWindow(), session.tabId, session.grabX, session.grabY, closeTab);
    };

    const end = (event: DragEvent) => {
      dragDepth = 0;
      const session = dragSession.current;
      if (!session) return;
      dragSession.current = null;
      setDraggingTabId(null);
      cancelAutoScroll();
      logTabDrag("end", {
        landed: session.landed,
        leftWindow: session.leftWindow,
        claimed: session.claimed,
        dropEffect: event.dataTransfer?.dropEffect ?? null,
        leftAgoMs:
          session.leftAt === null ? null : Math.round(performance.now() - session.leftAt),
      });

      // A reported drop is already settled: the strip reordered the tab and it
      // stays here. Everything else has to be decided, because the platform's
      // drag result cannot be trusted to say where the release happened.
      if (session.landed) return;
      void settle(session);
    };

    window.addEventListener("dragstart", begin, true);
    window.addEventListener("dragenter", enter, true);
    window.addEventListener("dragover", over, true);
    window.addEventListener("dragleave", leave, true);
    window.addEventListener("drop", drop, true);
    window.addEventListener("dragend", end, true);
    return () => {
      window.removeEventListener("dragstart", begin, true);
      window.removeEventListener("dragenter", enter, true);
      window.removeEventListener("dragover", over, true);
      window.removeEventListener("dragleave", leave, true);
      window.removeEventListener("drop", drop, true);
      window.removeEventListener("dragend", end, true);
      cancelAutoScroll();
    };
  }, [activateTab, closeTab]);

  // The other end of a handoff: a window that took one of this window's tabs
  // says so through the shell, and this is where that answer becomes a closed
  // tab. Mounted for the window's lifetime rather than per drag, because the
  // answer can arrive after the drag that sent it is over.
  useEffect(() => {
    const appWindow = getAppWindow();
    if (!appWindow) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    void appWindow.listen<TabHandoffClaim>("tab-handoff-claimed", ({ payload }) => {
      const token = payload?.token;
      if (typeof token !== "string") return;
      const tabId = takeClaimedTabHandoff(token);
      if (!tabId) return;
      logTabDrag("claim", { token, tabId, target: payload?.target ?? null });

      // A drag of this very tab may still be running: hand the close to
      // `settle`, which runs once the platform's gesture is over.
      const session = dragSession.current;
      if (session && session.tabId === tabId) {
        session.claimed = true;
        return;
      }
      closeTab(tabId);
    }).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [closeTab]);

  const syncScrollButtons = useCallback(() => {
    const strip = stripRef.current;
    if (!strip) return;
    const maxScrollLeft = strip.scrollWidth - strip.clientWidth;
    const left = strip.scrollLeft > 1;
    const right = strip.scrollLeft < maxScrollLeft - 1;
    // The strip fires `scroll` for every pixel of auto-scroll and tab-drag
    // reordering; only an actual change of either edge is worth a render, so
    // an unchanged answer returns the previous object and React bails out.
    setCanScroll((previous) =>
      previous.left === left && previous.right === right ? previous : { left, right },
    );
  }, []);

  useEffect(() => {
    syncScrollButtons();
    window.addEventListener("resize", syncScrollButtons);
    return () => window.removeEventListener("resize", syncScrollButtons);
  }, [syncScrollButtons, tabs.length]);

  useTabPill({ activeTabId, pillRef, stripRef, tabs });

  const scrollStrip = (direction: 1 | -1) => {
    stripRef.current?.scrollBy({ left: direction * TAB_STRIP_SCROLL_AMOUNT, behavior: "smooth" });
  };

  return (
    <div className="flex h-full flex-col">
      <TabDropIndicator geometry={dropGeometry} />
      {/* Window chrome: one 44px bar carrying the tab trough and the native
          window controls, painted in the frame tone (`bg-chrome`, the rail's
          own rung). The bar is the top of the L of chrome the window is drawn
          as, so the idle chips are bare labels on it — no dividers, no fills —
          and the travelling pill under the current tab is the only surface up
          here. Under a window material the frame stops painting and the
          platform's backdrop shows through it (App.css, THE WINDOW-MATERIAL
          SEAM). */}
      <header
        className="flex h-tab-strip shrink-0 items-stretch bg-chrome"
        data-tab-bar="true"
        data-tauri-drag-region="deep"
      >
        {/* The window's own mark, in the drag region before the first tab. It
            is outside the tablist on purpose: the strip measures its chips by
            `[data-tab-active]` and routes drags by hit-testing them, so a
            decoration that joined the row would have to be excluded from both.
            `deep` on the header already makes it part of the window's drag
            surface, which is the only interaction a mark should have. */}
        <span aria-hidden="true" className="brand-stripe ml-3.5 self-center" />
        {/* The wordmark is the other half of the reference's `.mark`: the
            stripes carry the motif and the name says whose window it is. The
            strip's first tab used to sit alone against the stripes with 4px
            between them, which read as a stray chip rather than as the tab
            row's start. */}
        <span aria-hidden="true" className="brand-wordmark mr-2.5 ml-2.5 self-center">
          dae
        </span>
        <StripScrollButton
          aria-label={t("tabs.scrollLeft")}
          direction={-1}
          onClick={() => scrollStrip(-1)}
          visible={canScroll.left}
        />
        <div
          ref={stripRef}
          aria-label={t("tabs.ariaLabel")}
          // `mt-2` and no bottom margin, so the tab row sits 8px below the
          // window edge and 8px above the content row: that row's own `p-2`
          // gutter supplies the space underneath, and the two sides have to
          // measure the same or the row reads as pinned to the top.
          className="tab-trough relative my-1 flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto px-1 scrollbar-none [&::-webkit-scrollbar]:hidden"
          // The header above is a `deep` drag region, so the strip has to opt
          // out explicitly: a tab is dragged as a tab, never as the window.
          data-tauri-drag-region="false"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {/* The plate under the current tab, and the node that makes the switch
              a TRAVEL rather than a swap — see `useTabPill`. It is the strip's
              first child so the chips paint over it; `pointer-events: none`
              keeps it out of the gesture. */}
          <span aria-hidden="true" className="tab-pill" data-hold="true" ref={pillRef} />
          {tabs.map((tab) => (
            <TabStripItem
              isActive={tab.id === activeTabId}
              isDragging={tab.id === draggingTabId}
              key={tab.id}
              tab={tab}
            />
          ))}
          {/* Inside the scroller, hard after the last chip: the plus belongs to
              the tab row, so it travels with it and sits one tab-gap away from
              the tab it would extend. The trade-off is that a tablist now owns
              a button; the alternative — a `role="tablist"` wrapper inside the
              scroller — would move the role off the element `tab-drag.ts`
              measures and scrolls. */}
          <button
            aria-label={t("tabs.newTab")}
            className="flex size-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors duration-fast hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/35 focus-visible:outline-none"
            data-tauri-drag-region="false"
            onClick={createTab}
            title={t("tabs.newTabShortcut", { modifier: MOD_KEY })}
            type="button"
          >
            <AddIcon className="size-3.5" />
          </button>
        </div>
        <StripScrollButton
          aria-label={t("tabs.scrollRight")}
          direction={1}
          onClick={() => scrollStrip(1)}
          visible={canScroll.right}
        />
        {/* The reference puts the palette's trigger in the title bar, not in
            the toolbar: it is a *window* action, available from every surface,
            and it earns the one capsule the strip spends. The overview's own
            field remains the discoverable version of the same gesture. */}
        <button
          aria-label={t("titleBar.commandBarAria")}
          className="flex h-7 shrink-0 items-center gap-2 rounded-full border border-border px-2.5 text-caption text-muted-foreground transition-[border-color,color] duration-fast ease-standard hover:border-foreground/16 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/35 focus-visible:outline-none max-[860px]:hidden"
          data-tauri-drag-region="false"
          onClick={() => {
            setCommandBarMode("commands");
            setCommandBarOpen(true);
          }}
          title={t("titleBar.commandBarTitle", { shortcut: commandBarBinding })}
          type="button"
        >
          <MagnifierIcon className="size-3.5 shrink-0" />
          <span className="max-w-40 truncate">{t("titleBar.commandBarShort")}</span>
          <Kbd className="shrink-0">{commandBarBinding}</Kbd>
        </button>
        <WindowControls />
      </header>

      {/* FLAT WINDOW: the reference draws no paper on this row at all — the rail
          and the listing sit on the window's own void, divided by whitespace and
          by the toolbar's hairline. The paper plate (`content-panel`) and the
          10px gutters it needed are gone with it, which returns 20px of width
          and 20px of height to the listing.

          The row is three columns: `rail | main | inspector`. The inspector is
          a sibling of `main`, not a child of it, so the terminal drawer and the
          status strip — both of which live in `main` — end at its hairline
          instead of running under it. Which pane's inspector that column shows
          is arbitrated by `inspectorAtom` (the focused pane publishes; see
          `inspector-panel.tsx`). */}
      <div className="flex min-h-0 flex-1 bg-chrome">
        <Sidebar />
        {/* The document plane. It paints the canvas at full strength on purpose
            (`bg-background`, which stays opaque under a window material — see
            THE WINDOW-MATERIAL SEAM in App.css): the rail and the tab strip
            above are the two regions the platform's backdrop is allowed
            through, and everything the user *reads* — toolbar, listing,
            terminal drawer, status strip, inspector — sits on this plate.
            Without it the whole row is `bg-chrome`, i.e. nothing paints, and
            the material blurs the entire window. */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
          {/* Chrome-style keep-alive: every tab's surface stays mounted and
              the surfaces are stacked; switching tabs flips visibility
              instead of swapping the key, so nothing remounts — directory
              listings, sort sessions and scroll positions survive the
              switch, and switch latency stops scaling with directory size.
              Inactive layers keep their geometry (TanStack virtual's
              measurements stay valid) while skipping paint and hit-testing;
              their keyboard shortcuts are gated through the `active` prop
              chain (see workspace-surface.tsx). */}
          <div className="relative min-h-0 flex-1 overflow-hidden">
            {tabs.map((tab) => {
              const isActive = tab.id === activeTabId;
              return (
                <div
                  key={tab.id}
                  aria-hidden={!isActive}
                  className={cn(
                    "absolute inset-0 flex min-h-0 flex-col",
                    isActive ? "visible" : "invisible pointer-events-none",
                  )}
                >
                  <TabPerfProfiler id="Surface">
                    <WorkspaceSurfaceView active={isActive} tabId={tab.id} />
                  </TabPerfProfiler>
                </div>
              );
            })}
          </div>
          {terminalMounted && (
            <Suspense fallback={null}>
              <TerminalPanel />
            </Suspense>
          )}
          {/* The reference's order, bottom of the main column: the terminal
              drawer, then the status strip. The strip used to be painted by
              the pane, which put it ABOVE the drawer — a status line sandwiched
              between the list and the shell it describes. The focused pane
              publishes the facts (see `explorerStatusAtom`); the window owns the
              row, and `FocusedPaneStatusBar` is the only subscriber. */}
          <FocusedPaneStatusBar />
        </div>
        {/* The reference's third column: full height beside `main`, so the
            inspector runs from the tab strip down past the terminal drawer. */}
        <InspectorPanel />
      </div>
    </div>
  );
}

function StripScrollButton({
  "aria-label": ariaLabel,
  direction,
  onClick,
  visible,
}: {
  "aria-label": string;
  direction: 1 | -1;
  onClick: () => void;
  visible: boolean;
}) {
  const Icon = direction === -1 ? AltArrowLeftIcon : AltArrowRightIcon;

  // A scroll button that has nothing to scroll belongs to no layout: an
  // `invisible` one still claimed 24px at the head of the strip, which pushed
  // the first tab a whole gutter away from the wordmark. Mounting it only when
  // it can act costs a reflow on overflow, which is the moment the row is
  // re-flowing anyway.
  if (!visible) return null;

  return (
    <button
      aria-label={ariaLabel}
      className={cn(
        "flex w-6 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
      )}
      onClick={onClick}
      type="button"
    >
      <Icon className="size-3.5" />
    </button>
  );
}

/** Live insertion preview while a tab from another window is dragged over
 * this one: the whole window gains a subtle accept ring and the tab strip
 * shows where the tab would land. Mounts nothing until the first hover.
 *
 *  The geometry is measured here rather than in the drag plumbing that feeds
 *  it, because the answer is a fact about *this* window's strip and layout —
 *  the drag itself only reports where the pointer is. */
function TabDropIndicator({ geometry }: { geometry: DropIndicatorGeometry | null }) {
  if (!geometry) return null;

  return (
    <>
      <div
        aria-hidden="true"
        className="pointer-events-none fixed inset-0 z-40 rounded-lg ring-2 ring-primary/50 ring-inset"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none fixed z-50 w-0.5 rounded-full bg-primary"
        style={{ left: geometry.left - 1, top: geometry.top, height: geometry.height }}
      />
    </>
  );
}

function surfaceTitle(
  surface: WorkspaceSurface,
  folderTitle: string,
  spaceName: string | undefined,
  t: TFunction,
): string {
  switch (surface.kind) {
    case "overview":
      return t("tabs.overview");
    case "recents":
      return t("tabs.recents");
    case "favorites":
      return t("tabs.favorites");
    case "trash":
      return t("tabs.trash");
    case "space":
      return spaceName ?? t("tabs.space");
    case "folder":
      return folderTitle;
  }
}

/** Puts the tab in a window of its own and drops it from this strip.
 *
 *  `grabX`/`grabY` keep the same point of the window pinned under the cursor.
 *  No cursor point is passed: the drop that tears a tab off is a release the
 *  platform's drag loop owns, so on Wayland (where a client can neither read
 *  the global pointer nor place its own toplevels) the shell has nothing to
 *  aim at and the compositor decides where the window lands. On X11 the shell
 *  reads the live pointer itself, which at this instant is still the release
 *  point. */
async function detachTab(
  appWindow: TauriWindow | null,
  tabId: string,
  grabX: number,
  grabY: number,
  closeTab: (tabId: string) => void,
): Promise<void> {
  // The browser preview bridge has no native windows to create.
  if (!appWindow) return;
  try {
    await commands.tearOffTab(appWindow.label, serializeTabHandoff(tabId), grabX, grabY, null, null);
    closeTab(tabId);
  } catch (error) {
    console.error("Failed to detach tab", error);
  }
}

function TabStripItem({
  isActive,
  isDragging,
  tab,
}: {
  isActive: boolean;
  isDragging: boolean;
  tab: ExplorerTab;
}) {
  const { t } = useTranslation("explorer");
  const activateTab = useSetAtom(activateTabAtom);
  const closeTab = useSetAtom(closeTabAtom);
  const surface = useAtomValue(tabSurfaceFamily(tab.id));
  const splitEnabled = useAtomValue(splitEnabledFamily(tab.id));
  const spaces = useAtomValue(spacesAtom);
  const navigator = getTabNavigator(tab.id);
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const splitNavigator = getSplitNavigator(tab.id);
  const splitState = useSyncExternalStore(splitNavigator.subscribe, splitNavigator.getSnapshot);
  const directory = state.directory;
  const splitDirectory = splitState.directory;
  const spaceName =
    surface.kind === "space"
      ? spaces?.find((space) => space.id === surface.spaceId)?.name
      : undefined;
  const folderTitle =
    splitEnabled && surface.kind === "folder"
      ? `${directory?.breadcrumbs.at(-1)?.name ?? t("tabs.loading")} · ${
          splitDirectory?.breadcrumbs.at(-1)?.name ?? t("tabs.loading")
        }`
      : (directory?.breadcrumbs.at(-1)?.name ?? t("tabs.loading"));
  const title = surfaceTitle(surface, folderTitle, spaceName, t);
  const elementRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isActive) {
      elementRef.current?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }, [isActive]);

  // Folder tabs carry the folder's type glyph; workspace surfaces keep their
  // Solar UI glyphs, which are app chrome rather than file types.
  const FolderTabIcon = getFolderPresentation().icon;
  const WorkspaceTabIcon = surface.kind === "folder" ? null : WORKSPACE_TAB_ICONS[surface.kind];
  const tabContent = (
    <>
      {FolderTabIcon ? (
        <FolderTabIcon className="ml-2 size-[15px] shrink-0" />
      ) : WorkspaceTabIcon ? (
        <WorkspaceTabIcon className="ml-2 size-[15px] shrink-0 text-muted-foreground" />
      ) : null}
      <span className="min-w-0 truncate pr-6 pl-1.5">{title}</span>
    </>
  );

  return (
    <div
      aria-grabbed={isDragging}
      aria-selected={isActive}
      className={cn(
        // Linear tab: a compact 32px bare label floating in the trough. Idle
        // tabs carry no shell of their own — a hairline between neighbours
        // (App.css, THE DIVIDER) is all that separates them, and the persistent
        // `.tab-pill` behind the current one is the single raised plate in
        // the row, sliding to whichever tab you click. So the active tab's
        // only job here is ink and weight; the state layer's wash is the
        // idle tab's whole hover.
        //
        // The chip is `draggable`, so the platform owns the gesture from the
        // first few pixels of travel: it runs the drag loop, carries the image
        // and reports the drop, which is what lets a tab cross into another
        // window at all. `data-dragging` marks the one being carried, and the
        // `data-pressed` wash is driven by the drag rather than by `:active`:
        // the drag swallows the release that would clear `:active`, and a chip
        // still wearing that wash reads as a tab still held down after it was
        // put back.
        "group state-layer relative flex h-[30px] max-w-[230px] min-w-[6.5rem] shrink-0 touch-none cursor-grab items-center rounded-md text-caption select-none transition-[background-color,color,opacity] duration-fast ease-standard data-[pressed=true]:cursor-grabbing",
        isActive ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
        isDragging && "opacity-30",
      )}
      data-dragging={isDragging ? "true" : "false"}
      data-pressed={isDragging ? "true" : "false"}
      // The plate's anchor: `useTabPill` finds the chip to measure by this
      // attribute, so it never has to know which tab is current a second time.
      data-tab-active={isActive ? "true" : "false"}
      data-tab-id={tab.id}
      data-tauri-drag-region="false"
      draggable
      // The middle button is every browser's and every file manager's "close
      // this tab", and it costs nothing here: a left-button drag never reaches
      // this handler, so this is the only one a middle click can hit.
      onAuxClick={(event) => {
        if (event.button !== 1) return;
        event.preventDefault();
        closeTab(tab.id);
      }}
      onClick={() => activateTab(tab.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          activateTab(tab.id);
        }
      }}
      ref={elementRef}
      role="tab"
      tabIndex={0}
      title={
        surface.kind === "folder"
          ? splitEnabled
            ? `${directory?.path ?? title} · ${splitDirectory?.path ?? ""}`
            : (directory?.path ?? title)
          : title
      }
    >
      {tabContent}
      <button
        aria-label={t("tabs.closeTab", { title })}
        className={cn(
          "absolute top-1/2 right-1 flex size-[18px] -translate-y-1/2 items-center justify-center rounded-[4px] transition-colors hover:bg-accent",
          isActive
            ? "text-muted-foreground hover:text-foreground"
            : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
        onClick={(event) => {
          event.stopPropagation();
          closeTab(tab.id);
        }}
        onPointerDown={(event) => event.stopPropagation()}
        type="button"
      >
        <CloseIcon className="size-[11px]" />
      </button>
    </div>
  );
}
