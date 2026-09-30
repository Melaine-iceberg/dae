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
import { type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
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
          className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto px-1.5 scrollbar-none [&::-webkit-scrollbar]:hidden"
          // A tab drag pins its pointer capture here (see `tab-drag.ts`), and the
          // header above is a `deep` drag region, so the strip has to opt out
          // explicitly or the captured events read as a request to move the
          // window.
          data-tauri-drag-region="false"
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
): Promise<NativeDragOutcome> {
  const outcome = await commands.startTabDrag(
    source,
    preview,
    // The grab point travels in the preview bitmap's own pixels, the grid the
    // platform spots the drag image against, so the snapshot's cushion and the
    // device pixel ratio are both part of it.
    (grabX + TAB_DRAG_PREVIEW_PAD) * window.devicePixelRatio,
    (grabY + TAB_DRAG_PREVIEW_PAD) * window.devicePixelRatio,
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
  const dragRef = useRef<TabDragController | null>(null);
  const [dragPreview, setDragPreview] = useState<TabDragGhost | null>(null);
  const [dragActive, setDragActive] = useState(false);
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
      onDragging: setDragActive,
      capturePreview: () => snapshotTabDragPreview(tabId),
      readNativeOutside: () =>
        appWindow ? commands.tabDragOutside(appWindow.label) : Promise.resolve(false),
      handOffToNative: (preview) =>
        appWindow
          ? runNativeTabDrag(appWindow.label, preview, grabX, grabY)
          : Promise.resolve({ action: "keep" }),
      detach: (cursor) => detachTab(appWindow, tabId, grabX, grabY, cursor, closeTab),
      merge: (target, x, y) => mergeTabIntoWindow(appWindow, tabId, target, x, y, closeTab),
    });
  };

  // Folder tabs carry the folder's type glyph; workspace surfaces keep their
  // Lucide UI glyphs, which are app chrome rather than file types.
  const FolderTabIcon = getFolderPresentation().icon;
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
      {/* No elevation on the ghost, deliberately: the ghost *is* the drag
          image. The OS composites a snapshot of this element under the pointer
          with nothing behind it, where a drop shadow is a smear on the desktop
          rather than a float over the shell, so the two only stay identical if
          neither carries one. */}
      {dragPreview &&
        createPortal(
          <div
            aria-hidden="true"
            className="pointer-events-none fixed top-0 left-0 z-50 flex items-center rounded-sm border border-border bg-card text-body text-foreground select-none"
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
