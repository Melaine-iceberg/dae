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
import { commands } from "@/bindings";
import { Sidebar } from "@/features/sidebar/sidebar";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import { WorkspaceSurfaceView } from "@/features/workspace/workspace-surface";
import type { WorkspaceSurface } from "@/features/workspace/types";
import { isWindowsPlatform, MOD_KEY } from "@/lib/platform";
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

function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  const clampedRadius = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + clampedRadius, y);
  context.arcTo(x + width, y, x + width, y + height, clampedRadius);
  context.arcTo(x + width, y + height, x, y + height, clampedRadius);
  context.arcTo(x, y + height, x, y, clampedRadius);
  context.arcTo(x, y, x + width, y, clampedRadius);
  context.closePath();
}

/** Renders a compact PNG for the OS drag loop. Unlike a DOM portal, the
 * native drag image is not clipped at the WebView window boundary. */
function createNativeTabDragPreview(
  element: HTMLElement,
  title: string,
  width: number,
  height: number,
): string | null {
  const scale = window.devicePixelRatio || 1;
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(width * scale);
  canvas.height = Math.ceil(height * scale);
  const context = canvas.getContext("2d");
  if (!context) return null;

  const rootStyle = getComputedStyle(document.documentElement);
  const elementStyle = getComputedStyle(element);
  const card = rootStyle.getPropertyValue("--card").trim() || elementStyle.backgroundColor;
  const foreground = rootStyle.getPropertyValue("--foreground").trim() || elementStyle.color;
  const muted = rootStyle.getPropertyValue("--muted-foreground").trim() || foreground;
  const border = rootStyle.getPropertyValue("--border").trim() || "transparent";

  context.scale(scale, scale);
  roundedRect(context, 0.5, 0.5, width - 1, height - 1, 6);
  context.fillStyle = card;
  context.fill();
  context.strokeStyle = border;
  context.lineWidth = 1;
  context.stroke();

  let textX = 10;
  const icon = element.querySelector(":scope > img");
  if (icon instanceof HTMLImageElement && icon.complete && icon.naturalWidth > 0) {
    try {
      context.drawImage(icon, 8, (height - 14) / 2, 14, 14);
      textX = 28;
    } catch {
      // The title still makes a useful preview if an icon cannot be painted.
    }
  }

  const closeCenterX = width - 11;
  context.strokeStyle = muted;
  context.lineCap = "round";
  context.lineWidth = 1.25;
  context.beginPath();
  context.moveTo(closeCenterX - 3, height / 2 - 3);
  context.lineTo(closeCenterX + 3, height / 2 + 3);
  context.moveTo(closeCenterX + 3, height / 2 - 3);
  context.lineTo(closeCenterX - 3, height / 2 + 3);
  context.stroke();

  context.fillStyle = foreground;
  context.font = `${elementStyle.fontSize} ${elementStyle.fontFamily}`;
  context.textBaseline = "middle";
  const maxTextWidth = Math.max(0, closeCenterX - textX - 9);
  let previewTitle = title;
  if (context.measureText(previewTitle).width > maxTextWidth) {
    while (
      previewTitle.length > 1 &&
      context.measureText(`${previewTitle}…`).width > maxTextWidth
    ) {
      previewTitle = previewTitle.slice(0, -1);
    }
    previewTitle += "…";
  }
  context.fillText(previewTitle, textX, height / 2, maxTextWidth);

  try {
    return canvas.toDataURL("image/png");
  } catch {
    return null;
  }
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
      <header
        className="flex h-10 shrink-0 items-stretch border-b border-border/50 bg-background"
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
          className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto px-2.5 scrollbar-none [&::-webkit-scrollbar]:hidden"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {tabs.map((tab) => (
            <TabStripItem key={tab.id} isActive={tab.id === activeTabId} tab={tab} />
          ))}
          <button
            aria-label={t("tabs.newTab")}
            className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-fast hover:bg-accent hover:text-foreground"
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

      {/* Island shell: panels float on the canvas separated by 10px gutters.
          The tab bar stays flush with the window edge so the native window
          controls and snap layouts keep working. */}
      <div className="flex min-h-0 flex-1 gap-2.5 px-2.5 pt-1 pb-2.5">
        <Sidebar />
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2.5">
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card shadow-ambient-xs dark:inset-shadow-[0_1px_0_rgb(255_255_255/0.05)]">
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
        "flex w-7 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
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

function TabStripItem({ isActive, tab }: { isActive: boolean; tab: ExplorerTab }) {
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
  const isDragging = dragPreview !== null;

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
    let nativePreview: string | null = null;
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

    const startNativeDrag = async () => {
      if (!appWindow || nativeDragStarted || disposed || tearingOff) return;
      nativeDragStarted = true;
      stopPolling();

      try {
        const outcome = await commands.startTabDrag(
          appWindow.label,
          nativePreview,
          grabX * window.devicePixelRatio,
          grabY * window.devicePixelRatio,
        );
        if (disposed || tearingOff) return;

        if (outcome.released && outcome.outside) {
          await tearOff({ x: outcome.cursorX, y: outcome.cursorY });
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
        if (outside && beginNativeDrag && isWindowsPlatform && !nativeDragStarted) {
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
      if (moveEvent.pointerId !== pointerId || disposed || pointerReleased) return;

      if (!dragStarted) {
        const distance = Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY);
        if (distance < TAB_DRAG_START_DISTANCE) return;

        dragStarted = true;
        nativePreview = createNativeTabDragPreview(element!, title, bounds.width, bounds.height);
        if (appWindow) {
          pollTimer = window.setInterval(() => void pollOutside(), TAB_OUTSIDE_POLL_INTERVAL);
        }
      }

      // Keep the original grab point under the cursor, with at most one
      // visual update per frame regardless of the mouse's polling rate.
      pointerX = moveEvent.clientX;
      pointerY = moveEvent.clientY;
      previewFrame ??= window.requestAnimationFrame(updatePreview);
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
      // The native OLE loop owns the gesture after it crosses the window edge;
      // losing DOM capture at that point must not cancel the pending result.
      if (nativeDragStarted) return;
      // Pointerup implicitly releases capture; let its final native bounds
      // check finish instead of cancelling a quick drop outside the window.
      if (cancelEvent.type === "lostpointercapture" && pointerReleased) return;
      if (cancelEvent.pointerId === pointerId) cleanup();
    }

    function handleKeyDown(keyEvent: KeyboardEvent) {
      if (keyEvent.key !== "Escape") return;
      keyEvent.preventDefault();
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
        <FolderTabIcon className="ml-2 size-3.5 shrink-0" />
      ) : WorkspaceTabIcon ? (
        <WorkspaceTabIcon className="ml-2 size-3.5 shrink-0 text-muted-foreground" />
      ) : null}
      <span className="w-full truncate pr-7 pl-1.5">{title}</span>
    </>
  );

  return (
    <div
      aria-grabbed={isDragging}
      aria-selected={isActive}
      className={cn(
        "group relative flex h-8 w-52 shrink-0 touch-none cursor-grab items-center rounded-md text-[13px] select-none transition-[background-color,color,box-shadow,scale,opacity] duration-fast ease-spring-fast active:scale-[0.98] active:cursor-grabbing",
        isActive
          ? "bg-card text-foreground shadow-ambient-sm ring-1 ring-border dark:inset-shadow-[0_1px_0_rgb(255_255_255/0.06)]"
          : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
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
          "absolute top-1/2 right-1 flex size-5 -translate-y-1/2 items-center justify-center rounded-xs transition-colors hover:bg-accent",
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
            className="pointer-events-none fixed top-0 left-0 z-50 flex items-center rounded-md bg-card text-[13px] text-foreground shadow-ambient-lg ring-1 ring-border select-none"
            data-tab-drag-preview=""
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
