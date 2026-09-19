/**
 * Entry ordering, shared by the main thread and the sort worker.
 *
 * A streaming listing shows up as a sequence of growing snapshots: every
 * batch hands the explorer a slightly longer array. Ordering each snapshot
 * from scratch costs `n log n` collator comparisons per batch, and
 * `Intl.Collator` — with `numeric: true` in particular — is expensive enough
 * that a 35,803 entry directory spent ~123 ms of main-thread time doing it
 * (measured, 8 batches).
 *
 * Two things fix that here. Sort primitives are collected once per entry
 * instead of being re-derived inside the comparator, and an incoming batch is
 * merged into the previous order rather than re-sorting the whole snapshot.
 *
 * Nothing in this module may import React, i18n, or the icon tables: the sort
 * worker bundles it onto its own thread, where an i18n instance would be a
 * second, divergent copy. The main-thread-only pieces that collect primitives
 * live in `preferences.ts`.
 */

export type ExplorerSortKey = "name" | "modified" | "type" | "size";
export type ExplorerSortOrder = "asc" | "desc";

export interface OrderOptions {
  sortKey: ExplorerSortKey;
  sortOrder: ExplorerSortOrder;
  /** Directories group ahead of everything else regardless of the sort key. */
  foldersFirst: boolean;
}

/**
 * The comparable form of a batch of entries, aligned by index.
 *
 * `primaries` is the only array whose element type varies: `"name"` and
 * `"type"` compare strings through the collator, `"modified"` and `"size"`
 * compare numbers. `names` is always present because every comparison falls
 * back to it to break ties, whichever key is active.
 */
export interface SortPrimitives {
  primaries: string[] | Float64Array;
  names: string[];
  /** 1 for directories, so `foldersFirst` needs no per-entry metadata. */
  directoryFlags: Uint8Array;
}

/** Whether the active key compares numbers rather than collated strings. */
export function isNumericSortKey(sortKey: ExplorerSortKey): boolean {
  return sortKey === "modified" || sortKey === "size";
}

/**
 * The collator every comparison in the app uses, for every sort key.
 *
 * It is deliberately not specialised per key. The name comparison is the
 * tie-break for *all* keys, so dropping `numeric` for a numeric key would
 * reorder same-size or same-timestamp entries ("file2" before "file10"
 * becomes "file10" before "file2") — a visible change for no gain, since the
 * main cost was never this option but the number of comparisons.
 */
export const SORT_COLLATOR_OPTIONS: Intl.CollatorOptions = {
  numeric: true,
  sensitivity: "base",
};

/** Stable identity for a set of ordering options, for cache invalidation. */
export function orderSignature(options: OrderOptions): string {
  return `${options.sortKey}|${options.sortOrder}|${options.foldersFirst ? "folders" : "flat"}`;
}

/**
 * Builds the comparator for one ordering pass. Everything constant is resolved
 * here — the numeric/string branch, the direction, the folder grouping — so the
 * returned function only touches the arrays.
 *
 * The ordering reproduces the previous in-place comparator exactly:
 * `foldersFirst` is applied outside the direction multiplier (directories lead
 * in both directions), the key comparison is scaled by the direction, and the
 * name tie-break is always ascending.
 */
export function createComparator(
  primitives: SortPrimitives,
  options: OrderOptions,
  collator: Intl.Collator,
): (left: number, right: number) => number {
  const { names, directoryFlags } = primitives;
  const direction = options.sortOrder === "asc" ? 1 : -1;
  const foldersFirst = options.foldersFirst;

  if (isNumericSortKey(options.sortKey)) {
    const values = primitives.primaries as Float64Array;

    return foldersFirst
      ? (left, right) => {
          const folder = directoryFlags[right] - directoryFlags[left];
          if (folder !== 0) return folder;
          const difference = values[left] - values[right];
          if (difference !== 0) return difference * direction;
          return collator.compare(names[left], names[right]);
        }
      : (left, right) => {
          const difference = values[left] - values[right];
          if (difference !== 0) return difference * direction;
          return collator.compare(names[left], names[right]);
        };
  }

  const values = primitives.primaries as string[];

  return foldersFirst
    ? (left, right) => {
        const folder = directoryFlags[right] - directoryFlags[left];
        if (folder !== 0) return folder;
        const difference = collator.compare(values[left], values[right]);
        if (difference !== 0) return difference * direction;
        return collator.compare(names[left], names[right]);
      }
    : (left, right) => {
        const difference = collator.compare(values[left], values[right]);
        if (difference !== 0) return difference * direction;
        return collator.compare(names[left], names[right]);
      };
}

/** `[0, count)` ordered by `compare`. */
export function sortIndices(
  count: number,
  compare: (left: number, right: number) => number,
): number[] {
  const indices: number[] = [];
  for (let index = 0; index < count; index++) {
    indices.push(index);
  }

  indices.sort(compare);
  return indices;
}

/**
 * Folds two runs that `compare` already ordered into one, in linear time with
 * exactly `left.length + right.length` comparisons in the worst case.
 *
 * Ties keep the left run first, which is what a stable sort would do, so
 * merging streamed batches yields the same order as one sort over the whole
 * snapshot.
 */
export function mergeRuns(
  left: readonly number[],
  right: readonly number[],
  compare: (left: number, right: number) => number,
): number[] {
  const merged: number[] = [];
  let leftIndex = 0;
  let rightIndex = 0;

  while (leftIndex < left.length && rightIndex < right.length) {
    merged.push(
      compare(right[rightIndex], left[leftIndex]) < 0
        ? right[rightIndex++]
        : left[leftIndex++],
    );
  }
  while (leftIndex < left.length) merged.push(left[leftIndex++]);
  while (rightIndex < right.length) merged.push(right[rightIndex++]);

  return merged;
}
