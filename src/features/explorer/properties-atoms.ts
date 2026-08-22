import { atom } from "jotai";

import type { DirectoryEntry } from "./types";

/** The entry whose properties dialog is open; `null` keeps the dialog closed.
 *  Kept in a global atom so any context menu (list/grid/columns) can open it
 *  without threading callbacks through every view. Lives apart from the
 *  dialog component so eager modules can set it without pulling the dialog
 *  chunk into the entry bundle. */
export const propertiesTargetAtom = atom<DirectoryEntry | null>(null);
