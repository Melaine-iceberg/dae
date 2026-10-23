import { atom } from "jotai";

import type { FileOperationProgress } from "./types";

/**
 * What the window's bottom strip is currently saying.
 *
 * The strip is a property of the *window*, not of a pane: there is one
 * terminal drawer and one status strip under it, and both have to describe
 * whichever folder pane is focused. The pane knows the facts (item count,
 * selection, a running operation); the window owns the row they are painted
 * on. This atom is the seam between the two.
 *
 * `owner` is the publishing pane's stable id. A pane that goes inactive may
 * only clear the strip if the strip is still *its* — otherwise switching tabs
 * in a split view or between two folder tabs races, and the outgoing pane's
 * cleanup can erase the incoming pane's freshly published status.
 */
export interface ExplorerStatusSnapshot {
  owner: string;
  /** A running file operation takes the strip over from the idle facts. */
  progress: FileOperationProgress | null;
  isLoading: boolean;
  isSearching: boolean;
  itemCount: number;
  path: string | null;
  selectedCount: number;
  /** The focused pane's repository, if its folder is inside one. Kept here
   *  rather than passed to the strip as a prop because the strip is mounted by
   *  the window, not by the pane — the same seam as every other fact above. */
  git: { branch: string; root: string } | null;
}

export const explorerStatusAtom = atom<ExplorerStatusSnapshot | null>(null);
