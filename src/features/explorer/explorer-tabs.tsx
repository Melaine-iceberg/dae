import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  CaretLeftIcon,
  CaretRightIcon,
  ClockCounterClockwiseIcon,
  FolderIcon,
  HouseIcon,
  PlusIcon,
  SquaresFourIcon,
  StarIcon,
  XIcon,
} from "@phosphor-icons/react";

import { WindowControls } from "@/components/window-controls";
import { Sidebar } from "@/features/sidebar/sidebar";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import { WorkspaceSurfaceView } from "@/features/workspace/workspace-surface";
import type { WorkspaceSurface } from "@/features/workspace/types";
import { cn } from "@/lib/utils";

import {
  activeTabIdAtom,
  activateTabAtom,
  closeTabAtom,
  createTabAtom,
  getTabNavigator,
  tabsAtom,
  type ExplorerTab,
} from "./tabs";

const TAB_STRIP_SCROLL_AMOUNT = 512;

export function ExplorerTabs() {
  const tabs = useAtomValue(tabsAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const createTab = useSetAtom(createTabAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const stripRef = useRef<HTMLDivElement>(null);
  const [canScroll, setCanScroll] = useState({ left: false, right: false });

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
        className="flex h-9 shrink-0 items-stretch border-b bg-muted/50"
        data-tauri-drag-region="deep"
      >
        <StripScrollButton
          aria-label="向左滚动标签页"
          direction={-1}
          onClick={() => scrollStrip(-1)}
          visible={canScroll.left}
        />
        <div
          ref={stripRef}
          aria-label="文件标签页"
          className="flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto px-1 scrollbar-none [&::-webkit-scrollbar]:hidden"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {tabs.map((tab) => (
            <TabStripItem key={tab.id} isActive={tab.id === activeTabId} tab={tab} />
          ))}
          <button
            aria-label="新建标签页"
            className="mb-1 flex size-6 shrink-0 items-center justify-center rounded-[5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={createTab}
            title="新建标签页 (Ctrl+T)"
            type="button"
          >
            <PlusIcon className="size-3.5" />
          </button>
        </div>
        <StripScrollButton
          aria-label="向右滚动标签页"
          direction={1}
          onClick={() => scrollStrip(1)}
          visible={canScroll.right}
        />
        <WindowControls />
      </header>

      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <WorkspaceSurfaceView key={activeTabId} tabId={activeTabId} />
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
  const Icon = direction === -1 ? CaretLeftIcon : CaretRightIcon;

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
): string {
  switch (surface.kind) {
    case "overview":
      return "概览";
    case "recents":
      return "最近使用";
    case "favorites":
      return "收藏";
    case "space":
      return spaceName ?? "空间";
    case "folder":
      return folderTitle;
  }
}

function TabStripItem({ isActive, tab }: { isActive: boolean; tab: ExplorerTab }) {
  const activateTab = useSetAtom(activateTabAtom);
  const closeTab = useSetAtom(closeTabAtom);
  const surface = useAtomValue(tabSurfaceFamily(tab.id));
  const spaces = useAtomValue(spacesAtom);
  const navigator = getTabNavigator(tab.id);
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const directory = state.directory;
  const spaceName =
    surface.kind === "space"
      ? spaces?.find((space) => space.id === surface.spaceId)?.name
      : undefined;
  const title = surfaceTitle(surface, directory?.breadcrumbs.at(-1)?.name ?? "加载中…", spaceName);
  const elementRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isActive) {
      elementRef.current?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }, [isActive]);

  const TabIcon = (() => {
    switch (surface.kind) {
      case "overview":
        return HouseIcon;
      case "recents":
        return ClockCounterClockwiseIcon;
      case "favorites":
        return StarIcon;
      case "space":
        return SquaresFourIcon;
      case "folder":
        return FolderIcon;
    }
  })();

  return (
    <div
      aria-selected={isActive}
      className={cn(
        "group relative flex h-8 w-52 shrink-0 items-center rounded-t-lg border border-b-0 text-[13px] select-none",
        isActive
          ? "border-border bg-background"
          : "cursor-default border-transparent text-muted-foreground hover:bg-accent/60 hover:text-foreground",
      )}
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
      title={surface.kind === "folder" ? (directory?.path ?? title) : title}
    >
      <TabIcon
        className={cn(
          "ml-2 size-3.5 shrink-0",
          surface.kind === "folder" ? "text-folder" : "text-muted-foreground",
        )}
        weight={surface.kind === "folder" ? "fill" : "regular"}
      />
      <span className="w-full truncate pr-7 pl-1.5">{title}</span>
      <button
        aria-label={`关闭标签页 ${title}`}
        className={cn(
          "absolute top-1/2 right-1 flex size-5 -translate-y-1/2 items-center justify-center rounded-[4px] transition-colors hover:bg-accent",
          isActive
            ? "text-muted-foreground hover:text-foreground"
            : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
        onClick={(event) => {
          event.stopPropagation();
          closeTab(tab.id);
        }}
        type="button"
      >
        <XIcon className="size-3" />
      </button>
    </div>
  );
}
