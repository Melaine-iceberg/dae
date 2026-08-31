import { atom } from "jotai";
import { atomWithStorage } from "jotai/utils";

import { commands, type StoredCloudAccount, type StoredConnection } from "@/bindings";

import type { Favorite, PlaceKind } from "./types";

export const sidebarVisibleAtom = atomWithStorage("sidebar-visible", true);

/** Collapsible location groups in the sidebar. All start collapsed so cold
 *  start never pays for disk enumeration, the `wsl.exe` probe, or the
 *  connections store read — each group loads on first expand. */
export type LocationSectionId = "disks" | "wsl" | "network" | "cloud";

export const collapsedLocationSectionsAtom = atomWithStorage<
  Record<LocationSectionId, boolean>
>("sidebar-collapsed-locations", {
  disks: true,
  wsl: true,
  network: true,
  cloud: true,
});

export const toggleLocationSectionAtom = atom(null, (get, set, id: LocationSectionId) => {
  const collapsed = get(collapsedLocationSectionsAtom);
  set(collapsedLocationSectionsAtom, { ...collapsed, [id]: !collapsed[id] });
});

export const expandLocationSectionAtom = atom(null, (get, set, id: LocationSectionId) => {
  const collapsed = get(collapsedLocationSectionsAtom);
  if (!collapsed[id]) return;
  set(collapsedLocationSectionsAtom, { ...collapsed, [id]: false });
});

// `null` means the connections have not been loaded yet (lazy, on first
// expand of the network section).
export const connectionsAtom = atom<StoredConnection[] | null>(null);

export const ensureConnectionsLoadedAtom = atom(null, async (get, set) => {
  if (get(connectionsAtom) !== null) return;

  try {
    set(connectionsAtom, await commands.listConnections());
  } catch (error) {
    console.warn("Unable to list connections", error);
    set(connectionsAtom, []);
  }
});

/** Reloads after a save/delete, regardless of current loaded state. */
export const reloadConnectionsAtom = atom(null, async (_get, set) => {
  try {
    set(connectionsAtom, await commands.listConnections());
  } catch (error) {
    console.warn("Unable to list connections", error);
  }
});

// `null` means the cloud accounts have not been loaded yet (lazy, on first
// expand of the cloud section).
export const cloudAccountsAtom = atom<StoredCloudAccount[] | null>(null);

export const ensureCloudAccountsLoadedAtom = atom(null, async (get, set) => {
  if (get(cloudAccountsAtom) !== null) return;

  try {
    set(cloudAccountsAtom, await commands.listCloudAccounts());
  } catch (error) {
    console.warn("Unable to list cloud accounts", error);
    set(cloudAccountsAtom, []);
  }
});

/** Reloads after an authorize/delete, regardless of current loaded state. */
export const reloadCloudAccountsAtom = atom(null, async (_get, set) => {
  try {
    set(cloudAccountsAtom, await commands.listCloudAccounts());
  } catch (error) {
    console.warn("Unable to list cloud accounts", error);
  }
});

export const hiddenPlacesAtom = atomWithStorage<PlaceKind[]>("sidebar-hidden-places", []);

// `null` means the favorites have not been loaded yet.
export const favoritesAtom = atom<Favorite[] | null>(null);

export const ensureFavoritesLoadedAtom = atom(null, async (get, set) => {
  if (get(favoritesAtom) !== null) return;

  try {
    set(favoritesAtom, await commands.loadFavorites());
  } catch (error) {
    console.warn("Unable to load favorites", error);
    set(favoritesAtom, []);
  }
});

export const addFavoritePathsAtom = atom(null, (get, set, paths: string[]) => {
  const favorites = get(favoritesAtom) ?? [];
  const existing = new Set(favorites.map((favorite) => favorite.path));
  const additions = paths
    .filter((path) => !existing.has(path))
    .map((path) => ({ path, name: favoriteNameFromPath(path) }));

  if (additions.length === 0) return;

  persistFavorites(set, [...favorites, ...additions]);
});

export const removeFavoriteAtom = atom(null, (get, set, path: string) => {
  const favorites = get(favoritesAtom) ?? [];
  if (!favorites.some((favorite) => favorite.path === path)) return;

  persistFavorites(
    set,
    favorites.filter((favorite) => favorite.path !== path),
  );
});

export const toggleFavoriteAtom = atom(null, (get, set, favorite: Favorite) => {
  const favorites = get(favoritesAtom) ?? [];
  if (favorites.some((item) => item.path === favorite.path)) {
    persistFavorites(
      set,
      favorites.filter((item) => item.path !== favorite.path),
    );
    return;
  }

  persistFavorites(set, [...favorites, favorite]);
});

export const reorderFavoritesAtom = atom(null, (_get, set, favorites: Favorite[]) => {
  persistFavorites(set, favorites);
});

function persistFavorites(
  set: (atom: typeof favoritesAtom, value: Favorite[]) => void,
  favorites: Favorite[],
) {
  set(favoritesAtom, favorites);
  void commands
    .saveFavorites(favorites)
    .catch((error: unknown) => console.warn("Unable to save favorites", error));
}

export function favoriteNameFromPath(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const separatorIndex = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const name = separatorIndex >= 0 ? trimmed.slice(separatorIndex + 1) : trimmed;

  return name || path;
}
