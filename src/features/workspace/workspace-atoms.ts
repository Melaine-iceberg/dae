import { atom } from "jotai";

import {
  activePaneFamily,
  activeTabIdAtom,
  getPaneNavigator,
  splitEnabledFamily,
} from "@/features/explorer/tabs";

import { tabSurfaceFamily } from "./tab-surface";
import type { WorkspaceSurface } from "./types";

export { tabSurfaceFamily } from "./tab-surface";

/** The surface of the currently active tab. */
export const activeSurfaceAtom = atom(
  (get) => get(tabSurfaceFamily(get(activeTabIdAtom))),
  (get, set, surface: WorkspaceSurface) => {
    set(tabSurfaceFamily(get(activeTabIdAtom)), surface);
  },
);

/** Opens a workspace surface (Overview / Recents / Favorites / a Space) in the active tab. */
export const openSurfaceAtom = atom(null, (get, set, surface: WorkspaceSurface) => {
  set(tabSurfaceFamily(get(activeTabIdAtom)), surface);
});

/**
 * Switches the active tab to the folder surface and navigates its explorer
 * navigator to the given path. In the dual-pane layout the focused pane
 * receives the navigation. The navigator keeps its own history, so
 * back/forward keep working across surface switches.
 */
export const navigateToFolderAtom = atom(null, (get, set, path: string) => {
  const tabId = get(activeTabIdAtom);
  set(tabSurfaceFamily(tabId), { kind: "folder" });
  const pane = get(splitEnabledFamily(tabId)) ? get(activePaneFamily(tabId)) : "primary";
  void getPaneNavigator(tabId, pane).navigate(path);
});
