import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { createPortal } from "react-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { emitTo, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { Window as TauriWindow } from "@tauri-apps/api/window";
import {
  ChevronLeft,
  ChevronRight,
  History,
  Home,
  Plus,
  LayoutGrid,
  Star,
  Trash2,
  X,
} from "lucide-react";

import { WindowControls } from "@/components/window-controls";
import { commands, events } from "@/bindings";
import { Sidebar } from "@/features/sidebar/sidebar";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import { WorkspaceSurfaceView } from "@/features/workspace/workspace-surface";
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
  moveTab,
  serializeTabHandoff,
  splitEnabledFamily,
  tabsAtom,
  type ExplorerTab,
} from "./tabs";

const TAB_STRIP_SCROLL_AMOUNT = 512;
const TAB_DRAG_START_DISTANCE = 6;
const TAB_OUTSIDE_POLL_INTERVAL = 50;

type TabDragPreview = {
  x: number;
  y: number;
  width: number;
  height: number;
};

/** Room around the ghost for its --shadow-ambient-lg drop shadow (~12px
 *  sideways, ~32px below) so the native drag image keeps the same floating
 *  look as the in-window ghost. */
const DRAG_PREVIEW_PAD = { top: 6, right: 14, bottom: 40, left: 14 } as const;

/**
 * Event name of [`TabMergedIntoWindow`], mirroring the value `bindings.ts`
 * generates for `events.tabMergedIntoWindow`.
 *
 * The generated `events.x(target).emit()` helper cannot address a single
 * window: it calls `Window.emit`, which broadcasts to every window and ignores
 * the label of the instance it is called on. `emitTo` is the only API that
 * routes a handoff to the window the drop landed on.
 */
const TAB_MERGED_INTO_WINDOW = "tab-merged-into-window";

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

/** The ghost portal mounts one React commit after the drag threshold, so the
 *  snapshot waits a bounded number of frames for it to appear. */
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

/** Rasterizes the live ghost portal (plus shadow padding) to a PNG data URL
 *  for the OS drag loop, so the native drag image is pixel-equal to the tab
 *  the user was dragging in-window. Unlike a DOM portal it is also not
 *  clipped at the WebView boundary. Returns null on any failure; the Rust
 *  side then falls back to the application icon. */
function snapshotTabDragPreview(tabId: string): Promise<string | null> {
  return (async () => {
    const portal = await waitForDragPreviewPortal(tabId);
    if (!portal) return null;

    const bounds = portal.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    const width = bounds.width + DRAG_PREVIEW_PAD.left + DRAG_PREVIEW_PAD.right;
    const height = bounds.height + DRAG_PREVIEW_PAD.top + DRAG_PREVIEW_PAD.bottom;

    const clone = portal.cloneNode(true) as HTMLElement;
    await inlineSubtree(portal, clone);
    clone.style.position = "absolute";
    clone.style.inset = "auto";
    clone.style.left = `${DRAG_PREVIEW_PAD.left}px`;
    clone.style.top = `${DRAG_PREVIEW_PAD.top}px`;
    clone.style.margin = "0";
    clone.style.transform = "none";

    // The foreignObject viewport is sized in device pixels; the wrapper lays
    // the clone out in CSS pixels and scales it so text rasterizes crisply at
    // the display's pixel ratio instead of relying on drawImage upscaling.
    const bitmapWidth = Math.round(width * scale);
    const bitmapHeight = Math.round(height * scale);
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
  overview: Home,
  recents: History,
  favorites: Star,
  trash: Trash2,
  space: LayoutGrid,
} as const;

export function ExplorerTabs() {
  const { t } = useTranslation("explorer");
  const tabs = useAtomValue(tabsAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const createTab = useSetAtom(createTabAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const terminalVisible = useAtomValue(terminalVisibleAtom);
  const stripRef = useRef<HTMLDivElement>(null);
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

  const syncScrollButtons = useCallback(() => {
    const strip = stripRef.current;
    if (!strip) return;
    const maxScrollLeft = strip.scrollWidth - strip.clientWidth;
    setCanScroll({
      left: strip.scrollLeft > 1,
      right: strip.scrollLeft < maxScrollLeft - 1,
    });
  }, []);

  useEffect(() => {
    syncScrollButtons();
    window.addEventListener("resize", syncScrollButtons);
    return () => window.removeEventListener("resize", syncScrollButtons);
  }, [syncScrollButtons, tabs.length]);

  const scrollStrip = (direction: 1 | -1) => {
    stripRef.current?.scrollBy({ left: direction * TAB_STRIP_SCROLL_AMOUNT, behavior: "smooth" });
  };

  return (
    <div className="flex h-full flex-col">
      <TabDropIndicator />
      {/* Window chrome: one flat 38px bar carrying the tab strip and the native
          window controls. It sits on the canvas rung so the frame reads as the
          window itself rather than as a third panel, and the 1px hairline
          below it is what separates the frame from the content plane. */}
      <header
        className="flex h-tab-strip shrink-0 items-stretch border-b border-border bg-background"
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
          className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto px-1.5 scrollbar-none [&::-webkit-scrollbar]:hidden"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {tabs.map((tab, index) => (
            <TabStripItem key={tab.id} index={index} isActive={tab.id === activeTabId} tab={tab} />
          ))}
          <button
            aria-label={t("tabs.newTab")}
            className="flex size-6 shrink-0 items-center justify-center rounded-sm text-muted-foreground transition-colors duration-fast hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none"
            onClick={createTab}
            title={t("tabs.newTabShortcut", { modifier: MOD_KEY })}
            type="button"
          >
            <Plus className="size-3.5" />
          </button>
        </div>
        <StripScrollButton
          aria-label={t("tabs.scrollRight")}
          direction={1}
          onClick={() => scrollStrip(1)}
          visible={canScroll.right}
        />
        <WindowControls />
      </header>

      {/* Flat shell: the sidebar is a tonal column divided from the content
          plane by a single hairline, and the content plane itself is borderless
          — elevation is spent on overlays alone. The tab bar stays flush with
          the window edge so native window controls and snap layouts keep
          working. */}
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-card">
            <WorkspaceSurfaceView key={activeTabId} tabId={activeTabId} />
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
  const Icon = direction === -1 ? ChevronLeft : ChevronRight;

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

/** Index at which a tab dropped at window-space `x` belongs in the strip:
 * before the first tab whose midpoint is right of the drop point. */
function tabInsertionIndexAt(strip: HTMLElement, x: number): number {
  const tabs = Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]'));
  for (let index = 0; index < tabs.length; index++) {
    const rect = tabs[index].getBoundingClientRect();
    if (x < rect.left + rect.width / 2) return index;
  }
  return tabs.length;
}

/** Reorder target for the tab being dragged inside this window: the index
 * among the *other* tabs whose midpoint the cursor has crossed. Excluding
 * the dragged element keeps midpoints stable while the strip shifts. */
function tabReorderIndexAt(strip: HTMLElement, x: number, dragged: HTMLElement): number {
  const tabs = Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]')).filter(
    (element) => element !== dragged,
  );
  for (let index = 0; index < tabs.length; index++) {
    const rect = tabs[index].getBoundingClientRect();
    if (x < rect.left + rect.width / 2) return index;
  }
  return tabs.length;
}

type DropIndicatorGeometry = {
  left: number;
  top: number;
  height: number;
};

/** Viewport-space placement of the drop indicator for a hover/drop at
 * window-space `x`, aligned with the tab gap the insertion would occupy. */
function dropIndicatorGeometryAt(x: number): DropIndicatorGeometry | null {
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
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const [dragPreview, setDragPreview] = useState<TabDragPreview | null>(null);
  const [dragActive, setDragActive] = useState(false);
  const isDragging = dragActive;

  useEffect(() => {
    if (isActive) {
      elementRef.current?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }, [isActive]);

  useEffect(() => () => dragCleanupRef.current?.(), []);

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !event.isPrimary) return;
    if ((event.target as HTMLElement).closest("button")) return;

    const appWindow = getAppWindow();
    const element = elementRef.current;
    if (!element) return;

    event.preventDefault();
    event.stopPropagation();
    activateTab(tab.id);
    dragCleanupRef.current?.();

    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startY = event.clientY;
    const originalIndex = index;
    const bounds = element.getBoundingClientRect();
    const grabX = startX - bounds.left;
    const grabY = startY - bounds.top;
    let pointerX = startX;
    let pointerY = startY;
    let previewFrame: number | undefined;
    let dragStarted = false;
    let pointerReleased = false;
    let disposed = false;
    let nativeDragStarted = false;
    let tearingOff = false;
    let lastReorderIndex = -1;
    let nativePreview: Promise<string | null> | null = null;
    let pollTimer: number | undefined;
    let outsideRequest: Promise<boolean> | null = null;

    const stopPolling = () => {
      window.clearInterval(pollTimer);
      pollTimer = undefined;
    };

    const cleanup = () => {
      if (disposed) return;
      disposed = true;
      stopPolling();
      if (previewFrame !== undefined) window.cancelAnimationFrame(previewFrame);
      window.removeEventListener("pointermove", handlePointerMove, true);
      window.removeEventListener("pointerup", handlePointerEnd, true);
      window.removeEventListener("pointercancel", handlePointerCancel, true);
      window.removeEventListener("keydown", handleKeyDown, true);
      element.removeEventListener("lostpointercapture", handlePointerCancel);
      if (element.hasPointerCapture(pointerId)) element.releasePointerCapture(pointerId);
      dragCleanupRef.current = null;
      setDragPreview(null);
      setDragActive(false);
    };

    const updatePreview = () => {
      previewFrame = undefined;
      if (disposed) return;
      setDragPreview({
        x: pointerX - grabX,
        y: pointerY - grabY,
        width: bounds.width,
        height: bounds.height,
      });
    };

    const readOutside = () => {
      if (!appWindow) return Promise.resolve(false);
      outsideRequest ??= commands
        .tabDragOutside(appWindow.label)
        .finally(() => (outsideRequest = null));
      return outsideRequest;
    };

    const tearOff = async (cursor?: { x: number; y: number }) => {
      if (!appWindow || disposed || tearingOff) return;
      tearingOff = true;
      stopPolling();

      try {
        const payload = serializeTabHandoff(tab.id);
        await commands.tearOffTab(
          appWindow.label,
          payload,
          startX,
          startY,
          cursor?.x ?? null,
          cursor?.y ?? null,
        );
        cleanup();
        closeTab(tab.id);
      } catch (error) {
        console.error("Failed to detach tab", error);
        cleanup();
      }
    };

    /** Hands the tab to the window the native drop landed on. Returns false
     * when the handoff could not be delivered, so the tab stays put. */
    const mergeIntoWindow = async (targetLabel: string, x: number, y: number): Promise<boolean> => {
      try {
        const payload = serializeTabHandoff(tab.id);
        // Addressed rather than broadcast: the receiving window is the only one
        // that may consume this handoff, and above all the source window must
        // not, or it would re-insert the tab it is about to close.
        await emitTo(targetLabel, TAB_MERGED_INTO_WINDOW, { payload, x, y });
        return true;
      } catch (error) {
        console.error("Failed to merge the tab into the target window", error);
        return false;
      }
    };

    const startNativeDrag = async () => {
      if (!appWindow || nativeDragStarted || disposed || tearingOff) return;
      nativeDragStarted = true;
      stopPolling();

      try {
        // The snapshot needs the ghost portal to still be in the DOM.
        const preview = await nativePreview;
        if (disposed || tearingOff) return;

        // The OS drag image takes over the gesture; the in-window ghost would
        // otherwise stay frozen, half-clipped at the WebView edge.
        setDragPreview(null);
        const outcome = await commands.startTabDrag(
          appWindow.label,
          preview,
          (grabX + DRAG_PREVIEW_PAD.left) * window.devicePixelRatio,
          (grabY + DRAG_PREVIEW_PAD.top) * window.devicePixelRatio,
        );
        if (disposed || tearingOff) return;

        if (outcome.released && outcome.outside) {
          if (outcome.target) {
            // The drop landed on another window of this app: merge the tab
            // into it rather than tearing off a new window.
            const merged = await mergeIntoWindow(
              outcome.target,
              outcome.targetX ?? 0,
              outcome.targetY ?? 0,
            );
            if (merged) {
              cleanup();
              closeTab(tab.id);
            } else {
              cleanup();
            }
          } else {
            await tearOff({ x: outcome.cursorX, y: outcome.cursorY });
          }
        } else {
          cleanup();
        }
      } catch (error) {
        console.error("Failed to start native tab drag", error);
        cleanup();
      }
    };

    const pollOutside = async (beginNativeDrag = true): Promise<boolean> => {
      if (disposed || !dragStarted || tearingOff) return false;

      try {
        const outside = await readOutside();
        if (disposed || tearingOff) return outside;
        if (outside && beginNativeDrag && !nativeDragStarted) {
          await startNativeDrag();
        }
        return outside;
      } catch (error) {
        console.error("Failed to track tab drag", error);
        cleanup();
        return false;
      }
    };

    function handlePointerMove(moveEvent: PointerEvent) {
      if (moveEvent.pointerId !== pointerId || disposed || pointerReleased || nativeDragStarted) {
        return;
      }

      if (!dragStarted) {
        const distance = Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY);
        if (distance < TAB_DRAG_START_DISTANCE) return;

        dragStarted = true;
        setDragActive(true);
        nativePreview = snapshotTabDragPreview(tab.id);
        if (appWindow) {
          pollTimer = window.setInterval(() => void pollOutside(), TAB_OUTSIDE_POLL_INTERVAL);
        }
      }

      // Keep the original grab point under the cursor, with at most one
      // visual update per frame regardless of the mouse's polling rate.
      pointerX = moveEvent.clientX;
      pointerY = moveEvent.clientY;
      previewFrame ??= window.requestAnimationFrame(updatePreview);

      // Live reorder: once the cursor crosses a neighbour's midpoint the tab
      // swaps into that slot while its ghost keeps following the cursor. The
      // recomputed midpoints stay stable because the dragged element is
      // excluded from the measurement. (Hoisted function declarations do not
      // inherit the pointerdown guard's non-null narrowing, hence the ref.)
      const dragged = elementRef.current;
      const strip = dragged?.closest<HTMLElement>('[role="tablist"]');
      if (dragged && strip) {
        const target = tabReorderIndexAt(strip, moveEvent.clientX, dragged);
        if (target !== lastReorderIndex) {
          lastReorderIndex = target;
          moveTab(tab.id, target);
        }
      }

      void pollOutside();
    }

    function handlePointerEnd(endEvent: PointerEvent) {
      if (endEvent.pointerId !== pointerId || disposed || nativeDragStarted) return;
      pointerReleased = true;
      if (!dragStarted) {
        cleanup();
        return;
      }

      // Pointer capture may deliver the release even after the cursor leaves
      // the webview. Query the native bounds one final time and only detach now,
      // never merely because the pointer crossed the edge.
      void pollOutside(false).then((outside) => {
        if (disposed || tearingOff) return;
        if (outside) void tearOff();
        else cleanup();
      });
    }

    function handlePointerCancel(cancelEvent: PointerEvent) {
      // The platform's native drag loop owns the gesture after it crosses the
      // window edge; losing DOM capture at that point must not cancel the
      // pending result.
      if (nativeDragStarted) return;
      // Pointerup implicitly releases capture; let its final native bounds
      // check finish instead of cancelling a quick drop outside the window.
      if (cancelEvent.type === "lostpointercapture" && pointerReleased) return;
      if (cancelEvent.pointerId === pointerId) cleanup();
    }

    function handleKeyDown(keyEvent: KeyboardEvent) {
      if (keyEvent.key !== "Escape") return;
      keyEvent.preventDefault();
      // Cancelling a reorder puts the tab back where the drag began; moveTab
      // no-ops when it never left its original slot.
      if (dragStarted) moveTab(tab.id, originalIndex);
      cleanup();
    }

    element.setPointerCapture(pointerId);
    dragCleanupRef.current = cleanup;
    window.addEventListener("pointermove", handlePointerMove, true);
    window.addEventListener("pointerup", handlePointerEnd, true);
    window.addEventListener("pointercancel", handlePointerCancel, true);
    window.addEventListener("keydown", handleKeyDown, true);
    element.addEventListener("lostpointercapture", handlePointerCancel);
  };

  // Folder tabs carry the Catppuccin artwork for the tab's folder
  // name (src, node_modules, .git, ... with a generic folder fallback while
  // the directory is still loading); workspace surfaces keep their Lucide
  // UI glyphs, which are out of the catppuccin icon scope.
  const folderName = directory?.breadcrumbs.at(-1)?.name ?? "";
  const FolderTabIcon = getFolderPresentation(folderName).icon;
  const WorkspaceTabIcon = surface.kind === "folder" ? null : WORKSPACE_TAB_ICONS[surface.kind];
  const tabContent = (
    <>
      {FolderTabIcon ? (
        <FolderTabIcon className="ml-2 size-4 shrink-0" />
      ) : WorkspaceTabIcon ? (
        <WorkspaceTabIcon className="ml-2 size-4 shrink-0 text-muted-foreground" />
      ) : null}
      <span className="w-full truncate pr-7 pl-1.5">{title}</span>
    </>
  );

  return (
    <div
      aria-grabbed={isDragging}
      aria-selected={isActive}
      className={cn(
        // Linear tab: a compact 28px chip inside the 38px strip. The active
        // tab is the only raised surface in the shell — `bg-card` fill, one
        // hairline, and the sanctioned 1px inset top edge — while inactive
        // tabs stay flat text until hovered, so the strip reads as a row of
        // destinations rather than a row of buttons.
        "group state-layer relative flex h-7 w-52 shrink-0 touch-none cursor-grab items-center rounded-sm text-body select-none transition-[background-color,color,opacity] duration-fast ease-standard active:cursor-grabbing",
        isActive
          ? "tab-chip-active border border-border bg-card font-medium text-foreground"
          : "text-muted-foreground hover:text-foreground",
        isDragging && "opacity-30",
      )}
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
        <X className="size-3" />
      </button>
      {dragPreview &&
        createPortal(
          <div
            aria-hidden="true"
            className="pointer-events-none fixed top-0 left-0 z-50 flex items-center rounded-sm border border-border bg-card text-body text-foreground shadow-ambient-lg select-none"
            data-tab-drag-preview={tab.id}
            style={{
              width: dragPreview.width,
              height: dragPreview.height,
              transform: `translate3d(${dragPreview.x}px, ${dragPreview.y}px, 0)`,
            }}
          >
            {tabContent}
            <span className="absolute top-1/2 right-1 flex size-5 -translate-y-1/2 items-center justify-center text-muted-foreground">
              <X className="size-3" />
            </span>
          </div>,
          // Escape the tab strip's overflow clipping and the source tab's opacity.
          document.body,
        )}
    </div>
  );
}
