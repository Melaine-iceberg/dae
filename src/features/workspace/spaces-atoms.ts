import { atom, getDefaultStore } from "jotai";

import { commands, type Space } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";

const store = getDefaultStore();

/** `null` means the spaces have not been loaded from the backend yet. */
export const spacesAtom = atom<Space[] | null>(null);

/**
 * Why the last load failed, or `null`. A failed read leaves `spacesAtom` at
 * `null` so a retry re-queries instead of short-circuiting — see the note in
 * `recents-atoms.ts`.
 */
export const spacesErrorAtom = atom<string | null>(null);

/**
 * Set by the sidebar's "Rename space" action so the space view opens directly
 * in rename mode. Holds a space id, or null when idle.
 */
export const spaceRenameRequestAtom = atom<string | null>(null);

export const ensureSpacesLoadedAtom = atom(null, async (get, set) => {
  if (get(spacesAtom) !== null) return;

  set(spacesErrorAtom, null);
  try {
    set(spacesAtom, await commands.listSpaces());
  } catch (error) {
    console.warn("Unable to load spaces", error);
    set(spacesErrorAtom, getFileOperationErrorMessage(error));
  }
});

export async function createSpace(name: string): Promise<Space | null> {
  try {
    const space = await commands.createSpace(name);
    store.set(spacesAtom, [...(store.get(spacesAtom) ?? []), space]);
    return space;
  } catch (error) {
    console.warn("Unable to create space", error);
    return null;
  }
}

export async function renameSpace(spaceId: string, name: string): Promise<boolean> {
  try {
    const updated = await commands.renameSpace(spaceId, name);
    replaceSpace(updated);
    return true;
  } catch (error) {
    console.warn("Unable to rename space", error);
    return false;
  }
}

export async function deleteSpace(spaceId: string): Promise<boolean> {
  try {
    await commands.deleteSpace(spaceId);
    store.set(
      spacesAtom,
      (store.get(spacesAtom) ?? []).filter((space) => space.id !== spaceId),
    );
    return true;
  } catch (error) {
    console.warn("Unable to delete space", error);
    return false;
  }
}

/** Pins paths inside a space. Paths already present are kept as-is. */
export async function addItemsToSpace(spaceId: string, paths: string[]): Promise<void> {
  for (const path of paths) {
    try {
      replaceSpace(await commands.addSpaceItem(spaceId, path));
    } catch (error) {
      console.warn(`Unable to add ${path} to space ${spaceId}`, error);
    }
  }
}

export async function removeSpaceItem(spaceId: string, path: string): Promise<void> {
  try {
    replaceSpace(await commands.removeSpaceItem(spaceId, path));
  } catch (error) {
    console.warn(`Unable to remove ${path} from space ${spaceId}`, error);
  }
}

function replaceSpace(updated: Space): void {
  store.set(
    spacesAtom,
    (store.get(spacesAtom) ?? []).map((space) => (space.id === updated.id ? updated : space)),
  );
}
