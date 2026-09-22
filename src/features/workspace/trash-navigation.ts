/**
 * Keyboard model for the Trash list.
 *
 * Pure index arithmetic moved to {@link @/lib/list-navigation} when the
 * explorer's file list and grid grew the same behaviour; only the bin's own
 * selection rule lives here now.
 */

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
