/** Transfer intent of a drag gesture, including Windows-style "link"
 * (create shortcut, Alt-drag). */
export type FileTransferOperation = "copy" | "move" | "link";

/** Operations the copy/move transfer pipeline understands; "link" shortcuts
 * are handled by their own command and never reach the conflict dialog. */
export type TransferOperation = Exclude<FileTransferOperation, "link">;

/** Resolves the Windows-explorer-style modifier state to a drag operation:
 * Alt (or Ctrl+Shift) creates shortcuts, Ctrl copies, plain/Shift moves. */
export function dragOperationFromModifiers(event: {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): FileTransferOperation {
  if (event.altKey || (event.ctrlKey && event.shiftKey)) return "link";
  if (event.ctrlKey || event.metaKey) return "copy";
  return "move";
}

/** Same Windows conventions for drags that leave the window, where the plain
 * gesture copies (Explorer's cross-volume default). */
export function dragOutModeFromModifiers(event: {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): "copy" | "move" | "link" {
  if (event.altKey || (event.ctrlKey && event.shiftKey)) return "link";
  if (event.shiftKey) return "move";
  return "copy";
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
