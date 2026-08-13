import { atomWithStorage } from "jotai/utils";

export type ExplorerViewMode = "list" | "grid" | "column";
export type ExplorerDensity = "compact" | "comfortable" | "spacious";

export const DENSITY_ROW_HEIGHT: Record<ExplorerDensity, number> = {
  compact: 36,
  comfortable: 44,
  spacious: 52,
};

export const viewModeAtom = atomWithStorage<ExplorerViewMode>("explorer.viewMode", "list");
export const densityAtom = atomWithStorage<ExplorerDensity>("explorer.density", "comfortable");
