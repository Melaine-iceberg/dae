import type { DragOutMode, KeyModifiers } from "@/bindings";

/** Transfer intent of a drag gesture, including Windows-style "link"
 * (create shortcut, Alt-drag). */
export type FileTransferOperation = "copy" | "move" | "link";

/** Operations the copy/move transfer pipeline understands; "link" shortcuts
 * are handled by their own command and never reach the conflict dialog. */
export type TransferOperation = Exclude<FileTransferOperation, "link">;

/** Whether a modifier key pins the drag to one effect. */
function isForcedDragOperation(modifiers: KeyModifiers): boolean {
  return modifiers.altKey || modifiers.ctrlKey || modifiers.metaKey || modifiers.shiftKey;
}

/** Resolves the Windows-explorer-style modifier state to a drag operation:
 * Alt (or Ctrl+Shift) creates shortcuts, Ctrl copies, plain/Shift moves. */
export function dragOperationFromModifiers(event: KeyModifiers): FileTransferOperation {
  if (event.altKey || (event.ctrlKey && event.shiftKey)) return "link";
  if (event.ctrlKey || event.metaKey) return "copy";
  return "move";
}

/** The native drag-out a gesture advertises: a modifier key pins it to one
 * effect, while the plain gesture offers copy + move + link and lets the drop
 * target choose. Which effect a plain drop lands on is the *target's* call — a
 * Shell folder moves same-volume drops and copies across volumes — and the
 * source cannot make that choice itself: it does not know where the drop will
 * land. */
export function dragOutModeFromModifiers(event: KeyModifiers): DragOutMode {
  return isForcedDragOperation(event) ? dragOperationFromModifiers(event) : "any";
}

/** The operation a drop of outside files performs: the modifiers held at the
 * drop pin one effect, a plain gesture follows the volume rule. */
export function resolveDropOperation(
  modifiers: KeyModifiers,
  sourcePaths: string[],
  destinationPath: string,
): FileTransferOperation {
  if (isForcedDragOperation(modifiers)) return dragOperationFromModifiers(modifiers);
  return defaultTransferOperation(sourcePaths, destinationPath);
}

/** Explorer's plain-drag default: a transfer moves when source and destination
 * share a volume and copies when it crosses one. A batch only moves when every
 * source shares the destination's volume — a mixed batch copies, so nothing
 * leaves its volume without a modifier key saying so. */
export function defaultTransferOperation(
  sourcePaths: string[],
  destinationPath: string,
): TransferOperation {
  if (sourcePaths.length === 0) return "copy";

  const destinationRoot = volumeRoot(destinationPath);
  return sourcePaths.every((sourcePath) => volumeRoot(sourcePath) === destinationRoot)
    ? "move"
    : "copy";
}

/** Identity of the volume a path lives on — two paths share one exactly when
 * moving between them is a rename. Drive letters and UNC shares are exact;
 * `scheme://host` network paths treat one host as one volume, and POSIX paths
 * only separate `/Volumes/<name>` mounts from the root filesystem, which is
 * coarse but conservative (an unreadable distinction copies instead of
 * moving). */
function volumeRoot(path: string): string {
  let rest = path;

  // `\\?\C:\x`, `\\?\UNC\server\share\x`: a verbatim prefix must come off
  // before the drive and UNC rules below can read the path — and a verbatim
  // UNC path spells its root as `UNC\server\share`, not `\\server\share`.
  const verbatim = /^[\\/]{2}\?[\\/](.+)$/.exec(rest);
  if (verbatim) {
    rest = verbatim[1].replace(/^UNC[\\/]/i, "\\\\");
  }

  const drive = /^([a-zA-Z]):/.exec(rest);
  if (drive) return drive[1].toLowerCase();

  const unc = /^[\\/]{2}([^\\/]+)[\\/]([^\\/]+)/.exec(rest);
  if (unc) return `\\\\${unc[1]}\\${unc[2]}`.toLowerCase();

  const networkUrl = /^([a-zA-Z][a-zA-Z0-9+.-]*):\/\/([^/]*)/.exec(rest);
  if (networkUrl) return `${networkUrl[1]}://${networkUrl[2]}`.toLowerCase();

  const mountedVolume = /^\/Volumes\/([^/]+)/.exec(rest);
  return mountedVolume ? `/Volumes/${mountedVolume[1]}`.toLowerCase() : "/";
}

const DIRECTORY_DROP_TARGET_SELECTOR = "[data-explorer-directory-drop-target]";
const DROP_TARGET_SELECTOR = "[data-explorer-drop-target]";
const SIDEBAR_FAVORITES_DROP_TARGET_SELECTOR = "[data-sidebar-favorites-drop-target]";
const SIDEBAR_SPACE_DROP_TARGET_SELECTOR = "[data-sidebar-space-drop-target]";

export function getExplorerDropTargetAtPoint(x: number, y: number): string | null {
  const element = document.elementFromPoint(x, y);
  if (!(element instanceof HTMLElement)) return null;

  const directoryTarget = element.closest<HTMLElement>(DIRECTORY_DROP_TARGET_SELECTOR);
  if (directoryTarget?.dataset.explorerDirectoryDropTarget) {
    return directoryTarget.dataset.explorerDirectoryDropTarget;
  }

  return element.closest<HTMLElement>(DROP_TARGET_SELECTOR)?.dataset.explorerDropTarget ?? null;
}

/** Whether the point is over any explorer surface at all, including its
 *  header, status bar and dialogs — a much wider net than the drop-target
 *  lookup above, which only matches the list rows and container. External
 *  drag-and-drop uses this to hand the drop to the explorer under the
 *  pointer instead of the pane that happened to be listening first. */
export function isExplorerContainerAtPoint(x: number, y: number): boolean {
  const element = document.elementFromPoint(x, y);
  return (
    element instanceof HTMLElement &&
    element.closest('[data-explorer-container="true"]') !== null
  );
}

export function isOverSidebarFavoritesAtPoint(x: number, y: number): boolean {
  const element = document.elementFromPoint(x, y);
  return (
    element instanceof HTMLElement &&
    element.closest(SIDEBAR_FAVORITES_DROP_TARGET_SELECTOR) !== null
  );
}

/** Returns the id of the sidebar space under the pointer, if any. */
export function getSidebarSpaceDropTargetAtPoint(x: number, y: number): string | null {
  const element = document.elementFromPoint(x, y);
  if (!(element instanceof HTMLElement)) return null;

  return (
    element.closest<HTMLElement>(SIDEBAR_SPACE_DROP_TARGET_SELECTOR)?.dataset
      .sidebarSpaceDropTarget ?? null
  );
}

export function canDropEntries(sourcePaths: string[], destinationPath: string): boolean {
  if (sourcePaths.length === 0 || sourcePaths.includes(destinationPath)) {
    return false;
  }

  return !sourcePaths.every((sourcePath) => parentPath(sourcePath) === destinationPath);
}

/** Mirrors the backend's scheme detection: only a `scheme://` prefix whose
 *  scheme part contains no path separators counts as a network path. */
export function isLocalExplorerPath(path: string): boolean {
  const separatorIndex = path.indexOf("://");
  if (separatorIndex < 1) return true;

  const scheme = path.slice(0, separatorIndex);
  if (scheme.includes("/") || scheme.includes("\\")) return true;

  return scheme.toLowerCase() === "file";
}

function parentPath(path: string): string | null {
  const separatorIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (separatorIndex < 0) return null;

  const parent = path.slice(0, separatorIndex);
  if (/^[a-zA-Z]:$/.test(parent)) {
    return `${parent}${path[separatorIndex]}`;
  }

  return parent || path[separatorIndex];
}
