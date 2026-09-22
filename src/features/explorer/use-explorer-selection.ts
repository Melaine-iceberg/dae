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
import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";

import { allPaths, entriesWhere, type ListingView } from "./listing-view";
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
    // With nothing selected there is nothing to prune, and building the
    // available-path set would walk the whole listing to filter an empty array.
    // This runs on every streamed batch, so on a large directory it's the
    // difference between a scan per batch and none.
    if (selectedPathsRef.current.length === 0) {
      return;
    }

    const availablePaths = new Set(allPaths(displayedListing));
    setSelectedPaths((paths) => {
      const availableSelection = paths.filter((path) => availablePaths.has(path));
      return availableSelection.length === paths.length ? paths : availableSelection;
    });
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
