import { useEffect, useRef, type MouseEvent as ReactMouseEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ClipboardPasteIcon,
  CopyIcon,
  FileIcon,
  FolderIcon,
  FolderOpenIcon,
  LinkIcon,
  PencilIcon,
  ScissorsIcon,
  ShapesIcon,
  Trash2Icon,
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

import type { DirectoryEntry, EntryKind } from "./types";

interface FileListProps {
  entries: DirectoryEntry[];
  hasClipboard: boolean;
  initialScrollOffset?: number;
  isLoading: boolean;
  isOperationPending: boolean;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpenDirectory: (path: string) => void;
  onPaste: () => void;
  onRename: () => void;
  onScrollOffsetChange?: (offset: number) => void;
  onSelectedPathsChange: (paths: string[]) => void;
  selectedPaths: string[];
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

export function FileList({
  entries,
  hasClipboard,
  initialScrollOffset = 0,
  isLoading,
  isOperationPending,
  onCopy,
  onCut,
  onDelete,
  onOpenDirectory,
  onPaste,
  onRename,
  onScrollOffsetChange,
  onSelectedPathsChange,
  selectedPaths,
}: FileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectionAnchorIndexRef = useRef<number | null>(null);
  const selectedPathSet = new Set(selectedPaths);
  const actionsDisabled = isLoading || isOperationPending;
  const selectedCount = selectedPaths.length;
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    initialOffset: initialScrollOffset,
    overscan: 10,
  });

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

  const openEntry = (entry: DirectoryEntry) => {
    if (actionsDisabled) return;

    if (entry.kind === "directory") {
      onOpenDirectory(entry.path);
      return;
    }

    void openFile(entry.path);
  };

  return (
    <section aria-label="文件列表" className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <h1 className="text-sm font-medium">文件</h1>
        <p aria-live="polite" className="text-xs text-muted-foreground">
          {isLoading
            ? "正在读取…"
            : `${entries.length} 个项目${selectedCount ? `，已选择 ${selectedCount} 个` : ""}`}
        </p>
      </div>

      {entries.length === 0 && !isLoading ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FolderOpenIcon />
            </EmptyMedia>
            <EmptyTitle>这个文件夹是空的</EmptyTitle>
            <EmptyDescription>此位置暂时没有文件或子文件夹。</EmptyDescription>
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
                      isLast={virtualRow.index === entries.length - 1}
                      isSelected={selectedPathSet.has(entry.path)}
                      isSingleSelection={selectedCount === 1}
                      onContextMenu={() => selectForContextMenu(entry, virtualRow.index)}
                      onCopy={onCopy}
                      onCut={onCut}
                      onDelete={onDelete}
                      onOpen={() => openEntry(entry)}
                      onPaste={onPaste}
                      onRename={onRename}
                      onSelect={(event) => selectEntry(entry, virtualRow.index, event)}
                    />
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function FileListRow({
  canPaste,
  entry,
  isActionDisabled,
  isLast,
  isSelected,
  isSingleSelection,
  onContextMenu,
  onCopy,
  onCut,
  onDelete,
  onOpen,
  onPaste,
  onRename,
  onSelect,
}: {
  canPaste: boolean;
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isLast: boolean;
  isSelected: boolean;
  isSingleSelection: boolean;
  onContextMenu: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpen: () => void;
  onPaste: () => void;
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
            "flex cursor-default items-center whitespace-nowrap text-sm transition-colors select-none hover:bg-muted/50 focus-visible:bg-muted focus-visible:outline-none",
            !isLast && "border-b",
            isSelected && "bg-accent text-accent-foreground",
          )}
          onClick={onSelect}
          onDoubleClick={onOpen}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onOpen();
            }
          }}
          role="option"
          tabIndex={0}
          title={entry.path}
          style={{ height: ROW_HEIGHT }}
        >
          <div className="flex min-w-0 flex-1 items-center gap-2 p-2">
            <EntryIcon className={cn(entry.kind !== "directory" && "text-muted-foreground")} />
            <span className="truncate">{entry.name}</span>
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
