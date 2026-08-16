/**
 * The primary information architecture of the file workspace.
 *
 * Every tab shows exactly one surface at a time. `folder` is the classic
 * explorer surface; the other surfaces come from the workspace design system
 * (Overview / Recent / Favorites / Spaces).
 */
export type WorkspaceSurface =
  | { kind: "overview" }
  | { kind: "recents" }
  | { kind: "favorites" }
  | { kind: "space"; spaceId: string }
  | { kind: "folder" };

export type WorkspaceSurfaceKind = WorkspaceSurface["kind"];

export const WORKSPACE_SURFACE_LABELS: Record<Exclude<WorkspaceSurfaceKind, "space">, string> = {
  overview: "概览",
  recents: "最近使用",
  favorites: "收藏",
  folder: "文件夹",
};
