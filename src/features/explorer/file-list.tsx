import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ClipboardCopyIcon,
  ClipboardPasteIcon,
  CopyIcon,
  FileIcon,
  FolderIcon,
  FolderOpenIcon,
  LinkIcon,
  PencilIcon,
  SearchXIcon,
  ScissorsIcon,
  ShapesIcon,
  Trash2Icon,
  TriangleAlertIcon,
  type LucideIcon,
} from "lucide-react";

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
  type FileTransferOperation,
} from "./drag-drop";
import type { DirectoryEntry, EntryKind } from "./types";

interface FileListProps {
  currentDirectoryPath: string;
  entries: DirectoryEntry[];
  externalDropItemCount: number;
  externalDropTargetPath: string | null;
  hasClipboard: boolean;
  initialScrollOffset?: number;
  isLoading: boolean;
  isOperationPending: boolean;
  onCopy: () => void;
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

interface EntryPresentation {
  icon: LucideIcon;
  label: string;
}

const ENTRY_PRESENTATION: Record<EntryKind, EntryPresentation> = {
  directory: { icon: FolderIcon, label: "文件夹" },
  file: { icon: FileIcon, label: "文件" },
  symlink: { icon: LinkIcon, label: "符号链接" },
  other: { icon: ShapesIcon, label: "其他" },
};

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

const ROW_HEIGHT = 48;
const DRAG_START_DISTANCE_PX = 6;

type InternalDragState = {
  operation: FileTransferOperation;
  pointerId: number;
  position: { x: number; y: number };
  sourcePaths: string[];
  targetPath: string | null;
};

type DragCandidate = {
  pointerId: number;
  startX: number;
  startY: number;
  sourcePaths: string[];
};

export function FileList({
  currentDirectoryPath,
  entries,
  externalDropItemCount,
  externalDropTargetPath,
  hasClipboard,
  initialScrollOffset = 0,
  isLoading,
  isOperationPending,
  onCopy,
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
  const [internalDrag, setInternalDrag] = useState<InternalDragState | null>(null);
  const selectedPathSet = new Set(selectedPaths);
  const listIsLoading = isLoading || searchState?.isSearching === true;
  const actionsDisabled = listIsLoading || isOperationPending;
  const selectedCount = selectedPaths.length;
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
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
  }, [actionsDisabled, hasClipboard, onCopy, onCut, onDelete, onPaste, onRename, selectedCount]);

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
      const targetPath = getExplorerDropTargetAtPoint(event.clientX, event.clientY);
      const nextTargetPath =
        targetPath && canDropEntries(candidate.sourcePaths, targetPath) ? targetPath : null;
      const previousDrag = internalDragRef.current;

      if (
        previousDrag &&
        previousDrag.position.x === event.clientX &&
        previousDrag.position.y === event.clientY &&
        previousDrag.operation === operation &&
        previousDrag.targetPath === nextTargetPath
      ) {
        return;
      }

      event.preventDefault();
      updateDrag({
        operation,
        pointerId: candidate.pointerId,
        position: { x: event.clientX, y: event.clientY },
        sourcePaths: candidate.sourcePaths,
        targetPath: nextTargetPath,
      });
    };

    const handlePointerUp = (event: PointerEvent) => {
      const candidate = dragCandidateRef.current;
      const activeDrag = internalDragRef.current;
      if (!candidate || candidate.pointerId !== event.pointerId) return;

      if (activeDrag) {
        suppressNextClickRef.current = true;
        if (activeDrag.targetPath) {
          onDropEntries(activeDrag.sourcePaths, activeDrag.targetPath, activeDrag.operation);
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
  }, [onDropEntries]);

  const selectEntry = (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => {
    if (actionsDisabled) return;

    const isToggleSelection = event.ctrlKey || event.metaKey;
    const anchorIndex = selectionAnchorIndexRef.current;

    if (event.shiftKey && anchorIndex !== null) {
      const [start, end] = [anchorIndex, index].sort((left, right) => left - right);
      const range = entries.slice(start, end + 1).map((item) => item.path);
      const nextSelection = isToggleSelection
        ? new Set([...selectedPaths, ...range])
        : new Set(range);
      onSelectedPathsChange([...nextSelection]);
      return;
    }

    selectionAnchorIndexRef.current = index;

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

    selectionAnchorIndexRef.current = index;
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

  return (
    <section
      aria-label="文件列表"
      className="relative flex min-h-0 flex-1 flex-col"
      data-explorer-drop-target={currentDirectoryPath}
    >
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <h1
          className="min-w-0 truncate text-sm font-medium"
          title={searchState ? `“${searchState.query}”的搜索结果` : undefined}
        >
          {searchState ? `搜索：“${searchState.query}”` : "文件"}
        </h1>
        <p aria-live="polite" className="shrink-0 text-xs text-muted-foreground">
          {listIsLoading
            ? searchState
              ? "正在搜索…"
              : "正在读取…"
            : searchState?.error
              ? "搜索失败"
              : `${entries.length} 个${searchState ? "匹配项" : "项目"}${searchState?.truncated ? "，结果已截断" : ""}${selectedCount ? `，已选择 ${selectedCount} 个` : ""}`}
        </p>
      </div>

      {entries.length === 0 && !listIsLoading ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              {searchState ? (
                searchState.error ? (
                  <TriangleAlertIcon />
                ) : (
                  <SearchXIcon />
                )
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
      ) : (
        <div
          ref={scrollRef}
          className="min-h-0 flex-1 overflow-auto"
          onScroll={(event) => onScrollOffsetChange?.(event.currentTarget.scrollTop)}
        >
          <div className="min-w-160">
            <div className="flex h-10 items-center whitespace-nowrap border-b text-sm font-medium text-foreground">
              <div className="min-w-0 flex-1 px-2">名称</div>
              <div className="w-44 px-2">修改日期</div>
              <div className="w-28 px-2">类型</div>
              <div className="w-24 px-2 text-right">大小</div>
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
                      canPaste={hasClipboard}
                      entry={entry}
                      isActionDisabled={actionsDisabled}
                      isDragging={internalDrag?.sourcePaths.includes(entry.path) ?? false}
                      isDropTarget={
                        internalDrag?.targetPath === entry.path ||
                        externalDropTargetPath === entry.path
                      }
                      isLast={virtualRow.index === entries.length - 1}
                      isSelected={selectedPathSet.has(entry.path)}
                      isSingleSelection={selectedCount === 1}
                      onContextMenu={() => selectForContextMenu(entry, virtualRow.index)}
                      onCopy={onCopy}
                      onCut={onCut}
                      onDelete={onDelete}
                      onOpen={() => openEntry(entry)}
                      onPaste={onPaste}
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
          {internalDrag.operation === "copy" ? <CopyIcon /> : <ScissorsIcon />}
          {internalDrag.operation === "copy" ? "复制" : "移动"} {internalDrag.sourcePaths.length}{" "}
          个项目
        </div>
      )}
    </section>
  );
}

function FileListRow({
  canPaste,
  entry,
  isActionDisabled,
  isDragging,
  isDropTarget,
  isLast,
  isSelected,
  isSingleSelection,
  onContextMenu,
  onCopy,
  onCut,
  onDelete,
  onOpen,
  onPaste,
  onPointerDown,
  onRename,
  onSelect,
}: {
  canPaste: boolean;
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isLast: boolean;
  isSelected: boolean;
  isSingleSelection: boolean;
  onContextMenu: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpen: () => void;
  onPaste: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelect: (event: ReactMouseEvent) => void;
}) {
  const presentation = ENTRY_PRESENTATION[entry.kind];
  const EntryIcon = presentation.icon;

  return (
    <ContextMenu>
      <ContextMenuTrigger onContextMenu={onContextMenu}>
        <div
          aria-selected={isSelected}
          className={cn(
            "flex cursor-grab items-center whitespace-nowrap text-sm transition-colors select-none hover:bg-muted/50 focus-visible:bg-muted focus-visible:outline-none",
            !isLast && "border-b",
            isSelected && "bg-accent text-accent-foreground",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-accent ring-2 ring-primary ring-inset",
          )}
          data-explorer-directory-drop-target={entry.kind === "directory" ? entry.path : undefined}
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
          style={{ height: ROW_HEIGHT }}
        >
          <div className="flex min-w-0 flex-1 items-center gap-2 p-2">
            <EntryIcon className={cn(entry.kind !== "directory" && "text-muted-foreground")} />
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
          <div className="w-44 p-2 text-muted-foreground">{formatModifiedAt(entry.modifiedAt)}</div>
          <div className="w-28 p-2 text-muted-foreground">{presentation.label}</div>
          <div
            className="w-24 p-2 text-right text-muted-foreground"
            title={entry.size === null ? undefined : `${entry.size.toLocaleString("zh-CN")} 字节`}
          >
            {formatFileSize(entry.size)}
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem disabled={isActionDisabled} onClick={onOpen}>
            <FolderOpenIcon />
            打开
            <ContextMenuShortcut>Enter</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={isActionDisabled || !isSingleSelection} onClick={onRename}>
            <PencilIcon />
            重命名
            <ContextMenuShortcut>F2</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem onClick={() => void copyEntryPath(entry.path)}>
            <ClipboardCopyIcon />
            复制文件地址
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem disabled={isActionDisabled} onClick={onCopy}>
            <CopyIcon />
            复制
            <ContextMenuShortcut>Ctrl+C</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={isActionDisabled} onClick={onCut}>
            <ScissorsIcon />
            剪切
            <ContextMenuShortcut>Ctrl+X</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={isActionDisabled || !canPaste} onClick={onPaste}>
            <ClipboardPasteIcon />
            粘贴
            <ContextMenuShortcut>Ctrl+V</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem disabled={isActionDisabled} onClick={onDelete} variant="destructive">
            <Trash2Icon />
            删除
            <ContextMenuShortcut>Delete</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
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

async function copyEntryPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
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
