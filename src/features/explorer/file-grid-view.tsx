import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { GitEntryStatusKind } from "@/bindings";
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn, formatBytes } from "@/lib/utils";

import { DRAG_SOURCE_CLASS, EntryIconFrame, HIDDEN_ENTRY_CLASS } from "./entry-badges";
import { EntryContextMenuContent } from "./entry-context-menu";
import { getEntryPresentation } from "./file-icons";
import type { ListingNavContext, MenuActions } from "./file-list";
import { TypeIconTile } from "./icon-tile";
import { getEntryGitStatus, GitStatusBadge, type ExplorerGitStatus } from "./git-status";
import { entriesInRange, type ListingView } from "./listing-view";
import { MarqueeOverlay, useMarqueeSelection, type MarqueeRect } from "./marquee";
import { NativeIconImage, useNativeIconFor } from "./native-icon";
import { densityAtom, type ExplorerDensity } from "./preferences";
import { isThumbnailSupported, ThumbnailImage } from "./thumbnail";
import type { DirectoryEntry } from "./types";

const GRID_CELL_MIN_WIDTH: Record<ExplorerDensity, number> = {
  // 96 rather than 88: the text box of a compact cell is cell minus 16px of
  // padding, and 80px at 12px font-medium is what common names like
  // "package.json" need for one line. At 88 they wrapped mid-token —
  // "package.jso / n" — which reads as a rendering bug, not as a long name.
  compact: 120,
  comfortable: 148,
  spacious: 172,
};

/** Native shell icons stay bare; sized a step under the type tiles. */
/** The glyph a shell icon and a thumbnail fall back to, at the well's scale. */
const GRID_GLYPH_SIZE: Record<ExplorerDensity, number> = {
  compact: 28,
  comfortable: 30,
  spacious: 34,
};

/**
 * The reference's `.card .thumb`: a 56px well of 4% ink that seats whatever
 * the entry has — a picture, the shell's icon, or the type glyph. It is the
 * card's one recessed plate, and the reason the grid reads as a field of
 * objects rather than as labelled swatches.
 */
const GRID_THUMB_WELL_CLASS: Record<ExplorerDensity, string> = {
  compact: "h-10",
  comfortable: "h-14",
  spacious: "h-16",
};

/**
 * Fixed per-density cell height keeps virtualization math uniform.
 *
 * The number is measured off the card, not guessed: 14px of vertical padding
 * top and bottom, the well (40/56/64), two 10px gaps, one 19px name line and a
 * 16px mono meta line. Anything less clips the meta row on the last virtual
 * row; anything more leaves a gap between rows that the eye reads as a
 * different gap than the column gap.
 */
const GRID_CELL_HEIGHT: Record<ExplorerDensity, number> = {
  compact: 126,
  comfortable: 142,
  spacious: 150,
};

const GRID_GAP_PX = 12;
/**
 * The grid's inset on all four sides — and it has to be the same number the
 * markup below paints, because it is subtracted twice in the geometry: once by
 * `availableWidth` (which decides how many columns fit) and once by the
 * marquee's hit test (which turns a pointer rectangle back into cell indices).
 * The painted inset is `px-3` / `inset-x-3` / `pb-3` = 0.75rem = 12px, and
 * `GRID_PADDING_PX` said 14: every marquee that swept the first column missed
 * it by 2px, and the cells were laid out 4px narrower than the pane.
 */
const GRID_PADDING_PX = 12;

/**
 * How far a cell may shrink below the density's preferred width to keep two
 * columns alive in a narrow listing pane (preview panel docked beside it).
 * Past this floor two cramped columns read worse than one honest one. Safe
 * because names are `line-clamp-2` + `overflow-wrap: anywhere` — see the note
 * by the name span — so a squeezed name wraps at real break opportunities and
 * clamps instead of overflowing the fixed cell height.
 */
const GRID_SQUEEZE_SLACK_PX = 12;

export interface FileGridViewProps {
  actionsDisabled: boolean;
  /** The keyboard cursor's entry index — the grid's roving tab stop. */
  cursorIndex: number | null;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  entries: ListingView;
  gitStatus?: ExplorerGitStatus | null;
  menuActions: MenuActions;
  /** The shared list keyboard model from `FileList`; the grid supplies its own
   *  geometry (column count, stride, virtual-row scroll) as the context. */
  onNavKeyDown: (event: ReactKeyboardEvent<HTMLDivElement>, context: ListingNavContext) => void;
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
  cursorIndex,
  draggingPaths,
  dropTargetPath,
  entries,
  gitStatus,
  menuActions,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onNavKeyDown,
  onOpenEntry,
  onPointerDownEntry,
  onSelectEntry,
  onSelectedPathsChange,
  selectedCount,
  selectedPathSet,
}: FileGridViewProps) {
  const density = useAtomValue(densityAtom);
  const { t } = useTranslation("explorer");
  const directoryLabel = t("fileType.directory");
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

  const availableWidth = Math.max(0, viewportWidth - GRID_PADDING_PX * 2);
  const regularColumnCount = Math.max(
    1,
    Math.floor((availableWidth + GRID_GAP_PX) / (cellMinWidth + GRID_GAP_PX)),
  );
  // A lone column stretches every cell to the full pane width — one giant tile
  // per row. Prefer squeezing cells to the floor and taking a second column.
  const squeezeMinWidth = cellMinWidth - GRID_SQUEEZE_SLACK_PX;
  const squeezedColumnCount = Math.floor(
    (availableWidth + GRID_GAP_PX) / (squeezeMinWidth + GRID_GAP_PX),
  );
  const columnCount =
    regularColumnCount === 1
      ? Math.min(Math.max(1, squeezedColumnCount), 2)
      : regularColumnCount;
  const naturalCellWidth =
    (availableWidth - GRID_GAP_PX * (columnCount - 1)) / columnCount;
  // When even two squeezed columns don't fit, keep the single cell at normal
  // tile width and let the row end short — Explorer's fixed-tile grid — rather
  // than stretching one tile across the pane. `hitTest` uses the same width.
  const cellWidth =
    columnCount === 1 ? Math.min(naturalCellWidth, cellMinWidth + GRID_GAP_PX) : naturalCellWidth;
  const rowCount = Math.ceil(entries.count / columnCount);
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

        const path = entries.pathAt(row * columnCount + column);
        if (path !== undefined) matchedPaths.push(path);
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
        // `pb-14` keeps the last row clear of the floating contextual action
        // bar (`bottom-4 h-9`) while anything is selected.
        className={`h-full overflow-auto ${selectedCount > 0 ? "pb-14" : ""}`}
        onKeyDown={(event) =>
          onNavKeyDown(event, {
            container: event.currentTarget,
            columns: columnCount,
            leadingOffset: GRID_PADDING_PX,
            rowStride,
            // The virtualizer windows whole rows: bring the target's row into
            // view and the cell inside it mounts with it.
            scrollToEntry: (index) =>
              virtualizer.scrollToIndex(Math.floor(index / Math.max(1, columnCount)), {
                align: "auto",
              }),
          })
        }
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
                gridTemplateColumns: `repeat(${columnCount}, minmax(0, ${cellWidth}px))`,
                height: cellHeight,
                transform: `translateY(${GRID_PADDING_PX + virtualRow.start}px)`,
              }}
            >
              {entriesInRange(
                entries,
                virtualRow.index * columnCount,
                (virtualRow.index + 1) * columnCount,
              ).map((entry, sliceIndex) => (
                <GridCell
                  density={density}
                  directoryLabel={directoryLabel}
                  entry={entry}
                  gitStatus={gitStatus}
                  index={virtualRow.index * columnCount + sliceIndex}
                  isActionDisabled={actionsDisabled}
                  isCursor={virtualRow.index * columnCount + sliceIndex === (cursorIndex ?? 0)}
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
                  selectedPathSet={selectedPathSet}
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
  directoryLabel,
  entry,
  gitStatus,
  index,
  isActionDisabled,
  isCursor,
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
  selectedPathSet,
}: {
  density: ExplorerDensity;
  /** Localised `Folder`, resolved once per grid rather than once per cell. */
  directoryLabel: string;
  entry: DirectoryEntry;
  gitStatus?: ExplorerGitStatus | null;
  index: number;
  isActionDisabled: boolean;
  /** The roving tab stop — exactly one cell answers Tab. */
  isCursor: boolean;
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
  selectedPathSet: Set<string>;
}) {
  const isDirectory = entry.kind === "directory";
  const presentation = getEntryPresentation(entry);
  const showNativeIcon = useNativeIconFor(entry);
  const showThumbnail = isThumbnailSupported(entry);
  const entryStatus: GitEntryStatusKind | undefined = getEntryGitStatus(gitStatus, entry);
  // The glyph the well seats when there is no picture to show. A type glyph is
  // a silhouette that already carries its category colour, so it needs no tile
  // plate inside the well — only the well's own 4% wash.
  const typeGlyph = (
    <TypeIconTile iconSize={GRID_GLYPH_SIZE[density]} presentation={presentation} />
  );
  // The reference's `.cm`: a folder says what it is, a file says how big it is.
  const meta = isDirectory
    ? directoryLabel
    : entry.size === null
      ? "—"
      : formatBytes(entry.size);

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // Desktop cell: a plate at rest, not only under the pointer — the
            // reference draws a grid as a field of cards, so the fill and the
            // hairline are the resting state and the hover is a wash of the
            // cell's own ink on top of them. Selection keeps the accent fill and
            // the accent edge, and drops the neutral ring it replaces.
            //
            // `items-stretch` rather than `items-center`: the well is the only
            // centred thing in the card. The reference's `.card` sets its text
            // flush left under a centred thumb, and centring the name instead is
            // what made a grid of long file names read as a wall of jagged
            // labels.
            "render-contain state-layer relative flex cursor-grab flex-col items-stretch gap-2.5 rounded-xl bg-card/45 px-3 py-3.5 text-left shadow-[inset_0_0_0_1px_var(--border)] transition-[background-color,box-shadow,opacity] duration-fast ease-standard select-none focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset",
            entry.hidden && HIDDEN_ENTRY_CLASS,
            isSelected &&
              "bg-selection shadow-[inset_0_0_0_1px_var(--selection-edge)]",
            isDragging && DRAG_SOURCE_CLASS,
            isDropTarget && "drop-target",
          )}
          data-entry-path={entry.path}
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
          tabIndex={isCursor ? 0 : -1}
          title={entry.path}
        >
          {/* The reference's `.thumb`: one 4% well at a 6px corner, the only
              recessed plate in the card. Thumbnail, shell icon and type glyph
              all sit in it, so the grid's first row is a row of objects rather
              than a row of differently sized images. */}
          <span
            className={cn(
              "flex w-full shrink-0 items-center justify-center overflow-hidden rounded-md bg-foreground/4",
              GRID_THUMB_WELL_CLASS[density],
            )}
          >
            {showThumbnail ? (
              <EntryIconFrame
                badgeSize="md"
                className="flex size-full items-center justify-center"
                entry={entry}
              >
                <ThumbnailImage
                  className="size-full"
                  displaySize={128}
                  entry={entry}
                  fallback={typeGlyph}
                  plate={false}
                />
              </EntryIconFrame>
            ) : showNativeIcon ? (
              <EntryIconFrame badgeSize="md" entry={entry}>
                <NativeIconImage
                  className="shrink-0"
                  entry={entry}
                  fallback={typeGlyph}
                  pixelSize={GRID_GLYPH_SIZE[density]}
                />
              </EntryIconFrame>
            ) : (
              <EntryIconFrame badgeSize="md" entry={entry}>
                {typeGlyph}
              </EntryIconFrame>
            )}
          </span>
          {/* One line, ellipsis — the reference's `.cn`. Two clamped lines made
              the cell's height depend on its own label, and a grid whose rows
              are ragged is not a grid. The full name is in the tooltip and the
              context menu. */}
          <span className="w-full min-w-0 truncate text-caption font-medium">
            {entry.name}
          </span>
          <span className="w-full truncate font-mono text-[0.65625rem] leading-4 text-meta tabular-nums">
            {meta}
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
          onDeletePermanent={menuActions.onDeletePermanent}
          onDuplicate={menuActions.onDuplicate}
          onExtract={menuActions.onExtract}
          onMoveTo={menuActions.onMoveTo}
          onOpen={() => onOpenEntry(entry)}
          onOpenWith={() => menuActions.onOpenWith(entry.path)}
          onRename={menuActions.onRename}
          onTogglePreview={menuActions.onTogglePreview}
          selectedPaths={[...selectedPathSet]}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}
