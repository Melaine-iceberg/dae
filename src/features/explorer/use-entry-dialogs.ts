/**
 * The explorer's dialog flows: rename, bulk rename, create entry, permanent
 * delete, archive password retry and the "Open With" target.
 *
 * State and the handlers that mutate it live together here so the view only
 * wires them to the presentational dialogs in `explorer-dialogs.tsx` and the
 * request openers (`requestRename`, `requestCreate`, `openDeleteDialog`) to
 * its menus, list and command bus. Like the view before it, every flow runs
 * through `performFileOperation`, which owns pending state and the error
 * banner; the permanent-delete flow restores the selection it optimistically
 * clears when the backend rejects the batch.
 */
import { useCallback, useEffect, useState, type Dispatch, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import type { FormEvent } from "react";

import { commands, type RenameRequest } from "@/bindings";

import { getCreateEntryErrorMessage, isWrongPasswordError } from "./explorer-errors";
import type { DirectoryEntry, NewEntryKind } from "./types";
import type { PerformFileOperation } from "./use-file-operations";

/** The password dialog serves two entry points: an encrypted compress the
 *  user asked for, and an extract that answered with a wrong-password error. */
export type ArchivePasswordRequest =
  | { mode: "compress" }
  | { archivePath: string; mode: "extract" };

export function useEntryDialogs({
  performFileOperation,
  setOperationError,
  isOperationPending,
  selectedEntries,
  directoryPath,
  searchActive,
  setSelectedPaths,
  searchQuery,
}: {
  performFileOperation: PerformFileOperation;
  setOperationError: (error: string | null) => void;
  isOperationPending: boolean;
  selectedEntries: DirectoryEntry[];
  directoryPath: string | undefined;
  /** Creating entries is disabled while a search replaces the listing. */
  searchActive: boolean;
  setSelectedPaths: Dispatch<SetStateAction<string[]>>;
  searchQuery: string;
}) {
  const { t } = useTranslation("explorer");

  const [renameTarget, setRenameTarget] = useState<DirectoryEntry | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [bulkRenameOpen, setBulkRenameOpen] = useState(false);
  const [bulkRenameError, setBulkRenameError] = useState<string | null>(null);
  const [newEntryKind, setNewEntryKind] = useState<NewEntryKind | null>(null);
  const [newEntryValue, setNewEntryValue] = useState("");
  const [newEntryError, setNewEntryError] = useState<string | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<DirectoryEntry[]>([]);
  const [archivePasswordRequest, setArchivePasswordRequest] =
    useState<ArchivePasswordRequest | null>(null);
  const [archivePasswordError, setArchivePasswordError] = useState<string | null>(null);
  const [archivePasswordPending, setArchivePasswordPending] = useState(false);
  const [openWithTarget, setOpenWithTarget] = useState<string | null>(null);

  // Navigating (or re-querying) away closes everything that referred to the
  // previous listing. The selection and the operation/preview resets live in
  // their own hooks; this is the dialog half of the same reset.
  useEffect(() => {
    setRenameTarget(null);
    setBulkRenameOpen(false);
    setNewEntryKind(null);
    setDeleteTargets([]);
  }, [directoryPath, searchQuery]);

  const requestRename = useCallback(() => {
    if (selectedEntries.length === 0) return;

    // A single entry keeps the classic inline dialog; multi-selections open
    // the patterned bulk rename (numbering, replace, case).
    if (selectedEntries.length > 1) {
      setBulkRenameOpen(true);
      setBulkRenameError(null);
      setOperationError(null);
      return;
    }

    const [entry] = selectedEntries;
    setRenameTarget(entry);
    setRenameValue(entry.name);
    setRenameError(null);
    setOperationError(null);
  }, [selectedEntries, setOperationError]);

  const closeRenameDialog = () => {
    if (isOperationPending) return;

    setRenameTarget(null);
    setRenameError(null);
  };

  const submitRename = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!renameTarget) return;

    const nextName = renameValue.trim();
    if (!nextName) {
      setRenameError(t("explorer:validation.nameEmpty"));
      return;
    }

    setRenameError(null);
    setOperationError(null);
    const sourcePath = renameTarget.path;

    void performFileOperation(() => commands.renameEntry(sourcePath, nextName)).then((result) => {
      if (!result.ok) {
        setRenameError(result.error);
        return;
      }

      setRenameTarget(null);
      setSelectedPaths([]);
    });
  };

  /** Applies the bulk-rename plan; the whole batch stays one undo step. */
  const applyBulkRename = useCallback(
    (requests: RenameRequest[]) => {
      setBulkRenameError(null);
      setOperationError(null);

      void performFileOperation(
        (operationId) => commands.renameEntriesBatch(requests, operationId!),
        "move",
      ).then((result) => {
        if (!result.ok) {
          setBulkRenameError(result.error);
          return;
        }

        setBulkRenameOpen(false);
        setSelectedPaths([]);
      });
    },
    [performFileOperation, setOperationError, setSelectedPaths],
  );

  const closeBulkRename = () => {
    if (!isOperationPending) {
      setBulkRenameOpen(false);
      setBulkRenameError(null);
    }
  };

  const requestCreate = useCallback(
    (kind: NewEntryKind) => {
      if (!directoryPath || searchActive) return;

      setNewEntryKind(kind);
      setNewEntryValue(
        kind === "file"
          ? t("explorer:newEntry.defaultNameExt", {
              name: t("explorer:newEntry.fileDefaultName"),
            })
          : t("explorer:newEntry.folderDefaultName"),
      );
      setNewEntryError(null);
      setOperationError(null);
    },
    [directoryPath, searchActive, setOperationError, t],
  );

  const closeCreateDialog = () => {
    if (isOperationPending) return;

    setNewEntryKind(null);
    setNewEntryError(null);
  };

  const submitCreate = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!newEntryKind || !directoryPath) return;

    const nextName = newEntryValue.trim();
    if (!nextName) {
      setNewEntryError(t("explorer:validation.nameEmpty"));
      return;
    }

    setNewEntryError(null);
    setOperationError(null);
    const kind = newEntryKind;
    let createdPath: string | null = null;

    void performFileOperation(async () => {
      createdPath = await commands.createEntry(directoryPath, nextName, kind);
    }).then((result) => {
      if (!result.ok) {
        setNewEntryError(getCreateEntryErrorMessage(result.error, result.rawError));
        return;
      }

      setNewEntryKind(null);
      setSelectedPaths(createdPath ? [createdPath] : []);
    });
  };

  const openDeleteDialog = useCallback(
    (entries: DirectoryEntry[]) => {
      setDeleteTargets(entries);
      setOperationError(null);
    },
    [setOperationError],
  );

  const closeDeleteDialog = () => {
    if (!isOperationPending) {
      setDeleteTargets([]);
    }
  };

  const confirmDelete = () => {
    if (deleteTargets.length === 0) return;

    setOperationError(null);
    const paths = deleteTargets.map((entry) => entry.path);
    const targets = deleteTargets;

    setDeleteTargets([]);
    setSelectedPaths([]);

    void performFileOperation(
      (operationId) => commands.deleteEntries(paths, operationId!),
      "delete",
    ).then((result) => {
      if (!result.ok) {
        setOperationError(result.error);
        setDeleteTargets(targets);
        setSelectedPaths(paths);
        return;
      }
    });
  };

  /** Opens the password dialog for a flow the view initiated; the previous
   *  attempt's error is stale by definition. */
  const openArchivePassword = useCallback((request: ArchivePasswordRequest) => {
    setArchivePasswordError(null);
    setArchivePasswordRequest(request);
  }, []);

  const closeArchivePassword = () => {
    if (!archivePasswordPending) {
      setArchivePasswordRequest(null);
      setArchivePasswordError(null);
    }
  };

  /** Retries the pending archive operation with the supplied password. */
  const submitArchivePassword = useCallback(
    (password: string) => {
      if (!archivePasswordRequest || !directoryPath) return;

      setArchivePasswordError(null);
      setArchivePasswordPending(true);

      const operation =
        archivePasswordRequest.mode === "extract"
          ? (operationId?: string) =>
              commands.extractArchive(
                archivePasswordRequest.archivePath,
                null,
                password,
                operationId!,
              )
          : (operationId?: string) =>
              commands.compressEntries(
                selectedEntries.map((entry) => entry.path),
                directoryPath,
                "7z",
                password,
                operationId!,
              );

      void performFileOperation(operation, archivePasswordRequest.mode).then((result) => {
        setArchivePasswordPending(false);

        if (result.ok) {
          setArchivePasswordRequest(null);
          return;
        }

        // A wrong password keeps the dialog open so it can be corrected.
        if (archivePasswordRequest.mode === "extract" && isWrongPasswordError(result.rawError)) {
          setArchivePasswordError(result.error);
          return;
        }

        setArchivePasswordRequest(null);
        setOperationError(result.error);
      });
    },
    [archivePasswordRequest, directoryPath, performFileOperation, selectedEntries, setOperationError],
  );

  return {
    renameTarget,
    renameValue,
    renameError,
    setRenameValue,
    requestRename,
    closeRenameDialog,
    submitRename,
    bulkRenameOpen,
    bulkRenameError,
    applyBulkRename,
    closeBulkRename,
    newEntryKind,
    newEntryValue,
    newEntryError,
    setNewEntryValue,
    requestCreate,
    closeCreateDialog,
    submitCreate,
    deleteTargets,
    openDeleteDialog,
    closeDeleteDialog,
    confirmDelete,
    archivePasswordRequest,
    archivePasswordError,
    archivePasswordPending,
    openArchivePassword,
    closeArchivePassword,
    submitArchivePassword,
    openWithTarget,
    setOpenWithTarget,
  };
}
