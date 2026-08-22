/**
 * The primary information architecture of the file workspace.
 *
 * Every tab shows exactly one surface at a time. `folder` is the classic
 * explorer surface; the other surfaces come from the workspace design system
 * (Overview / Recent / Favorites / Spaces).
 */
import { i18n } from "@/i18n";

export type WorkspaceSurface =
  | { kind: "overview" }
  | { kind: "recents" }
  | { kind: "favorites" }
  | { kind: "space"; spaceId: string }
  | { kind: "folder" };

export type WorkspaceSurfaceKind = WorkspaceSurface["kind"];

/** Display labels for non-space surfaces, resolved against the active locale. */
export const WORKSPACE_SURFACE_LABELS: Record<Exclude<WorkspaceSurfaceKind, "space">, string> = {
  get overview() {
    return i18n.t("workspace:surfaces.overview");
  },
  get recents() {
    return i18n.t("workspace:surfaces.recents");
  },
  get favorites() {
    return i18n.t("workspace:surfaces.favorites");
  },
  get folder() {
    return i18n.t("workspace:surfaces.folder");
  },
};

/** Seed names the preset spaces ship with, used to detect unrenamed presets. */
export const PRESET_SPACE_SEED_NAMES: Record<string, string> = {
  work: "工作",
  personal: "个人",
  shared: "共享",
  archive: "归档",
};

/**
 * Locale-aware space display name: a preset space that still carries its seed
 * name is shown translated, while renamed (or custom) spaces keep their name.
 */
export function getSpaceDisplayName(space: {
  id: string;
  isPreset: boolean;
  name: string;
}): string {
  if (space.isPreset && PRESET_SPACE_SEED_NAMES[space.id] === space.name) {
    return i18n.t(`workspace:spaces.${space.id}`);
  }
  return space.name;
}
