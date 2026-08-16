import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtomValue } from "jotai";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { ArchiveFormat } from "@/bindings";
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { EntryContextMenuContent } from "./entry-context-menu";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "./file-icons";
import { MarqueeOverlay, useMarqueeSelection, type MarqueeRect } from "./marquee";
import { densityAtom, type ExplorerDensity } from "./preferences";
import { isThumbnailSupported, ThumbnailImage } from "./thumbnail";
import type { DirectoryEntry } from "./types";

const GRID_CELL_MIN_WIDTH: Record<ExplorerDensity, number> = {
  compact: 88,
  comfortable: 104,
  spacious: 120,
};

const GRID_ICON_SIZE: Record<ExplorerDensity, number> = {
  compact: 30,
  comfortable: 36,
  spacious: 42,
};

/** Fixed per-density cell height keeps virtualization math uniform. */
const GRID_CELL_HEIGHT: Record<ExplorerDensity, number> = {
  compact: 88,
  comfortable: 96,
  spacious: 104,
};

/** Static height classes matching GRID_ICON_SIZE so thumbnails keep geometry. */
const GRID_IMAGE_ZONE_CLASS: Record<ExplorerDensity, string> = {
  compact: "h-[30px]",
  comfortable: "h-[36px]",
  spacious: "h-[42px]",
};

const GRID_GAP_PX = 6;
const GRID_PADDING_PX = 12;

export interface FileGridViewProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  entries: DirectoryEntry[];
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onCompress: (format: ArchiveFormat) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  onSelectedPathsChange: (paths: string[]) => void;
  selectedCount: number;
  selectedPathSet: Set<string>;
}

/**
 * Virtualized grid: rows are windowed via @tanstack/react-virtual so large
 * directories only render visible cells (SKILL.md §48).
 */
export function FileGridView({
  actionsDisabled,
  draggingPaths,
  dropTargetPath,
  entries,
  onAddToFavorites,
  onAddToSpace,
  onCompress,
  onContextMenuEntry,
  onCopy,
  onCut,
  onDelete,
  onDuplicate,
  onExtract,
  onMoveTo,
  onOpenEntry,
  onPointerDownEntry,
  onRename,
  onSelectEntry,
  onSelectedPathsChange,
  selectedCount,
  selectedPathSet,
}: FileGridViewProps) {
  const density = useAtomValue(densityAtom);
  const cellMinWidth = GRID_CELL_MIN_WIDTH[density];
  const cellHeight = GRID_CELL_HEIGHT[density];
  const rowStride = cellHeight + GRID_GAP_PX;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [viewportWidth, setViewportWidth] = useState(0);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;

    const observer = new ResizeObserver((observedEntries) => {
      for (const observed of observedEntries) {
        setViewportWidth(observed.contentRect.width);
      }
    });
    observer.observe(element);
    setViewportWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  const columnCount = Math.max(
    1,
    Math.floor(
      (viewportWidth - GRID_PADDING_PX * 2 + GRID_GAP_PX) / (cellMinWidth + GRID_GAP_PX),
    ),
  );
  const rowCount = Math.ceil(entries.length / columnCount);
  const virtualizer = useVirtualizer({
    count: rowCount,
    estimateSize: () => rowStride,
    getScrollElement: () => scrollRef.current,
    overscan: 4,
    scrollMargin: GRID_PADDING_PX,
  });

  const hitTest = useCallback(
    (rect: MarqueeRect) => {
      const container = scrollRef.current;
      if (!container || viewportWidth === 0) return [];

      const bounds = container.getBoundingClientRect();
      const cellWidth =
        (viewportWidth - GRID_PADDING_PX * 2 - GRID_GAP_PX * (columnCount - 1)) / columnCount;
      const cellStrideX = cellWidth + GRID_GAP_PX;
      const topContent = rect.top - bounds.top + container.scrollTop - GRID_PADDING_PX;
      const bottomContent = rect.bottom - bounds.top + container.scrollTop - GRID_PADDING_PX;
      const leftContent = rect.left - bounds.left + container.scrollLeft - GRID_PADDING_PX;
      const rightContent = rect.right - bounds.left + container.scrollLeft - GRID_PADDING_PX;
      const firstRow = Math.max(0, Math.floor(topContent / rowStride));
      const lastRow = Math.min(
        rowCount - 1,
        Math.max(0, Math.ceil(bottomContent / rowStride) - 1),
      );
      if (lastRow < firstRow) return [];

      const matchedPaths: string[] = [];
      for (let row = firstRow; row <= lastRow; row += 1) {
        for (let column = 0; column < columnCount; column += 1) {
          const cellLeft = column * cellStrideX;
          if (cellLeft + cellWidth < leftContent || cellLeft > rightContent) continue;

          const entry = entries[row * columnCount + column];
          if (entry) matchedPaths.push(entry.path);
        }
      }
      return matchedPaths;
    },
    [columnCount, entries, rowCount, rowStride, viewportWidth],
  );

  const marquee = useMarqueeSelection({
    enabled: !actionsDisabled,
    getBaseSelection: () => [...selectedPathSet],
    hitTest,
    onSelectionChange: onSelectedPathsChange,
    scrollElementRef: scrollRef,
  });

  return (
    <div className="relative min-h-0 flex-1">
      <div
        aria-multiselectable="true"
        className="h-full overflow-auto"
        onPointerDown={(event) => {
          if (
            event.target instanceof Element &&
            event.target.closest('[role="option"]')
          ) {
            return;
          }
          marquee.beginMarquee(event);
        }}
        ref={scrollRef}
        role="listbox"
      >
        <div
          className="relative px-3 pb-3"
          style={{ height: virtualizer.getTotalSize() + GRID_PADDING_PX * 2 }}
        >
          {virtualizer.getVirtualItems().map((virtualRow) => (
            <div
              className="absolute inset-x-3 top-0 grid"
              key={virtualRow.key}
              role="presentation"
              style={{
                gap: GRID_GAP_PX,
                gridTemplateColumns: `repeat(${columnCount}, minmax(0, 1fr))`,
                height: cellHeight,
                transform: `translateY(${GRID_PADDING_PX + virtualRow.start}px)`,
              }}
            >
              {entries
                .slice(virtualRow.index * columnCount, (virtualRow.index + 1) * columnCount)
                .map((entry, sliceIndex) => (
                  <GridCell
                    density={density}
                    entry={entry}
                    isActionDisabled={actionsDisabled}
                    isDragging={draggingPaths.has(entry.path)}
                    isDropTarget={dropTargetPath === entry.path}
                    isSelected={selectedPathSet.has(entry.path)}
                    isSingleSelection={selectedCount === 1}
                    key={entry.path}
                    onAddToFavorites={() => onAddToFavorites(entry)}
                    onAddToSpace={(spaceId) => onAddToSpace(entry, spaceId)}
                    onCompress={onCompress}
                    onContextMenu={() => onContextMenuEntry(entry, virtualRow.index * columnCount + sliceIndex)}
                    onCopy={onCopy}
                    onCut={onCut}
                    onDelete={onDelete}
                    onDuplicate={onDuplicate}
                    onExtract={onExtract}
                    onMoveTo={onMoveTo}
                    onOpen={() => onOpenEntry(entry)}
                    onPointerDown={(event) => onPointerDownEntry(entry, event)}
                    onRename={onRename}
                    onSelect={(event) =>
                      onSelectEntry(entry, virtualRow.index * columnCount + sliceIndex, event)
                    }
                  />
                ))}
            </div>
          ))}
        </div>
      </div>
      <MarqueeOverlay rect={marquee.rect} />
    </div>
  );
}

function GridCell({
  density,
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
  density: ExplorerDensity;
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
  const isDirectory = entry.kind === "directory";
  const presentation = isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);
  const EntryIcon = presentation.icon;
  const iconSize = GRID_ICON_SIZE[density];
  const showThumbnail = isThumbnailSupported(entry);

  return (
    <ContextMenu>
      <ContextMenuTrigger onContextMenu={onContextMenu}>
        <div
          aria-selected={isSelected}
          className={cn(
            "flex cursor-grab flex-col items-center gap-1.5 rounded-lg px-2 py-2.5 text-center transition-colors select-none hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none",
            isSelected && "bg-selection ring-1 ring-primary/30 ring-inset",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-selection ring-2 ring-primary ring-inset",
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
        >
          {showThumbnail ? (
            <ThumbnailImage
              className={cn("w-full shrink-0 self-center", GRID_IMAGE_ZONE_CLASS[density])}
              entry={entry}
              requestSize={128}
            />
          ) : (
            <EntryIcon
              className={cn("shrink-0", isDirectory ? "text-folder" : "text-muted-foreground")}
              size={iconSize}
              weight={isDirectory ? "fill" : undefined}
            />
          )}
          <span className="line-clamp-2 text-xs leading-snug break-all">{entry.name}</span>
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
