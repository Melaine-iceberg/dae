import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type FormEvent,
} from "react";
import { useAtom } from "jotai";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  TriangleAlertIcon,
} from "lucide-react";

import { commands, events } from "@/bindings";

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
import { cn } from "@/lib/utils";

import { DirectorySearch, useDirectorySearch } from "./directory-search";
import { getExplorerDropTargetAtPoint, type FileTransferOperation } from "./drag-drop";
import { ExplorerPathBar } from "./explorer-path-bar";
import { FileList, FileListSkeleton } from "./file-list";
import type { ExplorerNavigator } from "./navigation";
import { fileClipboardAtom } from "./tabs";
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

export function ExplorerView({ navigator }: ExplorerViewProps) {
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const [clipboard, setClipboard] = useAtom(fileClipboardAtom);
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [renameTarget, setRenameTarget] = useState<DirectoryEntry | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [newEntryKind, setNewEntryKind] = useState<NewEntryKind | null>(null);
  const [newEntryValue, setNewEntryValue] = useState("");
  const [newEntryError, setNewEntryError] = useState<string | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<DirectoryEntry[]>([]);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [isOperationPending, setIsOperationPending] = useState(false);
  const [fileOperationProgress, setFileOperationProgress] = useState<FileOperationProgress | null>(
    null,
  );
  const [externalDrop, setExternalDrop] = useState<ExternalDrop | null>(null);
  const directory = state.directory;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;
  const search = useDirectorySearch(directoryPath ?? null, directory);
  const displayedEntries = useMemo(
    () => (search.isActive ? (search.response?.entries ?? []) : (directory?.entries ?? [])),
    [directory?.entries, search.isActive, search.response],
  );
  const selectedEntries = useMemo(
    () => displayedEntries.filter((entry) => selectedPaths.includes(entry.path)),
    [displayedEntries, selectedPaths],
  );

  useEffect(() => {
    if (navigator.getSnapshot().status === "idle") {
      void navigator.initialize();
    }
  }, [navigator]);

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
        if (
          !currentProgress ||
          currentProgress.operationId !== payload.operationId ||
          currentProgress.phase === "completed"
        ) {
          return currentProgress;
        }

        return payload;
      });
    });

    return () => {
      void unlistenProgressPromise.then((unlisten) => unlisten());
    };
  }, []);

  const performFileOperation = useCallback(
    async (
      operation: (operationId?: string) => Promise<unknown>,
      progressOperation?: FileOperationKind,
    ): Promise<FileOperationResult> => {
      if (!directoryPath) {
        return { error: "当前目录不可用", ok: false };
      }

      const operationId = progressOperation ? crypto.randomUUID() : undefined;
      if (operationId && progressOperation) {
        setFileOperationProgress({
          operationId,
          operation: progressOperation,
          phase: "preparing",
          completed: 0,
          total: null,
          currentPath: null,
        });
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
        setIsOperationPending(false);
      }
    },
    [directoryPath, navigator],
  );

  const transferEntries = useCallback(
    (sourcePaths: string[], destinationPath: string, operation: FileTransferOperation) => {
      if (sourcePaths.length === 0) return;

      setOperationError(null);
      void performFileOperation(
        (operationId) =>
          operation === "copy"
            ? commands.copyEntries(sourcePaths, destinationPath, operationId!)
            : commands.moveEntries(sourcePaths, destinationPath, operationId!),
        operation,
      ).then((result) => {
        if (!result.ok) {
          setOperationError(result.error);
          return;
        }

        setSelectedPaths([]);
      });
    },
    [performFileOperation],
  );

  const copyExternalEntries = useCallback(
    (sourcePaths: string[], destinationPath: string) => {
      if (sourcePaths.length === 0) return;

      setOperationError(null);
      void performFileOperation(
        (operationId) => commands.copyEntries(sourcePaths, destinationPath, operationId!),
        "copy",
      ).then((result) => {
        if (!result.ok) {
          setOperationError(result.error);
          return;
        }
        setSelectedPaths([]);
      });
    },
    [performFileOperation],
  );

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

    setClipboard({ operation: "copy", sourcePaths: selectedEntries.map((entry) => entry.path) });
    setOperationError(null);
  }, [selectedEntries, setClipboard]);

  const cutSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    setClipboard({ operation: "cut", sourcePaths: selectedEntries.map((entry) => entry.path) });
    setOperationError(null);
  }, [selectedEntries, setClipboard]);

  const pasteClipboard = useCallback(() => {
    if (!clipboard || !directoryPath) return;

    setOperationError(null);
    const pastedClipboard = clipboard;

    void performFileOperation(
      (operationId) =>
        pastedClipboard.operation === "copy"
          ? commands.copyEntries(pastedClipboard.sourcePaths, directoryPath, operationId!)
          : commands.moveEntries(pastedClipboard.sourcePaths, directoryPath, operationId!),
      pastedClipboard.operation === "copy" ? "copy" : "move",
    ).then((result) => {
      if (!result.ok) {
        setOperationError(result.error);
        return;
      }

      if (pastedClipboard.operation === "cut") {
        setClipboard(null);
      }
      setSelectedPaths([]);
    });
  }, [clipboard, directoryPath, performFileOperation, setClipboard]);

  const requestRename = useCallback(() => {
    if (selectedEntries.length !== 1) return;

    const [entry] = selectedEntries;
    setRenameTarget(entry);
    setRenameValue(entry.name);
    setRenameError(null);
    setOperationError(null);
  }, [selectedEntries]);

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

    void performFileOperation(() => commands.renameEntry(sourcePath, nextName)).then(
      (result) => {
        if (!result.ok) {
          setRenameError(result.error);
          return;
        }

        setRenameTarget(null);
        setSelectedPaths([]);
      },
    );
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

  const requestDelete = useCallback(() => {
    if (selectedEntries.length === 0) return;

    setDeleteTargets(selectedEntries);
    setOperationError(null);
  }, [selectedEntries]);

  const closeDeleteDialog = () => {
    if (!isOperationPending) {
      setDeleteTargets([]);
    }
  };

  const confirmDelete = () => {
    if (deleteTargets.length === 0) return;

    setOperationError(null);
    const paths = deleteTargets.map((entry) => entry.path);

    void performFileOperation(
      (operationId) => commands.deleteEntries(paths, operationId!),
      "delete",
    ).then((result) => {
      if (!result.ok) {
        setOperationError(result.error);
        return;
      }

      setDeleteTargets([]);
      setSelectedPaths([]);
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

  return (
    <main className="h-full bg-background">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <header
          className="flex h-14 shrink-0 items-center gap-1 border-b px-2 sm:px-3"
          data-tauri-drag-region="deep"
        >
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              aria-label="后退"
              disabled={!canGoBack}
              onClick={() => void navigator.goBack()}
              size="icon-sm"
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
              size="icon-sm"
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
              size="icon-sm"
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
              size="icon-sm"
              title="刷新"
              type="button"
              variant="ghost"
            >
              <RefreshCwIcon className={cn(isLoading && "animate-spin")} />
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
              <Skeleton className="h-4 w-48 max-w-full" />
            )}
          </div>

          <DirectorySearch
            directoryName={directory?.breadcrumbs.at(-1)?.name ?? null}
            disabled={isLoading}
            search={search}
          />
        </header>

        {state.error && directory && (
          <div className="shrink-0 p-3 pb-0">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        )}

        {operationError && (
          <div className="shrink-0 p-3 pb-0">
            <Alert variant="destructive">
              <TriangleAlertIcon />
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
          <FileList
            currentDirectoryPath={directory.path}
            entries={displayedEntries}
            externalDropItemCount={externalDrop?.sourcePaths.length ?? 0}
            externalDropTargetPath={externalDrop?.targetPath ?? null}
            hasClipboard={clipboard !== null}
            initialScrollOffset={search.isActive ? 0 : navigator.getScrollOffset(directory.path)}
            isLoading={isLoading}
            isOperationPending={isOperationPending}
            onCopy={copySelection}
            onCreateDirectory={() => requestCreate("directory")}
            onCreateFile={() => requestCreate("file")}
            onCut={cutSelection}
            onDelete={requestDelete}
            onDropEntries={transferEntries}
            onOpenDirectory={(path) => void navigator.navigate(path)}
            onPaste={pasteClipboard}
            onRename={requestRename}
            onScrollOffsetChange={
              search.isActive
                ? undefined
                : (offset) => navigator.setScrollOffset(directory.path, offset)
            }
            onSelectedPathsChange={setSelectedPaths}
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
            viewId={search.isActive ? `${directory.path}::search::${search.query}` : directory.path}
          />
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
        {fileOperationProgress && <FileOperationStatusBar progress={fileOperationProgress} />}
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
    </main>
  );
}

function FileOperationStatusBar({ progress }: { progress: FileOperationProgress }) {
  const operationLabel: Record<FileOperationKind, string> = {
    copy: "复制",
    move: "移动",
    delete: "删除",
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
    <footer aria-live="polite" className="flex shrink-0 items-center gap-3 border-t px-4 py-2">
      <LoaderCircleIcon
        className={cn(
          "size-4 text-muted-foreground",
          progress.phase !== "completed" && "animate-spin",
        )}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-3 text-xs">
          <span className="truncate font-medium">{statusText}</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {total === null
              ? "正在计算项目数"
              : `${progress.completed.toLocaleString("zh-CN")} / ${total.toLocaleString("zh-CN")}（${percentage}%）`}
          </span>
        </div>
        <p className="truncate text-xs text-muted-foreground" title={currentPath ?? undefined}>
          {currentPath ?? "请稍候…"}
        </p>
        <Progress className="mt-1 w-full" value={percentage} />
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
      ? `“${entries[0].name}”将被永久删除，且无法恢复。`
      : `将永久删除所选的 ${entries.length} 个项目，且无法恢复。`;

  return (
    <Dialog onOpenChange={onOpenChange} open={entries.length > 0}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>确认删除</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
            取消
          </Button>
          <Button disabled={isPending} onClick={onConfirm} type="button" variant="destructive">
            删除
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ExplorerErrorAlert({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <Alert variant="destructive">
      <TriangleAlertIcon />
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
