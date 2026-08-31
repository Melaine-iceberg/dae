import { atomWithStorage } from "jotai/utils";

import { localeCollator } from "@/i18n/format";

import {
  DIRECTORY_PRESENTATION,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
  getFileExtension,
  getFilePresentation,
} from "./file-icons";
import type { DirectoryEntry } from "./types";

export type ExplorerViewMode = "list" | "grid" | "column";
export type ExplorerDensity = "compact" | "comfortable" | "spacious";
export type ExplorerSortKey = "name" | "modified" | "type" | "size";
export type ExplorerSortOrder = "asc" | "desc";
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
 */
export function applyEntryFilters(
  entries: readonly DirectoryEntry[],
  filters: ExplorerEntryFilters,
): DirectoryEntry[] {
  if (!hasActiveEntryFilters(filters)) {
    return entries as DirectoryEntry[];
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

  return entries.filter((entry) => {
    if (filters.kind === "folders") {
      if (entry.kind !== "directory") return false;
    } else if (filters.kind === "files") {
      if (entry.kind !== "file") return false;
    } else if (filters.kind === "images") {
      if (entry.kind !== "file" || !IMAGE_EXTENSIONS.has(getFileExtension(entry.name))) {
        return false;
      }
    }

    if (modifiedCutoff > 0 && (entry.modifiedAt ?? 0) < modifiedCutoff) return false;
    if (sizeRange && entry.kind === "file") {
      const size = entry.size ?? 0;
      if (size < sizeRange[0] || size >= sizeRange[1]) return false;
    }

    return true;
  });
}

const NAME_COLLATOR_OPTIONS: Intl.CollatorOptions = { numeric: true, sensitivity: "base" };

function entryTypeLabel(entry: DirectoryEntry): string {
  switch (entry.kind) {
    case "directory":
      return DIRECTORY_PRESENTATION.label;
    case "symlink":
      return SYMLINK_PRESENTATION.label;
    case "other":
      return OTHER_PRESENTATION.label;
    default:
      return getFilePresentation(entry.name).label;
  }
}

/**
 * Sorts entries for display. With `foldersFirst` (default) directories
 * always group ahead of files regardless of the active key (predictable
 * spatial convention); names break ties with a natural-order collator so
 * file2 < file10. When disabled, entries interleave purely by the key.
 */
export function sortEntries(
  entries: readonly DirectoryEntry[],
  key: ExplorerSortKey,
  order: ExplorerSortOrder,
  foldersFirst = true,
): DirectoryEntry[] {
  const direction = order === "asc" ? 1 : -1;
  const collator = localeCollator(NAME_COLLATOR_OPTIONS);

  return [...entries].sort((left, right) => {
    if (foldersFirst) {
      const folderDiff = Number(right.kind === "directory") - Number(left.kind === "directory");
      if (folderDiff !== 0) return folderDiff;
    }

    let comparison = 0;
    if (key === "name") {
      comparison = collator.compare(left.name, right.name);
    } else if (key === "modified") {
      comparison = (left.modifiedAt ?? 0) - (right.modifiedAt ?? 0);
    } else if (key === "size") {
      comparison = (left.size ?? 0) - (right.size ?? 0);
    } else {
      comparison = collator.compare(entryTypeLabel(left), entryTypeLabel(right));
    }

    if (comparison !== 0) return comparison * direction;
    return collator.compare(left.name, right.name);
  });
}
