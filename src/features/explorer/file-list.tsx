import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtom, useAtomValue } from "jotai";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ArchiveFormat } from "@/bindings";
import {
  CaretDownIcon,
  CaretUpIcon,
  ClipboardIcon,
  CopyIcon,
  FilePlusIcon,
  FolderIcon,
  FolderPlusIcon,
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
  getExplorerDropTargetAtPoint,
  getSidebarSpaceDropTargetAtPoint,
  isOverSidebarFavoritesAtPoint,
  type FileTransferOperation,
} from "./drag-drop";
import { EntryContextMenuContent } from "./entry-context-menu";
import { FileColumnView } from "./file-column-view";
import { getEntryPresentation } from "./file-icons";
import { FileGridView } from "./file-grid-view";
import { MarqueeOverlay, useMarqueeSelection, type MarqueeRect } from "./marquee";
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
  currentDirectoryPath: string;
  entries: DirectoryEntry[];
  externalDropItemCount: number;
  externalDropTargetPath: string | null;
  hasClipboard: boolean;
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
  onDuplicate: () => void;
  onDropEntries: (
    sourcePaths: string[],
    destinationPath: string,
    operation: FileTransferOperation,
  ) => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpenDirectory: (path: string) => void;
  onOpenTerminal: () => void;
  onPaste: () => void;
  onRename: () => void;
  onScrollOffsetChange?: (offset: number) => void;
  onSelectedPathsChange: (paths: string[]) => void;
  onTogglePreview: () => void;
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

const MODIFIED_DATE_FORMATTER = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const FILE_SIZE_FORMATTER = new Intl.NumberFormat("zh-CN", {
  maximumFractionDigits: 1,
});

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
  currentDirectoryPath,
  entries,
  externalDropItemCount,
  externalDropTargetPath,
  hasClipboard,
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
  onDuplicate,
  onDropEntries,
  onExtract,
  onMoveTo,
  onOpenDirectory,
  onOpenTerminal,
  onPaste,
  onRename,
  onScrollOffsetChange,
  onSelectedPathsChange,
  onTogglePreview,
  searchState,
  selectedPaths,
  viewId,
}: FileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectionAnchorIndexRef = useRef<number | null>(null);
  const dragCandidateRef = useRef<DragCandidate | null>(null);
  const internalDragRef = useRef<InternalDragState | null>(null);
  const suppressNextClickRef = useRef(false);
  const entriesRef = useRef(entries);
  entriesRef.current = entries;
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

  useEffect(() => {
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

      if (hasModifier && !event.altKey && key === "v" && hasClipboard && !actionsDisabled) {
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
        onSelectedPathsChange(entriesRef.current.map((entry) => entry.path));
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

      if (event.key === "Delete" && selectedCount > 0 && !actionsDisabled) {
        event.preventDefault();
        onDelete();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    actionsDisabled,
    hasClipboard,
    listIsLoading,
    onCopy,
    onCut,
    onDelete,
    onOpenTerminal,
    onPaste,
    onRename,
    onSelectedPathsChange,
    onTogglePreview,
    selectedCount,
  ]);

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

      const operation: FileTransferOperation = event.ctrlKey || event.metaKey ? "copy" : "move";
      const nextTarget = resolveDragTarget(
        entriesRef.current,
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
          onDropEntries(activeDrag.sourcePaths, activeDrag.target.path, activeDrag.operation);
        } else if (activeDrag.target?.kind === "favorites") {
          onAddToFavorites(draggableDirectoryPaths(entriesRef.current, activeDrag.sourcePaths));
        } else if (activeDrag.target?.kind === "space") {
          onAddToSpace(
            activeDrag.target.spaceId,
            draggableDirectoryPaths(entriesRef.current, activeDrag.sourcePaths),
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
  }, [onAddToFavorites, onAddToSpace, onDropEntries]);

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

  const draggingPaths = new Set(internalDrag?.sourcePaths ?? []);
  const internalDropTargetPath =
    internalDrag?.target?.kind === "directory" ? internalDrag.target.path : null;

  const blankMenuDisabled = actionsDisabled || Boolean(searchState);

  // Rubber-band selection over uniform-height rows (SKILL.md §19).
  const listMarquee = useMarqueeSelection({
    enabled: !actionsDisabled && activeViewMode === "list",
    getBaseSelection: () => selectedPaths,
    hitTest: (rect: MarqueeRect) => {
      const container = scrollRef.current;
      if (!container) return [];

      const bounds = container.getBoundingClientRect();
      const topContent = rect.top - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      const bottomContent = rect.bottom - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      if (bottomContent <= 0) return [];

      const firstRow = Math.max(0, Math.floor(topContent / rowHeight));
      const lastRow = Math.min(
        entries.length - 1,
        Math.ceil(bottomContent / rowHeight) - 1,
      );
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

  const viewControls = {
    actionsDisabled,
    draggingPaths,
    dropTargetPath: internalDropTargetPath ?? externalDropTargetPath,
    onAddToFavorites: addEntryToFavorites,
    onAddToSpace: addEntryToSpace,
    onContextMenuEntry: (entry: DirectoryEntry, index = entries.indexOf(entry)) =>
      selectForContextMenu(entry, index),
    onCopy,
    onCut,
    onDelete,
    onDuplicate,
    onCompress,
    onExtract,
    onMoveTo,
    onOpenEntry: openEntry,
    onPointerDownEntry: prepareInternalDrag,
    onRename,
    onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => {
      if (suppressNextClickRef.current) {
        suppressNextClickRef.current = false;
        return;
      }
      selectEntry(entry, index, event);
    },
    onSelectedPathsChange,
    selectedCount,
    selectedPathSet,
  };

  const columnControls = {
    ...viewControls,
    onSelectEntry: (entry: DirectoryEntry, event: ReactMouseEvent) => {
      if (suppressNextClickRef.current) {
        suppressNextClickRef.current = false;
        return;
      }
      selectEntry(entry, entries.indexOf(entry), event);
    },
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
            aria-label="文件列表"
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
                  ? `搜索未完成：${searchState.error}`
                  : `未找到名称包含“${searchState.query}”的项目`
                : "此文件夹为空。"}
            </p>
          </div>
        ) : activeViewMode === "grid" ? (
          <FileGridView {...viewControls} entries={entries} />
        ) : activeViewMode === "column" ? (
          <FileColumnView {...columnControls} rootEntries={entries} viewId={viewId} />
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
              <div className="sticky top-0 z-10 grid h-7 shrink-0 items-center whitespace-nowrap border-b bg-card text-xs text-muted-foreground [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]">
                <SortHeaderCell
                  active={sortKey === "name"}
                  label="名称"
                  onSort={() => applySort("name")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "modified"}
                  label="修改日期"
                  onSort={() => applySort("modified")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "type"}
                  label="类型"
                  onSort={() => applySort("type")}
                  order={sortOrder}
                />
                <SortHeaderCell
                  active={sortKey === "size"}
                  align="right"
                  label="大小"
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

                  return (
                    <div
                      key={entry.path}
                      className="absolute inset-x-0 top-0"
                      style={{ transform: `translateY(${virtualRow.start}px)` }}
                    >
                      <FileListRow
                        densityRowHeight={rowHeight}
                        entry={entry}
                        isActionDisabled={actionsDisabled}
                        isDragging={draggingPaths.has(entry.path)}
                        isDropTarget={
                          internalDropTargetPath === entry.path ||
                          externalDropTargetPath === entry.path
                        }
                        isSelected={selectedPathSet.has(entry.path)}
                        isSingleSelection={selectedCount === 1}
                        onAddToFavorites={() => addEntryToFavorites(entry)}
                        onAddToSpace={(spaceId) => addEntryToSpace(entry, spaceId)}
                        onCompress={onCompress}
                        onContextMenu={() => selectForContextMenu(entry, virtualRow.index)}
                        onCopy={onCopy}
                        onCut={onCut}
                        onDelete={onDelete}
                        onDuplicate={onDuplicate}
                        onExtract={onExtract}
                        onMoveTo={onMoveTo}
                        onOpen={() => openEntry(entry)}
                        onPointerDown={(event) => prepareInternalDrag(entry, event)}
                        onRename={onRename}
                        onSelect={(event) => {
                          if (suppressNextClickRef.current) {
                            suppressNextClickRef.current = false;
                            return;
                          }
                          selectEntry(entry, virtualRow.index, event);
                        }}
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
            松开以复制 {externalDropItemCount} 个项目
          </div>
        )}
        <MarqueeOverlay rect={listMarquee.rect} />
        {internalDrag && (
          <div
            aria-hidden="true"
            className="pointer-events-none fixed z-50 flex items-center gap-2 rounded-full border border-foreground/5 bg-popover px-3 py-1.5 text-[13px] text-popover-foreground shadow-lg"
            style={{ left: internalDrag.position.x + 14, top: internalDrag.position.y + 14 }}
          >
            {internalDrag.target?.kind === "favorites" ? (
              <>
                <StarIcon />
                添加 {draggableDirectoryPaths(entries, internalDrag.sourcePaths).length}{" "}
                个文件夹到收藏
              </>
            ) : internalDrag.target?.kind === "space" ? (
              <>
                <SquaresFourIcon />
                添加 {draggableDirectoryPaths(entries, internalDrag.sourcePaths).length}{" "}
                个文件夹到空间
              </>
            ) : (
              <>
                {internalDrag.operation === "copy" ? <CopyIcon /> : <ScissorsIcon />}
                {internalDrag.operation === "copy" ? "复制" : "移动"}{" "}
                {internalDrag.sourcePaths.length} 个项目
              </>
            )}
          </div>
        )}
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={onCreateFile}>
            <FilePlusIcon />
            新建文件
          </ContextMenuItem>
          <ContextMenuItem onClick={onCreateDirectory}>
            <FolderPlusIcon />
            新建文件夹
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={onOpenTerminal}>
            <TerminalIcon />
            在终端中打开
            <ContextMenuShortcut>{MOD_KEY}+`</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={!hasClipboard} onClick={onPaste}>
            <ClipboardIcon />
            粘贴
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
  return (
    <div
      aria-sort={active ? (order === "asc" ? "ascending" : "descending") : "none"}
      className={cn("min-w-0", align === "right" && "text-right")}
      role="columnheader"
    >
      <button
        className={cn(
          "flex min-w-0 items-center gap-1 rounded-xs px-2.5 py-1 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          active && "text-foreground",
          align === "right" && "flex-row-reverse",
        )}
        onClick={onSort}
        title={active ? `按${label}${order === "asc" ? "升序" : "降序"}排列，点击切换` : `按${label}排序`}
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

function FileListRow({
  densityRowHeight,
  entry,
  isActionDisabled,
  isDragging,
  isDropTarget,
  isSelected,
  isSingleSelection,
  onAddToFavorites,
  onAddToSpace,
  onCompress,
  onContextMenu,
  onCopy,
  onCut,
  onDelete,
  onDuplicate,
  onExtract,
  onMoveTo,
  onOpen,
  onPointerDown,
  onRename,
  onSelect,
}: {
  densityRowHeight: number;
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  isSingleSelection: boolean;
  onAddToFavorites: () => void;
  onAddToSpace: (spaceId: string) => void;
  onCompress: (format: ArchiveFormat) => void;
  onContextMenu: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpen: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelect: (event: ReactMouseEvent) => void;
}) {
  const presentation = getEntryPresentation(entry);
  const EntryIcon = presentation.icon;
  const isDirectory = entry.kind === "directory";

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            "grid cursor-grab items-center rounded-md whitespace-nowrap text-[13px] transition-colors select-none hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]",
            isSelected && "bg-selection ring-1 ring-primary/30 ring-inset",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-selection ring-2 ring-primary ring-inset",
          )}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={onSelect}
          onContextMenu={onContextMenu}
          onDoubleClick={onOpen}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onOpen();
            }
          }}
          onPointerDown={onPointerDown}
          role="option"
          tabIndex={0}
          title={entry.path}
          style={{ height: densityRowHeight }}
        >
          <div className="flex min-w-0 items-center gap-2.5 px-3">
            <EntryIcon
              className={cn("shrink-0", isDirectory ? "text-folder" : "text-muted-foreground")}
              size={18}
              weight={isDirectory ? "fill" : undefined}
            />
            <span className="min-w-0 truncate">{entry.name}</span>
            {entry.relativePath && (
              <span
                className="ml-auto max-w-[45%] shrink-0 truncate text-xs text-muted-foreground"
                title={entry.relativePath}
              >
                {formatRelativeLocation(entry.relativePath)}
              </span>
            )}
          </div>
          <div className="px-2.5 text-muted-foreground">{formatModifiedAt(entry.modifiedAt)}</div>
          <div className="px-2.5 text-muted-foreground">{presentation.label}</div>
          <div
            className="px-2.5 text-right text-muted-foreground"
            title={entry.size === null ? undefined : `${entry.size.toLocaleString("zh-CN")} 字节`}
          >
            {formatFileSize(entry.size)}
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <EntryContextMenuContent
          entry={entry}
          isActionDisabled={isActionDisabled}
          isSingleSelection={isSingleSelection}
          onAddToFavorites={onAddToFavorites}
          onAddToSpace={onAddToSpace}
          onCompress={onCompress}
          onCopy={onCopy}
          onCut={onCut}
          onDelete={onDelete}
          onDuplicate={onDuplicate}
          onExtract={onExtract}
          onMoveTo={onMoveTo}
          onOpen={onOpen}
          onRename={onRename}
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
  return modifiedAt === null ? "—" : MODIFIED_DATE_FORMATTER.format(modifiedAt);
}

function formatFileSize(size: number | null): string {
  if (size === null) {
    return "—";
  }

  if (size === 0) {
    return "0 B";
  }

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  const value = size / 1024 ** unitIndex;

  return `${FILE_SIZE_FORMATTER.format(value)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

function formatRelativeLocation(relativePath: string): string {
  const separatorIndex = Math.max(relativePath.lastIndexOf("/"), relativePath.lastIndexOf("\\"));
  if (separatorIndex < 0) {
    return "当前目录";
  }

  return relativePath.slice(0, separatorIndex).replaceAll(/[\\/]/g, " › ");
}

export function FileListSkeleton() {
  return (
    <section aria-label="正在加载文件列表" className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 shrink-0 items-center justify-between border-b px-3">
        <Skeleton className="h-3.5 w-12" />
        <Skeleton className="h-3 w-16" />
      </div>
      <Table className="min-w-160 table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead>名称</TableHead>
            <TableHead className="w-44">修改日期</TableHead>
            <TableHead className="w-28">类型</TableHead>
            <TableHead className="w-24 text-right">大小</TableHead>
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
