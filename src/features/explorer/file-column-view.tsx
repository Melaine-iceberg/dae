import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { CaretRightIcon, CircleNotchIcon, WarningIcon } from "@phosphor-icons/react";

import { commands } from "@/bindings";

import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { EntryIconFrame, HIDDEN_ENTRY_CLASS } from "./entry-badges";
import { EntryContextMenuContent } from "./entry-context-menu";
import { getEntryPresentation, getPresentationIconClassName } from "./file-icons";
import type { MenuActions } from "./file-list";
import { isNativeIconSupported, NativeIconImage } from "./native-icon";
import {
  filterHiddenEntries,
  foldersFirstAtom,
  showHiddenFilesAtom,
  sortEntries,
  sortKeyAtom,
  sortOrderAtom,
} from "./preferences";
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
  rootEntries: DirectoryEntry[];
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
        {...shared}
      />
      {drillChain.map((path, index) => (
        <ChildPane
          activeChildPath={drillChain[index + 1] ?? null}
          depth={index + 1}
          key={path}
          onDrill={drillTo}
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
  onDrill: (path: string, depth: number) => void;
  path: string;
}

function ChildPane({ path, ...paneProps }: ChildPaneProps) {
  const { data, isError, isFetching } = useQuery({
    queryKey: ["explorer-column", path],
    queryFn: () => commands.readDirectory(path),
    retry: false,
  });
  const sortKey = useAtomValue(sortKeyAtom);
  const sortOrder = useAtomValue(sortOrderAtom);
  const foldersFirst = useAtomValue(foldersFirstAtom);
  const showHiddenFiles = useAtomValue(showHiddenFilesAtom);
  // Child panes share the parent's sort and visibility preferences (SKILL.md §18).
  const sortedEntries = sortEntries(
    filterHiddenEntries(data?.entries ?? [], showHiddenFiles),
    sortKey,
    sortOrder,
    foldersFirst,
  );

  return (
    <Pane
      entries={sortedEntries}
      isError={isError}
      isLoading={isFetching && !data}
      {...paneProps}
    />
  );
}

interface PaneProps extends SharedRowProps {
  activeChildPath: string | null;
  depth: number;
  entries: DirectoryEntry[];
  isError?: boolean;
  isLoading?: boolean;
  onDrill: (path: string, depth: number) => void;
}

const PANE_ROW_HEIGHT = 32;
const PANE_VERTICAL_PADDING_PX = 8;

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
}: PaneProps) {
  const { t } = useTranslation("explorer");
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: entries.length,
    estimateSize: () => PANE_ROW_HEIGHT,
    getScrollElement: () => scrollRef.current,
    overscan: 8,
    scrollMargin: PANE_VERTICAL_PADDING_PX,
  });

  return (
    <div
      className="flex w-56 shrink-0 flex-col overflow-y-auto border-r border-border/60 px-1.5 py-2"
      ref={scrollRef}
    >
      {isLoading && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground">
          <CircleNotchIcon className="animate-spin" size={14} />
          {t("explorer:columnView.loading")}
        </div>
      )}
      {!isLoading && isError && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-destructive">
          <WarningIcon size={14} />
          {t("explorer:columnView.readError")}
        </div>
      )}
      {!isLoading && !isError && entries.length === 0 && (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">
          {t("explorer:columnView.emptyFolder")}
        </p>
      )}
      {entries.length > 0 && (
        <div
          className="relative"
          style={{ height: virtualizer.getTotalSize() + PANE_VERTICAL_PADDING_PX * 2 }}
        >
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const entry = entries[virtualRow.index];
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
  virtualStart: number;
}) {
  const isDirectory = entry.kind === "directory";
  const presentation = getEntryPresentation(entry);
  const EntryIcon = presentation.icon;
  const isExpanded = activeChildPath === entry.path;

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // Desktop row: tonal hover via state-layer, flat selection fill,
            // no pill morph so rows keep a constant corner radius.
            "state-layer absolute inset-x-0 top-0 flex h-8 cursor-grab items-center gap-2 rounded-xs px-2.5 select-none transition-[background-color,opacity] duration-fast ease-standard focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring/60 focus-visible:ring-inset",
            entry.hidden && HIDDEN_ENTRY_CLASS,
            (isSelected || isExpanded) && "bg-selection",
            isDragging && "cursor-grabbing opacity-50",
            dropTargetPath === entry.path && "bg-selection ring-2 ring-primary ring-inset",
          )}
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
          style={{ transform: `translateY(${PANE_VERTICAL_PADDING_PX + virtualStart}px)` }}
          tabIndex={0}
          title={entry.path}
        >
          <EntryIconFrame entry={entry}>
            {isNativeIconSupported(entry) ? (
              <NativeIconImage
                className="shrink-0 entry-icon-pop"
                entry={entry}
                fallback={
                  <EntryIcon className={getPresentationIconClassName(presentation)} size={16} />
                }
                pixelSize={16}
              />
            ) : (
              <EntryIcon
                className={cn(
                  "size-4 shrink-0 entry-icon-pop",
                  getPresentationIconClassName(presentation),
                )}
                weight={isDirectory ? "fill" : undefined}
              />
            )}
          </EntryIconFrame>
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-sm",
              // Expressive type scale: the name carries the row's weight and
              // steps up to semibold while selected or expanded.
              isSelected || isExpanded ? "font-semibold" : "font-medium",
            )}
          >
            {entry.name}
          </span>
          {isDirectory && <CaretRightIcon className="size-3 shrink-0 text-muted-foreground" />}
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
          onDuplicate={menuActions.onDuplicate}
          onExtract={menuActions.onExtract}
          onMoveTo={menuActions.onMoveTo}
          onOpen={() => onOpenEntry(entry)}
          onOpenWith={() => menuActions.onOpenWith(entry.path)}
          onRename={menuActions.onRename}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}
