import { atom, getDefaultStore } from "jotai";
import { atomFamily } from "jotai/utils";

import { getAppWindow } from "@/lib/app-window";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import type { WorkspaceSurface } from "@/features/workspace/types";

import { ExplorerNavigator } from "./navigation";

export interface ExplorerTab {
  id: string;
}

export type FileClipboard = {
  operation: "copy" | "cut";
  sourcePaths: string[];
};

/** Undo/redo availability mirrored from the backend history stacks. */
export interface UndoRedoStatus {
  canUndo: boolean;
  canRedo: boolean;
}

const navigators = new Map<string, ExplorerNavigator>();
let nextTabId = 0;

function createTabEntry(): ExplorerTab {
  const id = `tab-${nextTabId++}`;
  navigators.set(id, new ExplorerNavigator());
  return { id };
}

export function getTabNavigator(tabId: string): ExplorerNavigator {
  const navigator = navigators.get(tabId);
  if (!navigator) {
    throw new Error(`No navigator registered for tab "${tabId}"`);
  }
  return navigator;
}

/** Panes of the dual-pane (split) layout; every tab starts on the primary
 *  pane and gains a split pane on demand. */
export type ExplorerPaneId = "primary" | "split";

const SPLIT_NAVIGATOR_SUFFIX = "::split";

/** Whether the dual-pane layout is enabled, per tab. */
export const splitEnabledFamily = atomFamily((_tabId: string) => atom(false));

/** Which pane currently owns keyboard focus and command routing, per tab. */
export const activePaneFamily = atomFamily((_tabId: string) => atom<ExplorerPaneId>("primary"));

/** Primary pane width as a fraction of the split container, per tab. */
export const splitRatioFamily = atomFamily((_tabId: string) => atom(0.5));

/** The split pane keeps its own history stack, created lazily on first use
 *  so single-pane tabs pay nothing. */
export function getSplitNavigator(tabId: string): ExplorerNavigator {
  const splitId = `${tabId}${SPLIT_NAVIGATOR_SUFFIX}`;
  let navigator = navigators.get(splitId);
  if (!navigator) {
    navigator = new ExplorerNavigator();
    navigators.set(splitId, navigator);
  }
  return navigator;
}

export function getPaneNavigator(tabId: string, pane: ExplorerPaneId): ExplorerNavigator {
  return pane === "split" ? getSplitNavigator(tabId) : getTabNavigator(tabId);
}

/** Navigator of the focused pane in the active tab: sidebar navigation,
 *  the command bar and the terminal all resolve their folder through it. */
export const activePaneNavigatorAtom = atom((get) => {
  const tabId = get(activeTabIdAtom);
  if (get(splitEnabledFamily(tabId)) && get(activePaneFamily(tabId)) === "split") {
    return getSplitNavigator(tabId);
  }
  return getTabNavigator(tabId);
});

/** Non-reactive variant for call sites outside React (default-store reads). */
export function getActivePaneNavigator(): ExplorerNavigator {
  return getDefaultStore().get(activePaneNavigatorAtom);
}

/** Toggles the dual-pane layout of the active tab; closing the split pane
 *  hands focus back to the primary pane. */
export const toggleSplitViewAtom = atom(null, (get, set) => {
  const tabId = get(activeTabIdAtom);
  const enabled = get(splitEnabledFamily(tabId));
  set(splitEnabledFamily(tabId), !enabled);
  if (enabled) {
    set(activePaneFamily(tabId), "primary");
  }
});

const initialTab = createTabEntry();

export const tabsAtom = atom<ExplorerTab[]>([initialTab]);
export const activeTabIdAtom = atom<string>(initialTab.id);
export const fileClipboardAtom = atom<FileClipboard | null>(null);
export const undoRedoAtom = atom<UndoRedoStatus>({ canUndo: false, canRedo: false });

export const createTabAtom = atom(null, (_get, set) => {
  const tab = createTabEntry();
  set(tabsAtom, (tabs) => [...tabs, tab]);
  set(activeTabIdAtom, tab.id);
});

/** Creates and activates a tab showing the given workspace surface. */
export const createTabWithSurfaceAtom = atom(null, (_get, set, surface: WorkspaceSurface) => {
  const tab = createTabEntry();
  set(tabsAtom, (tabs) => [...tabs, tab]);
  set(activeTabIdAtom, tab.id);
  set(tabSurfaceFamily(tab.id), surface);
});

export const openInNewTabAtom = atom(null, (_get, set, path: string) => {
  const tab = createTabEntry();
  set(tabsAtom, (tabs) => [...tabs, tab]);
  set(activeTabIdAtom, tab.id);
  set(tabSurfaceFamily(tab.id), { kind: "folder" });
  void getTabNavigator(tab.id).navigate(path);
});

export const activateTabAtom = atom(null, (_get, set, tabId: string) => {
  set(activeTabIdAtom, tabId);
});

export const closeTabAtom = atom(null, (get, set, tabId: string) => {
  const tabs = get(tabsAtom);
  const index = tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) return;

  const remaining = tabs.filter((tab) => tab.id !== tabId);
  navigators.delete(tabId);
  navigators.delete(`${tabId}${SPLIT_NAVIGATOR_SUFFIX}`);
  splitEnabledFamily.remove(tabId);
  activePaneFamily.remove(tabId);
  splitRatioFamily.remove(tabId);

  if (remaining.length === 0) {
    getAppWindow()?.close();
    return;
  }

  set(tabsAtom, remaining);

  if (get(activeTabIdAtom) === tabId) {
    set(activeTabIdAtom, remaining[Math.min(index, remaining.length - 1)].id);
  }
});
