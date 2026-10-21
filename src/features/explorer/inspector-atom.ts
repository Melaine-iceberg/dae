import { atom } from "jotai";

import type { DirectoryEntry } from "./types";

/**
 * What the window's inspector column is currently showing.
 *
 * The inspector is a property of the *window*, not of a pane: the reference
 * draws one column to the right of everything (`rail | main | insp`), beside
 * the main column rather than inside it, so the terminal drawer and the status
 * strip end at its hairline instead of running under it. The pane knows the
 * facts (its selection, whether its own inspector is open, what its buttons
 * do); the window owns the column they are painted in. This atom is the seam
 * between the two, exactly as `explorerStatusAtom` is for the bottom strip.
 *
 * `owner` is the publishing pane's stable id. A pane that loses focus may only
 * clear the column if the column is still *its* — otherwise the outgoing pane's
 * cleanup races the incoming pane's publish and erases the panel that was just
 * put up. A split therefore shows the focused pane's inspector and nothing of
 * the other's; each pane keeps its own open flag, so F6 brings back whichever
 * panel that side had.
 */
export interface InspectorSnapshot {
  owner: string;
  /** The publishing pane's own inspector toggle. Closed means no column at
   *  all — an empty 300px plate is not an inspector. */
  isOpen: boolean;
  entries: DirectoryEntry[];
  /** Collapses the panel; the pane owns the flag, so it has to clear it. */
  onClose: () => void;
  /** Opens the selection from the panel's own action row. */
  onOpen: () => void;
}

export const inspectorAtom = atom<InspectorSnapshot | null>(null);
