import { atomWithStorage } from "jotai/utils";

import { localeCollator } from "@/i18n/format";

import {
  DIRECTORY_PRESENTATION,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
  getFileExtension,
  getFilePresentation,
} from "./file-icons";
import type { DirectoryEntry, EntryKind } from "./types";
import { filteredListingView, sortedListingView, type ListingView } from "./listing-view";

import {
  createComparator,
  isNumericSortKey,
  SORT_COLLATOR_OPTIONS,
  sortIndices,
  type ExplorerSortKey,
  type ExplorerSortOrder,
  type SortPrimitives,
} from "./entry-order";

export type { ExplorerSortKey, ExplorerSortOrder } from "./entry-order";

export type ExplorerViewMode = "list" | "grid" | "column";
export type ExplorerDensity = "compact" | "comfortable" | "spacious";
export type ExplorerIconStyle = "system" | "themed";
export type ExplorerKindFilter = "all" | "folders" | "files" | "images";
export type ExplorerModifiedFilter = "any" | "today" | "week" | "month";
export type ExplorerSizeFilter = "any" | "small" | "medium" | "large";

export const DENSITY_ROW_HEIGHT: Record<ExplorerDensity, number> = {
  compact: 28,
  comfortable: 34,
  spacious: 42,
};

/** Sensible initial direction when switching to a sort key (SKILL.md §18). */
export const DEFAULT_SORT_ORDER: Record<ExplorerSortKey, ExplorerSortOrder> = {
  name: "asc",
  modified: "desc",
  type: "asc",
  size: "desc",
};

export const viewModeAtom = atomWithStorage<ExplorerViewMode>("explorer.viewMode", "list");
export const densityAtom = atomWithStorage<ExplorerDensity>("explorer.density", "comfortable");
export const sortKeyAtom = atomWithStorage<ExplorerSortKey>("explorer.sortKey", "name");
export const sortOrderAtom = atomWithStorage<ExplorerSortOrder>("explorer.sortOrder", "asc");
export const foldersFirstAtom = atomWithStorage<boolean>("explorer.foldersFirst", true);

/**
 * Which icons a listing draws.
 *
 * `"system"` asks the shell for every file and folder — the OS's own artwork is
 * what makes a file manager read as part of the desktop rather than as an app
 * drawn in its colours. `"themed"` keeps the built-in glyph set and defers to
 * the shell only where a glyph cannot say what the file is (Windows shortcuts,
 * installers, and extensions the map does not know); see `native-icon.tsx`.
 */
export const iconStyleAtom = atomWithStorage<ExplorerIconStyle>("explorer.iconStyle", "system");

/**
 * Whether hidden entries are listed at all. Defaults to `true` (the app has
 * always shown them); toggled from the filter menu, Ctrl/Cmd+H, or the
 * command bar, and persisted across sessions.
 */
export const showHiddenFilesAtom = atomWithStorage<boolean>("explorer.showHiddenFiles", true);

/** Strips hidden entries when visibility is off; passes the list through untouched otherwise. */
export function filterHiddenEntries(
  entries: readonly DirectoryEntry[],
  showHiddenFiles: boolean,
): DirectoryEntry[] {
  return showHiddenFiles ? (entries as DirectoryEntry[]) : entries.filter((entry) => !entry.hidden);
}

/**
 * The same rule over a view. `hidden` is a header flag on a packed row, so the
 * scan reads one byte per entry instead of building an entry to look at it.
 */
export function filterHidden(view: ListingView, showHiddenFiles: boolean): ListingView {
  return showHiddenFiles
    ? view
    : filteredListingView(view, (index) => !view.hiddenAt(index));
}

export interface ExplorerEntryFilters {
  kind: ExplorerKindFilter;
  modified: ExplorerModifiedFilter;
  size: ExplorerSizeFilter;
}

export const DEFAULT_ENTRY_FILTERS: ExplorerEntryFilters = {
  kind: "all",
  modified: "any",
  size: "any",
};

export const entryFiltersAtom = atomWithStorage<ExplorerEntryFilters>(
  "explorer.entryFilters",
  DEFAULT_ENTRY_FILTERS,
);

export function hasActiveEntryFilters(filters: ExplorerEntryFilters): boolean {
  return (
    filters.kind !== DEFAULT_ENTRY_FILTERS.kind ||
    filters.modified !== DEFAULT_ENTRY_FILTERS.modified ||
    filters.size !== DEFAULT_ENTRY_FILTERS.size
  );
}

/** Size buckets in bytes (SKILL.md §16 filters). */
const SIZE_FILTER_RANGES: Record<Exclude<ExplorerSizeFilter, "any">, [number, number]> = {
  small: [0, 1024 * 1024],
  medium: [1024 * 1024, 100 * 1024 * 1024],
  large: [100 * 1024 * 1024, Number.MAX_SAFE_INTEGER],
};

const IMAGE_EXTENSIONS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "bmp",
  "svg",
  "ico",
  "tif",
  "tiff",
  "heic",
]);

const MS_PER_DAY = 86_400_000;

/**
 * Filters entries by kind / modified time / size (SKILL.md §16). Runs before
 * sorting on every render, so the predicate stays allocation-free and cheap;
 * a single pass skips early when no filter is active.
 *
 * Reads the view's scalar accessors rather than materialising rows: the scan
 * covers every entry, and building an object per entry to look at two fields of
 * it would both cost the allocation and evict the painted rows from the row
 * cache.
 */
export function applyEntryFilters(
  view: ListingView,
  filters: ExplorerEntryFilters,
): ListingView {
  if (!hasActiveEntryFilters(filters)) {
    return view;
  }

  const now = Date.now();
  const modifiedCutoff =
    filters.modified === "today"
      ? new Date(now).setHours(0, 0, 0, 0)
      : filters.modified === "week"
        ? now - 7 * MS_PER_DAY
        : filters.modified === "month"
          ? now - 30 * MS_PER_DAY
          : 0;
  const sizeRange = filters.size === "any" ? null : SIZE_FILTER_RANGES[filters.size];

  return filteredListingView(view, (index) => {
    const kind = view.kindAt(index);

    if (filters.kind === "folders") {
      if (kind !== "directory") return false;
    } else if (filters.kind === "files") {
      if (kind !== "file") return false;
    } else if (filters.kind === "images") {
      const name = view.nameAt(index);
      if (kind !== "file" || name === undefined || !IMAGE_EXTENSIONS.has(getFileExtension(name))) {
        return false;
      }
    }

    if (modifiedCutoff > 0 && (view.modifiedAt(index) ?? 0) < modifiedCutoff) return false;
    if (sizeRange && kind === "file") {
      const size = view.sizeAt(index) ?? 0;
      if (size < sizeRange[0] || size >= sizeRange[1]) return false;
    }

    return true;
  });
}

function entryTypeLabel(kind: EntryKind, name: string): string {
  switch (kind) {
    case "directory":
      return DIRECTORY_PRESENTATION.label;
    case "symlink":
      return SYMLINK_PRESENTATION.label;
    case "other":
      return OTHER_PRESENTATION.label;
    default:
      return getFilePresentation(name).label;
  }
}

/**
 * Collects the comparable form of `view[from, to)` for the active sort key.
 *
 * The point is that it runs once per entry rather than once per comparison.
 * The `"type"` key used to resolve a localized label inside the comparator,
 * which for a 35,803 entry directory meant ~1.7M `i18n.t` lookups and icon
 * table probes per ordering pass.
 *
 * For `"name"` the primary *is* the name array, so building the primitives
 * costs one pass and no extra allocation.
 *
 * The range form is what a streamed listing uses: the ordering hook only ever
 * asks for the rows a batch added, because the worker keeps everything before
 * them.
 *
 * `"type"` is the one key the worker cannot derive for itself — the label it
 * sorts by is localized, and `entry-order.ts` may not reach for i18n — so its
 * primaries are always collected here, on the main thread.
 */
export function collectSortPrimitives(
  view: ListingView,
  key: ExplorerSortKey,
  from = 0,
  to = view.count,
): SortPrimitives {
  const count = Math.max(0, to - from);
  const names: string[] = [];
  const directoryFlags = new Uint8Array(count);

  for (let offset = 0; offset < count; offset++) {
    const index = from + offset;
    names.push(view.nameAt(index) ?? "");
    directoryFlags[offset] = view.kindAt(index) === "directory" ? 1 : 0;
  }

  if (key === "name") {
    return { directoryFlags, names, primaries: names };
  }

  if (isNumericSortKey(key)) {
    const primaries = new Float64Array(count);
    for (let offset = 0; offset < count; offset++) {
      const index = from + offset;
      // Directories carry no size and no timestamp below the root; the
      // previous in-place comparator folded those to 0 too.
      primaries[offset] = (key === "modified" ? view.modifiedAt(index) : view.sizeAt(index)) ?? 0;
    }
    return { directoryFlags, names, primaries };
  }

  const primaries: string[] = [];
  for (let offset = 0; offset < count; offset++) {
    const index = from + offset;
    primaries.push(entryTypeLabel(view.kindAt(index) ?? "other", view.nameAt(index) ?? ""));
  }
  return { directoryFlags, names, primaries };
}

/**
 * The collator the whole app orders with, built for the active locale.
 * Shared with the sort worker so both threads collate identically.
 */
export function sortCollator(): Intl.Collator {
  return localeCollator(SORT_COLLATOR_OPTIONS);
}

/**
 * Sorts a view for display, returning a view that reads the source through the
 * permutation. With `foldersFirst` (default) directories always group ahead of
 * files regardless of the active key (predictable spatial convention); names
 * break ties with a natural-order collator so file2 < file10. When disabled,
 * entries interleave purely by the key.
 *
 * This is the one-shot form, for lists that arrive whole and for the snapshots
 * below the streaming threshold. Streamed listings go through
 * `useSortedListingView`, which folds each batch into the previous order
 * instead of re-sorting the snapshot.
 */
export function sortListingView(
  view: ListingView,
  key: ExplorerSortKey,
  order: ExplorerSortOrder,
  foldersFirst = true,
): ListingView {
  if (view.count === 0) {
    return view;
  }

  const compare = createComparator(
    collectSortPrimitives(view, key),
    { foldersFirst, sortKey: key, sortOrder: order },
    sortCollator(),
  );

  return sortedListingView(view, Int32Array.from(sortIndices(view.count, compare)));
}
