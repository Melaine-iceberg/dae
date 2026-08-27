import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtom, useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openPath } from "@tauri-apps/plugin-opener";
import { commands, type ArchiveFormat } from "@/bindings";
import { i18n } from "@/i18n";
import { localeDateTimeFormat, localeNumber, localeNumberFormat } from "@/i18n/format";
import {
  CaretDownIcon,
  CaretUpIcon,
  ClipboardIcon,
  CopyIcon,
  FilePlusIcon,
  FolderIcon,
  FolderPlusIcon,
  LinkIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
  TerminalIcon,
  WarningIcon,
} from "@phosphor-icons/react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { MOD_KEY } from "@/lib/platform";
import { cn } from "@/lib/utils";

import { recordRecentItem } from "@/features/workspace/recents-atoms";

import {
  canDropEntries,
  dragOperationFromModifiers,
  dragOutModeFromModifiers,
  getExplorerDropTargetAtPoint,
  getSidebarSpaceDropTargetAtPoint,
  isLocalExplorerPath,
  isOverSidebarFavoritesAtPoint,
  type FileTransferOperation,
  type TransferOperation,
} from "./drag-drop";
import { EntryContextMenuContent } from "./entry-context-menu";
import { FileColumnView } from "./file-column-view";
import { getEntryPresentation, getPresentationIconClassName } from "./file-icons";
import { FileGridView } from "./file-grid-view";
import { getEntryGitStatus, GitStatusBadge, type ExplorerGitStatus } from "./git-status";
import { MarqueeOverlay, useMarqueeSelection, type MarqueeRect } from "./marquee";
import { isNativeIconSupported, NativeIconImage } from "./native-icon";
import {
  DEFAULT_SORT_ORDER,
  DENSITY_ROW_HEIGHT,
  densityAtom,
  sortKeyAtom,
  sortOrderAtom,
  viewModeAtom,
  type ExplorerSortKey,
} from "./preferences";
import type { DirectoryEntry } from "./types";

interface FileListProps {
  canRedo: boolean;
  canUndo: boolean;
  currentDirectoryPath: string;
  entries: DirectoryEntry[];
  externalDropItemCount: number;
  externalDropTargetPath: string | null;
  gitStatus?: ExplorerGitStatus | null;
  initialScrollOffset?: number;
  isLoading: boolean;
  isOperationPending: boolean;
  onAddToFavorites: (paths: string[]) => void;
  onAddToSpace: (spaceId: string, paths: string[]) => void;
  onCompress: (format: ArchiveFormat) => void;
  onCopy: () => void;
  onCreateDirectory: () => void;
  onCreateFile: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDeletePermanent: () => void;
  onDuplicate: () => void;
  onDropEntries: (
    sourcePaths: string[],
    destinationPath: string,
    operation: TransferOperation,
  ) => void;
  onCreateShortcuts: (sourcePaths: string[], destinationPath: string) => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpenDirectory: (path: string) => void;
  onOpenTerminal: () => void;
  onPaste: () => void;
  onRedo: () => void;
  onRename: () => void;
  onScrollOffsetChange?: (offset: number) => void;
  onSelectedPathsChange: (paths: string[]) => void;
  onTogglePreview: () => void;
  onUndo: () => void;
  searchState?: FileListSearchState;
  selectedPaths: string[];
  viewId: string;
}

interface FileListSearchState {
  error: string | null;
  isSearching: boolean;
  query: string;
  truncated: boolean;
}

const MODIFIED_DATE_FORMAT_OPTIONS: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
};

const FILE_SIZE_FORMAT_OPTIONS: Intl.NumberFormatOptions = {
  maximumFractionDigits: 1,
};

const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

const DRAG_START_DISTANCE_PX = 6;
const LIST_HEADER_HEIGHT_PX = 28;

type InternalDragTarget =
  | { kind: "directory"; path: string }
  | { kind: "favorites" }
  | { kind: "space"; spaceId: string };

type InternalDragState = {
  operation: FileTransferOperation;
  pointerId: number;
  position: { x: number; y: number };
  sourcePaths: string[];
  target: InternalDragTarget | null;
};

type DragCandidate = {
  pointerId: number;
  startX: number;
  startY: number;
  sourcePaths: string[];
};

function resolveDragTarget(
  entries: DirectoryEntry[],
  sourcePaths: string[],
  x: number,
  y: number,
): InternalDragTarget | null {
  const draggableDirectories = draggableDirectoryPaths(entries, sourcePaths);

  if (draggableDirectories.length > 0 && isOverSidebarFavoritesAtPoint(x, y)) {
    return { kind: "favorites" };
  }

  const spaceId = getSidebarSpaceDropTargetAtPoint(x, y);
  if (draggableDirectories.length > 0 && spaceId !== null) {
    return { kind: "space", spaceId };
  }

  const targetPath = getExplorerDropTargetAtPoint(x, y);
  if (targetPath && canDropEntries(sourcePaths, targetPath)) {
    return { kind: "directory", path: targetPath };
  }

  return null;
}

function targetsAreEqual(
  left: InternalDragTarget | null,
  right: InternalDragTarget | null,
): boolean {
  if (left === right) return true;
  if (!left || !right || left.kind !== right.kind) return false;

  if (left.kind === "favorites") return true;
  if (left.kind === "space") return left.spaceId === (right as { spaceId: string }).spaceId;

  return left.path === (right as { path: string }).path;
}

function draggableDirectoryPaths(entries: DirectoryEntry[], sourcePaths: string[]): string[] {
  const directoryPaths = new Set(
    entries.filter((entry) => entry.kind === "directory").map((entry) => entry.path),
  );

  return sourcePaths.filter((path) => directoryPaths.has(path));
}

export function FileList({
  canRedo,
  canUndo,
  currentDirectoryPath,
  entries,
  externalDropItemCount,
  externalDropTargetPath,
  gitStatus,
  initialScrollOffset = 0,
  isLoading,
  isOperationPending,
  onAddToFavorites,
  onAddToSpace,
  onCompress,
  onCopy,
  onCreateDirectory,
  onCreateFile,
  onCut,
  onDelete,
  onDeletePermanent,
  onDuplicate,
  onDropEntries,
  onCreateShortcuts,
  onExtract,
  onMoveTo,
  onOpenDirectory,
  onOpenTerminal,
  onPaste,
  onRedo,
  onRename,
  onScrollOffsetChange,
  onSelectedPathsChange,
  onTogglePreview,
  onUndo,
  searchState,
  selectedPaths,
  viewId,
}: FileListProps) {
  const { t } = useTranslation("explorer");
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectionAnchorIndexRef = useRef<number | null>(null);
  const dragCandidateRef = useRef<DragCandidate | null>(null);
  const internalDragRef = useRef<InternalDragState | null>(null);
  const suppressNextClickRef = useRef(false);
  const [internalDrag, setInternalDrag] = useState<InternalDragState | null>(null);
  const viewMode = useAtomValue(viewModeAtom);
  const density = useAtomValue(densityAtom);
  const [sortKey, setSortKey] = useAtom(sortKeyAtom);
  const [sortOrder, setSortOrder] = useAtom(sortOrderAtom);
  const rowHeight = DENSITY_ROW_HEIGHT[density];
  const selectedPathSet = new Set(selectedPaths);
  const listIsLoading = isLoading || searchState?.isSearching === true;
  const actionsDisabled = listIsLoading || isOperationPending;
  const selectedCount = selectedPaths.length;
  const activeViewMode = viewMode === "column" && searchState ? "list" : viewMode;
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    initialOffset: initialScrollOffset,
    overscan: 10,
  });

  useEffect(() => {
    selectionAnchorIndexRef.current = null;
    if (scrollRef.current) {
      scrollRef.current.scrollTop = initialScrollOffset;
    }
  }, [initialScrollOffset, viewId]);

  const handleKeyDown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing || isEditableElement(event.target)) return;

    if (event.key === "Escape" && selectedCount > 0) {
      onSelectedPathsChange([]);
      return;
    }

    const hasModifier = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();

    if (hasModifier && !event.altKey && key === "c" && selectedCount > 0 && !actionsDisabled) {
      event.preventDefault();
      onCopy();
      return;
    }

    if (hasModifier && !event.altKey && key === "x" && selectedCount > 0 && !actionsDisabled) {
      event.preventDefault();
      onCut();
      return;
    }

    if (hasModifier && !event.altKey && key === "v" && !actionsDisabled) {
      event.preventDefault();
      onPaste();
      return;
    }

    // Ctrl/Cmd+` opens the system terminal in the current directory.
    if (hasModifier && !event.altKey && key === "`") {
      event.preventDefault();
      onOpenTerminal();
      return;
    }

    if (hasModifier && !event.altKey && key === "a" && !listIsLoading) {
      event.preventDefault();
      onSelectedPathsChange(entries.map((entry) => entry.path));
      return;
    }

    if (event.key === "F2" && selectedCount === 1 && !actionsDisabled) {
      event.preventDefault();
      onRename();
      return;
    }

    // Space toggles the preview surface for the selection (SKILL.md §30).
    if (event.key === " " && selectedCount > 0 && !actionsDisabled) {
      event.preventDefault();
      onTogglePreview();
      return;
    }

    // Ctrl/Cmd+Z undoes the most recent file operation; Ctrl+Shift+Z and
    // Ctrl/Cmd+Y redo the most recently undone one.
    if (
      hasModifier &&
      !event.altKey &&
      key === "z" &&
      !event.shiftKey &&
      canUndo &&
      !actionsDisabled
    ) {
      event.preventDefault();
      onUndo();
      return;
    }

    if (
      hasModifier &&
      !event.altKey &&
      ((key === "z" && event.shiftKey) || key === "y") &&
      canRedo &&
      !actionsDisabled
    ) {
      event.preventDefault();
      onRedo();
      return;
    }

    if (event.key === "Delete" && selectedCount > 0 && !actionsDisabled) {
      event.preventDefault();
      // Plain Delete moves to the trash (undoable); Shift+Delete is permanent.
      if (event.shiftKey) {
        onDeletePermanent();
      } else {
        onDelete();
      }
    }
  };

  // The Compiler keeps `handleKeyDown` referentially stable across renders,
  // so this subscribes once unless a callback it reads actually changes.
  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  useEffect(() => {
    const updateDrag = (nextDrag: InternalDragState | null) => {
      internalDragRef.current = nextDrag;
      setInternalDrag(nextDrag);
    };

    const stopDragging = () => {
      dragCandidateRef.current = null;
      updateDrag(null);
    };

    const handlePointerMove = (event: PointerEvent) => {
      const candidate = dragCandidateRef.current;
      if (!candidate || candidate.pointerId !== event.pointerId) return;

      const distanceX = event.clientX - candidate.startX;
      const distanceY = event.clientY - candidate.startY;
      const isDragging = internalDragRef.current !== null;
      if (!isDragging && Math.hypot(distanceX, distanceY) < DRAG_START_DISTANCE_PX) return;

      // Chromium keeps delivering pointermove while the button is held, even
      // outside the webview, so an out-of-bounds position means the user is
      // dragging towards another app. Hand the gesture to a native OS drag
      // (OLE DoDragDrop); dropping back onto our own window still works via
      // the external-drop path.
      if (
        isDragging &&
        (event.clientX < 0 ||
          event.clientY < 0 ||
          event.clientX >= window.innerWidth ||
          event.clientY >= window.innerHeight)
      ) {
        const dragPaths = internalDragRef.current?.sourcePaths ?? [];
        suppressNextClickRef.current = true;
        stopDragging();

        const localPaths = dragPaths.filter(isLocalExplorerPath);
        if (localPaths.length > 0) {
          // Windows conventions: plain/Ctrl copies out, Shift moves,
          // Alt (or Ctrl+Shift) creates shortcuts at the drop target.
          const dragOutMode = dragOutModeFromModifiers(event);
          void commands.startDragOut(localPaths, dragOutMode).catch((error) => {
            console.warn("Unable to start the native drag-out", error);
          });
        }
        return;
      }

      // Windows conventions inside the window: Alt (or Ctrl+Shift) links,
      // Ctrl copies, plain/Shift moves (Explorer's same-volume default).
      const operation: FileTransferOperation = dragOperationFromModifiers(event);
      const nextTarget = resolveDragTarget(
        entries,
        candidate.sourcePaths,
        event.clientX,
        event.clientY,
      );
      const previousDrag = internalDragRef.current;

      if (
        previousDrag &&
        previousDrag.position.x === event.clientX &&
        previousDrag.position.y === event.clientY &&
        previousDrag.operation === operation &&
        targetsAreEqual(previousDrag.target, nextTarget)
      ) {
        return;
      }

      event.preventDefault();
      updateDrag({
        operation,
        pointerId: candidate.pointerId,
        position: { x: event.clientX, y: event.clientY },
        sourcePaths: candidate.sourcePaths,
        target: nextTarget,
      });
    };

    const handlePointerUp = (event: PointerEvent) => {
      const candidate = dragCandidateRef.current;
      const activeDrag = internalDragRef.current;
      if (!candidate || candidate.pointerId !== event.pointerId) return;

      if (activeDrag) {
        suppressNextClickRef.current = true;
        if (activeDrag.target?.kind === "directory") {
          if (activeDrag.operation === "link") {
            onCreateShortcuts(activeDrag.sourcePaths, activeDrag.target.path);
          } else {
            onDropEntries(activeDrag.sourcePaths, activeDrag.target.path, activeDrag.operation);
          }
        } else if (activeDrag.target?.kind === "favorites") {
          onAddToFavorites(draggableDirectoryPaths(entries, activeDrag.sourcePaths));
        } else if (activeDrag.target?.kind === "space") {
          onAddToSpace(
            activeDrag.target.spaceId,
            draggableDirectoryPaths(entries, activeDrag.sourcePaths),
          );
        }
      }

      stopDragging();
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", stopDragging);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", stopDragging);
    };
  }, [entries, onAddToFavorites, onAddToSpace, onCreateShortcuts, onDropEntries]);

  const selectEntry = (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => {
    if (actionsDisabled) return;

    const isToggleSelection = event.ctrlKey || event.metaKey;
    const anchorIndex = selectionAnchorIndexRef.current;

    if (event.shiftKey && anchorIndex !== null && index >= 0) {
      const [start, end] = [anchorIndex, index].sort((left, right) => left - right);
      const range = entries.slice(start, end + 1).map((item) => item.path);
      const nextSelection = isToggleSelection
        ? new Set([...selectedPaths, ...range])
        : new Set(range);
      onSelectedPathsChange([...nextSelection]);
      return;
    }

    if (index >= 0) {
      selectionAnchorIndexRef.current = index;
    }

    if (isToggleSelection) {
      const nextSelection = new Set(selectedPaths);
      if (nextSelection.has(entry.path)) {
        nextSelection.delete(entry.path);
      } else {
        nextSelection.add(entry.path);
      }
      onSelectedPathsChange([...nextSelection]);
      return;
    }

    onSelectedPathsChange([entry.path]);
  };

  const selectForContextMenu = (entry: DirectoryEntry, index: number) => {
    if (actionsDisabled || selectedPathSet.has(entry.path)) return;

    if (index >= 0) {
      selectionAnchorIndexRef.current = index;
    }

    onSelectedPathsChange([entry.path]);
  };

  const prepareInternalDrag = (entry: DirectoryEntry, event: ReactPointerEvent) => {
    if (actionsDisabled || event.button !== 0 || event.shiftKey) {
      return;
    }

    const sourcePaths = selectedPathSet.has(entry.path) ? selectedPaths : [entry.path];
    if (!selectedPathSet.has(entry.path) && !event.ctrlKey && !event.metaKey) {
      onSelectedPathsChange(sourcePaths);
    }

    dragCandidateRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      sourcePaths,
    };
  };

  const openEntry = (entry: DirectoryEntry) => {
    if (actionsDisabled) return;

    if (entry.kind === "directory") {
      onOpenDirectory(entry.path);
      return;
    }

    void openFile(entry.path);
  };

  /**
   * Column view child panes render entries fetched on their own, so the
   * dragged entry may not be part of the root `entries`. Fall back to the
   * entry's own kind for those instead of silently dropping them.
   */
  const directoryPathsForEntry = (entry: DirectoryEntry): string[] => {
    const sourcePaths = selectedPathSet.has(entry.path) ? selectedPaths : [entry.path];
    const knownDirectories = new Set(
      entries.filter((item) => item.kind === "directory").map((item) => item.path),
    );

    return sourcePaths.filter((path) =>
      path === entry.path ? entry.kind === "directory" : knownDirectories.has(path),
    );
  };

  const addEntryToFavorites = (entry: DirectoryEntry) => {
    onAddToFavorites(directoryPathsForEntry(entry));
  };

  const addEntryToSpace = (entry: DirectoryEntry, spaceId: string) => {
    onAddToSpace(spaceId, directoryPathsForEntry(entry));
  };

  /** Swallows the click that ends an internal drag before it changes selection. */
  const selectEntryIfNotDragging = (
    entry: DirectoryEntry,
    index: number,
    event: ReactMouseEvent,
  ) => {
    if (suppressNextClickRef.current) {
      suppressNextClickRef.current = false;
      return;
    }
    selectEntry(entry, index, event);
  };

  const draggingPaths = new Set(internalDrag?.sourcePaths ?? []);
  const internalDropTargetPath =
    internalDrag?.target?.kind === "directory" ? internalDrag.target.path : null;

  // Bulk menu action callbacks shared by every row (SKILL.md §10).
  const menuActions = {
    onCompress,
    onCopy,
    onCut,
    onDelete,
    onDuplicate,
    onExtract,
    onMoveTo,
    onRename,
  };

  const blankMenuDisabled = actionsDisabled || Boolean(searchState);

  // Rubber-band selection over uniform-height rows (SKILL.md §19).
  const listMarquee = useMarqueeSelection({
    enabled: !actionsDisabled && activeViewMode === "list",
    getBaseSelection: () => selectedPaths,
    hitTest: (rect: MarqueeRect) => {
      const container = scrollRef.current;
      if (!container) return [];

      // Windows behavior: rows end at the last column's edge, so the
      // marquee must cross that content box horizontally too — a band
      // drawn over the blank area right of the columns selects nothing.
      const rowElement = container.querySelector<HTMLElement>('[role="option"]');
      if (!rowElement) return [];
      const rowBounds = rowElement.getBoundingClientRect();
      if (rect.right <= rowBounds.left || rect.left >= rowBounds.right) return [];

      const bounds = container.getBoundingClientRect();
      const topContent = rect.top - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      const bottomContent = rect.bottom - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      if (bottomContent <= 0) return [];

      const firstRow = Math.max(0, Math.floor(topContent / rowHeight));
      const lastRow = Math.min(entries.length - 1, Math.ceil(bottomContent / rowHeight) - 1);
      if (lastRow < firstRow) return [];

      return entries.slice(firstRow, lastRow + 1).map((entry) => entry.path);
    },
    onSelectionChange: onSelectedPathsChange,
    scrollElementRef: scrollRef,
  });

  /** Clicking the active column toggles direction; a new column starts at its
   * default direction (SKILL.md §18). */
  const applySort = (key: ExplorerSortKey) => {
    if (key === sortKey) {
      setSortOrder(sortOrder === "asc" ? "desc" : "asc");
      return;
    }

    setSortKey(key);
    setSortOrder(DEFAULT_SORT_ORDER[key]);
  };

  // Shared control bundle for grid and column views; the Compiler memoizes
  // both the object and every child prop automatically.
  const viewControls = {
    actionsDisabled,
    draggingPaths,
    dropTargetPath: internalDropTargetPath ?? externalDropTargetPath,
    gitStatus,
    menuActions,
    onAddToFavorites: addEntryToFavorites,
    onAddToSpace: addEntryToSpace,
    onContextMenuEntry: selectForContextMenu,
    onOpenEntry: openEntry,
    onPointerDownEntry: prepareInternalDrag,
    onSelectEntry: selectEntryIfNotDragging,
    onSelectedPathsChange,
    selectedCount,
    selectedPathSet,
  };

  return (
    <ContextMenu disabled={blankMenuDisabled}>
      <ContextMenuTrigger
        onContextMenu={(event) => {
          // Entries own their context menu and set the selection themselves;
          // only a right-click on blank space should clear it.
          if (
            event.target instanceof Element &&
            event.target.closest('[role="option"], [role="columnheader"]')
          ) {
            return;
          }
          selectionAnchorIndexRef.current = null;
          onSelectedPathsChange([]);
        }}
        render={
          <section
            aria-label={t("explorer:list.ariaLabel")}
            className="relative flex min-h-0 flex-1 flex-col"
            data-explorer-drop-target={currentDirectoryPath}
          />
        }
      >
        {entries.length === 0 && !listIsLoading ? (
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center select-none">
            {searchState?.error ? (
              <WarningIcon className="size-5 text-muted-foreground" />
            ) : (
              <FolderIcon className="size-7 text-folder" weight="fill" />
            )}
            <p className="text-[13px] text-muted-foreground">
              {searchState
                ? searchState.error
                  ? t("explorer:list.searchError", { error: searchState.error })
                  : t("explorer:list.searchEmpty", { query: searchState.query })
                : t("explorer:list.emptyFolder")}
            </p>
          </div>
        ) : activeViewMode === "grid" ? (
          <FileGridView {...viewControls} entries={entries} />
        ) : activeViewMode === "column" ? (
          <FileColumnView {...viewControls} rootEntries={entries} viewId={viewId} />
        ) : (
          <div
            ref={scrollRef}
            className="min-h-0 flex-1 overflow-auto"
            onPointerDown={(event) => {
              if (
                event.target instanceof Element &&
                event.target.closest('[role="option"], [role="columnheader"]')
              ) {
                return;
              }
              listMarquee.beginMarquee(event);
            }}
            onScroll={(event) => onScrollOffsetChange?.(event.currentTarget.scrollTop)}
          >
            <div className="min-w-160">
              <div className="sticky top-0 z-10 grid h-7 shrink-0 items-center whitespace-nowrap border-b border-border/60 bg-card text-label uppercase text-muted-foreground [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]">
                <SortHeaderCell
                  active={sortKey === "name"}
                  label={t("explorer:columns.name")}
                  onSort={() => applySort("name")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "modified"}
                  label={t("explorer:columns.modified")}
                  onSort={() => applySort("modified")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "type"}
                  label={t("explorer:columns.type")}
                  onSort={() => applySort("type")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "size"}
                  align="right"
                  label={t("explorer:columns.size")}
                  onSort={() => applySort("size")}
                  order={sortOrder}
                />
              </div>
              <div
                aria-multiselectable="true"
                className="relative"
                role="listbox"
                style={{ height: virtualizer.getTotalSize() }}
              >
                {virtualizer.getVirtualItems().map((virtualRow) => {
                  const entry = entries[virtualRow.index];

                  // Windows detail-view geometry: rows stop at the size
                  // column's right edge (34+11+7+6 rem = 58rem) instead of
                  // stretching across the window, so the area right of the
                  // columns stays blank background for clicks and marquee
                  // starts.
                  return (
                    <div
                      key={entry.path}
                      className="absolute left-0 top-0 w-full max-w-232"
                      style={{ transform: `translateY(${virtualRow.start}px)` }}
                    >
                      <FileListRow
                        densityRowHeight={rowHeight}
                        entry={entry}
                        gitStatus={gitStatus}
                        index={virtualRow.index}
                        isActionDisabled={actionsDisabled}
                        isDragging={draggingPaths.has(entry.path)}
                        isDropTarget={
                          internalDropTargetPath === entry.path ||
                          externalDropTargetPath === entry.path
                        }
                        isSelected={selectedPathSet.has(entry.path)}
                        menuActions={menuActions}
                        onAddEntryToFavorites={addEntryToFavorites}
                        onAddEntryToSpace={addEntryToSpace}
                        onContextMenuEntry={selectForContextMenu}
                        onOpenEntry={openEntry}
                        onPointerDownEntry={prepareInternalDrag}
                        onSelectEntry={selectEntryIfNotDragging}
                        selectedCount={selectedCount}
                      />
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        )}
        {externalDropItemCount > 0 && (
          <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-lg border-2 border-dashed border-primary/50 bg-primary/5 text-[13px] font-medium text-primary">
            {t("explorer:drag.dropToCopy", { count: externalDropItemCount })}
          </div>
        )}
        <MarqueeOverlay rect={listMarquee.rect} />
        {internalDrag && (
          <div
            aria-hidden="true"
            className="pointer-events-none fixed z-50 flex items-center gap-2 rounded-md bg-popover px-3 py-1.5 text-[13px] text-popover-foreground shadow-ambient ring-1 ring-border"
            style={{ left: internalDrag.position.x + 14, top: internalDrag.position.y + 14 }}
          >
            {internalDrag.target?.kind === "favorites" ? (
              <>
                <StarIcon />
                {t("explorer:drag.addToFavorites", {
                  count: draggableDirectoryPaths(entries, internalDrag.sourcePaths).length,
                })}
              </>
            ) : internalDrag.target?.kind === "space" ? (
              <>
                <SquaresFourIcon />
                {t("explorer:drag.addToSpace", {
                  count: draggableDirectoryPaths(entries, internalDrag.sourcePaths).length,
                })}
              </>
            ) : (
              <>
                {internalDrag.operation === "copy" ? (
                  <CopyIcon />
                ) : internalDrag.operation === "link" ? (
                  <LinkIcon />
                ) : (
                  <ScissorsIcon />
                )}
                {internalDrag.operation === "copy"
                  ? t("explorer:drag.opCopy")
                  : internalDrag.operation === "link"
                    ? t("explorer:drag.opLink")
                    : t("explorer:drag.opMove")}{" "}
                {t("explorer:drag.items", { count: internalDrag.sourcePaths.length })}
              </>
            )}
          </div>
        )}
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={onCreateFile}>
            <FilePlusIcon />
            {t("explorer:contextMenu.newFile")}
          </ContextMenuItem>
          <ContextMenuItem onClick={onCreateDirectory}>
            <FolderPlusIcon />
            {t("explorer:contextMenu.newFolder")}
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={onOpenTerminal}>
            <TerminalIcon />
            {t("explorer:contextMenu.openInTerminal")}
            <ContextMenuShortcut>{MOD_KEY}+`</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem onClick={onPaste}>
            <ClipboardIcon />
            {t("explorer:contextMenu.paste")}
            <ContextMenuShortcut>{MOD_KEY}+V</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function SortHeaderCell({
  active,
  align = "left",
  label,
  onSort,
  order,
}: {
  active: boolean;
  align?: "left" | "right";
  label: string;
  onSort: () => void;
  order: "asc" | "desc";
}) {
  const { t } = useTranslation("explorer");

  return (
    <div
      aria-sort={active ? (order === "asc" ? "ascending" : "descending") : "none"}
      className={cn("min-w-0", align === "right" && "text-right")}
      role="columnheader"
    >
      <button
        className={cn(
          "state-layer flex min-w-0 items-center gap-1 rounded-xs px-2.5 py-1 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          active && "text-foreground",
          align === "right" && "flex-row-reverse",
        )}
        onClick={onSort}
        title={
          active
            ? t("explorer:sort.activeTitle", {
                column: label,
                direction:
                  order === "asc" ? t("explorer:sort.ascending") : t("explorer:sort.descending"),
              })
            : t("explorer:sort.inactiveTitle", { column: label })
        }
        type="button"
      >
        <span className="truncate">{label}</span>
        {active &&
          (order === "asc" ? (
            <CaretUpIcon className="size-3 shrink-0" />
          ) : (
            <CaretDownIcon className="size-3 shrink-0" />
          ))}
      </button>
    </div>
  );
}

/** Bulk action callbacks shared by every row's context menu. */
export interface MenuActions {
  onCompress: (format: ArchiveFormat) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onRename: () => void;
}

/**
 * List row. The React Compiler memoizes this component's render output, so
 * scroll, selection, and drag churn only re-renders rows whose props
 * actually changed.
 */
function FileListRow({
  densityRowHeight,
  entry,
  gitStatus,
  index,
  isActionDisabled,
  isDragging,
  isDropTarget,
  isSelected,
  menuActions,
  onAddEntryToFavorites,
  onAddEntryToSpace,
  onContextMenuEntry,
  onOpenEntry,
  onPointerDownEntry,
  onSelectEntry,
  selectedCount,
}: {
  densityRowHeight: number;
  entry: DirectoryEntry;
  gitStatus?: ExplorerGitStatus | null;
  index: number;
  isActionDisabled: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  menuActions: MenuActions;
  onAddEntryToFavorites: (entry: DirectoryEntry) => void;
  onAddEntryToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
}) {
  const { t } = useTranslation("explorer");
  const presentation = getEntryPresentation(entry);
  const EntryIcon = presentation.icon;
  const isDirectory = entry.kind === "directory";
  const entryStatus = getEntryGitStatus(gitStatus, entry);
  const displaySize = isDirectory ? null : entry.size;
  const handleSelect = (event: ReactMouseEvent) => onSelectEntry(entry, index, event);

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // Desktop row: tonal hover via state-layer, flat selection fill,
            // constant corner radius — no pill morph.
            "state-layer grid cursor-grab items-center rounded-xs whitespace-nowrap transition-[background-color,box-shadow,opacity] duration-fast ease-standard select-none focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring/60 focus-visible:ring-inset [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]",
            isSelected && "bg-selection",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-selection ring-2 ring-primary ring-inset",
          )}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={handleSelect}
          onContextMenu={() => onContextMenuEntry(entry, index)}
          onDoubleClick={() => onOpenEntry(entry)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onOpenEntry(entry);
            }
          }}
          onPointerDown={(event) => onPointerDownEntry(entry, event)}
          role="option"
          tabIndex={0}
          title={entry.path}
          style={{ height: densityRowHeight }}
        >
          <div className="flex min-w-0 items-center gap-2.5 px-3">
            {isNativeIconSupported(entry) ? (
              <NativeIconImage
                className="shrink-0"
                entry={entry}
                fallback={
                  <EntryIcon className={getPresentationIconClassName(presentation)} size={18} />
                }
                pixelSize={18}
              />
            ) : (
              <EntryIcon
                className={cn("shrink-0", getPresentationIconClassName(presentation))}
                size={18}
                weight={isDirectory ? "fill" : undefined}
              />
            )}
            <span
              className={cn(
                "min-w-0 truncate text-sm",
                // Expressive type scale: the name carries the row's weight and
                // steps up to semibold while selected.
                isSelected ? "font-semibold" : "font-medium",
              )}
            >
              {entry.name}
            </span>
            {entryStatus && <GitStatusBadge kind={entryStatus} />}
            {entry.relativePath && (
              <span
                className="ml-auto max-w-[45%] shrink-0 truncate text-xs text-muted-foreground"
                title={entry.relativePath}
              >
                {formatRelativeLocation(entry.relativePath)}
              </span>
            )}
          </div>
          <div className="px-2.5 text-xs text-muted-foreground">
            {formatModifiedAt(entry.modifiedAt)}
          </div>
          <div className="px-2.5 text-xs text-muted-foreground">{presentation.label}</div>
          <div
            className="px-2.5 text-right text-xs text-muted-foreground"
            title={
              displaySize === null
                ? undefined
                : t("explorer:list.bytesTitle", { size: localeNumber(displaySize) })
            }
          >
            {formatFileSize(displaySize)}
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <EntryContextMenuContent
          entry={entry}
          isActionDisabled={isActionDisabled}
          isSingleSelection={selectedCount === 1}
          onAddToFavorites={() => onAddEntryToFavorites(entry)}
          onAddToSpace={(spaceId) => onAddEntryToSpace(entry, spaceId)}
          onCompress={menuActions.onCompress}
          onCopy={menuActions.onCopy}
          onCut={menuActions.onCut}
          onDelete={menuActions.onDelete}
          onDuplicate={menuActions.onDuplicate}
          onExtract={menuActions.onExtract}
          onMoveTo={menuActions.onMoveTo}
          onOpen={() => onOpenEntry(entry)}
          onRename={menuActions.onRename}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}

async function openFile(path: string): Promise<void> {
  recordRecentItem(path, "file", "opened");

  try {
    await openPath(path);
  } catch (error) {
    console.warn(`Unable to open ${path}`, error);
  }
}

function isEditableElement(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      target.tagName === "INPUT" ||
      target.tagName === "TEXTAREA" ||
      target.tagName === "SELECT")
  );
}

function formatModifiedAt(modifiedAt: number | null): string {
  return modifiedAt === null
    ? "—"
    : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(modifiedAt);
}

function formatFileSize(size: number | null): string {
  if (size === null) {
    return "";
  }

  if (size === 0) {
    return "0 B";
  }

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  const value = size / 1024 ** unitIndex;

  return `${localeNumberFormat(FILE_SIZE_FORMAT_OPTIONS).format(value)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

function formatRelativeLocation(relativePath: string): string {
  const separatorIndex = Math.max(relativePath.lastIndexOf("/"), relativePath.lastIndexOf("\\"));
  if (separatorIndex < 0) {
    return i18n.t("explorer:list.currentFolder");
  }

  return relativePath.slice(0, separatorIndex).replaceAll(/[\\/]/g, " › ");
}

export function FileListSkeleton() {
  const { t } = useTranslation("explorer");

  return (
    <section
      aria-label={t("explorer:list.loadingAriaLabel")}
      className="flex min-h-0 flex-1 flex-col"
    >
      <div className="flex h-7 shrink-0 items-center justify-between border-b px-3">
        <Skeleton className="h-3.5 w-12" />
        <Skeleton className="h-3 w-16" />
      </div>
      <Table className="min-w-160 table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead>{t("explorer:columns.name")}</TableHead>
            <TableHead className="w-44">{t("explorer:columns.modified")}</TableHead>
            <TableHead className="w-28">{t("explorer:columns.type")}</TableHead>
            <TableHead className="w-24 text-right">{t("explorer:columns.size")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {Array.from({ length: 8 }, (_, index) => (
            <TableRow key={index}>
              <TableCell>
                <div className="flex h-8 items-center gap-2 px-2">
                  <Skeleton className="size-4" />
                  <Skeleton className={index % 3 === 0 ? "h-4 w-48" : "h-4 w-32"} />
                </div>
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-30" />
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-14" />
              </TableCell>
              <TableCell>
                <Skeleton className="ml-auto h-4 w-12" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}
