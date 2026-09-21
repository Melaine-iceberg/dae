import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useHotkeys } from "@tanstack/react-hotkeys";
import { useVirtualizer } from "@tanstack/react-virtual";
import { RotateCw, Undo2, Check, Folder, Trash2, Trash } from "lucide-react";

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

import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
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
import { Kbd } from "@/components/ui/kbd";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "@/features/explorer/file-icons";
import { TypeIconTile } from "@/features/explorer/icon-tile";
import { HOTKEY_COMMON_OPTIONS, asHotkey, guardedAction } from "@/features/settings/hotkeys";
import { formatBinding, resolveBinding } from "@/features/settings/shortcut-registry";
import { appSettingsAtom, hotkeysPausedAtom, useBinding } from "@/features/settings/settings-atoms";

import {
  jumpTrashIndex,
  stepTrashIndex,
  trashPurgeTargets,
  typeAheadTrashIndex,
} from "./trash-navigation";
import { navigateToFolderAtom } from "./workspace-atoms";
import { WorkspacePage, WorkspacePageHeader, baseNameOf } from "./workspace-components";

/** How long the finished progress bar stays visible before clearing. */
const COMPLETED_OPERATION_STATUS_DURATION_MS = 900;

/** How long a type-ahead buffer survives after the last keystroke. */
const TYPE_AHEAD_TIMEOUT_MS = 800;

/**
 * Row geometry: a 22px icon cell plus 12px of vertical padding.
 *
 * The virtualizer estimates with this number and the row sets it as an inline
 * height, so the two cannot drift — a mismatch shows up as rows creeping away
 * from the scroller's position, which is the classic fixed-height-virtualization
 * bug and is invisible in code review.
 */
const TRASH_ROW_HEIGHT = 34;

/** Rows kept mounted beyond the viewport. Wide enough that the `last:` border
 *  rule on a row is never applied to a row the reader can see. */
const TRASH_OVERSCAN = 12;

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

  const requestPurge = useCallback(
    (ids: string[]) => {
      if (ids.length === 0 || isOperationPending) return;
      setPurgeRequest({ kind: "selection", ids });
    },
    [isOperationPending],
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

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((ids) =>
      ids.includes(id) ? ids.filter((candidate) => candidate !== id) : [...ids, id],
    );
  }, []);

  const allSelected = (entries?.length ?? 0) > 0 && selectedIds.length === entries?.length;
  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? [] : (entries ?? []).map((entry) => entry.id));
  };

  const totalBytes = useMemo(
    () => (entries ?? []).reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0),
    [entries],
  );
  const summary =
    entries === null
      ? t("trash.description")
      : t("trash.summary", { count: entries.length, size: formatBytes(totalBytes) });

  const clearSelectionBinding = formatBinding(useBinding("explorer.clearSelection"));

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
                  <Undo2 />
                  {t("trash.restoreSelected", { count: selectedIds.length })}
                </Button>
                <Button
                  disabled={isOperationPending}
                  onClick={() => requestPurge(selectedIds)}
                  size="sm"
                  type="button"
                  variant="destructive"
                >
                  <Trash />
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
                <Trash2 />
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
              <RotateCw />
            </Button>
          </>
        }
        description={
          <>
            {summary}
            {/* A selection is the one state where "how do I get out of this"
                is a real question — same chip the explorer puts beside its
                listing stats, and only while there is something to clear. */}
            {selectedIds.length > 0 && (
              <span className="ml-2.5 inline-flex items-center gap-1">
                <Kbd className="h-4 px-1 text-nano">{clearSelectionBinding}</Kbd>
                {t("trash.clearSelectionHint")}
              </span>
            )}
          </>
        }
        title={t("trash.title")}
      />

      {progress && <TrashProgress progress={progress} />}

      {loadError && (
        // An error banner with no way forward is a dead end; the recycle bin
        // read is cheap, so the retry is the whole recovery path.
        <Alert variant="destructive">
          <AlertTitle>{t("trash.loadErrorTitle")}</AlertTitle>
          <AlertDescription>{loadError}</AlertDescription>
          <AlertAction>
            <Button onClick={() => void reload()} size="xs" type="button" variant="outline">
              {t("loadError.retry")}
            </Button>
          </AlertAction>
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
              <Trash2 />
            </EmptyMedia>
            <EmptyTitle>{t("trash.emptyTitle")}</EmptyTitle>
            <EmptyDescription>{t("trash.emptyDescription")}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <TrashList
          allSelected={allSelected}
          entries={entries}
          isOperationPending={isOperationPending}
          onNavigateToOriginalLocation={navigateToFolder}
          onPurge={requestPurge}
          onClearSelection={() => setSelectedIds([])}
          onRestore={restoreIds}
          onSelectAll={toggleSelectAll}
          onToggleSelected={toggleSelected}
          selectedIds={selectedIds}
        />
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

/**
 * The bin's rows: a virtualized listbox with a keyboard cursor.
 *
 * Two problems, one answer. Rows used to be rendered in full and reachable by
 * pointer only, so a bin with a few thousand entries paid for every row on
 * every selection change and could not be walked at all from the keyboard.
 *
 * The list is a `role="listbox"` that holds focus itself and points at the
 * cursor row with `aria-activedescendant`. That is what makes virtualization
 * compatible with the keyboard: focus never has to move to a row, so a row
 * scrolling out of the mounted window cannot take the focus with it and leave
 * the list unresponsive. Arrow keys, Home/End, paging and type-to-jump are
 * handled here; the actions that *are* rebindable (select all, delete, clear
 * selection) go through the registry-backed hotkeys below, so a rebound key
 * keeps working and the chords the header advertises are really listening.
 */
function TrashList({
  allSelected,
  entries,
  isOperationPending,
  onClearSelection,
  onNavigateToOriginalLocation,
  onPurge,
  onRestore,
  onSelectAll,
  onToggleSelected,
  selectedIds,
}: {
  allSelected: boolean;
  entries: TrashEntry[];
  isOperationPending: boolean;
  onClearSelection: () => void;
  onNavigateToOriginalLocation: (path: string) => void;
  onPurge: (ids: string[]) => void;
  onRestore: (ids: string[]) => void;
  onSelectAll: () => void;
  onToggleSelected: (id: string) => void;
  selectedIds: string[];
}) {
  const { t } = useTranslation("workspace");
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const [activeIndex, setActiveIndex] = useState(-1);
  const typeAheadRef = useRef<{ buffer: string; timer: number | null }>({ buffer: "", timer: null });
  const purgeBinding = formatBinding(useBinding("explorer.trash"));
  const shortcuts = useAtomValue(appSettingsAtom)?.shortcuts;
  const hotkeysPaused = useAtomValue(hotkeysPausedAtom);
  const selectedIdSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  // Names for type-to-jump, kept out of the key handler so a keystroke does not
  // rebuild the array for every row.
  const names = useMemo(() => entries.map((entry) => entry.name), [entries]);

  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollerRef.current,
    estimateSize: () => TRASH_ROW_HEIGHT,
    overscan: TRASH_OVERSCAN,
  });

  // A cursor may not outlive its row: restoring or purging shortens the list
  // under the keyboard's feet, and an index past the end would leave Enter and
  // Delete with nothing to act on.
  useEffect(() => {
    setActiveIndex((index) => (index < entries.length ? index : entries.length - 1));
  }, [entries.length]);

  useEffect(
    () => () => {
      if (typeAheadRef.current.timer !== null) window.clearTimeout(typeAheadRef.current.timer);
    },
    [],
  );

  const moveCursor = useCallback(
    (index: number) => {
      if (index < 0) return;
      setActiveIndex(index);
      virtualizer.scrollToIndex(index, { align: "auto" });
    },
    [virtualizer],
  );

  const purgeCursor = useCallback(() => {
    if (isOperationPending) return;
    onPurge(trashPurgeTargets(entries, activeIndex, selectedIds));
  }, [activeIndex, entries, isOperationPending, onPurge, selectedIds]);

  // Delete acts on the selection when the cursor is inside it, exactly like the
  // explorer's trash command — a Delete press must never quietly spare the rows
  // the user can see highlighted.
  useHotkeys(
    [
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.trash")),
        callback: guardedAction(purgeCursor),
        options: { enabled: !hotkeysPaused && !isOperationPending },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.deletePermanent")),
        callback: guardedAction(purgeCursor),
        options: { enabled: !hotkeysPaused && !isOperationPending },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.selectAll")),
        callback: guardedAction(onSelectAll),
        options: { enabled: !hotkeysPaused },
      },
      {
        // The list header advertises this chord while anything is selected, so
        // the list has to answer it. `explorer.clearSelection` is registered by
        // the explorer's file list, and a tab renders exactly one surface: with
        // the bin up, nothing else was listening, and the chip was telling the
        // user about a key that did nothing. Same shape as the explorer's
        // registration, including `preventDefault: false` — Escape never
        // swallowed the key and still must not.
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.clearSelection")),
        callback: guardedAction(onClearSelection, { preventDefault: false }),
        options: { enabled: !hotkeysPaused && selectedIds.length > 0 },
      },
    ],
    HOTKEY_COMMON_OPTIONS,
  );

  const runTypeAhead = (character: string) => {
    const state = typeAheadRef.current;
    if (state.timer !== null) window.clearTimeout(state.timer);
    const buffer = state.buffer + character;
    typeAheadRef.current = {
      buffer,
      timer: window.setTimeout(() => {
        typeAheadRef.current = { buffer: "", timer: null };
      }, TYPE_AHEAD_TIMEOUT_MS),
    };

    moveCursor(typeAheadTrashIndex(names, buffer, activeIndex));
  };

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    // `isComposing` lives on the native event: React's keyboard type omits it,
    // and letting an IME composition through would type-jump on拼音 input.
    if (event.defaultPrevented || event.nativeEvent.isComposing) return;
    // Modified keys belong to the registered shortcuts, not to navigation.
    if (event.ctrlKey || event.metaKey || event.altKey) return;

    const count = entries.length;
    // One page is "as many rows as the scroller is showing", minus the row the
    // cursor is on, so paging always leaves a landmark on screen.
    const pageSize = Math.max(
      1,
      Math.floor((scrollerRef.current?.clientHeight ?? TRASH_ROW_HEIGHT) / TRASH_ROW_HEIGHT) - 1,
    );

    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp": {
        event.preventDefault();
        moveCursor(stepTrashIndex(activeIndex, count, event.key === "ArrowDown" ? 1 : -1));
        return;
      }
      case "Home":
      case "End":
      case "PageUp":
      case "PageDown": {
        event.preventDefault();
        const jump =
          event.key === "Home"
            ? "first"
            : event.key === "End"
              ? "last"
              : event.key === "PageUp"
                ? "pageUp"
                : "pageDown";
        moveCursor(jumpTrashIndex(activeIndex, count, jump, pageSize));
        return;
      }
      case "Enter": {
        if (activeIndex < 0) return;
        event.preventDefault();
        onRestore([entries[activeIndex].id]);
        return;
      }
      case " ": {
        if (activeIndex < 0) return;
        event.preventDefault();
        onToggleSelected(entries[activeIndex].id);
        return;
      }
      default: {
        if (event.key.length === 1 && !event.repeat) runTypeAhead(event.key);
      }
    }
  };

  return (
    <div className="overflow-hidden rounded-lg border border-border">
      {/* The column labels describe the whole list, so they stay outside the
          scroller: pinning them costs nothing and scrolling them away costs
          the reader a column. */}
      <div
        className={cn(
          ROW_GRID,
          "border-b border-border bg-muted/40 px-3 py-1.5 text-label text-muted-foreground uppercase",
        )}
      >
        <SelectAllToggle allSelected={allSelected} onToggle={onSelectAll} />
        <span>{t("trash.columns.name")}</span>
        <span>{t("trash.columns.originalLocation")}</span>
        <span>{t("trash.columns.deletedAt")}</span>
        <span className="text-right">{t("trash.columns.size")}</span>
      </div>
      <div
        aria-activedescendant={activeIndex >= 0 ? trashRowId(activeIndex) : undefined}
        aria-label={t("trash.listAriaLabel")}
        aria-multiselectable="true"
        className="group/list max-h-trash-list overflow-y-auto focus-visible:outline-none"
        onFocus={() => {
          // Tab into the list has to land on a row, not on nothing — but it
          // must not scroll: this handler also runs when a row click focuses
          // the list, and a scroll there would move the row out from under the
          // pointer before the click lands on it.
          setActiveIndex((index) => (index < 0 ? 0 : index));
        }}
        onKeyDown={handleKeyDown}
        ref={scrollerRef}
        role="listbox"
        tabIndex={0}
      >
        <div className="relative" role="presentation" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const entry = entries[virtualRow.index];
            const isSelected = selectedIdSet.has(entry.id);
            const isActive = virtualRow.index === activeIndex;

            return (
              <div
                // `last:` resolves against the last *mounted* row, which the
                // overscan keeps far below the viewport — so a row the reader
                // can see always keeps its separator.
                className="absolute inset-x-0 top-0 border-b last:border-b-0"
                key={entry.id}
                role="presentation"
                style={{
                  height: virtualRow.size,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <TrashRow
                  entry={entry}
                  id={trashRowId(virtualRow.index)}
                  isActive={isActive}
                  isSelected={isSelected}
                  onNavigateToOriginalLocation={() =>
                    onNavigateToOriginalLocation(entry.originalParent)
                  }
                  onPointerDown={() => scrollerRef.current?.focus({ preventScroll: true })}
                  onPurge={() => onPurge([entry.id])}
                  onRestore={() => onRestore([entry.id])}
                  onSetCursor={() => setActiveIndex(virtualRow.index)}
                  onToggleSelected={() => onToggleSelected(entry.id)}
                  purgeBinding={purgeBinding}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
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
      <div className="flex items-center justify-between gap-3 text-caption text-muted-foreground">
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
      {allSelected && <Check className="size-3 text-primary" />}
    </button>
  );
}

/** Stable DOM id for a row, so `aria-activedescendant` can point at it. */
function trashRowId(index: number): string {
  return `trash-row-${index}`;
}

function TrashRow({
  entry,
  id,
  isActive,
  isSelected,
  onNavigateToOriginalLocation,
  onPointerDown,
  onPurge,
  onRestore,
  onSetCursor,
  onToggleSelected,
  purgeBinding,
}: {
  entry: TrashEntry;
  id: string;
  isActive: boolean;
  isSelected: boolean;
  onNavigateToOriginalLocation: () => void;
  onPointerDown: () => void;
  onPurge: () => void;
  onRestore: () => void;
  onSetCursor: () => void;
  onToggleSelected: () => void;
  purgeBinding: string;
}) {
  const { t } = useTranslation("workspace");
  const presentation = entry.isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);
  // Entries without a put-back record (e.g. trashed outside Finder on macOS)
  // have no original location to show or navigate to.
  const originalLocation = entry.originalParent || t("trash.unknownOriginalLocation");
  const hasOriginalLocation = entry.originalParent.length > 0;

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        {/* The row body toggles selection; double-click restores, like Explorer.
            It is a role="option" rather than a button because the listbox owns
            focus — the row only has to be clickable and to describe itself. */}
        <div
          aria-label={entry.name}
          aria-selected={isSelected}
          className={cn(
            ROW_GRID,
            "h-full w-full px-3 text-left transition-colors hover:bg-accent/60",
            isSelected && "bg-selection",
            // The cursor is the row ring the rest of the app uses for rows, and
            // it only shows while the list itself holds keyboard focus.
            isActive &&
              "group-focus-visible/list:ring-1 group-focus-visible/list:ring-ring group-focus-visible/list:ring-inset",
          )}
          id={id}
          onClick={() => {
            onSetCursor();
            onToggleSelected();
          }}
          onContextMenu={onSetCursor}
          onDoubleClick={onRestore}
          onPointerDown={onPointerDown}
          role="option"
          title={`${entry.name} · ${originalLocation}`}
        >
          <RowCheckbox isSelected={isSelected} />
          <span className="flex min-w-0 items-center gap-2">
            <TypeIconTile
              className="size-tile-list tile-radius"
              iconSize={13}
              presentation={presentation}
            />
            <span className="truncate text-body">{entry.name}</span>
          </span>
          <span className="truncate text-caption text-muted-foreground">{originalLocation}</span>
          <span className="truncate text-caption text-muted-foreground tabular-nums">
            {formatDeletedTime(entry.timeDeleted)}
          </span>
          <span className="text-right text-caption text-muted-foreground tabular-nums">
            {entry.sizeBytes === null ? "—" : formatBytes(entry.sizeBytes)}
          </span>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={onRestore}>
            <Undo2 />
            {t("trash.restore")}
            {/* Enter is not a rebindable action — it is what double-click does
                — so this hint is a literal, like the explorer menu's. */}
            <ContextMenuShortcut>Enter</ContextMenuShortcut>
          </ContextMenuItem>
          {hasOriginalLocation && (
            <ContextMenuItem onClick={onNavigateToOriginalLocation}>
              <Folder />
              {t("trash.openOriginalLocation")}
            </ContextMenuItem>
          )}
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          {/* In the bin, delete *is* permanent, so this row carries the live
              `explorer.trash` binding — the chord the list actually answers to. */}
          <ContextMenuItem onClick={onPurge} variant="destructive">
            <Trash />
            {t("trash.deleteForever")}
            <ContextMenuShortcut>{purgeBinding}</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
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
      {isSelected && <Check className="size-3" />}
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
