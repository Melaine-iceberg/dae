import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwiseIcon,
  ArrowUUpLeftIcon,
  CheckIcon,
  FolderIcon,
  TrashIcon,
  TrashSimpleIcon,
} from "@phosphor-icons/react";

import {
  commands,
  events,
  type FileOperationKind,
  type FileOperationProgress,
  type TrashEntry,
} from "@/bindings";

import { i18n } from "@/i18n";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { cn, formatBytes } from "@/lib/utils";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DIRECTORY_PRESENTATION,
  getFilePresentation,
} from "@/features/explorer/file-icons";
import { TypeIconTile } from "@/features/explorer/icon-tile";

import { navigateToFolderAtom } from "./workspace-atoms";
import { WorkspacePage, WorkspacePageHeader, baseNameOf } from "./workspace-components";

/** How long the finished progress bar stays visible before clearing. */
const COMPLETED_OPERATION_STATUS_DURATION_MS = 900;

/** Row layout: checkbox · name · original location · deleted at · size. */
const ROW_GRID =
  "grid grid-cols-[28px_minmax(0,1.2fr)_minmax(0,1fr)_130px_90px] items-center gap-2";

/** A confirmed permanent deletion: either the selection or the whole trash. */
type PurgeRequest = { kind: "empty" } | { kind: "selection"; ids: string[] };

/**
 * The Trash surface: lists the system recycle bin so deleted entries can be
 * inspected, restored to their original locations, purged individually, or
 * the whole bin emptied — the fallback when the undo window was missed.
 */
export function TrashView() {
  const { t } = useTranslation("workspace");
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const [entries, setEntries] = useState<TrashEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [progress, setProgress] = useState<FileOperationProgress | null>(null);
  const [isOperationPending, setIsOperationPending] = useState(false);
  const [purgeRequest, setPurgeRequest] = useState<PurgeRequest | null>(null);
  // Guards against stale operation events after an unmount.
  const mountedRef = useRef(true);

  const reload = useCallback(async () => {
    try {
      const result = await commands.listTrash();
      setEntries(result);
      setLoadError(null);
      // Entries purged/restored elsewhere drop out of the selection too.
      setSelectedIds((ids) => {
        const available = new Set(result.map((entry) => entry.id));
        const kept = ids.filter((id) => available.has(id));
        return kept.length === ids.length ? ids : kept;
      });
    } catch (error) {
      setEntries(null);
      setLoadError(getFileOperationErrorMessage(error));
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void reload();
    return () => {
      mountedRef.current = false;
    };
  }, [reload]);

  // Progress events ride the same bus as explorer operations; only events
  // for the operation this view started move its bar.
  useEffect(() => {
    const unlistenPromise = events.explorerFileOperationProgress.listen(({ payload }) => {
      setProgress((current) => {
        if (
          !current ||
          current.operationId !== payload.operationId ||
          current.phase === "completed"
        ) {
          return current;
        }
        return payload;
      });
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const runTrashOperation = useCallback(
    async (kind: FileOperationKind, operation: (operationId: string) => Promise<unknown>) => {
      const operationId = crypto.randomUUID();
      setProgress({
        operationId,
        operation: kind,
        phase: "preparing",
        completed: 0,
        total: null,
        currentPath: null,
      });
      setOperationError(null);
      setIsOperationPending(true);

      try {
        await operation(operationId);
        await reload();
        setProgress((current) =>
          current?.operationId === operationId
            ? { ...current, phase: "completed", completed: current.total ?? current.completed }
            : current,
        );
        window.setTimeout(() => {
          if (!mountedRef.current) return;
          setProgress((current) => (current?.operationId === operationId ? null : current));
        }, COMPLETED_OPERATION_STATUS_DURATION_MS);
      } catch (error) {
        setProgress(null);
        setOperationError(getFileOperationErrorMessage(error));
        // Whatever the batch managed before failing should show up again.
        void reload();
      } finally {
        setIsOperationPending(false);
      }
    },
    [reload],
  );

  const restoreIds = useCallback(
    (ids: string[]) => {
      if (ids.length === 0 || isOperationPending) return;
      setSelectedIds([]);
      void runTrashOperation("move", (operationId) =>
        commands.restoreTrashEntries(ids, operationId),
      );
    },
    [isOperationPending, runTrashOperation],
  );

  const confirmPurge = () => {
    if (!purgeRequest || isOperationPending) return;

    const request = purgeRequest;
    setPurgeRequest(null);
    if (request.kind === "selection") setSelectedIds([]);

    void runTrashOperation("delete", (operationId) =>
      request.kind === "empty"
        ? commands.emptyTrash(operationId)
        : commands.deleteTrashEntries(request.ids, operationId),
    );
  };

  const toggleSelected = (id: string) => {
    setSelectedIds((ids) =>
      ids.includes(id) ? ids.filter((candidate) => candidate !== id) : [...ids, id],
    );
  };

  const allSelected = (entries?.length ?? 0) > 0 && selectedIds.length === entries?.length;
  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? [] : (entries ?? []).map((entry) => entry.id));
  };

  const totalBytes = useMemo(
    () => (entries ?? []).reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0),
    [entries],
  );
  const description =
    entries === null
      ? t("trash.description")
      : t("trash.summary", { count: entries.length, size: formatBytes(totalBytes) });

  return (
    <WorkspacePage aria-label={t("trash.title")}>
      <WorkspacePageHeader
        actions={
          <>
            {selectedIds.length > 0 && (
              <>
                <Button
                  disabled={isOperationPending}
                  onClick={() => restoreIds(selectedIds)}
                  size="sm"
                  type="button"
                >
                  <ArrowUUpLeftIcon />
                  {t("trash.restoreSelected", { count: selectedIds.length })}
                </Button>
                <Button
                  disabled={isOperationPending}
                  onClick={() => setPurgeRequest({ kind: "selection", ids: selectedIds })}
                  size="sm"
                  type="button"
                  variant="destructive"
                >
                  <TrashSimpleIcon />
                  {t("trash.deleteSelected", { count: selectedIds.length })}
                </Button>
              </>
            )}
            {(entries?.length ?? 0) > 0 && (
              <Button
                disabled={isOperationPending}
                onClick={() => setPurgeRequest({ kind: "empty" })}
                size="sm"
                type="button"
                variant="outline"
              >
                <TrashIcon />
                {t("trash.emptyTrash")}
              </Button>
            )}
            <Button
              aria-label={t("trash.refresh")}
              disabled={isOperationPending}
              onClick={() => void reload()}
              size="icon-sm"
              type="button"
              variant="ghost"
            >
              <ArrowClockwiseIcon />
            </Button>
          </>
        }
        description={description}
        title={t("trash.title")}
      />

      {progress && <TrashProgress progress={progress} />}

      {loadError && (
        <Alert variant="destructive">
          <AlertTitle>{t("trash.loadErrorTitle")}</AlertTitle>
          <AlertDescription>{loadError}</AlertDescription>
        </Alert>
      )}

      {operationError && (
        <Alert variant="destructive">
          <AlertTitle>{t("trash.operationErrorTitle")}</AlertTitle>
          <AlertDescription>{operationError}</AlertDescription>
        </Alert>
      )}

      {entries === null ? (
        loadError === null && (
          <div className="flex flex-col gap-1">
            {Array.from({ length: 8 }, (_, index) => (
              <Skeleton className="h-10 rounded-lg" key={index} />
            ))}
          </div>
        )
      ) : entries.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <TrashIcon />
            </EmptyMedia>
            <EmptyTitle>{t("trash.emptyTitle")}</EmptyTitle>
            <EmptyDescription>{t("trash.emptyDescription")}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="overflow-hidden rounded-xl border border-border/60">
          <div className={cn(ROW_GRID, "border-b px-3 py-2 text-label text-muted-foreground")}>
            <SelectAllToggle allSelected={allSelected} onToggle={toggleSelectAll} />
            <span>{t("trash.columns.name")}</span>
            <span>{t("trash.columns.originalLocation")}</span>
            <span>{t("trash.columns.deletedAt")}</span>
            <span className="text-right">{t("trash.columns.size")}</span>
          </div>
          <ul className="flex flex-col">
            {entries.map((entry) => (
              <TrashRow
                entry={entry}
                isSelected={selectedIds.includes(entry.id)}
                key={entry.id}
                onNavigateToOriginalLocation={() => navigateToFolder(entry.originalParent)}
                onPurge={() => setPurgeRequest({ kind: "selection", ids: [entry.id] })}
                onRestore={() => restoreIds([entry.id])}
                onToggleSelected={() => toggleSelected(entry.id)}
              />
            ))}
          </ul>
        </div>
      )}

      <Dialog
        onOpenChange={(open) => {
          if (!open && !isOperationPending) setPurgeRequest(null);
        }}
        open={purgeRequest !== null}
      >
        <DialogContent showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>
              {purgeRequest?.kind === "empty"
                ? t("trash.confirmEmptyTitle")
                : t("trash.confirmPurgeTitle", {
                    count: purgeRequest?.kind === "selection" ? purgeRequest.ids.length : 0,
                  })}
            </DialogTitle>
            <DialogDescription>
              {purgeRequest?.kind === "empty"
                ? t("trash.confirmEmptyDescription", { count: entries?.length ?? 0 })
                : t("trash.confirmPurgeDescription")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose render={<Button disabled={isOperationPending} variant="outline" />}>
              {t("trash.cancel")}
            </DialogClose>
            <Button disabled={isOperationPending} onClick={confirmPurge} variant="destructive">
              {t("trash.confirmPurge")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </WorkspacePage>
  );
}

function TrashProgress({ progress }: { progress: FileOperationProgress }) {
  const { t } = useTranslation("workspace");
  const percent =
    progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.completed / progress.total) * 100))
      : null;

  const label =
    progress.phase === "preparing" || progress.currentPath === null
      ? t("trash.progress.preparing")
      : t(progress.operation === "move" ? "trash.progress.restoring" : "trash.progress.deleting", {
          path: baseNameOf(progress.currentPath),
        });

  return (
    <div className="flex flex-col gap-1.5 rounded-xl bg-popover p-3 shadow-ambient ring-1 ring-border/80">
      <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span className="truncate">{label}</span>
        {progress.total !== null && (
          <span className="shrink-0 font-mono tabular-nums">
            {progress.completed}/{progress.total}
          </span>
        )}
      </div>
      <Progress value={percent ?? null} />
    </div>
  );
}

function SelectAllToggle({
  allSelected,
  onToggle,
}: {
  allSelected: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation("workspace");
  return (
    <button
      aria-label={t("trash.selectAll")}
      aria-pressed={allSelected}
      className="flex size-4 items-center justify-center rounded-xs border border-input transition-colors hover:border-primary"
      onClick={onToggle}
      type="button"
    >
      {allSelected && <CheckIcon className="size-3 text-primary" weight="bold" />}
    </button>
  );
}

function TrashRow({
  entry,
  isSelected,
  onNavigateToOriginalLocation,
  onPurge,
  onRestore,
  onToggleSelected,
}: {
  entry: TrashEntry;
  isSelected: boolean;
  onNavigateToOriginalLocation: () => void;
  onPurge: () => void;
  onRestore: () => void;
  onToggleSelected: () => void;
}) {
  const { t } = useTranslation("workspace");
  const presentation = entry.isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);

  return (
    <li className="border-b last:border-b-0">
      <ContextMenu>
        <ContextMenuTrigger>
          {/* The row body toggles selection; double-click restores, like Explorer. */}
          <button
            aria-label={entry.name}
            aria-pressed={isSelected}
            className={cn(
              ROW_GRID,
              "w-full px-3 py-2 text-left transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              isSelected && "bg-selection",
            )}
            onClick={onToggleSelected}
            onDoubleClick={onRestore}
            title={`${entry.name} · ${entry.originalParent}`}
            type="button"
          >
            <RowCheckbox isSelected={isSelected} />
            <span className="flex min-w-0 items-center gap-2">
              <TypeIconTile
                className="size-[22px] rounded-[7px]"
                iconSize={13}
                presentation={presentation}
              />
              <span className="truncate text-[13px]">{entry.name}</span>
            </span>
            <span className="truncate text-xs text-muted-foreground">{entry.originalParent}</span>
            <span className="truncate text-xs text-muted-foreground tabular-nums">
              {formatDeletedTime(entry.timeDeleted)}
            </span>
            <span className="text-right text-xs text-muted-foreground tabular-nums">
              {entry.sizeBytes === null ? "—" : formatBytes(entry.sizeBytes)}
            </span>
          </button>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuGroup>
            <ContextMenuItem onClick={onRestore}>
              <ArrowUUpLeftIcon />
              {t("trash.restore")}
            </ContextMenuItem>
            <ContextMenuItem onClick={onNavigateToOriginalLocation}>
              <FolderIcon />
              {t("trash.openOriginalLocation")}
            </ContextMenuItem>
          </ContextMenuGroup>
          <ContextMenuSeparator />
          <ContextMenuGroup>
            <ContextMenuItem onClick={onPurge} variant="destructive">
              <TrashSimpleIcon />
              {t("trash.deleteForever")}
            </ContextMenuItem>
          </ContextMenuGroup>
        </ContextMenuContent>
      </ContextMenu>
    </li>
  );
}

function RowCheckbox({ isSelected }: { isSelected: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex size-4 items-center justify-center rounded-xs border transition-colors",
        isSelected ? "border-primary bg-primary text-primary-foreground" : "border-input",
      )}
    >
      {isSelected && <CheckIcon className="size-3" weight="bold" />}
    </span>
  );
}

/** Locale-aware date and time for the deletion timestamp (unix seconds). */
function formatDeletedTime(unixSeconds: number): string {
  if (unixSeconds <= 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleString(i18n.language, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
