import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AltArrowRightIcon, DangerTriangleIcon, LoaderIcon } from "@solar-icons/react/linear";

import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { useDirectoryListing } from "./directory-listing";
import { DRAG_SOURCE_CLASS, HIDDEN_ENTRY_CLASS } from "./entry-badges";
import { EntryContextMenuContent } from "./entry-context-menu";
import { getEntryPresentation } from "./file-icons";
import type { MenuActions } from "./file-list";
import type { ListingView } from "./listing-view";
import {
  DENSITY_ROW_HEIGHT,
  densityAtom,
  filterHidden,
  foldersFirstAtom,
  showHiddenFilesAtom,
  sortKeyAtom,
  sortOrderAtom,
} from "./preferences";
import { RowEntryIcon } from "./row-icon";
import { useSortedListingView } from "./sorted-entries";
import type { DirectoryEntry } from "./types";

export interface FileColumnViewProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  menuActions: MenuActions;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  rootEntries: ListingView;
  selectedCount: number;
  selectedPathSet: Set<string>;
  viewId: string;
}

interface SharedRowProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  menuActions: MenuActions;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
  selectedPathSet: Set<string>;
}

export function FileColumnView({ rootEntries, viewId, ...shared }: FileColumnViewProps) {
  const [drillChain, setDrillChain] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** A pane that should take keyboard focus once its rows exist. Drilling and
   *  popping both end with focus on a row *in another pane*, and that pane may
   *  still be streaming its listing, so the request is handed down and each
   *  pane claims the one addressed to its depth. State rather than a ref,
   *  because the claim has to re-render an already-mounted pane: a request that
   *  only touched a ref would be seen by the pane that mounts and missed by the
   *  one that is already there — which is exactly the pop case. */
  const [focusRequest, setFocusRequest] = useState<PaneFocusRequest | null>(null);
  const focusTokenRef = useRef(0);

  useEffect(() => {
    setDrillChain([]);
  }, [viewId]);

  useEffect(() => {
    const container = scrollRef.current;
    if (container) {
      container.scrollLeft = container.scrollWidth;
    }
  }, [drillChain.length]);

  const drillTo = (path: string, depth: number) => {
    setDrillChain((chain) => [...chain.slice(0, depth), path]);
  };

  const focusPane = (depth: number, path: string | null) => {
    focusTokenRef.current += 1;
    setFocusRequest({ depth, path, token: focusTokenRef.current });
  };

  /** ArrowLeft out of a child pane: drop everything deeper than its parent and
   *  put focus back on the row that opened it. */
  const popTo = (depth: number) => {
    const openerPath = drillChain[depth - 1] ?? null;
    setDrillChain((chain) => chain.slice(0, depth - 1));
    focusPane(depth - 1, openerPath);
  };

  return (
    <div
      aria-multiselectable="true"
      className="flex min-h-0 flex-1 overflow-x-auto"
      ref={scrollRef}
      role="listbox"
    >
      <Pane
        activeChildPath={drillChain[0] ?? null}
        depth={0}
        entries={rootEntries}
        onDrill={drillTo}
        focusRequest={focusRequest}
        onFocusPane={focusPane}
        onPop={popTo}
        {...shared}
      />
      {drillChain.map((path, index) => (
        <ChildPane
          activeChildPath={drillChain[index + 1] ?? null}
          depth={index + 1}
          key={path}
          onDrill={drillTo}
          onFocusPane={focusPane}
          focusRequest={focusRequest}
          onPop={popTo}
          path={path}
          {...shared}
        />
      ))}
    </div>
  );
}

interface ChildPaneProps extends SharedRowProps {
  activeChildPath: string | null;
  depth: number;
  focusRequest: PaneFocusRequest | null;
  onDrill: (path: string, depth: number) => void;
  onFocusPane: (depth: number, path: string | null) => void;
  onPop: (depth: number) => void;
  path: string;
}

/** "Put focus on this row (or on the first row) of the column at `depth`."
 *  `token` makes two requests for the same target distinguishable, so a pane
 *  claims each one exactly once. */
interface PaneFocusRequest {
  depth: number;
  path: string | null;
  token: number;
}

function ChildPane({ path, ...paneProps }: ChildPaneProps) {
  const { listing, isError, isLoading } = useDirectoryListing(path);
  const sortKey = useAtomValue(sortKeyAtom);
  const sortOrder = useAtomValue(sortOrderAtom);
  const foldersFirst = useAtomValue(foldersFirstAtom);
  const showHiddenFiles = useAtomValue(showHiddenFilesAtom);
  // Child panes share the parent's sort and visibility preferences (SKILL.md §18).
  // Their listings stream in batches off the columnar channel and are never
  // materialised into an array, so both steps run over the view: filtering
  // keeps the source's identity when nothing is hidden, which is what lets the
  // ordering hook fold each batch into the order it already holds instead of
  // re-sorting every snapshot from scratch.
  const visible = useMemo(() => filterHidden(listing, showHiddenFiles), [listing, showHiddenFiles]);
  const sorted = useSortedListingView(visible, sortKey, sortOrder, foldersFirst);

  return (
    <Pane
      entries={sorted}
      isError={isError}
      // A cached listing keeps painting while its refresh streams, so the
      // spinner is only for a pane that has nothing to show yet.
      isLoading={isLoading && listing.count === 0}
      {...paneProps}
    />
  );
}

interface PaneProps extends SharedRowProps {
  activeChildPath: string | null;
  depth: number;
  entries: ListingView;
  isError?: boolean;
  isLoading?: boolean;
  onDrill: (path: string, depth: number) => void;
  focusRequest: PaneFocusRequest | null;
  onFocusPane: (depth: number, path: string | null) => void;
  onPop: (depth: number) => void;
}

/** Top and bottom breathing room inside a column, matching the `py-2` the pane
 *  paints. The row height is not a constant here any more — it is the density
 *  preference's, the same one the detail list and the grid already honour (see
 *  `DENSITY_ROW_HEIGHT`). */
const PANE_VERTICAL_PADDING_PX = 8;

/** How many frames a cross-pane focus request may wait for its row to mount and
 *  for the child listing to arrive. A second is the budget, as in the detail
 *  list: a streamed listing can take several frames and a focus that gives up
 *  early strands the keyboard in a pane the user has already left. */
const FOCUS_RETRY_FRAMES = 60;

/** The path, as a value an attribute selector can carry. `CSS.escape` is the
 *  right tool but escapes for selectors, not for quoted attribute values, so
 *  the backslash and the double quote are escaped by hand. */
function cssAttributeValue(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

/** Linear scan for a path in a listing — used only when a focus request has to
 *  scroll its row into range, never on the render path. */
function indexOfPath(entries: ListingView, path: string): number {
  for (let index = 0; index < entries.count; index += 1) {
    if (entries.pathAt(index) === path) return index;
  }
  return -1;
}

/** Move focus one row up or down inside the same pane.
 *
 *  Rows are virtualized, so the neighbour may not be mounted yet: the pane is
 *  nudged by one row height and the lookup retries on the next frame until the
 *  virtualizer paints it. Focus is restored by hand rather than left to the
 *  browser's tab order, which is the only way a column view keeps the arrow
 *  keys behaving like the detail list. */
function moveRowFocus(from: HTMLElement, delta: number, attempt = 0) {
  const pane = from.closest<HTMLElement>("[data-column-depth]");
  if (!pane || !from.isConnected) return;

  const rows = [...pane.querySelectorAll<HTMLElement>('[role="option"]')];
  const next = rows[rows.indexOf(from) + delta];
  if (next) {
    next.focus({ preventScroll: true });
    next.scrollIntoView({ block: "nearest" });
    return;
  }

  if (attempt >= FOCUS_RETRY_FRAMES) return;
  pane.scrollTop += delta * Math.max(from.offsetHeight, 1);
  requestAnimationFrame(() => moveRowFocus(from, delta, attempt + 1));
}

/** Each Miller-column pane windowizes its own rows (SKILL.md §48). */
function Pane({
  actionsDisabled,
  activeChildPath,
  depth,
  draggingPaths,
  dropTargetPath,
  entries,
  isError = false,
  isLoading = false,
  focusRequest,
  menuActions,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onDrill,
  onFocusPane,
  onOpenEntry,
  onPointerDownEntry,
  onPop,
  onSelectEntry,
  selectedCount,
  selectedPathSet,
}: PaneProps) {
  const { t } = useTranslation("explorer");
  // Columns used to be pinned at 38px while the list and the grid followed the
  // density preference, so one folder was up to three different row heights
  // depending on the view it was read in. The estimate and the painted row are
  // this one number, read once from the atom.
  const density = useAtomValue(densityAtom);
  const rowHeight = DENSITY_ROW_HEIGHT[density];
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: entries.count,
    estimateSize: () => rowHeight,
    getScrollElement: () => scrollRef.current,
    overscan: 8,
    scrollMargin: PANE_VERTICAL_PADDING_PX,
  });

  // Claim the focus request addressed to this depth. The row may not exist yet
  // — a freshly drilled pane is still streaming its listing, and the row we are
  // coming back to may be scrolled out of the virtual window — so the lookup
  // retries for a bounded number of frames, scrolling the target's index into
  // range first. Each token is claimed once: a later render of the same pane
  // must not pull focus back out of wherever the user has since put it.
  const claimedTokenRef = useRef<number | null>(null);
  const entriesRef = useRef(entries);
  entriesRef.current = entries;

  useEffect(() => {
    if (!focusRequest || focusRequest.depth !== depth) return;
    if (claimedTokenRef.current === focusRequest.token) return;
    claimedTokenRef.current = focusRequest.token;

    const { path } = focusRequest;
    let frame = 0;
    const settle = () => {
      const container = scrollRef.current;
      if (!container) return;

      const target = path
        ? container.querySelector<HTMLElement>(
            `[role="option"][data-entry-path="${cssAttributeValue(path)}"]`,
          )
        : container.querySelector<HTMLElement>('[role="option"]');

      if (target) {
        target.focus({ preventScroll: true });
        return;
      }

      if (frame++ >= FOCUS_RETRY_FRAMES) {
        // An empty or unreadable column has no row to land on, and leaving
        // focus in the pane we just left makes ArrowLeft a dead key there (the
        // parent row sees a depth-0 event). The column itself takes focus so
        // the horizontal arrows keep working from here.
        container.focus({ preventScroll: true });
        return;
      }

      if (path) {
        const index = indexOfPath(entriesRef.current, path);
        if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
      }
      requestAnimationFrame(settle);
    };

    requestAnimationFrame(settle);
  }, [depth, focusRequest, virtualizer]);

  /**
   * Miller-columns keyboard grammar: up/down walk the column, right drills into
   * the focused row (or hands focus to the child column), left climbs back out.
   * The detail list already owns single-column navigation; a column view is the
   * one place where the horizontal arrows mean something, and without them the
   * drill-down chain can only be driven by the mouse.
   */
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowLeft") {
      if (depth === 0) return;
      event.preventDefault();
      onPop(depth);
      return;
    }

    const rowElement = (event.target as HTMLElement | null)?.closest<HTMLElement>(
      '[role="option"]',
    );
    if (!rowElement) return;

    if (event.key === "ArrowRight") {
      const path = rowElement.dataset.entryPath;
      const kind = rowElement.dataset.entryKind;
      if (!path) return;
      event.preventDefault();
      if (kind === "directory") {
        onDrill(path, depth);
        onFocusPane(depth + 1, null);
      } else if (depth > 0) {
        // A file row has no column of its own; the only thing to the right is
        // the pane that is already there.
        return;
      }
      return;
    }

    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      moveRowFocus(rowElement, event.key === "ArrowDown" ? 1 : -1);
    }
  };

  return (
    <div
      // `pb-14` keeps the last row clear of the floating contextual action
      // bar (`bottom-4 h-9`) while anything is selected.
      className={`flex w-column-view shrink-0 flex-col overflow-y-auto border-r border-border px-2 py-2 last:border-r-0 ${
        selectedCount > 0 ? "pb-14" : ""
      }`}
      data-column-depth={depth}
      onKeyDown={handleKeyDown}
      ref={scrollRef}
      tabIndex={-1}
    >
      {isLoading && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-caption text-muted-foreground">
          <LoaderIcon className="animate-spin" size={14} />
          {t("explorer:columnView.loading")}
        </div>
      )}
      {!isLoading && isError && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-caption text-destructive">
          <DangerTriangleIcon size={14} />
          {t("explorer:columnView.readError")}
        </div>
      )}
      {!isLoading && !isError && entries.count === 0 && (
        <p className="px-2 py-1.5 text-caption text-muted-foreground">
          {t("explorer:columnView.emptyFolder")}
        </p>
      )}
      {entries.count > 0 && (
        <div
          className="relative"
          style={{ height: virtualizer.getTotalSize() + PANE_VERTICAL_PADDING_PX * 2 }}
        >
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const entry = entries.entryAt(virtualRow.index);
            if (!entry) return null;

            return (
              <PaneRow
                activeChildPath={activeChildPath}
                actionsDisabled={actionsDisabled}
                depth={depth}
                dropTargetPath={dropTargetPath}
                entry={entry}
                index={virtualRow.index}
                isDragging={draggingPaths.has(entry.path)}
                isSelected={selectedPathSet.has(entry.path)}
                key={entry.path}
                menuActions={menuActions}
                onAddToFavorites={onAddToFavorites}
                onAddToSpace={onAddToSpace}
                onContextMenuEntry={onContextMenuEntry}
                onDrill={onDrill}
                onOpenEntry={onOpenEntry}
                onPointerDownEntry={onPointerDownEntry}
                onSelectEntry={onSelectEntry}
                selectedCount={selectedCount}
                selectedPathSet={selectedPathSet}
                rowHeight={rowHeight}
                virtualStart={virtualRow.start}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

/**
 * Miller-column row. The React Compiler memoizes this component's render
 * output; selection/drag membership arrives as boolean props so re-renders
 * stay scoped to rows whose bits actually changed.
 */
function PaneRow({
  activeChildPath,
  actionsDisabled,
  depth,
  dropTargetPath,
  entry,
  index,
  isDragging,
  isSelected,
  menuActions,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onDrill,
  onOpenEntry,
  onPointerDownEntry,
  onSelectEntry,
  selectedCount,
  selectedPathSet,
  rowHeight,
  virtualStart,
}: {
  activeChildPath: string | null;
  actionsDisabled: boolean;
  depth: number;
  dropTargetPath: string | null;
  entry: DirectoryEntry;
  index: number;
  isDragging: boolean;
  isSelected: boolean;
  menuActions: MenuActions;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onDrill: (path: string, depth: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
  selectedPathSet: Set<string>;
  /** The density-derived row height, shared with the pane's virtualizer. */
  rowHeight: number;
  virtualStart: number;
}) {
  const isDirectory = entry.kind === "directory";
  const presentation = getEntryPresentation(entry);
  const isExpanded = activeChildPath === entry.path;

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // The detail list's row grammar, one column wide: a 13px label, a
            // muted glyph, an optional chevron. 6px corners (`--radius-md`) and
            // a half-strength hairline between rows, so the same folder reads the
            // same in the list and in a column instead of stepping down a type
            // size and up a corner radius when the view changes.
            "render-contain state-layer absolute inset-x-0 top-0 flex cursor-grab items-center gap-2.5 rounded-md px-2 select-none transition-[background-color,box-shadow,opacity] duration-fast ease-standard focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset",
            entry.hidden && HIDDEN_ENTRY_CLASS,
            // The column holding the selection ancestor keeps the softer fill
            // (60%) so the path stays readable; the chosen row itself takes the
            // full-strength tint AND the 1px `--selection-edge` hairline the
            // list row and the grid cell already carry, so "chosen" reads the
            // same in all three views. Only the drop target gets the 2px ring.
            isSelected && "bg-selection shadow-[inset_0_0_0_1px_var(--selection-edge)]",
            !isSelected && isExpanded && "bg-selection/60",
            !isSelected && "row-separator",
            isDragging && DRAG_SOURCE_CLASS,
            dropTargetPath === entry.path && "drop-target",
          )}
          data-entry-kind={entry.kind}
          data-entry-path={entry.path}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={(event) => {
            onSelectEntry(entry, index, event);
            if (isDirectory && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
              onDrill(entry.path, depth);
            }
          }}
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
          style={{
            height: rowHeight,
            transform: `translateY(${PANE_VERTICAL_PADDING_PX + virtualStart}px)`,
          }}
          tabIndex={0}
          title={entry.path}
        >
          <RowEntryIcon
            entry={entry}
            glyphSize={16}
            nativePixelSize={18}
            presentation={presentation}
          />
          <span
            // 13px like a list row, and selection changes the fill, never the
            // ink or the weight: a re-measuring label makes a column jump as the
            // cursor walks it.
            className="min-w-0 flex-1 truncate text-caption"
          >
            {entry.name}
          </span>
          {isDirectory && <AltArrowRightIcon className="size-3 shrink-0 text-muted-foreground" />}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <EntryContextMenuContent
          entry={entry}
          isActionDisabled={actionsDisabled}
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
