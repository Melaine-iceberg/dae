/**
 * The pane's selection: which rows are selected, pruning it against the
 * listing as batches stream in, and clearing it when the view's context
 * changes (new directory, new query, content search taking over the list).
 *
 * The prune path deliberately reads through a ref that is synced in an effect
 * rather than during render: a marquee drag rewrites the selection on every
 * pointer move, and a full-listing scan per move is exactly what the ref
 * exists to avoid.
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";

import { allPaths, entriesWhere, presentPaths, type ListingView } from "./listing-view";
import type { DirectoryEntry } from "./types";

/** Shared stand-in for "no listing yet": a fresh `[]` per render would defeat
 *  the identity checks the entry-ordering hook relies on — and the sentinel is
 *  exported because the view's source-listing fallback needs the same
 *  guarantee. */
export const NO_ENTRIES: DirectoryEntry[] = [];

export function useExplorerSelection({
  displayedListing,
  isContentSearchActive,
  directoryPath,
  searchQuery,
}: {
  displayedListing: ListingView;
  isContentSearchActive: boolean;
  directoryPath: string | undefined;
  searchQuery: string;
}): {
  selectedPaths: string[];
  setSelectedPaths: Dispatch<SetStateAction<string[]>>;
  selectedEntries: DirectoryEntry[];
  selectAll: () => void;
} {
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const selectedPathSet = useMemo(() => new Set(selectedPaths), [selectedPaths]);
  // Latest selection, for the prune effect below to read without making the
  // selection one of its dependencies.
  //
  // Synced in an effect rather than during render. A render-phase ref write is
  // an error to React Compiler's validator — verified by injecting one into a
  // component that compiles, which drops it from compilation — so writing it
  // during render would keep the view out of the compiler pass even after its
  // other bailouts are fixed. Declared *before* the prune effect, so the ref
  // is already current when that one runs in the same commit.
  const selectedPathsRef = useRef(selectedPaths);
  useEffect(() => {
    selectedPathsRef.current = selectedPaths;
  }, [selectedPaths]);

  // File selection belongs to the entry list, which content search replaces.
  useEffect(() => {
    if (isContentSearchActive) {
      setSelectedPaths([]);
    }
  }, [isContentSearchActive]);

  // A new directory or query starts from nothing selected.
  useEffect(() => {
    setSelectedPaths([]);
  }, [directoryPath, searchQuery]);

  useEffect(() => {
    const selection = selectedPathsRef.current;
    // With nothing selected there is nothing to prune.
    if (selection.length === 0) {
      return;
    }

    // `presentPaths` stops at the last path it is looking for, so a listing the
    // user is looking at costs a handful of rows rather than a scan of all of
    // them. It reads every row only when a selected path is gone, which is the
    // case the prune exists for.
    const present = presentPaths(displayedListing, selection);
    // Every selected path is still there: hand back the same array, so nothing
    // re-renders for a listing that did not change the selection.
    if (present.size === selection.length) {
      return;
    }

    setSelectedPaths((paths) => paths.filter((path) => present.has(path)));
  }, [displayedListing]);

  // Materialised for the selection only: the scan runs over paths, so rows
  // that are not selected are never built. Nothing selected is the common
  // case, and it is worth stating explicitly — the scan walks the whole
  // listing either way.
  const selectedEntries = useMemo(
    () =>
      selectedPaths.length === 0
        ? NO_ENTRIES
        : entriesWhere(displayedListing, (path) => selectedPathSet.has(path)),
    [displayedListing, selectedPathSet, selectedPaths.length],
  );

  const selectAll = useCallback(() => {
    setSelectedPaths(allPaths(displayedListing));
  }, [displayedListing]);

  return { selectedPaths, setSelectedPaths, selectedEntries, selectAll };
}
