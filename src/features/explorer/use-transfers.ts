/**
 * The transfer pipeline: conflict pre-check → (optional) conflict dialog →
 * copy/move execution, plus the shortcut-creation path that bypasses the
 * conflict dialog by design.
 *
 * Every transfer ultimately runs through `performFileOperation`, so progress
 * reporting, pending state and the error banner stay consistent across paste,
 * drag-drop and the context menus.
 */
import { useCallback, useState } from "react";

import { commands, type ConflictAction, type TransferConflict, type TransferItem } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";

import type { TransferOperation } from "./drag-drop";
import type { FileOperationResult, PerformFileOperation } from "./use-file-operations";

/** A transfer paused on the conflict dialog, waiting for per-item decisions. */
export type PendingTransfer = {
  conflicts: TransferConflict[];
  destinationPath: string;
  operation: TransferOperation;
  sourcePaths: string[];
  onSuccess: () => void;
};

export function useTransfers({
  performFileOperation,
  setOperationError,
  clearSelection,
}: {
  performFileOperation: PerformFileOperation;
  setOperationError: (error: string | null) => void;
  /** Runs after a successful transfer whose caller didn't supply its own
   *  completion work — the default is clearing the selection, since the moved
   *  rows are about to vanish from the listing anyway. */
  clearSelection: () => void;
}): {
  pendingTransfer: PendingTransfer | null;
  startTransfer: (
    sourcePaths: string[],
    destinationPath: string,
    operation: TransferOperation,
    onSuccess: () => void,
  ) => void;
  transferEntries: (
    sourcePaths: string[],
    destinationPath: string,
    operation: TransferOperation,
  ) => void;
  copyExternalEntries: (sourcePaths: string[], destinationPath: string) => void;
  createShortcutsEntries: (sourcePaths: string[], destinationPath: string) => void;
  resolveTransferConflicts: (decisions: Record<string, ConflictAction>) => void;
  cancelTransferConflicts: () => void;
} {
  const [pendingTransfer, setPendingTransfer] = useState<PendingTransfer | null>(null);

  /** Executes a transfer whose conflicts (if any) have already been resolved. */
  const executeTransfer = useCallback(
    (
      sourcePaths: string[],
      destinationPath: string,
      operation: TransferOperation,
      decisions: Record<string, ConflictAction>,
      onSuccess: () => void,
    ) => {
      const items: TransferItem[] = sourcePaths.map((path) => ({
        path,
        onConflict: decisions[path] ?? "fail",
      }));

      void performFileOperation(
        (operationId) =>
          operation === "copy"
            ? commands.copyEntries(items, destinationPath, operationId!)
            : commands.moveEntries(items, destinationPath, operationId!),
        operation,
      ).then((result: FileOperationResult) => {
        if (!result.ok) {
          setOperationError(result.error);
          return;
        }

        onSuccess();
      });
    },
    [performFileOperation, setOperationError],
  );

  /** Pre-checks conflicts, then either executes directly or opens the conflict dialog. */
  const startTransfer = useCallback(
    (
      sourcePaths: string[],
      destinationPath: string,
      operation: TransferOperation,
      onSuccess: () => void,
    ) => {
      if (sourcePaths.length === 0) return;

      setOperationError(null);
      commands
        .checkTransferConflicts(sourcePaths, destinationPath)
        .then((conflicts) => {
          if (conflicts.length === 0) {
            executeTransfer(sourcePaths, destinationPath, operation, {}, onSuccess);
            return;
          }

          setPendingTransfer({ conflicts, destinationPath, operation, sourcePaths, onSuccess });
        })
        .catch((error: unknown) => setOperationError(getFileOperationErrorMessage(error)));
    },
    [executeTransfer, setOperationError],
  );

  const transferEntries = useCallback(
    (sourcePaths: string[], destinationPath: string, operation: TransferOperation) => {
      startTransfer(sourcePaths, destinationPath, operation, clearSelection);
    },
    [clearSelection, startTransfer],
  );

  /** Explorer-style Alt-drag link: the backend creates .lnk shortcuts on
   *  Windows and real symlinks on macOS/Linux inside the destination. Name
   *  collisions resolve with " (2)"… suffixes, so no conflict dialog is
   *  needed here. */
  const createShortcutsEntries = useCallback(
    (sourcePaths: string[], destinationPath: string) => {
      setOperationError(null);
      commands
        .createShortcuts(sourcePaths, destinationPath)
        .then(clearSelection)
        .catch((error: unknown) => setOperationError(getFileOperationErrorMessage(error)));
    },
    [clearSelection, setOperationError],
  );

  const copyExternalEntries = useCallback(
    (sourcePaths: string[], destinationPath: string) => {
      startTransfer(sourcePaths, destinationPath, "copy", clearSelection);
    },
    [clearSelection, startTransfer],
  );

  const resolveTransferConflicts = useCallback(
    (decisions: Record<string, ConflictAction>) => {
      const transfer = pendingTransfer;
      if (!transfer) return;

      setPendingTransfer(null);
      executeTransfer(
        transfer.sourcePaths,
        transfer.destinationPath,
        transfer.operation,
        decisions,
        transfer.onSuccess,
      );
    },
    [executeTransfer, pendingTransfer],
  );

  const cancelTransferConflicts = useCallback(() => {
    setPendingTransfer(null);
  }, []);

  return {
    pendingTransfer,
    startTransfer,
    transferEntries,
    copyExternalEntries,
    createShortcutsEntries,
    resolveTransferConflicts,
    cancelTransferConflicts,
  };
}
