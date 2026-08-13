import type { MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent } from "react";
import { useAtomValue } from "jotai";

import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { EntryContextMenuContent } from "./entry-context-menu";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "./file-icons";
import { densityAtom, type ExplorerDensity } from "./preferences";
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

export interface FileGridViewProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  entries: DirectoryEntry[];
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
  selectedPathSet: Set<string>;
}

export function FileGridView({
  actionsDisabled,
  draggingPaths,
  dropTargetPath,
  entries,
  onAddToFavorites,
  onContextMenuEntry,
  onCopy,
  onCut,
  onDelete,
  onOpenEntry,
  onPointerDownEntry,
  onRename,
  onSelectEntry,
  selectedCount,
  selectedPathSet,
}: FileGridViewProps) {
  const density = useAtomValue(densityAtom);
  const cellMinWidth = GRID_CELL_MIN_WIDTH[density];

  return (
    <div
      aria-multiselectable="true"
      className="grid min-h-0 flex-1 content-start gap-1.5 overflow-auto p-3"
      role="listbox"
      style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${cellMinWidth}px, 1fr))` }}
    >
      {entries.map((entry, index) => (
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
          onContextMenu={() => onContextMenuEntry(entry, index)}
          onCopy={onCopy}
          onCut={onCut}
          onDelete={onDelete}
          onOpen={() => onOpenEntry(entry)}
          onPointerDown={(event) => onPointerDownEntry(entry, event)}
          onRename={onRename}
          onSelect={(event) => onSelectEntry(entry, index, event)}
        />
      ))}
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
  onContextMenu,
  onCopy,
  onCut,
  onDelete,
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
  onContextMenu: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpen: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelect: (event: ReactMouseEvent) => void;
}) {
  const isDirectory = entry.kind === "directory";
  const presentation = isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);
  const EntryIcon = presentation.icon;
  const iconSize = GRID_ICON_SIZE[density];

  return (
    <ContextMenu>
      <ContextMenuTrigger onContextMenu={onContextMenu}>
        <div
          aria-selected={isSelected}
          className={cn(
            "flex cursor-grab flex-col items-center gap-1.5 rounded-lg px-2 py-3 text-center transition-colors duration-100 select-none hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
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
        >
          <EntryIcon
            className={cn("shrink-0", isDirectory ? "text-primary" : "text-muted-foreground")}
            size={iconSize}
          />
          <span className="line-clamp-2 text-xs leading-snug break-all">{entry.name}</span>
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
