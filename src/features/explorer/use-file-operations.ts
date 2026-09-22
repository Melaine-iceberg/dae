/**
 * The file-operation engine behind the explorer: a single runner that wraps
 * every mutating backend command with progress reporting, pending state and
 * error capture, plus the progress-event subscription that adopts operation
 * kinds announced by the backend.
 *
 * Extracted from `explorer-view.tsx`; the view passes the current directory
 * and a refresh function and receives `performFileOperation` back.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { events } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";

import type { FileOperationKind, FileOperationProgress } from "./types";

export type FileOperationResult = { ok: true } | { error: string; ok: false; rawError?: unknown };

/** Signature of the runner handed to the transfer pipeline and dialog flows. */
export type PerformFileOperation = (
  operation: (operationId?: string) => Promise<unknown>,
  progressOperation?: FileOperationKind | "auto",
) => Promise<FileOperationResult>;

const COMPLETED_OPERATION_STATUS_DURATION_MS = 900;

export function useFileOperations({
  directoryPath,
  refresh,
}: {
  directoryPath: string | undefined;
  refresh: (path: string) => Promise<unknown>;
}): {
  performFileOperation: PerformFileOperation;
  fileOperationProgress: FileOperationProgress | null;
  isOperationPending: boolean;
  operationError: string | null;
  setOperationError: (error: string | null) => void;
} {
  const { t } = useTranslation("explorer");
  const [fileOperationProgress, setFileOperationProgress] = useState<FileOperationProgress | null>(
    null,
  );
  const [isOperationPending, setIsOperationPending] = useState(false);
  // A failure belongs in the banner above the list rather than in the menu
  // that has already closed.
  const [operationError, setOperationError] = useState<string | null>(null);
  // Operation IDs started with the "auto" progress kind: the backend announces
  // the kind with its first progress event, so the ID is adopted there.
  const deferredProgressIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const unlistenProgressPromise = events.explorerFileOperationProgress.listen(({ payload }) => {
      setFileOperationProgress((currentProgress) => {
        if (!currentProgress) {
          // "auto" operations (undo/redo) have no frontend-known kind; adopt
          // the backend's first event for an operation this view started.
          return deferredProgressIdsRef.current.has(payload.operationId) ? payload : null;
        }

        if (
          currentProgress.operationId !== payload.operationId ||
          currentProgress.phase === "completed"
        ) {
          return currentProgress;
        }

        return payload;
      });
      deferredProgressIdsRef.current.delete(payload.operationId);
    });

    return () => {
      void unlistenProgressPromise.then((unlisten) => unlisten());
    };
  }, []);

  const performFileOperation = useCallback<PerformFileOperation>(
    async (operation, progressOperation) => {
      if (!directoryPath) {
        return { error: t("explorer:errors.directoryUnavailable"), ok: false };
      }

      const operationId = progressOperation ? crypto.randomUUID() : undefined;
      const announcedProgressOperation =
        progressOperation && progressOperation !== "auto" ? progressOperation : null;
      if (announcedProgressOperation && operationId) {
        setFileOperationProgress({
          operationId,
          operation: announcedProgressOperation,
          phase: "preparing",
          completed: 0,
          total: null,
          currentPath: null,
        });
      } else if (operationId) {
        // "auto": the kind is only known to the backend (undo/redo); the
        // progress state is adopted from its first progress event.
        deferredProgressIdsRef.current.add(operationId);
      }
      setIsOperationPending(true);

      try {
        await operation(operationId);
        await refresh(directoryPath);
        if (operationId) {
          setFileOperationProgress((currentProgress) => {
            if (!currentProgress || currentProgress.operationId !== operationId) {
              return currentProgress;
            }

            return {
              ...currentProgress,
              phase: "completed",
              completed: currentProgress.total ?? currentProgress.completed,
            };
          });
          window.setTimeout(() => {
            setFileOperationProgress((currentProgress) =>
              currentProgress?.operationId === operationId ? null : currentProgress,
            );
          }, COMPLETED_OPERATION_STATUS_DURATION_MS);
        }
        return { ok: true };
      } catch (error) {
        if (operationId) {
          setFileOperationProgress((currentProgress) =>
            currentProgress?.operationId === operationId ? null : currentProgress,
          );
        }
        return {
          error: getFileOperationErrorMessage(error),
          ok: false,
          rawError: error,
        };
      } finally {
        if (operationId) {
          deferredProgressIdsRef.current.delete(operationId);
        }
        setIsOperationPending(false);
      }
    },
    [directoryPath, refresh, t],
  );

  return {
    performFileOperation,
    fileOperationProgress,
    isOperationPending,
    operationError,
    setOperationError,
  };
}
