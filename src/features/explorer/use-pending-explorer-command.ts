/**
 * Command-bus consumer for the explorer pane.
 *
 * The command bar drops intents into the bus; the mounted (active tab)
 * explorer consumes them. In the dual-pane layout only the focused pane
 * executes, so the same intent never runs twice. Ids are tracked so React
 * StrictMode's double effect invocation cannot execute a command twice.
 *
 * The switch below is the single mapping from command ids to the view's
 * actions; everything it names is passed in, so this module owns no state
 * beyond the subscription itself.
 */
import { useCallback, useEffect, useRef } from "react";
import { useAtomValue, useSetAtom } from "jotai";

import { toggleFavoriteAtom } from "@/features/sidebar/sidebar-atoms";
import {
  clearPendingExplorerCommand,
  pendingExplorerCommandAtom,
  type ExplorerCommandId,
} from "@/features/workspace/explorer-command-bus";

import type { ExplorerNavigator } from "./navigation";
import type { DirectoryView, NewEntryKind } from "./types";

export function usePendingExplorerCommand({
  isActivePane,
  navigator,
  directory,
  directoryPath,
  requestCreate,
  requestRename,
  requestDelete,
  copySelection,
  cutSelection,
  pasteClipboard,
  copySelectedPaths,
  selectAll,
  openTerminalHere,
  onToggleSplit,
}: {
  isActivePane: boolean;
  navigator: ExplorerNavigator;
  directory: DirectoryView | null;
  directoryPath: string | undefined;
  requestCreate: (kind: NewEntryKind) => void;
  requestRename: () => void;
  requestDelete: () => void;
  copySelection: () => void;
  cutSelection: () => void;
  pasteClipboard: () => void;
  copySelectedPaths: () => void;
  selectAll: () => void;
  openTerminalHere: (path: string) => void;
  onToggleSplit?: () => void;
}): void {
  const pendingCommand = useAtomValue(pendingExplorerCommandAtom);
  const toggleFavorite = useSetAtom(toggleFavoriteAtom);

  const executeExplorerCommand = useCallback(
    (command: ExplorerCommandId) => {
      switch (command) {
        case "create-folder":
          requestCreate("directory");
          break;
        case "create-file":
          requestCreate("file");
          break;
        case "rename":
          requestRename();
          break;
        case "delete":
          requestDelete();
          break;
        case "copy":
          copySelection();
          break;
        case "cut":
          cutSelection();
          break;
        case "paste":
          pasteClipboard();
          break;
        case "copy-paths":
          copySelectedPaths();
          break;
        case "select-all":
          selectAll();
          break;
        case "refresh":
          if (directory) void navigator.refresh(directory.path);
          break;
        case "go-back":
          void navigator.goBack();
          break;
        case "go-forward":
          void navigator.goForward();
          break;
        case "go-up":
          void navigator.goUp();
          break;
        case "open-terminal":
          if (directoryPath) openTerminalHere(directoryPath);
          break;
        case "toggle-favorite":
          if (directory) {
            toggleFavorite({
              path: directory.path,
              name: directory.breadcrumbs.at(-1)?.name ?? directory.path,
            });
          }
          break;
        case "toggle-split":
          onToggleSplit?.();
          break;
      }
    },
    [
      copySelectedPaths,
      copySelection,
      cutSelection,
      directory,
      directoryPath,
      navigator,
      onToggleSplit,
      openTerminalHere,
      pasteClipboard,
      requestCreate,
      requestDelete,
      requestRename,
      selectAll,
      toggleFavorite,
    ],
  );

  const executedCommandIdsRef = useRef(new Set<number>());

  useEffect(() => {
    if (!pendingCommand || !isActivePane) return;
    if (executedCommandIdsRef.current.has(pendingCommand.id)) return;

    executedCommandIdsRef.current.add(pendingCommand.id);
    clearPendingExplorerCommand();
    executeExplorerCommand(pendingCommand.command);
  }, [pendingCommand, executeExplorerCommand, isActivePane]);
}
