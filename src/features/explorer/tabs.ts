import { atom, getDefaultStore } from "jotai";
import { atomFamily } from "jotai-family";

import { commands } from "@/bindings";
import { getAppWindow } from "@/lib/app-window";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import type { WorkspaceSurface } from "@/features/workspace/types";

import {
  createFolderNavigationSnapshot,
  ExplorerNavigator,
  type ExplorerNavigatorSnapshot,
} from "./navigation";

export interface ExplorerTab {
  id: string;
}

interface ExplorerTabHandoff {
  version: 1;
  surface: WorkspaceSurface;
  primary: ExplorerNavigatorSnapshot;
  split: ExplorerNavigatorSnapshot;
  splitEnabled: boolean;
  activePane: ExplorerPaneId;
  splitRatio: number;
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

/** Serializes everything that belongs to one tab for a new webview window. */
export function serializeTabHandoff(tabId: string): string {
  const store = getDefaultStore();
  const handoff: ExplorerTabHandoff = {
    version: 1,
    surface: store.get(tabSurfaceFamily(tabId)),
    primary: getTabNavigator(tabId).createSnapshot(),
    split: getSplitNavigator(tabId).createSnapshot(),
    splitEnabled: store.get(splitEnabledFamily(tabId)),
    activePane: store.get(activePaneFamily(tabId)),
    splitRatio: store.get(splitRatioFamily(tabId)),
  };

  return JSON.stringify(handoff);
}

/**
 * Serializes a brand-new single-pane tab sitting on `path`, in the same shape
 * a torn-off tab carries.
 *
 * A window opened onto a folder needs a handoff but has no source tab to
 * serialize, and creating one here just to close it again would flash an extra
 * tab in this window's strip.
 */
function createFolderHandoff(path: string): string {
  const { primary, split } = createFolderNavigationSnapshot(path);
  const handoff: ExplorerTabHandoff = {
    version: 1,
    surface: { kind: "folder" },
    primary,
    split,
    splitEnabled: false,
    activePane: "primary",
    splitRatio: 0.5,
  };

  return JSON.stringify(handoff);
}

/** Applies a serialized tab handoff to a tab, overwriting its navigation
 * state and layout. Throws on malformed payloads. */
function applyTabHandoff(tabId: string, payload: string): void {
  const handoff = parseTabHandoff(payload);
  const store = getDefaultStore();

  getTabNavigator(tabId).restoreSnapshot(handoff.primary);
  getSplitNavigator(tabId).restoreSnapshot(handoff.split);
  store.set(tabSurfaceFamily(tabId), handoff.surface);
  store.set(splitEnabledFamily(tabId), handoff.splitEnabled);
  store.set(activePaneFamily(tabId), handoff.activePane);
  store.set(splitRatioFamily(tabId), handoff.splitRatio);
}

/** Applies a detached tab to the one initial tab created by this webview. */
export function restoreInitialTabHandoff(payload: string): void {
  applyTabHandoff(initialTab.id, payload);
}

/** Inserts a tab dropped from another window at `index` and activates it.
 * Mirrors `restoreInitialTabHandoff` but for a window that is already
 * running, so the merged tab lands beside the existing tabs. */
export function mergeTabFromHandoff(payload: string, index: number): void {
  const tab = createTabEntry();
  applyTabHandoff(tab.id, payload);

  const store = getDefaultStore();
  const tabs = store.get(tabsAtom);
  const clamped = Math.min(Math.max(index, 0), tabs.length);
  const merged = tabs.slice();
  merged.splice(clamped, 0, tab);
  store.set(tabsAtom, merged);
  store.set(activeTabIdAtom, tab.id);
}

/** Moves a tab to `insertionIndex` within the strip for drag reordering.
 * The index counts the other tabs (the list without the moved tab), the same
 * convention the drop indicator uses. No-op when the tab is unknown or
 * already at the target position. */
export function moveTab(tabId: string, insertionIndex: number): void {
  const store = getDefaultStore();
  const tabs = store.get(tabsAtom);
  const from = tabs.findIndex((tab) => tab.id === tabId);
  if (from === -1) return;

  const clamped = Math.min(Math.max(insertionIndex, 0), tabs.length - 1);
  if (clamped === from) return;

  const reordered = tabs.slice();
  const [tab] = reordered.splice(from, 1);
  reordered.splice(clamped, 0, tab);
  store.set(tabsAtom, reordered);
}

function parseTabHandoff(payload: string): ExplorerTabHandoff {
  const value: unknown = JSON.parse(payload);
  if (!isRecord(value) || value.version !== 1) {
    throw new Error("Unsupported tab handoff payload");
  }
  if (!isWorkspaceSurface(value.surface)) {
    throw new Error("Invalid tab surface in handoff payload");
  }
  if (!isNavigatorSnapshot(value.primary) || !isNavigatorSnapshot(value.split)) {
    throw new Error("Invalid tab navigation state in handoff payload");
  }
  if (
    typeof value.splitEnabled !== "boolean" ||
    (value.activePane !== "primary" && value.activePane !== "split") ||
    typeof value.splitRatio !== "number" ||
    !Number.isFinite(value.splitRatio)
  ) {
    throw new Error("Invalid split-view state in handoff payload");
  }

  return value as unknown as ExplorerTabHandoff;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isWorkspaceSurface(value: unknown): value is WorkspaceSurface {
  if (!isRecord(value) || typeof value.kind !== "string") return false;
  if (value.kind === "space") return typeof value.spaceId === "string";
  return ["overview", "recents", "favorites", "trash", "folder"].includes(value.kind);
}

function isNavigatorSnapshot(value: unknown): value is ExplorerNavigatorSnapshot {
  if (!isRecord(value) || !isRecord(value.state) || !Array.isArray(value.scrollOffsets)) {
    return false;
  }

  return (
    Array.isArray(value.state.history) &&
    value.state.history.every((path) => typeof path === "string") &&
    typeof value.state.historyIndex === "number" &&
    value.scrollOffsets.every(
      (entry) =>
        Array.isArray(entry) &&
        entry.length === 2 &&
        typeof entry[0] === "string" &&
        typeof entry[1] === "number",
    )
  );
}

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

/**
 * Cascade step, in logical pixels, for a window opened from a context menu.
 * The new window's top-left corner lands this far down-right of the source
 * window's, so the two read as a stack instead of hiding each other.
 */
const CASCADE_OFFSET = 32;

/**
 * Turns a cascade offset into the four placement arguments `tear_off_tab`
 * expects.
 *
 * That command places the window at `cursor - grab * scale`, where the cursor
 * is a physical desktop coordinate and `grab` is a logical offset inside the
 * window being dragged. Context menus have no drag, so the pointer has to be
 * taken out of the equation: pass a zero grab and hand the command an explicit
 * cursor sitting at the source window's corner, and that cursor value passes
 * straight through as the landing position. Offsetting through `grab` instead
 * is not possible — the command clamps it to `[0, width]`, so a negative grab
 * degenerates to zero and the window lands exactly on the source.
 *
 * Scaling by the source window's factor keeps the visual step identical on
 * mixed-DPI monitor layouts, where a physical pixel is not a fixed visual
 * size.
 */
function cascadePlacement(
  sourceOuter: { x: number; y: number },
  scale: number,
  offset: number,
): { grabX: number; grabY: number; cursorX: number; cursorY: number } {
  return {
    grabX: 0,
    grabY: 0,
    cursorX: sourceOuter.x + offset * scale,
    cursorY: sourceOuter.y + offset * scale,
  };
}

/**
 * Opens `path` in a window of its own, leaving this window's tabs untouched.
 *
 * Deliberately routed through the tab tear-off command: it already owns the
 * window sizing, the DPI-correct placement and the hidden-until-restored boot
 * sequence the destination frontend expects, so a folder opened here behaves
 * exactly like a tab dragged out of the strip — apart from where it lands,
 * which is cascaded off the source window rather than pinned to the pointer
 * the way an actual drag-out is.
 */
export async function openPathInNewWindow(path: string): Promise<void> {
  const appWindow = getAppWindow();
  // The browser preview bridge has no native windows to create.
  if (!appWindow) return;

  try {
    const [sourceOuter, scale] = await Promise.all([
      appWindow.outerPosition(),
      appWindow.scaleFactor(),
    ]);
    const placement = cascadePlacement(sourceOuter, scale, CASCADE_OFFSET);

    await commands.tearOffTab(
      appWindow.label,
      createFolderHandoff(path),
      placement.grabX,
      placement.grabY,
      placement.cursorX,
      placement.cursorY,
    );
  } catch (error) {
    console.error(`Unable to open ${path} in a new window`, error);
  }
}

export const openPathInNewWindowAtom = atom(null, (_get, _set, path: string) => {
  void openPathInNewWindow(path);
});

export const activateTabAtom = atom(null, (_get, set, tabId: string) => {
  set(activeTabIdAtom, tabId);
});

export const closeTabAtom = atom(null, (get, set, tabId: string) => {
  const tabs = get(tabsAtom);
  const index = tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) return;

  const remaining = tabs.filter((tab) => tab.id !== tabId);
  // Close the navigators' in-flight directory listings before dropping them.
  navigators.get(tabId)?.dispose();
  navigators.get(`${tabId}${SPLIT_NAVIGATOR_SUFFIX}`)?.dispose();
  navigators.delete(tabId);
  navigators.delete(`${tabId}${SPLIT_NAVIGATOR_SUFFIX}`);
  tabSurfaceFamily.remove(tabId);
  splitEnabledFamily.remove(tabId);
  activePaneFamily.remove(tabId);
  splitRatioFamily.remove(tabId);

  if (remaining.length === 0) {
    void getAppWindow()?.close();
    return;
  }

  set(tabsAtom, remaining);

  if (get(activeTabIdAtom) === tabId) {
    set(activeTabIdAtom, remaining[Math.min(index, remaining.length - 1)].id);
  }
});
