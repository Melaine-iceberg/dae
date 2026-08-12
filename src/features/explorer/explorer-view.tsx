import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type FormEvent,
} from "react";
import { useAtom } from "jotai";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  RefreshCwIcon,
  TriangleAlertIcon,
} from "lucide-react";

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
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

import { explorerApi } from "./api";
import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
import { FileList, FileListSkeleton } from "./file-list";
import type { ExplorerNavigator } from "./navigation";
import { fileClipboardAtom } from "./tabs";
import type { DirectoryEntry } from "./types";

const DIRECTORY_CHANGED_EVENT = "explorer-directory-changed";
const DIRECTORY_REFRESH_DELAY_MS = 150;
const appWindow = getCurrentWindow();

interface ExplorerViewProps {
  navigator: ExplorerNavigator;
}

type FileOperationResult = { ok: true } | { error: string; ok: false };

export function ExplorerView({ navigator }: ExplorerViewProps) {
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const [clipboard, setClipboard] = useAtom(fileClipboardAtom);
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [renameTarget, setRenameTarget] = useState<DirectoryEntry | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<DirectoryEntry[]>([]);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [isOperationPending, setIsOperationPending] = useState(false);
  const directory = state.directory;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;
  const selectedEntries = useMemo(
    () => directory?.entries.filter((entry) => selectedPaths.includes(entry.path)) ?? [],
    [directory?.entries, selectedPaths],
  );

  useEffect(() => {
    if (navigator.getSnapshot().status === "idle") {
      void navigator.initialize();
    }
  }, [navigator]);

  useEffect(() => {
    if (!directoryPath) return;

    void explorerApi
      .watchDirectory(directoryPath)
      .then(() => void navigator.refresh(directoryPath))
      .catch((error: unknown) => {
        console.warn("Unable to watch directory for changes", error);
      });
  }, [navigator, directoryPath]);

  useEffect(() => {
    setSelectedPaths([]);
    setRenameTarget(null);
    setDeleteTargets([]);
    setOperationError(null);
  }, [directoryPath]);

  useEffect(() => {
    if (!directory) return;

    const availablePaths = new Set(directory.entries.map((entry) => entry.path));
    setSelectedPaths((paths) => {
      const availableSelection = paths.filter((path) => availablePaths.has(path));
      return availableSelection.length === paths.length ? paths : availableSelection;
    });
  }, [directory]);

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

    const unlistenChangesPromise = listen<string>(DIRECTORY_CHANGED_EVENT, ({ payload }) => {
      scheduleRefresh(payload);
    });
    const unlistenFocusPromise = appWindow.onFocusChanged(({ payload: focused }) => {
      const currentPath = navigator.getSnapshot().directory?.path;
      if (focused && currentPath) scheduleRefresh(currentPath);
    });

    return () => {
      disposed = true;
      window.clearTimeout(refreshTimeout);
      void Promise.all([unlistenChangesPromise, unlistenFocusPromise]).then((unlisten) => {
        unlisten.forEach((stopListening) => stopListening());
      });
    };
  }, [navigator]);

  const performFileOperation = useCallback(
    async (operation: () => Promise<void>): Promise<FileOperationResult> => {
      if (!directoryPath) {
        return { error: "当前目录不可用", ok: false };
      }

      setIsOperationPending(true);

      try {
        await operation();
        await navigator.refresh(directoryPath);
        return { ok: true };
      } catch (error) {
        return { error: getErrorMessage(error), ok: false };
      } finally {
        setIsOperationPending(false);
      }
    },
    [directoryPath, navigator],
  );

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

    void performFileOperation(() =>
      pastedClipboard.operation === "copy"
        ? explorerApi.copyEntries(pastedClipboard.sourcePaths, directoryPath)
        : explorerApi.moveEntries(pastedClipboard.sourcePaths, directoryPath),
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

    void performFileOperation(() => explorerApi.renameEntry(sourcePath, nextName)).then(
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

    void performFileOperation(() => explorerApi.deleteEntries(paths)).then((result) => {
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

  return (
    <main className="h-full bg-background">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <header
          className="flex h-14 shrink-0 items-center gap-1 border-b px-2 sm:px-3"
          data-tauri-drag-region
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

          <div className="min-w-0 flex-1 px-2" data-tauri-drag-region>
            {directory ? (
              <ExplorerBreadcrumbs
                breadcrumbs={directory.breadcrumbs}
                onNavigate={(breadcrumb) => void navigator.navigateBreadcrumb(breadcrumb)}
              />
            ) : (
              <Skeleton className="h-4 w-48 max-w-full" />
            )}
          </div>
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
            entries={directory.entries}
            hasClipboard={clipboard !== null}
            initialScrollOffset={navigator.getScrollOffset(directory.path)}
            isLoading={isLoading}
            isOperationPending={isOperationPending}
            onCopy={copySelection}
            onCut={cutSelection}
            onDelete={requestDelete}
            onOpenDirectory={(path) => void navigator.navigate(path)}
            onPaste={pasteClipboard}
            onRename={requestRename}
            onScrollOffsetChange={(offset) => navigator.setScrollOffset(directory.path, offset)}
            onSelectedPathsChange={setSelectedPaths}
            selectedPaths={selectedPaths}
          />
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
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
