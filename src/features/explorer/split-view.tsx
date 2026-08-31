import { useCallback, useEffect, useRef, type PointerEvent as ReactPointerEvent } from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

import { ExplorerView } from "./explorer-view";
import {
  activePaneFamily,
  getSplitNavigator,
  getTabNavigator,
  splitEnabledFamily,
  splitRatioFamily,
  toggleSplitViewAtom,
} from "./tabs";

/** Horizontal space reserved for the narrower pane so its toolbar and
 *  path bar never collapse below usability. */
const SPLIT_RATIO_MIN = 0.25;
const SPLIT_RATIO_MAX = 0.75;

/**
 * Dual-pane layout for the folder surface: two fully independent explorers
 * (own navigator, selection and search) side by side. The focused pane owns
 * window-level keyboard shortcuts and command-bar intents; clicking either
 * pane or pressing F6 moves focus between them.
 */
export function SplitExplorerView({ tabId }: { tabId: string }) {
  const { t } = useTranslation("explorer");
  const splitEnabled = useAtomValue(splitEnabledFamily(tabId));
  const activePane = useAtomValue(activePaneFamily(tabId));
  const setActivePane = useSetAtom(activePaneFamily(tabId));
  const toggleSplit = useSetAtom(toggleSplitViewAtom);
  const [ratio, setRatio] = useAtom(splitRatioFamily(tabId));
  const containerRef = useRef<HTMLDivElement>(null);

  // F6 swaps keyboard focus between the panes while the split layout is up.
  useEffect(() => {
    if (!splitEnabled) return undefined;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "F6" || event.defaultPrevented || event.isComposing) return;
      if (isEditableElement(event.target)) return;

      event.preventDefault();
      setActivePane((pane) => (pane === "primary" ? "split" : "primary"));
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [setActivePane, splitEnabled]);

  const handleDividerPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      const container = containerRef.current;
      if (!container) return;

      const bounds = container.getBoundingClientRect();
      const handleMove = (moveEvent: PointerEvent) => {
        const next = (moveEvent.clientX - bounds.left) / bounds.width;
        setRatio(Math.min(SPLIT_RATIO_MAX, Math.max(SPLIT_RATIO_MIN, next)));
      };
      const handleUp = () => {
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
    },
    [setRatio],
  );

  if (!splitEnabled) {
    return (
      <ExplorerView
        isActivePane
        navigator={getTabNavigator(tabId)}
        onToggleSplit={toggleSplit}
        splitEnabled={false}
      />
    );
  }

  return (
    <div ref={containerRef} className="flex h-full min-h-0 min-w-0">
      <PaneShell
        className="shrink-0"
        isActive={activePane === "primary"}
        onActivate={() => setActivePane("primary")}
        style={{ width: `${ratio * 100}%` }}
      >
        <ExplorerView
          isActivePane={activePane === "primary"}
          navigator={getTabNavigator(tabId)}
          onToggleSplit={toggleSplit}
          splitEnabled
        />
      </PaneShell>
      {/* Slim draggable gutter; the hit area is wider than the visible line. */}
      <div
        aria-label={t("explorer:split.resizeAria")}
        aria-orientation="vertical"
        className="group flex w-2 shrink-0 cursor-col-resize items-stretch justify-center"
        onPointerDown={handleDividerPointerDown}
        role="separator"
      >
        <div className="w-px bg-border transition-colors duration-fast group-hover:bg-primary/60" />
      </div>
      <PaneShell
        className="flex-1"
        isActive={activePane === "split"}
        onActivate={() => setActivePane("split")}
      >
        <ExplorerView
          isActivePane={activePane === "split"}
          navigator={getSplitNavigator(tabId)}
          onToggleSplit={toggleSplit}
          splitEnabled
        />
      </PaneShell>
    </div>
  );
}

/** Wraps one pane, claiming focus for it on any pointer or focus capture and
 *  marking the focused pane with a thin accent along its top edge. */
function PaneShell({
  children,
  className,
  isActive,
  onActivate,
  style,
}: {
  children: React.ReactNode;
  className?: string;
  isActive: boolean;
  onActivate: () => void;
  style?: React.CSSProperties;
}) {
  return (
    <div
      className={cn("relative flex min-h-0 min-w-0 flex-col overflow-hidden", className)}
      onFocusCapture={onActivate}
      onPointerDownCapture={onActivate}
      style={style}
    >
      {isActive && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-3 top-0 z-30 h-0.5 rounded-b-full bg-primary"
        />
      )}
      {children}
    </div>
  );
}

function isEditableElement(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;

  const tagName = target.tagName;
  return tagName === "INPUT" || tagName === "TEXTAREA" || target.isContentEditable;
}
