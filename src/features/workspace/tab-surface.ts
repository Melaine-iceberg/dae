import { atom } from "jotai";
import { atomFamily } from "jotai/utils";

import type { WorkspaceSurface } from "./types";

/**
 * Each tab owns one surface, keyed by tab id. Kept in its own module (no
 * imports from the explorer) so both the tab model and workspace actions
 * can use it without an import cycle.
 *
 * New tabs land on the Overview surface: the design system requires the
 * default landing to be a workspace overview rather than a raw filesystem
 * path.
 */
export const tabSurfaceFamily = atomFamily((_tabId: string) =>
  atom<WorkspaceSurface>({ kind: "overview" }),
);
