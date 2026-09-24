/**
 * Window- and backend-level subscriptions the explorer pane keeps alive:
 * directory refresh on change/focus events, and external (OS) drag-drop.
 *
 * Both effects hear window-wide events once per mounted pane, so their
 * cleanup promises matter: a tab switch unmounts one pane and mounts another,
 * and stale listeners would refresh or claim drops for a directory nobody is
 * looking at.
 */
import { useEffect, useState } from "react";

import { commands, events, type KeyModifiers } from "@/bindings";

import { getAppWindow } from "@/lib/app-window";

import {
  getExplorerDropTargetAtPoint,
  isExplorerContainerAtPoint,
  resolveDropOperation,
  type FileTransferOperation,
} from "./drag-drop";
import type { ExplorerNavigator } from "./navigation";

const DIRECTORY_REFRESH_DELAY_MS = 150;

/** How often the held modifier keys are re-read while an outside drag hovers:
 *  no DOM key event fires during a native drag, and the operation the badge
 *  announces has to follow a key the user presses mid-hover. */
const DROP_MODIFIER_POLL_MS = 200;

const NO_KEY_MODIFIERS: KeyModifiers = {
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
};

const appWindow = getAppWindow();

/**
 * Refreshes the pane's directory when the backend reports it changed, or when
 * the window regains focus — with a short debounce, because watcher events
 * arrive in bursts around a write.
 */
export function useDirectoryRefresh(navigator: ExplorerNavigator): void {
  useEffect(() => {
    let disposed = false;
    let refreshTimeout: number | undefined;

    const scheduleRefresh = (path: string) => {
      if (disposed || navigator.getSnapshot().directory?.path !== path) return;

      window.clearTimeout(refreshTimeout);
      refreshTimeout = window.setTimeout(() => {
        refreshTimeout = undefined;
        void navigator.refresh(path);
      }, DIRECTORY_REFRESH_DELAY_MS);
    };

    const unlistenChangesPromise = events.explorerDirectoryChanged.listen(({ payload }) => {
      scheduleRefresh(payload);
    });
    const unlistenFocusPromise = appWindow
      ? appWindow.onFocusChanged(({ payload: focused }) => {
          const currentPath = navigator.getSnapshot().directory?.path;
          if (focused && currentPath) scheduleRefresh(currentPath);
        })
      : Promise.resolve(() => {});

    return () => {
      disposed = true;
      window.clearTimeout(refreshTimeout);
      void Promise.all([unlistenChangesPromise, unlistenFocusPromise]).then((unlisten) => {
        unlisten.forEach((stopListening) => stopListening());
      });
    };
  }, [navigator]);
}

/** Highlight state for a drag of files from outside the window. */
export type ExternalDrop = {
  sourcePaths: string[];
  targetPath: string | null;
  /** What dropping here would do right now — modifiers pin one effect, a
   *  plain gesture follows the volume rule (same volume moves, crossing one
   *  copies). */
  operation: FileTransferOperation;
};

/**
 * Tracks a window-level drag of external files: which pane claims it (via
 * hit-testing `drag-drop.ts`) and where the highlight sits while hovering.
 */
export function useExternalDrop({
  directoryPath,
  searchQuery,
  isActivePane,
  onDropPaths,
}: {
  directoryPath: string | undefined;
  searchQuery: string;
  isActivePane: boolean;
  /** Handles a claimed drop — the transfer pipeline's entry point for files
   *  dragged in from outside the window. */
  onDropPaths: (
    sourcePaths: string[],
    targetPath: string,
    operation: FileTransferOperation,
  ) => void;
}): { externalDrop: ExternalDrop | null } {
  const [hovered, setHovered] = useState<{
    sourcePaths: string[];
    targetPath: string | null;
  } | null>(null);
  const [modifiers, setModifiers] = useState<KeyModifiers>(NO_KEY_MODIFIERS);

  // A new directory or query means the surface being hovered no longer
  // exists; the highlight would point at rows that already went away.
  useEffect(() => {
    setHovered(null);
  }, [directoryPath, searchQuery]);

  // Modifiers arrive with DOM keyboard events, which the webview never gets
  // while another application owns the drag — ask the OS instead, once on
  // arrival and at a human interval afterwards.
  const dragActive = hovered !== null;
  useEffect(() => {
    if (!dragActive) return;
    let disposed = false;

    const refreshModifiers = () => {
      void commands.getKeyModifiers().then((next) => {
        if (!disposed) setModifiers(next);
      });
    };

    refreshModifiers();
    const poll = window.setInterval(refreshModifiers, DROP_MODIFIER_POLL_MS);
    return () => {
      disposed = true;
      window.clearInterval(poll);
    };
  }, [dragActive]);

  useEffect(() => {
    if (!appWindow) return;
    let disposed = false;

    /** Logical viewport coordinates of a drag-drop event position. */
    const toLogical = (position: { toLogical: (scaleFactor: number) => { x: number; y: number } }) =>
      position.toLogical(window.devicePixelRatio);

    const getTargetPath = (position: {
      toLogical: (scaleFactor: number) => { x: number; y: number };
    }) => {
      const logicalPosition = toLogical(position);
      return (
        getExplorerDropTargetAtPoint(logicalPosition.x, logicalPosition.y) ?? directoryPath ?? null
      );
    };

    const unlistenPromise = appWindow.onDragDropEvent(({ payload }) => {
      if (disposed) return;

      if (payload.type === "enter") {
        setHovered({
          sourcePaths: payload.paths,
          targetPath: getTargetPath(payload.position),
        });
        return;
      }

      if (payload.type === "over") {
        const targetPath = getTargetPath(payload.position);
        // "over" fires at mousemove frequency; only re-render when the
        // highlighted drop target actually changes.
        setHovered((currentDrop) =>
          currentDrop && currentDrop.targetPath !== targetPath
            ? { ...currentDrop, targetPath }
            : currentDrop,
        );
        return;
      }

      if (payload.type === "drop") {
        const targetPath = getTargetPath(payload.position);
        setHovered(null);
        // Every pane in the window hears the same drop. Without a hit-test
        // the unclaimed drop lands in each pane's own directory (via the
        // `?? directoryPath` fallback above), duplicating the transfer once
        // per pane. Only the explorer under the pointer claims it; a drop
        // over the sidebar, tab strip or terminal goes to the active pane.
        if (!targetPath) return;
        const { x, y } = toLogical(payload.position);
        if (getExplorerDropTargetAtPoint(x, y) === null && !isActivePane) {
          if (isExplorerContainerAtPoint(x, y)) return;
        }
        // The key state is read again rather than reused from the hover poll:
        // the modifiers held at the moment of the drop are the ones that count.
        void commands
          .getKeyModifiers()
          .catch(() => NO_KEY_MODIFIERS)
          .then((next) =>
            onDropPaths(
              payload.paths,
              targetPath,
              resolveDropOperation(next, payload.paths, targetPath),
            ),
          );
        return;
      }

      setHovered(null);
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [directoryPath, isActivePane, onDropPaths]);

  const externalDrop: ExternalDrop | null = hovered
    ? {
        sourcePaths: hovered.sourcePaths,
        targetPath: hovered.targetPath,
        operation: resolveDropOperation(
          modifiers,
          hovered.sourcePaths,
          hovered.targetPath ?? directoryPath ?? "",
        ),
      }
    : null;

  return { externalDrop };
}
