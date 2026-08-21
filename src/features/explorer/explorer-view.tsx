import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type FormEvent,
} from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ArrowClockwiseIcon,
  ArrowCounterClockwiseIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  CircleNotchIcon,
  EyeIcon,
  SidebarSimpleIcon,
  StarIcon,
  WarningIcon,
  XIcon,
} from "@phosphor-icons/react";

import {
  commands,
  events,
  type ArchiveFormat,
  type ConflictAction,
  type TransferConflict,
  type TransferItem,
  type UndoRedoOutcome,
} from "@/bindings";

import { getAppWindow } from "@/lib/app-window";

import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import {
  addFavoritePathsAtom,
  favoritesAtom,
  sidebarVisibleAtom,
  toggleFavoriteAtom,
} from "@/features/sidebar/sidebar-atoms";
import {
  clearPendingExplorerCommand,
  pendingExplorerCommandAtom,
  type ExplorerCommandId,
} from "@/features/workspace/explorer-command-bus";
import { addItemsToSpace } from "@/features/workspace/spaces-atoms";
import { recordRecentItem } from "@/features/workspace/recents-atoms";
import { cn } from "@/lib/utils";

import { ContentSearchResults, ContentSearchToolbar, useContentSearch } from "./content-search";
import { ContextualActionBar } from "./contextual-action-bar";
import { DirectorySearch, useDirectorySearch, type ExplorerSearchMode } from "./directory-search";
import {
  getExplorerDropTargetAtPoint,
  isLocalExplorerPath,
  type TransferOperation,
} from "./drag-drop";
import { EntryPreview } from "./entry-preview";
import { isArchiveFile } from "./entry-context-menu";
import { ExplorerPathBar } from "./explorer-path-bar";
import { ExplorerStatusBar } from "./explorer-status-bar";
import { FileList, FileListSkeleton } from "./file-list";
import { FilterMenu } from "./filter-menu";
import { useGitStatus } from "./git-status";
import type { ExplorerNavigator } from "./navigation";
import { SortMenu } from "./sort-menu";
import { TransferConflictDialog } from "./transfer-conflict-dialog";
import {
  applyEntryFilters,
  entryFiltersAtom,
  foldersFirstAtom,
  sortEntries,
  sortKeyAtom,
  sortOrderAtom,
} from "./preferences";
import { fileClipboardAtom, undoRedoAtom } from "./tabs";
import type {
  DirectoryEntry,
  FileOperationKind,
  FileOperationProgress,
  NewEntryKind,
} from "./types";

const DIRECTORY_REFRESH_DELAY_MS = 150;
const COMPLETED_OPERATION_STATUS_DURATION_MS = 900;
const appWindow = getAppWindow();

interface ExplorerViewProps {
  navigator: ExplorerNavigator;
}

type FileOperationResult = { ok: true } | { error: string; ok: false; rawError?: unknown };
type ExternalDrop = { sourcePaths: string[]; targetPath: string | null };

/** Floating hint after an undoable operation or an undo/redo step. */
type UndoRedoToast = {
  message: string;
  /** Follow-up action offered on the toast. */
  action: "undo" | "redo";
};

/** A transfer paused on the conflict dialog, waiting for per-item decisions. */
type PendingTransfer = {
  conflicts: TransferConflict[];
  destinationPath: string;
  operation: TransferOperation;
  sourcePaths: string[];
  onSuccess: () => void;
};

export function ExplorerView({ navigator }: ExplorerViewProps) {
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const [clipboard, setClipboard] = useAtom(fileClipboardAtom);
  const undoRedo = useAtomValue(undoRedoAtom);
  const favorites = useAtomValue(favoritesAtom) ?? [];
  const toggleFavorite = useSetAtom(toggleFavoriteAtom);
  const addFavoritePaths = useSetAtom(addFavoritePathsAtom);
  const [sidebarVisible, setSidebarVisible] = useAtom(sidebarVisibleAtom);
  const pendingCommand = useAtomValue(pendingExplorerCommandAtom);
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [renameTarget, setRenameTarget] = useState<DirectoryEntry | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [newEntryKind, setNewEntryKind] = useState<NewEntryKind | null>(null);
  const [newEntryValue, setNewEntryValue] = useState("");
  const [newEntryError, setNewEntryError] = useState<string | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<DirectoryEntry[]>([]);
  const [pendingTransfer, setPendingTransfer] = useState<PendingTransfer | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [isOperationPending, setIsOperationPending] = useState(false);
  const [fileOperationProgress, setFileOperationProgress] = useState<FileOperationProgress | null>(
    null,
  );
  const [externalDrop, setExternalDrop] = useState<ExternalDrop | null>(null);
  const [isPreviewOpen, setIsPreviewOpen] = useState(false);
  const [searchMode, setSearchMode] = useState<ExplorerSearchMode>("name");
  const [undoRedoToast, setUndoRedoToast] = useState<UndoRedoToast | null>(null);
  // Operation IDs started with the "auto" progress kind: the backend announces
  // the kind with its first progress event, so the ID is adopted there.
  const deferredProgressIdsRef = useRef<Set<string>>(new Set());
  const directory = state.directory;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;
  const search = useDirectorySearch(directoryPath ?? null, directory, searchMode === "name");
  const contentSearch = useContentSearch(
    directoryPath ?? null,
    directory,
    searchMode === "content",
  );
  const gitStatus = useGitStatus(directoryPath ?? null);
  const isContentSearchActive = searchMode === "content" && contentSearch.isActive;
  const sortKey = useAtomValue(sortKeyAtom);
  const sortOrder = useAtomValue(sortOrderAtom);
  const foldersFirst = useAtomValue(foldersFirstAtom);
  const entryFilters = useAtomValue(entryFiltersAtom);
  const displayedEntries = useMemo(() => {
    const sourceEntries = search.isActive
      ? (search.response?.entries ?? [])
      : (directory?.entries ?? []);
    return sortEntries(
      applyEntryFilters(sourceEntries, entryFilters),
      sortKey,
      sortOrder,
      foldersFirst,
    );
  }, [
    directory?.entries,
    entryFilters,
    foldersFirst,
    search.isActive,
    search.response,
    sortKey,
    sortOrder,
  ]);
  const selectedEntries = useMemo(
    () => displayedEntries.filter((entry) => selectedPaths.includes(entry.path)),
    [displayedEntries, selectedPaths],
  );

  useEffect(() => {
    if (navigator.getSnapshot().status === "idle") {
      void navigator.initialize();
    }
  }, [navigator]);

  // File selection belongs to the entry list, which content search replaces.
  useEffect(() => {
    if (isContentSearchActive) {
      setSelectedPaths([]);
    }
  }, [isContentSearchActive]);

  useEffect(() => {
    if (!directoryPath) return;

    void commands
      .watchDirectory(directoryPath)
      .then(() => void navigator.refresh(directoryPath))
      .catch((error: unknown) => {
        console.warn("Unable to watch directory for changes", error);
      });
  }, [navigator, directoryPath]);

  useEffect(() => {
    setSelectedPaths([]);
    setRenameTarget(null);
    setNewEntryKind(null);
    setDeleteTargets([]);
    setOperationError(null);
    setExternalDrop(null);
    setIsPreviewOpen(false);
  }, [directoryPath, search.query]);

  useEffect(() => {
    const availablePaths = new Set(displayedEntries.map((entry) => entry.path));
    setSelectedPaths((paths) => {
      const availableSelection = paths.filter((path) => availablePaths.has(path));
      return availableSelection.length === paths.length ? paths : availableSelection;
    });
  }, [displayedEntries]);

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

  const performFileOperation = useCallback(
    async (
      operation: (operationId?: string) => Promise<unknown>,
      progressOperation?: FileOperationKind | "auto",
    ): Promise<FileOperationResult> => {
      if (!directoryPath) {
        return { error: "当前目录不可用", ok: false };
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
        await navigator.refresh(directoryPath);
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
        return { error: getErrorMessage(error), ok: false, rawError: error };
      } finally {
        if (operationId) {
          deferredProgressIdsRef.current.delete(operationId);
        }
        setIsOperationPending(false);
      }
    },
    [directoryPath, navigator],
  );

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
      ).then((result) => {
        if (!result.ok) {
          setOperationError(result.error);
          return;
        }

        onSuccess();
      });
    },
    [performFileOperation],
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
        .catch((error: unknown) => setOperationError(getErrorMessage(error)));
    },
    [executeTransfer],
  );

  const transferEntries = useCallback(
    (sourcePaths: string[], destinationPath: string, operation: TransferOperation) => {
      startTransfer(sourcePaths, destinationPath, operation, () => setSelectedPaths([]));
    },
    [startTransfer],
  );

  /** Windows-style Alt-drag: create .lnk shortcuts for the sources inside the
   * destination. The backend resolves name collisions with " (2)"… suffixes,
   * so no conflict dialog is needed here. */
  const createShortcutsEntries = useCallback(
    (sourcePaths: string[], destinationPath: string) => {
      setOperationError(null);
      commands
        .createShortcuts(sourcePaths, destinationPath)
        .then(() => setSelectedPaths([]))
        .catch((error: unknown) => setOperationError(getErrorMessage(error)));
    },
    [],
  );

  const copyExternalEntries = useCallback(
    (sourcePaths: string[], destinationPath: string) => {
      startTransfer(sourcePaths, destinationPath, "copy", () => setSelectedPaths([]));
    },
    [startTransfer],
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

  useEffect(() => {
    if (!appWindow) return;
    let disposed = false;

    const getTargetPath = (position: {
      toLogical: (scaleFactor: number) => { x: number; y: number };
    }) => {
      const logicalPosition = position.toLogical(window.devicePixelRatio);
      return (
        getExplorerDropTargetAtPoint(logicalPosition.x, logicalPosition.y) ?? directoryPath ?? null
      );
    };

    const unlistenPromise = appWindow.onDragDropEvent(({ payload }) => {
      if (disposed) return;

      if (payload.type === "enter") {
        setExternalDrop({
          sourcePaths: payload.paths,
          targetPath: getTargetPath(payload.position),
        });
        return;
      }

      if (payload.type === "over") {
        const targetPath = getTargetPath(payload.position);
        setExternalDrop((currentDrop) =>
          currentDrop ? { ...currentDrop, targetPath } : currentDrop,
        );
        return;
      }

      if (payload.type === "drop") {
        const targetPath = getTargetPath(payload.position);
        setExternalDrop(null);
        if (targetPath) {
          copyExternalEntries(payload.paths, targetPath);
        }
        return;
      }

      setExternalDrop(null);
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [copyExternalEntries, directoryPath]);

  const copySelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setClipboard({ operation: "copy", sourcePaths });
    mirrorFilesToSystemClipboard(sourcePaths, false);
    setOperationError(null);
  }, [selectedEntries, setClipboard]);

  const cutSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setClipboard({ operation: "cut", sourcePaths });
    mirrorFilesToSystemClipboard(sourcePaths, true);
    setOperationError(null);
  }, [selectedEntries, setClipboard]);

  const pasteClipboard = useCallback(() => {
    if (!directoryPath) return;

    setOperationError(null);

    // The system clipboard wins when it holds a different file list, because
    // copying files in Explorer or another app replaces our mirror while the
    // app-internal atom keeps its previous contents. Network paths never
    // reach the system clipboard, so those stay app-internal.
    void commands
      .readFilesFromClipboard()
      .then((systemFiles) => {
        const systemPaths = systemFiles?.paths ?? [];
        const systemIsMirror =
          clipboard !== null && pathListsEqual(systemPaths, clipboard.sourcePaths);
        const fromSystem = systemPaths.length > 0 && !systemIsMirror;

        const paths = fromSystem ? systemPaths : clipboard?.sourcePaths ?? [];
        if (paths.length === 0) return;

        const isCut = fromSystem
          ? systemFiles?.cut === true
          : clipboard?.operation === "cut";
        const operation: TransferOperation = isCut ? "move" : "copy";

        startTransfer(paths, directoryPath, operation, () => {
          if (isCut && !fromSystem) {
            setClipboard(null);
          }
          setSelectedPaths([]);
        });
      })
      .catch((error) => {
        console.warn("Unable to read the system clipboard", error);
      });
  }, [clipboard, directoryPath, setClipboard, startTransfer]);

  const requestRename = useCallback(() => {
    if (selectedEntries.length !== 1) return;

    const [entry] = selectedEntries;
    setRenameTarget(entry);
    setRenameValue(entry.name);
    setRenameError(null);
    setOperationError(null);
  }, [selectedEntries]);

  /** Duplicates the selection in place; the backend picks unique "副本" names. */
  const duplicateSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    setOperationError(null);
    void performFileOperation(
      (operationId) =>
        commands.duplicateEntries(
          selectedEntries.map((entry) => entry.path),
          operationId!,
        ),
      "copy",
    ).then((result) => {
      if (!result.ok) setOperationError(result.error);
    });
  }, [performFileOperation, selectedEntries]);

  /** Compresses the selection into a unique archive next to the entries. */
  const compressSelection = useCallback(
    (format: ArchiveFormat) => {
      if (selectedEntries.length === 0 || !directoryPath) return;

      setOperationError(null);
      void performFileOperation(
        (operationId) =>
          commands.compressEntries(
            selectedEntries.map((entry) => entry.path),
            directoryPath,
            format,
            operationId!,
          ),
        "compress",
      ).then((result) => {
        if (!result.ok) setOperationError(result.error);
      });
    },
    [directoryPath, performFileOperation, selectedEntries],
  );

  /** Extracts an archive into a fresh folder next to it. */
  const extractSelection = useCallback(
    (archivePath: string) => {
      setOperationError(null);
      void performFileOperation(
        (operationId) => commands.extractArchive(archivePath, null, operationId!),
        "extract",
      ).then((result) => {
        if (!result.ok) setOperationError(result.error);
      });
    },
    [performFileOperation],
  );

  /** Native cross-platform folder picker feeding the existing move pipeline. */
  const moveSelectionTo = useCallback(() => {
    if (selectedEntries.length === 0 || !directoryPath) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setOperationError(null);

    void openDialog({
      defaultPath: directoryPath,
      directory: true,
      multiple: false,
      title: "选择目标文件夹",
    })
      .then((destination) => {
        if (typeof destination !== "string" || !destination || destination === directoryPath) {
          return;
        }

        transferEntries(sourcePaths, destination, "move");
      })
      .catch((error: unknown) => {
        console.warn("Unable to open destination picker", error);
      });
  }, [directoryPath, selectedEntries, transferEntries]);

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
      setRenameError("名称不能为空");
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

  const requestCreate = useCallback(
    (kind: NewEntryKind) => {
      if (!directoryPath || search.isActive) return;

      setNewEntryKind(kind);
      setNewEntryValue(kind === "file" ? "新建文件.txt" : "新建文件夹");
      setNewEntryError(null);
      setOperationError(null);
    },
    [directoryPath, search.isActive],
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
      setNewEntryError("名称不能为空");
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

  /** Moves the selection into the system trash; the batch stays undoable. */
  const trashSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const paths = selectedEntries.map((entry) => entry.path);
    setOperationError(null);
    setSelectedPaths([]);

    void performFileOperation(
      (operationId) => commands.trashEntries(paths, operationId!),
      "delete",
    ).then((result) => {
      if (!result.ok) {
        setOperationError(result.error);
        setSelectedPaths(paths);
        return;
      }
      setUndoRedoToast({
        message: `已将 ${paths.length.toLocaleString("zh-CN")} 个项目移到回收站`,
        action: "undo",
      });
    });
  }, [performFileOperation, selectedEntries]);

  /** Delete moves the selection to the trash when every entry is local;
   *  network locations have no recycle bin, so they keep the permanent-delete
   *  confirmation dialog. */
  const requestDelete = useCallback(() => {
    if (selectedEntries.length === 0) return;

    if (selectedEntries.every((entry) => isLocalExplorerPath(entry.path))) {
      trashSelection();
      return;
    }

    setDeleteTargets(selectedEntries);
    setOperationError(null);
  }, [selectedEntries, trashSelection]);

  /** Shift+Delete bypasses the trash and asks for permanent deletion. */
  const requestPermanentDelete = useCallback(() => {
    if (selectedEntries.length === 0) return;

    setDeleteTargets(selectedEntries);
    setOperationError(null);
  }, [selectedEntries]);

  /** Reverts the most recent recorded operation (move, rename, copy, trash,
   *  create, duplicate) through the backend history stack. */
  const undoLastOperation = useCallback(() => {
    if (!undoRedo.canUndo || isOperationPending) return;

    setUndoRedoToast(null);
    setOperationError(null);
    let outcome: UndoRedoOutcome | null = null;
    void performFileOperation(async (operationId) => {
      outcome = await commands.undoOperation(operationId!);
    }, "auto").then((result) => {
      if (!result.ok) {
        setOperationError(`撤销失败：${result.error}`);
        return;
      }
      if (outcome) {
        setUndoRedoToast({ message: outcome.message, action: "redo" });
      }
    });
  }, [isOperationPending, performFileOperation, undoRedo.canUndo]);

  /** Re-applies the most recently undone operation. */
  const redoLastOperation = useCallback(() => {
    if (!undoRedo.canRedo || isOperationPending) return;

    setUndoRedoToast(null);
    setOperationError(null);
    let outcome: UndoRedoOutcome | null = null;
    void performFileOperation(async (operationId) => {
      outcome = await commands.redoOperation(operationId!);
    }, "auto").then((result) => {
      if (!result.ok) {
        setOperationError(`重做失败：${result.error}`);
        return;
      }
      if (outcome) {
        setUndoRedoToast({ message: outcome.message, action: "undo" });
      }
    });
  }, [isOperationPending, performFileOperation, undoRedo.canRedo]);

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

  const retry = () => {
    if (directory) {
      void navigator.navigate(directory.path);
      return;
    }

    void navigator.initialize();
  };

  const navigateToPath = useCallback(
    (path: string) => navigator.navigate(path).then((result) => result !== undefined),
    [navigator],
  );

  const addToSpace = useCallback((spaceId: string, paths: string[]) => {
    void addItemsToSpace(spaceId, paths);
  }, []);

  const copySelectedPaths = useCallback(() => {
    if (selectedEntries.length === 0) return;

    void writeText(selectedEntries.map((entry) => entry.path).join("\n")).catch((error) => {
      console.warn("Unable to copy paths to clipboard", error);
    });
  }, [selectedEntries]);

  /** Opens the system default terminal at a directory (Windows Terminal,
   *  Terminal.app, or the desktop's default terminal on Linux). */
  const openTerminalHere = useCallback((path: string) => {
    setOperationError(null);
    void commands.openTerminal(path).catch((error: unknown) => {
      setOperationError(`无法打开终端：${getErrorMessage(error)}`);
    });
  }, []);

  /** Opens every selected file and navigates into the first selected folder. */
  const openSelectedEntries = useCallback(() => {
    if (selectedEntries.length === 0 || isOperationPending) return;

    const firstDirectory = selectedEntries.find((entry) => entry.kind === "directory");
    if (firstDirectory) {
      void navigator.navigate(firstDirectory.path);
    }

    for (const entry of selectedEntries) {
      if (entry.kind === "directory") continue;

      recordRecentItem(entry.path, "file", "opened");
      void openPath(entry.path).catch((error) => {
        console.warn(`Unable to open ${entry.path}`, error);
      });
    }
  }, [isOperationPending, navigator, selectedEntries]);

  const selectAll = useCallback(() => {
    setSelectedPaths(displayedEntries.map((entry) => entry.path));
  }, [displayedEntries]);

  /** Space toggles the preview surface for the first selected entry. */
  const togglePreview = useCallback(() => {
    setIsPreviewOpen((isOpen) => !isOpen);
  }, []);

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
      }
    },
    [
      copySelectedPaths,
      copySelection,
      cutSelection,
      directory,
      directoryPath,
      navigator,
      openTerminalHere,
      pasteClipboard,
      requestCreate,
      requestDelete,
      requestRename,
      selectAll,
      toggleFavorite,
    ],
  );

  // The command bar drops intents into the bus; the mounted (active tab)
  // explorer consumes them. Ids are tracked so React StrictMode's double
  // effect invocation cannot execute a command twice.
  const executedCommandIdsRef = useRef(new Set<number>());

  useEffect(() => {
    if (!pendingCommand) return;
    if (executedCommandIdsRef.current.has(pendingCommand.id)) return;

    executedCommandIdsRef.current.add(pendingCommand.id);
    clearPendingExplorerCommand();
    executeExplorerCommand(pendingCommand.command);
  }, [pendingCommand, executeExplorerCommand]);

  const isCurrentFavorited =
    directory !== null && favorites.some((favorite) => favorite.path === directory.path);

  return (
    <main className="h-full bg-card">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <header
          className="flex h-11 shrink-0 items-center gap-1 bg-background px-2"
          data-tauri-drag-region="deep"
        >
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              aria-label={sidebarVisible ? "隐藏侧边栏" : "显示侧边栏"}
              onClick={() => setSidebarVisible(!sidebarVisible)}
              size="icon"
              title={sidebarVisible ? "隐藏侧边栏" : "显示侧边栏"}
              type="button"
              variant="ghost"
            >
              <SidebarSimpleIcon />
            </Button>
            <ToolbarSeparator />
            <Button
              aria-label="后退"
              disabled={!canGoBack}
              onClick={() => void navigator.goBack()}
              size="icon"
              title="后退"
              type="button"
              variant="ghost"
            >
              <ArrowLeftIcon />
            </Button>
            <Button
              aria-label="前进"
              disabled={!canGoForward}
              onClick={() => void navigator.goForward()}
              size="icon"
              title="前进"
              type="button"
              variant="ghost"
            >
              <ArrowRightIcon />
            </Button>
            <Button
              aria-label="上一级"
              disabled={!canGoUp}
              onClick={() => void navigator.goUp()}
              size="icon"
              title="上一级"
              type="button"
              variant="ghost"
            >
              <ArrowUpIcon />
            </Button>
            <Button
              aria-label="刷新"
              disabled={isLoading || !directory}
              onClick={() => directory && void navigator.navigate(directory.path)}
              size="icon"
              title="刷新"
              type="button"
              variant="ghost"
            >
              <ArrowClockwiseIcon className={cn(isLoading && "animate-spin")} />
            </Button>
            <ToolbarSeparator />
            <Button
              aria-label={
                isCurrentFavorited ? "从常用位置移除当前目录" : "将当前目录添加到常用位置"
              }
              disabled={!directory}
              onClick={() =>
                directory &&
                toggleFavorite({
                  path: directory.path,
                  name: directory.breadcrumbs.at(-1)?.name ?? directory.path,
                })
              }
              size="icon"
              title={isCurrentFavorited ? "从常用位置移除当前目录" : "将当前目录添加到常用位置"}
              type="button"
              variant="ghost"
            >
              <StarIcon className={cn(isCurrentFavorited && "fill-amber-400 text-amber-500")} />
            </Button>
          </div>

          <div className="min-w-0 flex-1 px-2">
            {directory ? (
              <ExplorerPathBar
                directory={directory}
                onNavigate={(breadcrumb) => void navigator.navigateBreadcrumb(breadcrumb)}
                onNavigatePath={navigateToPath}
              />
            ) : (
              <Skeleton className="h-6 w-56 max-w-full rounded-full" />
            )}
          </div>

          <DirectorySearch
            contentSearch={contentSearch}
            directoryName={directory?.breadcrumbs.at(-1)?.name ?? null}
            disabled={isLoading}
            mode={searchMode}
            onModeChange={setSearchMode}
            search={search}
          />
          <SortMenu disabled={!directory} />
          <FilterMenu disabled={!directory} />
          <ToolbarSeparator />
          <Button
            aria-label={isPreviewOpen ? "收起预览面板" : "展开预览面板"}
            aria-pressed={isPreviewOpen}
            onClick={() => setIsPreviewOpen((isOpen) => !isOpen)}
            size="icon"
            title={isPreviewOpen ? "收起预览面板 (Space)" : "展开预览面板 (Space)"}
            type="button"
            variant="ghost"
          >
            <EyeIcon />
          </Button>
        </header>

        {isContentSearchActive && (
          <div className="shrink-0 border-b border-border/60 px-3 py-1.5">
            <ContentSearchToolbar search={contentSearch} />
          </div>
        )}

        {state.error && directory && (
          <div className="shrink-0 p-3 pb-0">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        )}

        {operationError && (
          <div className="shrink-0 p-3 pb-0">
            <Alert variant="destructive">
              <WarningIcon />
              <AlertTitle>文件操作未完成</AlertTitle>
              <AlertDescription>{operationError}</AlertDescription>
              <AlertAction>
                <Button
                  onClick={() => setOperationError(null)}
                  size="xs"
                  type="button"
                  variant="outline"
                >
                  关闭
                </Button>
              </AlertAction>
            </Alert>
          </div>
        )}

        {directory ? (
          <div className="flex min-h-0 flex-1">
            <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
              {isContentSearchActive ? (
                <ContentSearchResults
                  error={contentSearch.error}
                  isSearching={contentSearch.isSearching}
                  onOpenLocation={(location) => void navigator.navigate(location)}
                  query={contentSearch.query.trim()}
                  response={contentSearch.response}
                />
              ) : (
                <FileList
                  currentDirectoryPath={directory.path}
                  entries={displayedEntries}
                  externalDropItemCount={externalDrop?.sourcePaths.length ?? 0}
                  externalDropTargetPath={externalDrop?.targetPath ?? null}
                  gitStatus={gitStatus}
                  initialScrollOffset={
                    search.isActive ? 0 : navigator.getScrollOffset(directory.path)
                  }
                  isLoading={isLoading}
                  isOperationPending={isOperationPending}
                  canRedo={undoRedo.canRedo}
                  canUndo={undoRedo.canUndo}
                  onAddToFavorites={addFavoritePaths}
                  onAddToSpace={addToSpace}
                  onCompress={compressSelection}
                  onCopy={copySelection}
                  onCreateDirectory={() => requestCreate("directory")}
                  onCreateFile={() => requestCreate("file")}
                  onCut={cutSelection}
                  onDelete={requestDelete}
                  onDeletePermanent={requestPermanentDelete}
                  onDuplicate={duplicateSelection}
                  onDropEntries={transferEntries}
                  onCreateShortcuts={createShortcutsEntries}
                  onExtract={extractSelection}
                  onMoveTo={moveSelectionTo}
                  onOpenDirectory={(path) => void navigator.navigate(path)}
                  onOpenTerminal={() => directory.path && openTerminalHere(directory.path)}
                  onPaste={pasteClipboard}
                  onRename={requestRename}
                  onRedo={redoLastOperation}
                  onUndo={undoLastOperation}
                  onScrollOffsetChange={
                    search.isActive
                      ? undefined
                      : (offset) => navigator.setScrollOffset(directory.path, offset)
                  }
                  onSelectedPathsChange={setSelectedPaths}
                  onTogglePreview={togglePreview}
                  searchState={
                    search.isActive
                      ? {
                          error: search.error,
                          isSearching: search.isSearching,
                          query: search.query.trim(),
                          truncated: search.response?.truncated ?? false,
                        }
                      : undefined
                  }
                  selectedPaths={selectedPaths}
                  viewId={
                    search.isActive ? `${directory.path}::search::${search.query}` : directory.path
                  }
                />
              )}
              {selectedPaths.length > 0 && (
                <ContextualActionBar
                  archiveSelectionPath={
                    selectedEntries.length === 1 && isArchiveFile(selectedEntries[0])
                      ? selectedEntries[0].path
                      : null
                  }
                  hasDirectorySelection={selectedEntries.some(
                    (entry) => entry.kind === "directory",
                  )}
                  isActionDisabled={isOperationPending || isLoading}
                  isSingleSelection={selectedEntries.length === 1}
                  onAddToSpace={(spaceId) =>
                    addToSpace(
                      spaceId,
                      selectedEntries
                        .filter((entry) => entry.kind === "directory")
                        .map((entry) => entry.path),
                    )
                  }
                  onClearSelection={() => setSelectedPaths([])}
                  onCompress={compressSelection}
                  onCopy={copySelection}
                  onCopyPaths={copySelectedPaths}
                  onCut={cutSelection}
                  onDelete={requestDelete}
                  onDuplicate={duplicateSelection}
                  onExtract={extractSelection}
                  onMoveTo={moveSelectionTo}
                  onOpen={openSelectedEntries}
                  onRename={requestRename}
                  selectedCount={selectedPaths.length}
                />
              )}
              {undoRedoToast && (
                <div className="absolute bottom-4 left-1/2 z-40 -translate-x-1/2">
                  <div className="animate-float-in flex items-center gap-2 rounded-full bg-popover px-4 py-2 text-[13px] text-popover-foreground shadow-ambient-lg ring-1 ring-foreground/5">
                  {undoRedoToast.action === "redo" ? (
                    <ArrowClockwiseIcon className="size-4 shrink-0 text-muted-foreground" />
                  ) : (
                    <ArrowCounterClockwiseIcon className="size-4 shrink-0 text-muted-foreground" />
                  )}
                  <span className="whitespace-nowrap">{undoRedoToast.message}</span>
                  {undoRedoToast.action === "redo" ? (
                    <Button onClick={redoLastOperation} size="xs" type="button" variant="outline">
                      重做
                    </Button>
                  ) : (
                    <Button onClick={undoLastOperation} size="xs" type="button" variant="outline">
                      撤销
                    </Button>
                  )}
                  <Button
                    aria-label="关闭提示"
                    onClick={() => setUndoRedoToast(null)}
                    size="xs"
                    type="button"
                    variant="ghost"
                  >
                    <XIcon />
                  </Button>
                  </div>
                </div>
              )}
            </div>
            {isPreviewOpen && (
              <EntryPreview
                entry={selectedEntries[0] ?? null}
                onClose={() => setIsPreviewOpen(false)}
                onOpen={() => openSelectedEntries()}
              />
            )}
          </div>
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
        {fileOperationProgress && <FileOperationStatusBar progress={fileOperationProgress} />}
        <ExplorerStatusBar
          gitBranch={gitStatus?.branch ?? null}
          itemCount={
            isContentSearchActive
              ? (contentSearch.response?.files.length ?? 0)
              : displayedEntries.length
          }
          isLoading={isLoading || search.isSearching || contentSearch.isSearching}
          searchError={
            isContentSearchActive ? contentSearch.error : search.isActive ? search.error : null
          }
          searchQuery={
            isContentSearchActive
              ? contentSearch.query.trim()
              : search.isActive
                ? search.query.trim()
                : null
          }
          selectedCount={selectedPaths.length}
          truncated={
            isContentSearchActive
              ? (contentSearch.response?.truncated ?? false)
              : (search.response?.truncated ?? false)
          }
        />
      </section>

      <RenameDialog
        error={renameError}
        isPending={isOperationPending}
        onClose={closeRenameDialog}
        onOpenChange={(open) => {
          if (!open) closeRenameDialog();
        }}
        onSubmit={submitRename}
        onValueChange={setRenameValue}
        target={renameTarget}
        value={renameValue}
      />
      <CreateEntryDialog
        error={newEntryError}
        isPending={isOperationPending}
        kind={newEntryKind}
        onClose={closeCreateDialog}
        onOpenChange={(open) => {
          if (!open) closeCreateDialog();
        }}
        onSubmit={submitCreate}
        onValueChange={setNewEntryValue}
        value={newEntryValue}
      />
      <DeleteDialog
        entries={deleteTargets}
        isPending={isOperationPending}
        onClose={closeDeleteDialog}
        onConfirm={confirmDelete}
        onOpenChange={(open) => {
          if (!open) closeDeleteDialog();
        }}
      />
      {pendingTransfer && (
        <TransferConflictDialog
          conflicts={pendingTransfer.conflicts}
          operation={pendingTransfer.operation}
          onCancel={cancelTransferConflicts}
          onResolve={resolveTransferConflicts}
        />
      )}
    </main>
  );
}

function ToolbarSeparator() {
  return <div aria-hidden="true" className="mx-1 h-5 w-px bg-border" />;
}

/** Places local files on the OS clipboard (CF_HDROP) so Explorer, browsers,
 *  and chat apps accept a paste; network paths stay app-internal. */
function mirrorFilesToSystemClipboard(paths: string[], cut: boolean) {
  const localPaths = paths.filter(isLocalExplorerPath);
  if (localPaths.length === 0) return;

  void commands.writeFilesToClipboard(localPaths, cut).catch((error) => {
    console.warn("Unable to place files on the system clipboard", error);
  });
}

function pathListsEqual(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((path, index) => path === b[index]);
}

function FileOperationStatusBar({ progress }: { progress: FileOperationProgress }) {
  const operationLabel: Record<FileOperationKind, string> = {
    copy: "复制",
    move: "移动",
    delete: "删除",
    compress: "压缩",
    extract: "解压",
  };
  const total = progress.total;
  const percentage = total && total > 0 ? Math.round((progress.completed / total) * 100) : 0;
  const currentPath = progress.currentPath;
  const statusText =
    progress.phase === "preparing"
      ? `正在准备${operationLabel[progress.operation]}…`
      : progress.phase === "completed"
        ? `已完成${operationLabel[progress.operation]}`
        : `正在${operationLabel[progress.operation]}`;

  return (
    <footer
      aria-live="polite"
      className="flex h-10 shrink-0 items-center gap-3 border-t bg-background px-3"
    >
      <CircleNotchIcon
        className={cn(
          "size-3.5 shrink-0 text-primary",
          progress.phase !== "completed" && "animate-spin",
        )}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-3 text-xs">
          <span className="truncate">
            {statusText}
            {currentPath ? ` · ${currentPath}` : ""}
          </span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {total === null
              ? "正在计算项目数"
              : `${progress.completed.toLocaleString("zh-CN")} / ${total.toLocaleString("zh-CN")}（${percentage}%）`}
          </span>
        </div>
        <Progress className="mt-1.5 w-full" value={percentage} />
      </div>
    </footer>
  );
}

function RenameDialog({
  error,
  isPending,
  onClose,
  onOpenChange,
  onSubmit,
  onValueChange,
  target,
  value,
}: {
  error: string | null;
  isPending: boolean;
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onValueChange: (value: string) => void;
  target: DirectoryEntry | null;
  value: string;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={target !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>重命名</DialogTitle>
          <DialogDescription>为“{target?.name}”输入新名称。</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="rename-entry">新名称</FieldLabel>
              <Input
                aria-invalid={Boolean(error)}
                autoFocus
                disabled={isPending}
                id="rename-entry"
                onChange={(event) => onValueChange(event.target.value)}
                onFocus={(event) => event.currentTarget.select()}
                value={value}
              />
              <FieldError>{error}</FieldError>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
              取消
            </Button>
            <Button disabled={isPending} type="submit">
              重命名
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function CreateEntryDialog({
  error,
  isPending,
  kind,
  onClose,
  onOpenChange,
  onSubmit,
  onValueChange,
  value,
}: {
  error: string | null;
  isPending: boolean;
  kind: NewEntryKind | null;
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onValueChange: (value: string) => void;
  value: string;
}) {
  const isFile = kind === "file";

  return (
    <Dialog onOpenChange={onOpenChange} open={kind !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{isFile ? "新建文件" : "新建文件夹"}</DialogTitle>
          <DialogDescription>
            {isFile ? "输入文件名称，可包含扩展名（例如 notes.txt）。" : "输入文件夹名称。"}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="create-entry">名称</FieldLabel>
              <Input
                aria-invalid={Boolean(error)}
                autoFocus
                disabled={isPending}
                id="create-entry"
                onChange={(event) => onValueChange(event.target.value)}
                onFocus={(event) => {
                  const input = event.currentTarget;
                  const dotIndex = isFile ? input.value.lastIndexOf(".") : -1;
                  input.setSelectionRange(0, dotIndex > 0 ? dotIndex : input.value.length);
                }}
                value={value}
              />
              <FieldError>{error}</FieldError>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
              取消
            </Button>
            <Button disabled={isPending} type="submit">
              创建
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteDialog({
  entries,
  isPending,
  onClose,
  onConfirm,
  onOpenChange,
}: {
  entries: DirectoryEntry[];
  isPending: boolean;
  onClose: () => void;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
}) {
  const description =
    entries.length === 1
      ? `“${entries[0].name}”将被永久删除，无法从回收站恢复。`
      : `所选的 ${entries.length} 个项目将被永久删除，无法从回收站恢复。`;

  return (
    <Dialog onOpenChange={onOpenChange} open={entries.length > 0}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>永久删除</DialogTitle>
          <DialogDescription>
            {description}直接按 Delete 键则移入回收站，可随时用 Ctrl+Z 撤销。
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
            取消
          </Button>
          <Button disabled={isPending} onClick={onConfirm} type="button" variant="destructive">
            永久删除
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ExplorerErrorAlert({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <Alert variant="destructive">
      <WarningIcon />
      <AlertTitle>无法读取此位置</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
      <AlertAction>
        <Button onClick={onRetry} size="xs" type="button" variant="outline">
          重试
        </Button>
      </AlertAction>
    </Alert>
  );
}

function getCreateEntryErrorMessage(message: string, rawError: unknown): string {
  const kind =
    typeof rawError === "object" &&
    rawError !== null &&
    "kind" in rawError &&
    typeof (rawError as { kind?: unknown }).kind === "string"
      ? (rawError as { kind: string }).kind
      : null;

  switch (kind) {
    case "already_exists":
      return "已存在同名项目，请使用其他名称";
    case "permission_denied":
      return "没有权限在此位置创建项目";
    case "not_found":
      return "当前目录不存在或已被移动";
    case "not_directory":
      return "当前位置不是文件夹";
  }

  if (message.includes("must not contain a path separator")) {
    return "名称不能包含路径分隔符";
  }

  return message;
}

function getErrorMessage(error: unknown): string {
  const kind = extractErrorKind(error);
  if (kind) {
    const friendly = FILE_OPERATION_ERROR_MESSAGES[kind];
    if (friendly) {
      // Only the structured IPC payload carries a clean detail message; an
      // Error instance's message is the raw stringified form.
      const detail =
        typeof error === "object" &&
        error !== null &&
        !(error instanceof Error) &&
        "message" in error
          ? (error as { message: unknown }).message
          : undefined;
      return typeof detail === "string" && detail ? `${friendly}：${detail}` : friendly;
    }
  }

  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }

  return error instanceof Error ? error.message : String(error);
}

/** Extracts the FileSystemError kind, either from the structured Tauri IPC
 *  payload ({ kind, message }) or from the stringified form produced by the
 *  dev-invoke bridge when debugging in a plain browser. */
function extractErrorKind(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "kind" in error) {
    return String((error as { kind: unknown }).kind);
  }

  const raw = error instanceof Error ? error.message : typeof error === "string" ? error : null;
  const match = raw?.match(/"kind":\s*(?:String\()?"([a-z_]+)"/);
  return match?.[1] ?? null;
}

const FILE_OPERATION_ERROR_MESSAGES: Record<string, string> = {
  already_exists: "目标位置已存在同名项目",
  not_found: "项目不存在或已被移动",
  permission_denied: "没有足够的权限完成此操作",
  invalid_input: "无法完成此操作",
  not_a_directory: "目标不是文件夹",
  not_directory: "目标不是文件夹",
  io: "文件系统操作失败",
  internal: "操作失败",
};
