import { useEffect, useRef } from "react";
import { atom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { CaretDownIcon, FolderIcon } from "@phosphor-icons/react";

import { commands, type DirectoryEntry } from "@/bindings";
import { filterHiddenEntries, showHiddenFilesAtom } from "@/features/explorer/preferences";
import { cn } from "@/lib/utils";

import { FolderContextMenu } from "./folder-context-menu";

/** Natural, numeric-aware ordering for tree node names. */
const NAME_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Horizontal step added per nesting level. */
const INDENT_PX = 10;

export interface TreeChildrenState {
  status: "loading" | "ready" | "error";
  /** Subdirectories from the last successful read (kept visible while a refresh runs). */
  entries: DirectoryEntry[];
}

/** Paths whose children are currently rendered. Kept in an atom (rather than
 *  component state) so collapsing the "Local disks" section — which unmounts
 *  its body — does not lose the tree layout. */
export const treeExpandedPathsAtom = atom<ReadonlySet<string>>(new Set<string>());

const treeChildrenAtom = atom<ReadonlyMap<string, TreeChildrenState>>(new Map());

/** Reads a folder's subdirectories into the tree cache. A second read of the
 *  same path is skipped while one is already in flight. */
const loadTreeChildrenAtom = atom(null, async (get, set, path: string) => {
  const children = get(treeChildrenAtom);
  if (children.get(path)?.status === "loading") return;

  set(
    treeChildrenAtom,
    withChild(children, path, {
      status: "loading",
      entries: children.get(path)?.entries ?? [],
    }),
  );

  try {
    const view = await commands.readDirectory(path);
    const entries = view.entries
      .filter((entry) => entry.kind === "directory")
      .sort((a, b) => NAME_COLLATOR.compare(a.name, b.name));
    set(treeChildrenAtom, withChild(get(treeChildrenAtom), path, { status: "ready", entries }));
  } catch (error) {
    console.warn(`Unable to list tree children for ${path}`, error);
    set(treeChildrenAtom, withChild(get(treeChildrenAtom), path, { status: "error", entries: [] }));
  }
});

function withChild(
  children: ReadonlyMap<string, TreeChildrenState>,
  path: string,
  state: TreeChildrenState,
): ReadonlyMap<string, TreeChildrenState> {
  const next = new Map(children);
  next.set(path, state);
  return next;
}

/** User-initiated toggle: collapses instantly, re-reads children on expand so
 *  the tree stays fresh without a dedicated directory watcher. */
export const toggleTreeNodeAtom = atom(null, (get, set, path: string) => {
  const expanded = new Set(get(treeExpandedPathsAtom));
  if (expanded.delete(path)) {
    set(treeExpandedPathsAtom, expanded);
    return;
  }

  expanded.add(path);
  set(treeExpandedPathsAtom, expanded);
  void set(loadTreeChildrenAtom, path);
});

/** Auto-expand (following the active pane's folder): only loads when the node
 *  has never been read or failed before, never overriding fresh data. */
export const ensureTreeNodeExpandedAtom = atom(null, (get, set, path: string) => {
  const expanded = get(treeExpandedPathsAtom);
  if (!expanded.has(path)) {
    set(treeExpandedPathsAtom, new Set(expanded).add(path));
  }

  const state = get(treeChildrenAtom).get(path);
  if (!state || state.status === "error") {
    void set(loadTreeChildrenAtom, path);
  }
});

/** Case- and separator-insensitive containment check. Windows drive roots
 *  arrive as `C:\` while breadcrumbs may report `C:`. */
export function isPathWithin(path: string, root: string): boolean {
  const normalize = (value: string) => value.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  const normalizedRoot = normalize(root);
  if (!normalizedRoot) return false;

  const normalizedPath = normalize(path);
  return normalizedPath === normalizedRoot || normalizedPath.startsWith(`${normalizedRoot}/`);
}

/** Children of `rootPath` rendered as an indented, lazily loaded tree. */
export function FolderTree({
  currentPath,
  onNavigate,
  rootPath,
}: {
  currentPath: string | null;
  onNavigate: (path: string) => void;
  rootPath: string;
}) {
  return (
    <TreeChildrenList currentPath={currentPath} depth={0} onNavigate={onNavigate} path={rootPath} />
  );
}

function TreeChildrenList({
  currentPath,
  depth,
  onNavigate,
  path,
}: {
  currentPath: string | null;
  depth: number;
  onNavigate: (path: string) => void;
  path: string;
}) {
  const { t } = useTranslation("sidebar");
  const state = useAtomValue(treeChildrenAtom).get(path);
  const showHiddenFiles = useAtomValue(showHiddenFilesAtom);
  if (!state) return null;

  const entries = filterHiddenEntries(state.entries, showHiddenFiles);

  if (entries.length === 0) {
    const indent = { paddingLeft: depth * INDENT_PX + 20 };
    if (state.status === "loading") {
      return (
        <div aria-hidden="true" className="flex items-center py-1" style={indent}>
          <span className="h-4 w-24 animate-pulse rounded-xs bg-muted/70" />
        </div>
      );
    }

    return (
      <p className="py-0.5 text-xs text-muted-foreground" style={indent}>
        {t(state.status === "error" ? "tree.loadError" : "tree.empty")}
      </p>
    );
  }

  return entries.map((entry) => (
    <TreeNodeRow
      currentPath={currentPath}
      depth={depth}
      entry={entry}
      key={entry.path}
      onNavigate={onNavigate}
    />
  ));
}

function TreeNodeRow({
  currentPath,
  depth,
  entry,
  onNavigate,
}: {
  currentPath: string | null;
  depth: number;
  entry: DirectoryEntry;
  onNavigate: (path: string) => void;
}) {
  const { t } = useTranslation("sidebar");
  const expanded = useAtomValue(treeExpandedPathsAtom).has(entry.path);
  const state = useAtomValue(treeChildrenAtom).get(entry.path);
  const toggle = useSetAtom(toggleTreeNodeAtom);
  const ensureExpanded = useSetAtom(ensureTreeNodeExpandedAtom);
  const isActive = currentPath === entry.path;
  const rowRef = useRef<HTMLDivElement>(null);

  // Keep the highlighted node in view when navigation happens elsewhere
  // (breadcrumbs, path bar, back/forward).
  useEffect(() => {
    if (isActive) {
      rowRef.current?.scrollIntoView({ block: "nearest" });
    }
  }, [isActive]);

  // Nodes are assumed expandable until their first read proves otherwise.
  const mayHaveChildren = !state || state.status === "loading" || state.entries.length > 0;

  return (
    <FolderContextMenu isListed={false} path={entry.path}>
      <div ref={rowRef}>
        <div className="flex items-center" style={{ paddingLeft: depth * INDENT_PX }}>
          {mayHaveChildren ? (
            <button
              aria-expanded={expanded}
              aria-label={t(expanded ? "tree.collapse" : "tree.expand")}
              className="flex size-5 shrink-0 items-center justify-center rounded-xs text-muted-foreground/80 transition-colors hover:bg-accent/70 hover:text-foreground"
              onClick={() => toggle(entry.path)}
              type="button"
            >
              <CaretDownIcon
                aria-hidden="true"
                className={cn(
                  "size-3 transition-transform duration-fast ease-spring-fast",
                  !expanded && "-rotate-90",
                )}
              />
            </button>
          ) : (
            <span aria-hidden="true" className="size-5 shrink-0" />
          )}
          <button
            aria-current={isActive ? "page" : undefined}
            className={cn(
              "flex min-w-0 flex-1 items-center gap-1.5 rounded-sm py-[3px] pr-1.5 text-left text-[13px] transition-[background-color,color] duration-fast ease-spring-fast hover:bg-accent/70",
              isActive && "bg-selection font-medium text-accent-foreground",
              entry.hidden && "opacity-60",
            )}
            onClick={() => {
              onNavigate(entry.path);
              ensureExpanded(entry.path);
            }}
            title={entry.path}
            type="button"
          >
            <FolderIcon
              className={cn("size-4 shrink-0", isActive ? "text-primary" : "text-muted-foreground")}
              weight={isActive ? "fill" : "regular"}
            />
            <span className="min-w-0 truncate">{entry.name}</span>
          </button>
        </div>
        {expanded && (
          <TreeChildrenList
            currentPath={currentPath}
            depth={depth + 1}
            onNavigate={onNavigate}
            path={entry.path}
          />
        )}
      </div>
    </FolderContextMenu>
  );
}
