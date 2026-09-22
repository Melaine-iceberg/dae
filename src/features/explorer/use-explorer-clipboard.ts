/**
 * Copy / cut / paste for file rows.
 *
 * The app mirrors copied paths in a `fileClipboardAtom`, and also places the
 * local files on the *system* clipboard (CF_HDROP) so Explorer, browsers and
 * chat apps accept a paste. On paste the system clipboard wins when it holds a
 * different file list — copying files in another app replaces our mirror while
 * the atom keeps its previous contents. Network paths never reach the system
 * clipboard, so those stay app-internal.
 */
import { useCallback } from "react";
import { useAtom } from "jotai";

import { commands } from "@/bindings";

import { pathListsEqual } from "./explorer-errors";
import { isLocalExplorerPath, type TransferOperation } from "./drag-drop";
import { fileClipboardAtom } from "./tabs";
import type { DirectoryEntry } from "./types";

/** Places local files on the OS clipboard (CF_HDROP) so Explorer, browsers,
 *  and chat apps accept a paste; network paths stay app-internal. */
function mirrorFilesToSystemClipboard(paths: string[], cut: boolean) {
  const localPaths = paths.filter(isLocalExplorerPath);
  if (localPaths.length === 0) return;

  void commands.writeFilesToClipboard(localPaths, cut).catch((error) => {
    console.warn("Unable to place files on the system clipboard", error);
  });
}

export function useExplorerClipboard({
  selectedEntries,
  directoryPath,
  startTransfer,
  setOperationError,
  clearSelection,
}: {
  selectedEntries: DirectoryEntry[];
  directoryPath: string | undefined;
  startTransfer: (
    sourcePaths: string[],
    destinationPath: string,
    operation: TransferOperation,
    onSuccess: () => void,
  ) => void;
  setOperationError: (error: string | null) => void;
  clearSelection: () => void;
}): {
  copySelection: () => void;
  cutSelection: () => void;
  pasteClipboard: () => void;
} {
  const [clipboard, setClipboard] = useAtom(fileClipboardAtom);

  const copySelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setClipboard({ operation: "copy", sourcePaths });
    mirrorFilesToSystemClipboard(sourcePaths, false);
    setOperationError(null);
  }, [selectedEntries, setClipboard, setOperationError]);

  const cutSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setClipboard({ operation: "cut", sourcePaths });
    mirrorFilesToSystemClipboard(sourcePaths, true);
    setOperationError(null);
  }, [selectedEntries, setClipboard, setOperationError]);

  const pasteClipboard = useCallback(() => {
    if (!directoryPath) return;

    setOperationError(null);

    void commands
      .readFilesFromClipboard()
      .then((systemFiles) => {
        const systemPaths = systemFiles?.paths ?? [];
        const systemIsMirror =
          clipboard !== null && pathListsEqual(systemPaths, clipboard.sourcePaths);
        const fromSystem = systemPaths.length > 0 && !systemIsMirror;

        const paths = fromSystem ? systemPaths : (clipboard?.sourcePaths ?? []);
        if (paths.length === 0) return;

        const isCut = fromSystem ? systemFiles?.cut === true : clipboard?.operation === "cut";
        const operation: TransferOperation = isCut ? "move" : "copy";

        startTransfer(paths, directoryPath, operation, () => {
          if (isCut && !fromSystem) {
            setClipboard(null);
          }
          clearSelection();
        });
      })
      .catch((error) => {
        console.warn("Unable to read the system clipboard", error);
      });
  }, [clearSelection, clipboard, directoryPath, setClipboard, setOperationError, startTransfer]);

  return { copySelection, cutSelection, pasteClipboard };
}
