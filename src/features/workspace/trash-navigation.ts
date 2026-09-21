/**
 * Keyboard model for the Trash list.
 *
 * The recycle bin was the last surface reachable by pointer only: rows were
 * buttons you could click and nothing else. Arrow keys, Home/End, paging and
 * type-to-jump are not rebindable actions, so none of this belongs in the
 * shortcut registry — but it is exactly the part of a list that is worth
 * testing, and it cannot be tested through a component without a DOM.
 *
 * Everything here is therefore pure index arithmetic: the component owns the
 * focus, the scrolling and the selection, and asks these functions only "which
 * row is next".
 */

/** Page step for PageUp/PageDown, in rows. Callers pass the measured page. */
export type TrashJump = "first" | "last" | "pageUp" | "pageDown";

/**
 * Moves the cursor by whole rows and clamps it to the list.
 *
 * Nothing wraps: ArrowDown on the last row stays there, which is what both
 * Explorer and the platform list widgets do, and it keeps `activeIndex` a
 * trustworthy description of "the row the keyboard is on".
 *
 * From "no cursor" (-1) the first move lands on the first row downwards and on
 * the last row upwards, so the first arrow key press from a freshly focused
 * list lands somewhere sensible rather than being ignored.
 */
export function stepTrashIndex(current: number, count: number, delta: number): number {
  if (count <= 0) return -1;
  if (delta === 0) return clampIndex(current, count);
  if (current < 0) return delta > 0 ? 0 : count - 1;
  return clampIndex(current + delta, count);
}

/**
 * Home / End / PageUp / PageDown. `pageSize` is measured by the caller from the
 * scroller's height, so paging follows the real list instead of a guess.
 */
export function jumpTrashIndex(
  current: number,
  count: number,
  jump: TrashJump,
  pageSize: number,
): number {
  if (count <= 0) return -1;

  switch (jump) {
    case "first":
      return 0;
    case "last":
      return count - 1;
    case "pageUp":
      return clampIndex((current < 0 ? 0 : current) - Math.max(1, pageSize), count);
    case "pageDown":
      return clampIndex((current < 0 ? -1 : current) + Math.max(1, pageSize), count);
  }
}

/**
 * Windows-style type-ahead: a prefix match over the visible names, starting one
 * row past the cursor.
 *
 * The off-by-one is the whole behaviour. A single repeated character ("f", "f",
 * "f") has to *cycle* through the entries that start with it, so the search
 * starts after the current row; a longer buffer is a refinement, so it starts
 * *on* the current row and keeps the entry if it still matches. Matching is
 * case-insensitive and wraps, so the last match is not a dead end.
 *
 * Returns -1 when nothing matches, which the caller reads as "leave the cursor
 * where it is" — retyping a letter that matched nothing must not move the list.
 */
export function typeAheadTrashIndex(
  names: readonly string[],
  query: string,
  fromIndex: number,
): number {
  const count = names.length;
  const needle = query.trim().toLowerCase();
  if (count <= 0 || needle.length === 0) return -1;

  const start = fromIndex < 0 ? 0 : needle.length === 1 ? fromIndex + 1 : fromIndex;

  for (let offset = 0; offset < count; offset += 1) {
    const index = (start + offset) % count;
    if ((names[index] ?? "").toLowerCase().startsWith(needle)) return index;
  }

  return -1;
}

/**
 * Which entries a delete press acts on.
 *
 * Same rule the explorer's file operations use: the whole selection when the
 * row under the cursor is part of it, otherwise just that row. Without it,
 * pressing Delete while the cursor sits outside the selection would silently
 * destroy the selection the user can see highlighted.
 */
export function trashPurgeTargets(
  entries: readonly { id: string }[],
  activeIndex: number,
  selectedIds: readonly string[],
): string[] {
  const active = entries[activeIndex];
  if (!active) return [];

  return selectedIds.includes(active.id) ? [...selectedIds] : [active.id];
}

function clampIndex(index: number, count: number): number {
  if (count <= 0) return -1;
  return Math.min(count - 1, Math.max(0, index));
}
