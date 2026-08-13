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
import {
  ClipboardIcon,
  ColumnsIcon,
  CopyIcon,
  FilePlusIcon,
  FolderOpenIcon,
  FolderPlusIcon,
  ListIcon,
  MagnifyingGlassIcon,
  RowsIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

import {
  canDropEntries,
  getExplorerDropTargetAtPoint,
  isOverSidebarFavoritesAtPoint,
  type FileTransferOperation,
} from "./drag-drop";
import { EntryContextMenuContent } from "./entry-context-menu";
import { FileColumnView } from "./file-column-view";
import {
  DIRECTORY_PRESENTATION,
  getFilePresentation,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
} from "./file-icons";
import { FileGridView } from "./file-grid-view";
import { DENSITY_ROW_HEIGHT, densityAtom, viewModeAtom } from "./preferences";
import { SelectionToolbar } from "./selection-toolbar";
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
  onCopy: () => void;
  onCreateDirectory: () => void;
  onCreateFile: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDropEntries: (
    sourcePaths: string[],
    destinationPath: string,
    operation: FileTransferOperation,
  ) => void;
  onOpenDirectory: (path: string) => void;
  onPaste: () => void;
  onRename: () => void;
  onScrollOffsetChange?: (offset: number) => void;
  onSelectedPathsChange: (paths: string[]) => void;
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

type InternalDragTarget = { kind: "directory"; path: string } | { kind: "favorites" };

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
  if (
    draggableDirectoryPaths(entries, sourcePaths).length > 0 &&
    isOverSidebarFavoritesAtPoint(x, y)
  ) {
    return { kind: "favorites" };
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

  return left.kind === "favorites" || left.path === (right as { path: string }).path;
}

function draggableDirectoryPaths(entries: DirectoryEntry[], sourcePaths: string[]): string[] {
  const directoryPaths = new Set(
    entries.filter((entry) => entry.kind === "directory").map((entry) => entry.path),
  );

  return sourcePaths.filter((path) => directoryPaths.has(path));
}

function getEntryPresentation(entry: DirectoryEntry) {
  switch (entry.kind) {
    case "directory":
      return DIRECTORY_PRESENTATION;
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(entry.name);
  }
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
  onCopy,
  onCreateDirectory,
  onCreateFile,
  onCut,
  onDelete,
  onDropEntries,
  onOpenDirectory,
  onPaste,
  onRename,
  onScrollOffsetChange,
  onSelectedPathsChange,
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
  const [viewMode, setViewMode] = useAtom(viewModeAtom);
  const density = useAtomValue(densityAtom);
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

      if (event.key === "F2" && selectedCount === 1 && !actionsDisabled) {
        event.preventDefault();
        onRename();
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
    onCopy,
    onCut,
    onDelete,
    onPaste,
    onRename,
    onSelectedPathsChange,
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
  }, [onAddToFavorites, onDropEntries]);

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

  const addEntryToFavorites = (entry: DirectoryEntry) => {
    onAddToFavorites(
      draggableDirectoryPaths(
        entries,
        selectedPathSet.has(entry.path) ? selectedPaths : [entry.path],
      ),
    );
  };

  const draggingPaths = new Set(internalDrag?.sourcePaths ?? []);
  const internalDropTargetPath =
    internalDrag?.target?.kind === "directory" ? internalDrag.target.path : null;

  const blankMenuDisabled = actionsDisabled || Boolean(searchState);

  const viewControls = {
    actionsDisabled,
    draggingPaths,
    dropTargetPath: internalDropTargetPath ?? externalDropTargetPath,
    onAddToFavorites: addEntryToFavorites,
    onContextMenuEntry: (entry: DirectoryEntry, index = entries.indexOf(entry)) =>
      selectForContextMenu(entry, index),
    onCopy,
    onCut,
    onDelete,
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
        onContextMenu={() => {
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
        <div className="flex h-10 shrink-0 items-center gap-2 border-b px-4">
          <h1
            className="min-w-0 truncate text-sm font-medium"
            title={searchState ? `“${searchState.query}”的搜索结果` : undefined}
          >
            {searchState ? `搜索：“${searchState.query}”` : "文件"}
          </h1>
          <div className="ml-auto flex shrink-0 items-center gap-2">
            <p aria-live="polite" className="text-xs whitespace-nowrap text-muted-foreground">
              {listIsLoading
                ? searchState
                  ? "正在搜索…"
                  : "正在读取…"
                : searchState?.error
                  ? "搜索失败"
                  : `${entries.length} 个${searchState ? "匹配项" : "项目"}${searchState?.truncated ? "，结果已截断" : ""}`}
            </p>
            <ViewModeSwitcher value={activeViewMode} onChange={setViewMode} />
            <DensitySwitcher />
          </div>
        </div>

        {entries.length === 0 && !listIsLoading ? (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                {searchState ? (
                  searchState.error ? <WarningIcon /> : <MagnifyingGlassIcon />
                ) : (
                  <FolderOpenIcon />
                )}
              </EmptyMedia>
              <EmptyTitle>
                {searchState
                  ? searchState.error
                    ? "搜索未完成"
                    : "没有找到匹配项"
                  : "这个文件夹是空的"}
              </EmptyTitle>
              <EmptyDescription>
                {searchState
                  ? (searchState.error ??
                    `当前目录及子目录中没有名称包含“${searchState.query}”的文件或文件夹。`)
                  : "此位置暂时没有文件或子文件夹。"}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : activeViewMode === "grid" ? (
          <FileGridView {...viewControls} entries={entries} />
        ) : activeViewMode === "column" ? (
          <FileColumnView {...columnControls} rootEntries={entries} viewId={viewId} />
        ) : (
          <div
            ref={scrollRef}
            className="min-h-0 flex-1 overflow-auto"
            onScroll={(event) => onScrollOffsetChange?.(event.currentTarget.scrollTop)}
          >
            <div className="min-w-160">
              <div className="grid h-9 items-center whitespace-nowrap border-b text-xs font-medium text-muted-foreground [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]">
                <div className="min-w-0 px-2">名称</div>
                <div className="px-2">修改日期</div>
                <div className="px-2">类型</div>
                <div className="px-2 text-right">大小</div>
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
                        isLast={virtualRow.index === entries.length - 1}
                        isSelected={selectedPathSet.has(entry.path)}
                        isSingleSelection={selectedCount === 1}
                        onAddToFavorites={() => addEntryToFavorites(entry)}
                        onContextMenu={() => selectForContextMenu(entry, virtualRow.index)}
                        onCopy={onCopy}
                        onCut={onCut}
                        onDelete={onDelete}
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
          <div className="pointer-events-none absolute inset-3 flex items-center justify-center rounded-xl border-2 border-dashed border-primary/60 bg-accent/80 text-sm font-medium text-accent-foreground">
            松开以复制 {externalDropItemCount} 个项目
          </div>
        )}
        {internalDrag && (
          <div
            aria-hidden="true"
            className="pointer-events-none fixed z-50 flex items-center gap-2 rounded-lg border bg-popover px-3 py-2 text-sm text-popover-foreground shadow-lg"
            style={{ left: internalDrag.position.x + 14, top: internalDrag.position.y + 14 }}
          >
            {internalDrag.target?.kind === "favorites" ? (
              <>
                <StarIcon />
                添加 {draggableDirectoryPaths(entries, internalDrag.sourcePaths).length}{" "}
                个文件夹到常用位置
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
        <SelectionToolbar
          actionsDisabled={actionsDisabled}
          isSingleSelection={selectedCount === 1}
          onClear={() => onSelectedPathsChange([])}
          onCopy={onCopy}
          onCut={onCut}
          onDelete={onDelete}
          onOpenEntry={openEntry}
          onRename={onRename}
          selectedEntries={entries.filter((entry) => selectedPathSet.has(entry.path))}
        />
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
          <ContextMenuItem disabled={!hasClipboard} onClick={onPaste}>
            <ClipboardIcon />
            粘贴
            <ContextMenuShortcut>Ctrl+V</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

const VIEW_MODE_PRESENTATION = [
  { icon: ListIcon, label: "列表视图", value: "list" },
  { icon: ColumnsIcon, label: "分栏视图", value: "column" },
  { icon: SquaresFourIcon, label: "网格视图", value: "grid" },
] as const;

function ViewModeSwitcher({
  onChange,
  value,
}: {
  onChange: (value: "list" | "grid" | "column") => void;
  value: "list" | "grid" | "column";
}) {
  return (
    <div aria-label="视图模式" className="flex items-center gap-0.5 rounded-md bg-muted p-0.5" role="group">
      {VIEW_MODE_PRESENTATION.map(({ icon: ModeIcon, label, value: mode }) => (
        <button
          aria-label={label}
          aria-pressed={value === mode}
          className={cn(
            "flex size-6 items-center justify-center rounded-[5px] text-muted-foreground transition-colors duration-100 hover:text-foreground",
            value === mode && "bg-background text-foreground shadow-xs",
          )}
          onClick={() => onChange(mode)}
          title={label}
          type="button"
        >
          <ModeIcon size={14} />
        </button>
      ))}
    </div>
  );
}

const DENSITY_PRESENTATION = [
  { label: "紧凑", value: "compact" },
  { label: "舒适", value: "comfortable" },
  { label: "宽松", value: "spacious" },
] as const;

function DensitySwitcher() {
  const [density, setDensity] = useAtom(densityAtom);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label="显示密度"
        className="flex size-7 items-center justify-center rounded-[10px] text-muted-foreground transition-colors duration-100 hover:bg-muted hover:text-foreground"
        title="显示密度"
      >
        <RowsIcon size={16} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          onValueChange={(value) => setDensity(value as typeof density)}
          value={density}
        >
          {DENSITY_PRESENTATION.map((item) => (
            <DropdownMenuRadioItem key={item.value} value={item.value}>
              {item.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function FileListRow({
  densityRowHeight,
  entry,
  isActionDisabled,
  isDragging,
  isDropTarget,
  isLast,
  isSelected,
  isSingleSelection,
  onAddToFavorites,
  onContextMenu,
  onCopy,
  onCut,
  onDelete,
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
  isLast: boolean;
  isSelected: boolean;
  isSingleSelection: boolean;
  onAddToFavorites: () => void;
  onContextMenu: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
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
      <ContextMenuTrigger onContextMenu={onContextMenu}>
        <div
          aria-selected={isSelected}
          className={cn(
            "grid cursor-grab items-center whitespace-nowrap text-sm transition-colors duration-100 select-none hover:bg-accent focus-visible:bg-accent focus-visible:outline-none [grid-template-columns:minmax(0,34rem)_11rem_7rem_6rem] [justify-content:start]",
            !isLast && "border-b",
            isSelected && "bg-selection ring-1 ring-primary/25 ring-inset",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-accent ring-2 ring-primary ring-inset",
          )}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={onSelect}
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
          <div className="flex min-w-0 items-center gap-2 px-2">
            <EntryIcon
              className={cn(isDirectory ? "text-primary" : "text-muted-foreground")}
              size={18}
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
          <div className="px-2 text-muted-foreground">
            {formatModifiedAt(entry.modifiedAt)}
          </div>
          <div className="px-2 text-muted-foreground">{presentation.label}</div>
          <div
            className="px-2 text-right text-muted-foreground"
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
          onCopy={onCopy}
          onCut={onCut}
          onDelete={onDelete}
          onOpen={onOpen}
          onRename={onRename}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}

async function openFile(path: string): Promise<void> {
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
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <Skeleton className="h-4 w-12" />
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
