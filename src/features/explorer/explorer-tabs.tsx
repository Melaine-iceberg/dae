import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { Window as TauriWindow } from "@tauri-apps/api/window";
import {
  AddIcon,
  AltArrowLeftIcon,
  AltArrowRightIcon,
  CloseIcon,
  HistoryIcon,
  HomeIcon,
  StarIcon,
  TrashBinTrashIcon,
  WidgetIcon,
} from "@solar-icons/react/line-duotone";

import { Button } from "@/components/ui/button";
import { WindowControls } from "@/components/window-controls";
import { commands, events } from "@/bindings";
import { Sidebar } from "@/features/sidebar/sidebar";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
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
  TAB_DRAG_PREVIEW_PAD,
  beginTabDragGesture,
  dropIndicatorGeometryAt,
  tabInsertionIndexAt,
  type DropIndicatorGeometry,
  type NativeDragOutcome,
  type TabDragController,
  type TabDragGhost,
} from "./tab-drag";

const TAB_STRIP_SCROLL_AMOUNT = 512;

/** A generated event, optionally callable with a target to scope its binding. */
type WindowScopedEvent<T> = {
  (target: TauriWindow): { listen: (callback: EventCallback<T>) => Promise<UnlistenFn> };
  listen: (callback: EventCallback<T>) => Promise<UnlistenFn>;
};

/**
 * Binds a tab-drag event to the window this webview belongs to.
 *
 * Tauri matches JS listeners by the target they registered with, and the
 * generated `events.x.listen()` helper registers as `EventTarget::Any` — which
 * the backend hands every emit, including one addressed to a different window.
 * Unscoped, the source window therefore consumes its own handoff (re-inserting
 * the tab it just gave away, which also keeps its last tab from ever reaching
 * zero) and the drop preview lights up in every window. Listening through the
 * window object keeps this window's traffic to itself. Note this is no defence
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

/** The ghost mounts one React commit after the drag threshold, so the
 *  snapshot — which runs at the hand-off (see tab-drag.ts) — finds the portal
 *  mounted and resolves on its first check; the frame budget is the bound for
 *  the unlikely case the commit is still in flight when the cursor leaves the
 *  window almost immediately. */
function waitForDragPreviewPortal(tabId: string, frames = 12): Promise<HTMLElement | null> {
  return new Promise((resolve) => {
    const tick = (remaining: number) => {
      const portal = document.querySelector<HTMLElement>(`[data-tab-drag-preview="${tabId}"]`);
      if (portal) resolve(portal);
      else if (remaining > 0) requestAnimationFrame(() => tick(remaining - 1));
      else resolve(null);
    };
    requestAnimationFrame(() => tick(frames));
  });
}

/** Clones the ghost with every computed style inlined and <img> artwork
 *  embedded as data URLs: an SVG loaded as an image renders in isolation and
 *  can neither apply page stylesheets nor fetch external resources. */
async function inlineSubtree(source: Element, clone: Element): Promise<void> {
  const computed = getComputedStyle(source);
  for (let index = 0; index < computed.length; index++) {
    const property = computed.item(index);
    (clone as HTMLElement | SVGElement).style.setProperty(
      property,
      computed.getPropertyValue(property),
      computed.getPropertyPriority(property),
    );
  }

  if (source instanceof HTMLImageElement && clone instanceof HTMLImageElement) {
    try {
      const blob = await (await fetch(source.src)).blob();
      clone.src = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result as string);
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(blob);
      });
    } catch {
      clone.remove();
    }
  }

  const children = Array.from(source.children);
  const copies = Array.from(clone.children);
  for (let index = 0; index < children.length; index++) {
    await inlineSubtree(children[index]!, copies[index]!);
  }
}

/** Clears every box-shadow {@link inlineSubtree} inlined into the clone.
 *
 *  The native drag image is composited by the OS under the pointer with nothing
 *  behind it, so elevation does not read there as the in-window float it was
 *  drawn for — a rasterized copy of `--shadow-ambient-lg` trails the tab across
 *  the desktop as a stray smear, and its blurred edge is cut off wherever it
 *  overruns the bitmap. Kept as the snapshot's own guarantee rather than as a
 *  reflection of the ghost's current styling, and applied to the whole subtree,
 *  not just the root: a shadow on any descendant would be baked in the same
 *  way, and the snapshot must stay flat whatever the ghost becomes. */
function clearBoxShadows(root: Element): void {
  if (root instanceof HTMLElement) root.style.boxShadow = "none";
  for (const node of root.querySelectorAll<HTMLElement>("*")) {
    node.style.boxShadow = "none";
  }
}

/** Rasterizes the live ghost portal (plus the antialiasing cushion) to a PNG
 *  data URL for the OS drag loop, so the native drag image carries the same tab
 *  the user was dragging in-window. Unlike a DOM portal it is also not clipped
 *  at the WebView boundary. Returns null on any failure; the Rust side then
 *  falls back to the application icon. */
function snapshotTabDragPreview(tabId: string): Promise<string | null> {
  return (async () => {
    const portal = await waitForDragPreviewPortal(tabId);
    if (!portal) return null;

    const bounds = portal.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    const width = bounds.width + TAB_DRAG_PREVIEW_PAD * 2;
    const height = bounds.height + TAB_DRAG_PREVIEW_PAD * 2;

    const clone = portal.cloneNode(true) as HTMLElement;
    await inlineSubtree(portal, clone);
    clearBoxShadows(clone);
    clone.style.position = "absolute";
    clone.style.inset = "auto";
    clone.style.left = `${TAB_DRAG_PREVIEW_PAD}px`;
    clone.style.top = `${TAB_DRAG_PREVIEW_PAD}px`;
    clone.style.margin = "0";
    clone.style.transform = "none";

    // The foreignObject viewport is sized in device pixels; the wrapper lays
    // the clone out in CSS pixels and scales it so text rasterizes crisply at
    // the display's pixel ratio instead of relying on drawImage upscaling.
    // Rounded up, never down: at a fractional display scale a viewport rounded
    // short of the content shaves that fraction off the far edge, and `ceil`
    // only ever costs a transparent pixel that nothing paints.
    const bitmapWidth = Math.ceil(width * scale);
    const bitmapHeight = Math.ceil(height * scale);
    const svgNamespace = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(svgNamespace, "svg");
    svg.setAttribute("width", String(bitmapWidth));
    svg.setAttribute("height", String(bitmapHeight));
    const foreignObject = document.createElementNS(svgNamespace, "foreignObject");
    foreignObject.setAttribute("width", String(bitmapWidth));
    foreignObject.setAttribute("height", String(bitmapHeight));
    const wrapper = document.createElementNS("http://www.w3.org/1999/xhtml", "div");
    wrapper.setAttribute(
      "style",
      `width:${width}px;height:${height}px;transform:scale(${scale});transform-origin:0 0;`,
    );
    wrapper.appendChild(clone);
    foreignObject.appendChild(wrapper);
    svg.appendChild(foreignObject);

    // The serialized SVG reaches the image as a base64 `data:` URL, never a
    // `blob:` object URL: Chromium treats an SVG that contains a
    // `<foreignObject>` as cross-origin when it is loaded from a blob URL, so
    // drawing it taints the canvas below and `toDataURL` throws a
    // `SecurityError` — the snapshot then degraded to the application icon on
    // the OS drag image. A data URL stays origin-clean, and base64 keeps it at
    // 4/3 of the mark-up where percent-encoding tripled it. An SVG loaded as an
    // image is sandboxed — it can neither apply page stylesheets nor fetch
    // external resources — so this remains a purely local rasterization step
    // with no outbound request.
    const svgSource = new XMLSerializer().serializeToString(svg);
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(new Blob([svgSource], { type: "image/svg+xml" }));
    });
    const image = new Image();
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("The drag preview SVG failed to rasterize"));
      image.src = dataUrl;
    });

    const canvas = document.createElement("canvas");
    canvas.width = bitmapWidth;
    canvas.height = bitmapHeight;
    const context = canvas.getContext("2d");
    if (!context) return null;
    context.drawImage(image, 0, 0);
    return canvas.toDataURL("image/png");
  })().catch((error) => {
    // The Rust side falls back to the application icon, which looks enough
    // like a preview to hide a broken snapshot: a tainted canvas or a failed
    // rasterization has to be visible in the log, not just in the drag image.
    console.error("Failed to snapshot the tab drag preview", error);
    return null;
  });
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
 *  Written straight to the node instead of through state for the same reason
 *  the drag ghost is: a re-render of the strip on every measurement would
 *  re-render every chip's title, icon and close button to move one decoration.
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
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const terminalVisible = useAtomValue(terminalVisibleAtom);
  const stripRef = useRef<HTMLDivElement>(null);
  const pillRef = useRef<HTMLSpanElement>(null);
  const [canScroll, setCanScroll] = useState({ left: false, right: false });
  // Keep the terminal mounted after its first reveal so the PTY session
  // survives being hidden; before that there is nothing to keep alive.
  const [terminalMounted, setTerminalMounted] = useState(terminalVisible);

  useEffect(() => {
    if (terminalVisible) setTerminalMounted(true);
  }, [terminalVisible]);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  // Tabs dropped from other windows arrive as window-to-window events; the
  // handoff carries the full tab state and the drop point picks the
  // insertion index within this window's strip.
  useEffect(() => {
    const unlisten = listenInThisWindow(events.tabMergedIntoWindow, (event) => {
      const { payload: handoff, x } = event.payload;
      const strip = stripRef.current;
      const index = strip ? tabInsertionIndexAt(strip, x ?? 0) : Number.MAX_SAFE_INTEGER;
      try {
        mergeTabFromHandoff(handoff, index);
        void getAppWindow()?.setFocus();
      } catch (error) {
        console.error("Failed to merge a tab dropped from another window", error);
      }
    });
    return () => void unlisten.then((unlisten) => unlisten());
  }, []);

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
      <TabDropIndicator />
      {/* Window chrome: one 46px bar carrying the tab trough and the native
          window controls. It paints nothing of its own — `bg-chrome` is
          transparent, so the canvas `body` puts down runs through the frame
          and the content row as one field (see THE WINDOW-MATERIAL SEAM for
          the one case where that canvas is translucent). The idle chips are
          bare labels divided by hairlines; the travelling pill under the
          current tab is the only surface in the bar. */}
      <header
        className="flex h-tab-strip shrink-0 items-stretch bg-chrome"
        data-tab-bar="true"
        data-tauri-drag-region="deep"
      >
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
          className="tab-trough relative mt-2 flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto px-1 scrollbar-none [&::-webkit-scrollbar]:hidden"
          // A tab drag pins its pointer capture here (see `tab-drag.ts`), and the
          // header above is a `deep` drag region, so the strip has to opt out
          // explicitly or the captured events read as a request to move the
          // window.
          data-tauri-drag-region="false"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {/* The plate under the current tab, and the node that makes the switch
              a TRAVEL rather than a swap — see `useTabPill`. It is the strip's
              first child so the chips paint over it; `pointer-events: none`
              keeps it out of the gesture. */}
          <span aria-hidden="true" className="tab-pill" data-hold="true" ref={pillRef} />
          {tabs.map((tab, index) => (
            <TabStripItem key={tab.id} index={index} isActive={tab.id === activeTabId} tab={tab} />
          ))}
          {/* Inside the scroller, hard after the last chip: the plus belongs to
              the tab row, so it travels with it and sits one tab-gap away from
              the tab it would extend. The trade-off is that a tablist now owns
              a button; the alternative — a `role="tablist"` wrapper inside the
              scroller — would move the role off the element `tab-drag.ts`
              measures and scrolls. */}
          {/* The one accent fill in the row. Everything else up here is a
              tonal surface — canvas, paper, hairline — so a small solid square
              is what says "this adds something" without a label. It is the
              house `default` button: flat accent, contact shadow, hover
              brightening 10% toward its own ink, no shape change on press. */}
          <Button
            aria-label={t("tabs.newTab")}
            className="rounded-md"
            data-tauri-drag-region="false"
            onClick={createTab}
            size="icon-sm"
            title={t("tabs.newTabShortcut", { modifier: MOD_KEY })}
            type="button"
          >
            <AddIcon className="size-3.5" />
          </Button>
        </div>
        <StripScrollButton
          aria-label={t("tabs.scrollRight")}
          direction={1}
          onClick={() => scrollStrip(1)}
          visible={canScroll.right}
        />
        <WindowControls />
      </header>

      {/* Flat shell: the sidebar and the two content cards are panels on one
          canvas, each edged by a hairline. The tab bar's contents are inset,
          but the header itself still spans the full width and stays a drag
          region, so native window controls and snap layouts keep working. */}
      <div className="flex min-h-0 flex-1 gap-2 bg-background p-2">
        <Sidebar />
        {/* `bg-background`, not `bg-card`: this is the canvas the panels sit
            on, not a plane in its own right. It reads as nothing while the
            root and `body` paint that canvas for free — and under a window
            material it stops painting, so this row and the tab bar above it
            stay one field (App.css, THE WINDOW-MATERIAL SEAM).

            The air (`p-2 gap-2`) lives on the row now that the nav is a panel
            like the rest: the row's canvas backs every gutter. */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
          {/* Chrome-style keep-alive: every tab's surface stays mounted and
              the surfaces are stacked; switching tabs flips visibility
              instead of swapping the key, so nothing remounts — directory
              listings, sort sessions and scroll positions survive the
              switch, and switch latency stops scaling with directory size.
              Inactive layers keep their geometry (TanStack virtual's
              measurements stay valid) while skipping paint and hit-testing;
              their keyboard shortcuts are gated through the `active` prop
              chain (see workspace-surface.tsx). */}
          <div className="content-panel relative min-h-0 flex-1 overflow-hidden">
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
        </div>
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

  return (
    <button
      aria-label={ariaLabel}
      className={cn(
        "flex w-6 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
        !visible && "invisible",
      )}
      onClick={onClick}
      tabIndex={visible ? 0 : -1}
      type="button"
    >
      <Icon className="size-3.5" />
    </button>
  );
}

/** Live insertion preview while a tab from another window is dragged over
 * this one: the whole window gains a subtle accept ring and the tab strip
 * shows where the tab would land. Mounts nothing until the first hover. */
function TabDropIndicator() {
  const [geometry, setGeometry] = useState<DropIndicatorGeometry | null>(null);

  useEffect(() => {
    const unlistenHover = listenInThisWindow(events.tabDragHover, ({ payload }) => {
      // Release the host's in-flight slot before anything else: the hover
      // monitor holds the next position back until this push has rendered,
      // which is what keeps its `wry::eval` span chain on the host bounded to
      // a single level during a cross-window drag (an unbounded chain is
      // closed recursively, one stack frame per level, when the drag ends).
      void commands.tabDragHoverAck();
      if (payload.x == null) return;
      setGeometry((previous) => {
        const next = dropIndicatorGeometryAt(payload.x as number);
        if (!next) return previous;
        return previous && Math.abs(previous.left - next.left) < 0.5 ? previous : next;
      });
    });
    const unlistenLeave = listenInThisWindow(events.tabDragLeave, () => setGeometry(null));
    const unlistenMerge = listenInThisWindow(events.tabMergedIntoWindow, () => setGeometry(null));

    return () => {
      void unlistenHover.then((unlisten) => unlisten());
      void unlistenLeave.then((unlisten) => unlisten());
      void unlistenMerge.then((unlisten) => unlisten());
    };
  }, []);

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

/** Where the platform's drag loop left a tab that had already left the window.
 *
 *  Only the drop routing can answer where it went: a Wayland session reports
 *  neither a global pointer position nor a window position, so which of this
 *  app's windows accepted the drop *is* the evidence — and the source window
 *  answering for it is the platform saying the tab came back. */
async function runNativeTabDrag(
  source: string,
  preview: string | null,
  grabX: number,
  grabY: number,
  pressX: number,
  pressY: number,
): Promise<NativeDragOutcome> {
  const outcome = await commands.startTabDrag(
    source,
    preview,
    // The grab point travels in the preview bitmap's own pixels, the grid the
    // platform spots the drag image against, so the snapshot's cushion and the
    // device pixel ratio are both part of it.
    (grabX + TAB_DRAG_PREVIEW_PAD) * window.devicePixelRatio,
    (grabY + TAB_DRAG_PREVIEW_PAD) * window.devicePixelRatio,
    // Where the press happened in the window, which is what a Wayland drag
    // takes without settling; see `start_tab_drag`.
    pressX,
    pressY,
  );
  if (!outcome.released || !outcome.outside) return { action: "keep" };
  if (!outcome.target) {
    return { action: "detach", cursor: { x: outcome.cursorX, y: outcome.cursorY } };
  }
  return {
    action: "merge",
    target: outcome.target,
    x: outcome.targetX ?? 0,
    y: outcome.targetY ?? 0,
  };
}

/** Puts the tab in a window of its own and drops it from this strip.
 *
 *  `grabX`/`grabY` keep the same point of the window pinned under the cursor.
 *  `cursor` is the desktop point the platform drag was released over, where the
 *  new window lands; without one the backend reads the live pointer, which
 *  suits a release the WebView saw for itself at a position only it knew. */
async function detachTab(
  appWindow: TauriWindow | null,
  tabId: string,
  grabX: number,
  grabY: number,
  cursor: { x: number; y: number } | null,
  closeTab: (tabId: string) => void,
): Promise<void> {
  // The browser preview bridge has no native windows to create.
  if (!appWindow) return;
  try {
    await commands.tearOffTab(
      appWindow.label,
      serializeTabHandoff(tabId),
      grabX,
      grabY,
      cursor?.x ?? null,
      cursor?.y ?? null,
    );
    closeTab(tabId);
  } catch (error) {
    console.error("Failed to detach tab", error);
  }
}

/** Hands the tab to another of this app's windows, which inserts it at the drop
 *  point, and only then closes it here: a handoff that failed to deliver has to
 *  leave the tab where it is rather than lose it. */
async function mergeTabIntoWindow(
  appWindow: TauriWindow | null,
  tabId: string,
  target: string,
  x: number,
  y: number,
  closeTab: (tabId: string) => void,
): Promise<void> {
  if (!appWindow) return;
  try {
    // Addressed rather than broadcast: the receiving window is the only one that
    // may consume this handoff, and above all the source must not, or it would
    // re-insert the tab it is about to close. The backend does the emitting —
    // an `emitTo` here would run on the async runtime, which can freeze the app
    // on Linux; see the `merge_tab_into_window` command.
    await commands.mergeTabIntoWindow(target, serializeTabHandoff(tabId), x, y);
    closeTab(tabId);
  } catch (error) {
    console.error("Failed to merge the tab into the target window", error);
  }
}

function TabStripItem({
  index,
  isActive,
  tab,
}: {
  index: number;
  isActive: boolean;
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
  const ghostElementRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<TabDragController | null>(null);
  const [dragPreview, setDragPreview] = useState<TabDragGhost | null>(null);
  const [dragActive, setDragActive] = useState(false);
  const [dragPressed, setDragPressed] = useState(false);
  const isDragging = dragActive;

  useEffect(() => {
    if (isActive) {
      elementRef.current?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }, [isActive]);

  useEffect(() => () => dragRef.current?.end(), []);

  // The gesture itself — tracking, the band state machine, the live reorder —
  // is `beginTabDragGesture`'s. What is assembled here is only this tab's end
  // of it: the commands that move it between windows.
  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !event.isPrimary) return;
    if ((event.target as HTMLElement).closest("button")) return;

    const element = elementRef.current;
    if (!element) return;

    event.preventDefault();
    event.stopPropagation();
    activateTab(tab.id);
    dragRef.current?.end();

    const tabId = tab.id;
    const bounds = element.getBoundingClientRect();
    const grabX = event.clientX - bounds.left;
    const grabY = event.clientY - bounds.top;
    const pressX = event.clientX;
    const pressY = event.clientY;
    const appWindow = getAppWindow();

    dragRef.current = beginTabDragGesture({
      tabId,
      element,
      originalIndex: index,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      grabX,
      grabY,
      moveTab,
      onGhost: setDragPreview,
      moveGhost: (x, y) => {
        const ghost = ghostElementRef.current;
        if (ghost) ghost.style.transform = `translate3d(${x}px, ${y}px, 0)`;
      },
      onDragging: setDragActive,
      onPressed: setDragPressed,
      capturePreview: () => snapshotTabDragPreview(tabId),
      readNativeOutside: () =>
        appWindow ? commands.tabDragOutside(appWindow.label) : Promise.resolve(false),
      handOffToNative: (preview) =>
        appWindow
          ? runNativeTabDrag(appWindow.label, preview, grabX, grabY, pressX, pressY)
          : Promise.resolve({ action: "keep" }),
      detach: (cursor) => detachTab(appWindow, tabId, grabX, grabY, cursor, closeTab),
      merge: (target, x, y) => mergeTabIntoWindow(appWindow, tabId, target, x, y, closeTab),
    });
  };

  // Folder tabs carry the folder's type glyph; workspace surfaces keep their
  // Solar UI glyphs, which are app chrome rather than file types.
  const FolderTabIcon = getFolderPresentation().icon;
  const WorkspaceTabIcon = surface.kind === "folder" ? null : WORKSPACE_TAB_ICONS[surface.kind];
  const tabContent = (
    <>
      {FolderTabIcon ? (
        <FolderTabIcon className="ml-2 size-4 shrink-0" />
      ) : WorkspaceTabIcon ? (
        <WorkspaceTabIcon className="ml-2 size-4 shrink-0 text-foreground/72" />
      ) : null}
      <span className="w-full truncate pr-7 pl-1.5">{title}</span>
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
        // The press is the gesture's `data-pressed` rather than `:active`, and
        // the grabbing cursor with it: a tab dragged past the window edge is
        // handed to the platform's drag loop, which swallows the release that
        // would clear both, and a chip still wearing them reads as a tab still
        // held down after it was put back.
        "group state-layer relative flex h-8 w-52 shrink-0 touch-none cursor-grab items-center rounded-md text-body select-none transition-[background-color,color,opacity] duration-fast ease-standard data-[pressed=true]:cursor-grabbing",
        isActive ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
        isDragging && "opacity-30",
      )}
      data-pressed={dragPressed ? "true" : "false"}
      // The plate's anchor: `useTabPill` finds the chip to measure by this
      // attribute, so it never has to know which tab is current a second time.
      data-tab-active={isActive ? "true" : "false"}
      data-tauri-drag-region="false"
      onClick={() => activateTab(tab.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          activateTab(tab.id);
        }
      }}
      onPointerDown={handlePointerDown}
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
          "absolute top-1/2 right-1 flex size-5 -translate-y-1/2 items-center justify-center rounded-sm transition-colors hover:bg-accent",
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
        <CloseIcon className="size-3" />
      </button>
      {/* No elevation on the ghost, deliberately: the ghost *is* the drag
          image. The OS composites a snapshot of this element under the pointer
          with nothing behind it, where a drop shadow is a smear on the desktop
          rather than a float over the shell, so the two only stay identical if
          neither carries one. */}
      {dragPreview && (
        <TabDragGhostPortal elementRef={ghostElementRef} geometry={dragPreview} tabId={tab.id}>
          {tabContent}
          <span className="absolute top-1/2 right-1 flex size-5 -translate-y-1/2 items-center justify-center text-foreground/65">
            <CloseIcon className="size-3" />
          </span>
        </TabDragGhostPortal>
      )}
    </div>
  );
}

/** The drag ghost: the tab's chip, floating with the cursor. It mounts once
 *  per gesture and is positioned imperatively from then on — the drag's rAF
 *  loop writes `transform` through `moveGhost` (see tab-drag.ts) — so
 *  tracking the cursor at frame rate costs no re-render of the strip.
 *
 *  The transform is deliberately absent from the style prop: the live reorder
 *  re-renders this subtree mid-drag, and a style-prop transform would reset
 *  the imperative position to wherever the ghost mounted. The layout effect
 *  below is the only transform React ever writes — it places the ghost before
 *  its first paint, so it never flashes at the top-left of the window while
 *  waiting for the first tracking frame. */
function TabDragGhostPortal({
  children,
  elementRef,
  geometry,
  tabId,
}: {
  children: ReactNode;
  elementRef: RefObject<HTMLDivElement | null>;
  geometry: TabDragGhost;
  tabId: string;
}) {
  useLayoutEffect(() => {
    const ghost = elementRef.current;
    if (ghost) {
      ghost.style.transform = `translate3d(${geometry.x}px, ${geometry.y}px, 0)`;
    }
  }, [elementRef, geometry]);

  return createPortal(
    <div
      aria-hidden="true"
      className="pointer-events-none fixed top-0 left-0 z-50 flex items-center rounded-sm border border-border bg-card text-body text-foreground select-none"
      data-tab-drag-preview={tabId}
      ref={elementRef}
      style={{ width: geometry.width, height: geometry.height }}
    >
      {children}
    </div>,
    // Escape the tab strip's overflow clipping and the source tab's opacity.
    document.body,
  );
}
