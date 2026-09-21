import { atom, getDefaultStore } from "jotai";

import { commands, type EntryKind, type RecentItem, type RecentSource } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";

const MAX_RECENT_ITEMS = 300;
const store = getDefaultStore();

/** `null` means the recents have not been loaded from the backend yet. */
export const recentsAtom = atom<RecentItem[] | null>(null);

/**
 * Why the last load failed, or `null` while there is nothing to report.
 *
 * A failed read leaves `recentsAtom` at `null` rather than substituting `[]`:
 * `null` keeps meaning "not loaded", so the next `ensureRecentsLoaded` — a
 * remount, or the retry button — re-runs the query instead of short-circuiting
 * on a fabricated empty list, and the surfaces can tell "empty" apart from
 * "failed" instead of showing 还没有记录 for a broken read.
 */
export const recentsErrorAtom = atom<string | null>(null);

export const ensureRecentsLoadedAtom = atom(null, async (get, set) => {
  if (get(recentsAtom) !== null) return;

  set(recentsErrorAtom, null);
  try {
    set(recentsAtom, await commands.listRecents());
  } catch (error) {
    console.warn("Unable to load recent items", error);
    set(recentsErrorAtom, getFileOperationErrorMessage(error));
  }
});

/**
 * Records one access in the backend store and mirrors it into the atom when
 * the recents are loaded. Safe to call from non-React code (navigation,
 * file open handlers); failures only skip the recents entry.
 */
export function recordRecentItem(path: string, kind: EntryKind, source: RecentSource): void {
  const current = store.get(recentsAtom);

  if (current !== null) {
    const optimistic: RecentItem = {
      path,
      name: displayNameFromPath(path),
      kind,
      source,
      accessedAt: Date.now(),
    };
    store.set(
      recentsAtom,
      [optimistic, ...current.filter((item) => item.path !== path)].slice(0, MAX_RECENT_ITEMS),
    );
  }

  void commands
    .recordRecent(path, kind, source)
    .then((items) => {
      if (store.get(recentsAtom) !== null) {
        store.set(recentsAtom, items);
      }
    })
    .catch((error: unknown) => console.warn("Unable to record recent item", error));
}

export function removeRecentItem(path: string): void {
  const current = store.get(recentsAtom);
  if (current !== null) {
    store.set(
      recentsAtom,
      current.filter((item) => item.path !== path),
    );
  }

  void commands
    .removeRecent(path)
    .then((items) => store.set(recentsAtom, items))
    .catch((error: unknown) => console.warn("Unable to remove recent item", error));
}

export function clearRecentItems(): void {
  store.set(recentsAtom, []);
  void commands
    .clearRecents()
    .catch((error: unknown) => console.warn("Unable to clear recent items", error));
}

/** Mirrors the backend's `display_name_from_path` for optimistic updates. */
function displayNameFromPath(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const separatorIndex = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const name = separatorIndex >= 0 ? trimmed.slice(separatorIndex + 1) : trimmed;

  return name || path;
}
