import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { ChevronLeftIcon, ChevronRightIcon, PlusIcon, XIcon } from "lucide-react";

import { WindowControls } from "@/components/window-controls";
import { Sidebar } from "@/features/sidebar/sidebar";
import { cn } from "@/lib/utils";

import { ExplorerView } from "./explorer-view";
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
  const stripRef = useRef<HTMLDivElement>(null);
  const [canScroll, setCanScroll] = useState({ left: false, right: false });

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
        className="flex h-10 shrink-0 items-stretch border-b bg-muted/40"
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
          className="flex min-w-0 flex-1 items-stretch gap-1 overflow-x-auto px-1 pt-1.5 scrollbar-none [&::-webkit-scrollbar]:hidden"
          onScroll={syncScrollButtons}
          role="tablist"
        >
          {tabs.map((tab) => (
            <TabStripItem key={tab.id} isActive={tab.id === activeTabId} tab={tab} />
          ))}
          <button
            aria-label="新建标签页"
            className="my-auto flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            onClick={createTab}
            title="新建标签页"
            type="button"
          >
            <PlusIcon className="size-4" />
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
        <div className="min-h-0 min-w-0 flex-1">
          <ExplorerView key={activeTabId} navigator={getTabNavigator(activeTabId)} />
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
  const Icon = direction === -1 ? ChevronLeftIcon : ChevronRightIcon;

  return (
    <button
      aria-label={ariaLabel}
      className={cn(
        "flex w-7 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
        !visible && "invisible",
      )}
      onClick={onClick}
      tabIndex={visible ? 0 : -1}
      type="button"
    >
      <Icon className="size-4" />
    </button>
  );
}

function TabStripItem({ isActive, tab }: { isActive: boolean; tab: ExplorerTab }) {
  const activateTab = useSetAtom(activateTabAtom);
  const closeTab = useSetAtom(closeTabAtom);
  const navigator = getTabNavigator(tab.id);
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const title = state.directory?.path ?? "加载中…";
  const elementRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isActive) {
      elementRef.current?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }, [isActive]);

  return (
    <div
      aria-selected={isActive}
      className={cn(
        "group relative flex h-full w-64 shrink-0 cursor-pointer items-center rounded-t-md border border-b-0 text-sm select-none",
        isActive
          ? "border-border bg-background"
          : "border-transparent text-muted-foreground hover:bg-muted hover:text-foreground",
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
      title={title}
    >
      <span className="w-full truncate pr-8 pl-3 text-center">{title}</span>
      <button
        aria-label={`关闭标签页 ${title}`}
        className={cn(
          "absolute top-1/2 right-1.5 flex size-5 -translate-y-1/2 items-center justify-center rounded-sm transition-opacity hover:bg-muted",
          isActive ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
        onClick={(event) => {
          event.stopPropagation();
          closeTab(tab.id);
        }}
        type="button"
      >
        <XIcon className="size-3.5" />
      </button>
    </div>
  );
}
