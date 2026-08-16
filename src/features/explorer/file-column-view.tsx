import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { CaretRightIcon, CircleNotchIcon, WarningIcon } from "@phosphor-icons/react";

import { commands } from "@/bindings";

import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";

import { EntryContextMenuContent } from "./entry-context-menu";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "./file-icons";
import type { DirectoryEntry } from "./types";

export interface FileColumnViewProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelectEntry: (entry: DirectoryEntry, event: ReactMouseEvent) => void;
  rootEntries: DirectoryEntry[];
  selectedCount: number;
  selectedPathSet: Set<string>;
  viewId: string;
}

interface SharedRowProps {
  actionsDisabled: boolean;
  draggingPaths: Set<string>;
  dropTargetPath: string | null;
  onAddToFavorites: (entry: DirectoryEntry) => void;
  onAddToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onRename: () => void;
  onSelectEntry: (entry: DirectoryEntry, event: ReactMouseEvent) => void;
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

  return (
    <Pane
      entries={data?.entries ?? []}
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

function Pane({
  actionsDisabled,
  activeChildPath,
  depth,
  draggingPaths,
  dropTargetPath,
  entries,
  isError = false,
  isLoading = false,
  onAddToFavorites,
  onAddToSpace,
  onContextMenuEntry,
  onCopy,
  onCut,
  onDelete,
  onDrill,
  onOpenEntry,
  onPointerDownEntry,
  onRename,
  onSelectEntry,
  selectedCount,
  selectedPathSet,
}: PaneProps) {
  return (
    <div className="flex w-56 shrink-0 flex-col overflow-y-auto border-r px-1.5 py-2">
      {isLoading && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground">
          <CircleNotchIcon className="animate-spin" size={14} />
          正在读取…
        </div>
      )}
      {!isLoading && isError && (
        <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-destructive">
          <WarningIcon size={14} />
          无法读取此文件夹
        </div>
      )}
      {!isLoading && !isError && entries.length === 0 && (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">空文件夹</p>
      )}
      {entries.map((entry) => {
        const isDirectory = entry.kind === "directory";
        const presentation = isDirectory ? DIRECTORY_PRESENTATION : getFilePresentation(entry.name);
        const EntryIcon = presentation.icon;
        const isExpanded = activeChildPath === entry.path;
        const isSelected = selectedPathSet.has(entry.path);

        return (
          <ContextMenu key={entry.path}>
            <ContextMenuTrigger onContextMenu={() => onContextMenuEntry(entry)}>
              <div
                aria-selected={isSelected}
                className={cn(
                  "flex h-8 cursor-grab items-center gap-2 rounded-[5px] px-2 text-[13px] transition-colors select-none hover:bg-muted/70 focus-visible:bg-muted/70 focus-visible:outline-none",
                  (isSelected || isExpanded) && "bg-selection ring-1 ring-primary/30 ring-inset",
                  draggingPaths.has(entry.path) && "cursor-grabbing opacity-50",
                  dropTargetPath === entry.path && "bg-selection ring-2 ring-primary ring-inset",
                )}
                data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
                onClick={(event) => {
                  onSelectEntry(entry, event);
                  if (isDirectory && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
                    onDrill(entry.path, depth);
                  }
                }}
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
                <EntryIcon
                  className={cn(
                    "size-4 shrink-0",
                    isDirectory ? "text-folder" : "text-muted-foreground",
                  )}
                  weight={isDirectory ? "fill" : undefined}
                />
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                {isDirectory && (
                  <CaretRightIcon className="size-3 shrink-0 text-muted-foreground" />
                )}
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <EntryContextMenuContent
                entry={entry}
                isActionDisabled={actionsDisabled}
                isSingleSelection={selectedCount === 1}
                onAddToFavorites={() => onAddToFavorites(entry)}
                onAddToSpace={(spaceId) => onAddToSpace(entry, spaceId)}
                onCopy={onCopy}
                onCut={onCut}
                onDelete={onDelete}
                onOpen={() => onOpenEntry(entry)}
                onRename={onRename}
              />
            </ContextMenuContent>
          </ContextMenu>
        );
      })}
    </div>
  );
}
