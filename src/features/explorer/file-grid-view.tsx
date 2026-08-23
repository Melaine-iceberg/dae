import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtomValue } from "jotai";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { GitEntryStatusKind } from "@/bindings";
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { EntryContextMenuContent } from "./entry-context-menu";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "./file-icons";
import type { MenuActions } from "./file-list";
import { getEntryGitStatus, GitStatusBadge, type ExplorerGitStatus } from "./git-status";
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
  gitStatus?: ExplorerGitStatus | null;
  menuActions: MenuActions;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
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
  gitStatus,
  menuActions,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onOpenEntry,
  onPointerDownEntry,
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
    Math.floor((viewportWidth - GRID_PADDING_PX * 2 + GRID_GAP_PX) / (cellMinWidth + GRID_GAP_PX)),
  );
  const rowCount = Math.ceil(entries.length / columnCount);
  const virtualizer = useVirtualizer({
    count: rowCount,
    estimateSize: () => rowStride,
    getScrollElement: () => scrollRef.current,
    overscan: 4,
    scrollMargin: GRID_PADDING_PX,
  });

  const hitTest = (rect: MarqueeRect) => {
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
    const lastRow = Math.min(rowCount - 1, Math.max(0, Math.ceil(bottomContent / rowStride) - 1));
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
  };

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
          if (event.target instanceof Element && event.target.closest('[role="option"]')) {
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
                    gitStatus={gitStatus}
                    index={virtualRow.index * columnCount + sliceIndex}
                    isActionDisabled={actionsDisabled}
                    isDragging={draggingPaths.has(entry.path)}
                    isDropTarget={dropTargetPath === entry.path}
                    isSelected={selectedPathSet.has(entry.path)}
                    key={entry.path}
                    menuActions={menuActions}
                    onAddToFavorites={onAddToFavorites}
                    onAddToSpace={onAddToSpace}
                    onContextMenuEntry={onContextMenuEntry}
                    onOpenEntry={onOpenEntry}
                    onPointerDownEntry={onPointerDownEntry}
                    onSelectEntry={onSelectEntry}
                    selectedCount={selectedCount}
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

/**
 * Grid cell. The React Compiler memoizes this component's render output, so
 * scrolling and selection churn only re-renders cells whose props changed.
 */
function GridCell({
  density,
  entry,
  gitStatus,
  index,
  isActionDisabled,
  isDragging,
  isDropTarget,
  isSelected,
  menuActions,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onOpenEntry,
  onPointerDownEntry,
  onSelectEntry,
  selectedCount,
}: {
  density: ExplorerDensity;
  entry: DirectoryEntry;
  gitStatus?: ExplorerGitStatus | null;
  index: number;
  isActionDisabled: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  menuActions: MenuActions;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
}) {
  const isDirectory = entry.kind === "directory";
  const presentation = isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);
  const EntryIcon = presentation.icon;
  const iconSize = GRID_ICON_SIZE[density];
  const showThumbnail = isThumbnailSupported(entry);
  const entryStatus: GitEntryStatusKind | undefined = getEntryGitStatus(gitStatus, entry);

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // M3 Expressive cell: state-layer tints hover/focus/press; the
            // hover lift stays, and selection trades the ring for a tonal
            // fill plus a shape morph up the corner scale.
            "state-layer relative flex cursor-grab flex-col items-center gap-1.5 rounded-xl px-2 py-2.5 text-center transition-[background-color,border-radius,box-shadow,transform,opacity] duration-fast ease-spring-fast select-none hover:-translate-y-0.5 hover:shadow-ambient-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring/60 focus-visible:ring-inset",
            isSelected && "rounded-2xl bg-selection",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "rounded-2xl bg-selection ring-2 ring-primary ring-inset",
          )}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={(event) => onSelectEntry(entry, index, event)}
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
          <span
            className={cn(
              "line-clamp-2 text-xs leading-snug break-all",
              // Expressive type scale: the name carries the cell's weight and
              // steps up to semibold while selected.
              isSelected ? "font-semibold" : "font-medium",
            )}
          >
            {entry.name}
          </span>
          {entryStatus && (
            <span className="absolute right-1.5 top-1.5">
              <GitStatusBadge kind={entryStatus} />
            </span>
          )}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <EntryContextMenuContent
          entry={entry}
          isActionDisabled={isActionDisabled}
          isSingleSelection={selectedCount === 1}
          onAddToFavorites={() => onAddToFavorites(entry)}
          onAddToSpace={(spaceId) => onAddToSpace(entry, spaceId)}
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
