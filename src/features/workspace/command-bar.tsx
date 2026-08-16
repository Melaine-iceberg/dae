import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { atom, useAtom, useAtomValue, useSetAtom } from "jotai";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ArrowClockwiseIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  ArrowsDownUpIcon,
  CalendarIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  ClipboardIcon,
  ClipboardTextIcon,
  ClockCounterClockwiseIcon,
  ColumnsIcon,
  CopyIcon,
  FileIcon,
  FilePlusIcon,
  FolderIcon,
  FolderPlusIcon,
  FunnelIcon,
  HouseIcon,
  ListIcon,
  MagnifyingGlassIcon,
  MonitorIcon,
  MoonIcon,
  PencilIcon,
  RowsIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
  SunIcon,
  TextAaIcon,
  TrashIcon,
} from "@phosphor-icons/react";

import type { RecentItem, SearchEntry } from "@/bindings";
import { commands } from "@/bindings";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import {
  DEFAULT_ENTRY_FILTERS,
  DEFAULT_SORT_ORDER,
  densityAtom,
  entryFiltersAtom,
  sortKeyAtom,
  sortOrderAtom,
  viewModeAtom,
  type ExplorerDensity,
  type ExplorerKindFilter,
  type ExplorerSortKey,
} from "@/features/explorer/preferences";
import { activeTabIdAtom, getTabNavigator } from "@/features/explorer/tabs";
import { ensureFavoritesLoadedAtom, favoritesAtom } from "@/features/sidebar/sidebar-atoms";
import {
  dispatchExplorerCommand,
  type ExplorerCommandId,
} from "@/features/workspace/explorer-command-bus";
import {
  ensureRecentsLoadedAtom,
  recentsAtom,
  recordRecentItem,
} from "@/features/workspace/recents-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import {
  activeSurfaceAtom,
  navigateToFolderAtom,
  openSurfaceAtom,
} from "@/features/workspace/workspace-atoms";
import { rankByFuzzy, type RankedResult } from "@/lib/fuzzy";
import { MOD_KEY } from "@/lib/platform";
import { setThemePreference, type ThemePreference } from "@/lib/theme";
import { cn } from "@/lib/utils";

export const commandBarOpenAtom = atom(false);

const MAX_RECENT_ITEMS = 8;
const MAX_FILE_RESULTS = 12;
const FILE_SEARCH_DEBOUNCE_MS = 220;
const MIN_FILE_QUERY_LENGTH = 2;

const GROUP_ORDER = ["搜索", "导航", "空间", "收藏", "最近", "文件", "视图"] as const;
type CommandGroup = (typeof GROUP_ORDER)[number];

interface CommandItem {
  id: string;
  group: CommandGroup;
  label: string;
  hint?: string;
  keywords?: string;
  icon: ComponentType<{ className?: string }>;
  run: () => void;
}

/** Reads the active tab's current directory; null when unavailable. */
function getActiveFolderScope(tabId: string): string | null {
  try {
    return getTabNavigator(tabId).getSnapshot().directory?.path ?? null;
  } catch {
    return null;
  }
}

/**
 * Global command/search surface (SKILL.md §15). Opens with Ctrl/Cmd+K,
 * fuzzy-searches commands, surfaces, favorites and recents, and dispatches
 * file operations to the active explorer through the command bus.
 */
export function CommandBar() {
  const [open, setOpen] = useAtom(commandBarOpenAtom);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [fileResults, setFileResults] = useState<SearchEntry[]>([]);
  const [isSearchingFiles, setIsSearchingFiles] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const favorites = useAtomValue(favoritesAtom) ?? [];
  const recents = useAtomValue(recentsAtom) ?? [];
  const spaces = useAtomValue(spacesAtom) ?? [];
  const activeSurface = useAtomValue(activeSurfaceAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const ensureRecentsLoaded = useSetAtom(ensureRecentsLoadedAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openSurface = useSetAtom(openSurfaceAtom);
  const setDensity = useSetAtom(densityAtom);
  const setViewMode = useSetAtom(viewModeAtom);
  const setSortKey = useSetAtom(sortKeyAtom);
  const setSortOrder = useSetAtom(sortOrderAtom);
  const setEntryFilters = useSetAtom(entryFiltersAtom);

  useEffect(() => {
    if (!open) return;

    setQuery("");
    setActiveIndex(0);
    setFileResults([]);
    setIsSearchingFiles(false);
    inputRef.current?.focus();
    void ensureFavoritesLoaded();
    void ensureRecentsLoaded();
    void ensureSpacesLoaded();
  }, [ensureFavoritesLoaded, ensureRecentsLoaded, ensureSpacesLoaded, open]);

  // Cancel the backend traversal whenever the surface closes or unmounts.
  useEffect(
    () => () => {
      void commands.cancelSearch().catch(() => undefined);
    },
    [],
  );

  const folderActive = activeSurface.kind === "folder";

  /**
   * Progressive file search (SKILL.md §16): scoped to the active tab's
   * directory on folder surfaces, falling back to the home directory.
   * Debounced; a newer query or dismissal cancels the older traversal
   * through the backend's search generation.
   */
  useEffect(() => {
    if (!open || query.trim().length < MIN_FILE_QUERY_LENGTH) {
      setFileResults([]);
      setIsSearchingFiles(false);
      return;
    }

    let cancelled = false;
    const trimmedQuery = query.trim();

    const resolveScope = folderActive
      ? Promise.resolve(getActiveFolderScope(activeTabId))
      : commands.getHomeDirectory().catch(() => null);

    const timeout = window.setTimeout(() => {
      void resolveScope.then((scope) => {
        if (cancelled || !scope) return;

        setIsSearchingFiles(true);
        void commands
          .searchDirectory(scope, trimmedQuery)
          .then((response) => {
            if (!cancelled) setFileResults(response.entries);
          })
          .catch(() => {
            if (!cancelled) setFileResults([]);
          })
          .finally(() => {
            if (!cancelled) setIsSearchingFiles(false);
          });
      });
    }, FILE_SEARCH_DEBOUNCE_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
      void commands.cancelSearch().catch(() => undefined);
    };
  }, [activeTabId, folderActive, open, query]);

  const items = useMemo<CommandItem[]>(() => {
    const openRecentItem = (recent: RecentItem) => {
      recordRecentItem(recent.path, recent.kind, "opened");
      if (recent.kind === "directory") {
        navigateToFolder(recent.path);
        return;
      }

      void openPath(recent.path).catch((error) => {
        console.warn(`Unable to open ${recent.path}`, error);
      });
    };

    const surfaceItems: CommandItem[] = [
      {
        id: "surface:overview",
        group: "导航",
        label: "前往概览",
        keywords: "overview home start",
        icon: HouseIcon,
        run: () => openSurface({ kind: "overview" }),
      },
      {
        id: "surface:recents",
        group: "导航",
        label: "前往最近使用",
        keywords: "recent history",
        icon: ClockCounterClockwiseIcon,
        run: () => openSurface({ kind: "recents" }),
      },
      {
        id: "surface:favorites",
        group: "导航",
        label: "前往收藏",
        keywords: "favorites starred",
        icon: StarIcon,
        run: () => openSurface({ kind: "favorites" }),
      },
    ];

    const spaceItems: CommandItem[] = spaces.map((space) => ({
      id: `space:${space.id}`,
      group: "空间",
      label: `打开空间「${space.name}」`,
      keywords: "space workspace open",
      icon: SquaresFourIcon,
      run: () => openSurface({ kind: "space", spaceId: space.id }),
    }));

    const favoriteItems: CommandItem[] = favorites.map((favorite) => ({
      id: `favorite:${favorite.path}`,
      group: "收藏",
      label: favorite.name,
      hint: favorite.path,
      keywords: "favorite open folder",
      icon: FolderIcon,
      run: () => navigateToFolder(favorite.path),
    }));

    const recentItems: CommandItem[] = recents.slice(0, MAX_RECENT_ITEMS).map((recent) => ({
      id: `recent:${recent.path}`,
      group: "最近",
      label: recent.name,
      hint: recent.path,
      keywords: "recent open",
      icon: recent.kind === "directory" ? FolderIcon : FileIcon,
      run: () => openRecentItem(recent),
    }));

    const explorerCommands: ReadonlyArray<{
      id: string;
      label: string;
      hint?: string;
      keywords: string;
      icon: ComponentType<{ className?: string }>;
      command: ExplorerCommandId;
    }> = [
      {
        id: "create-folder",
        label: "新建文件夹",
        keywords: "new create folder directory",
        icon: FolderPlusIcon,
        command: "create-folder",
      },
      {
        id: "create-file",
        label: "新建文件",
        keywords: "new create file",
        icon: FilePlusIcon,
        command: "create-file",
      },
      {
        id: "rename",
        label: "重命名",
        hint: "F2",
        keywords: "rename",
        icon: PencilIcon,
        command: "rename",
      },
      {
        id: "delete",
        label: "删除",
        hint: "Delete",
        keywords: "delete remove trash",
        icon: TrashIcon,
        command: "delete",
      },
      {
        id: "copy",
        label: "复制",
        hint: `${MOD_KEY}+C`,
        keywords: "copy",
        icon: CopyIcon,
        command: "copy",
      },
      {
        id: "cut",
        label: "剪切",
        hint: `${MOD_KEY}+X`,
        keywords: "cut move",
        icon: ScissorsIcon,
        command: "cut",
      },
      {
        id: "paste",
        label: "粘贴",
        hint: `${MOD_KEY}+V`,
        keywords: "paste",
        icon: ClipboardIcon,
        command: "paste",
      },
      {
        id: "copy-paths",
        label: "复制文件地址",
        keywords: "copy path clipboard location",
        icon: ClipboardTextIcon,
        command: "copy-paths",
      },
      {
        id: "select-all",
        label: "全选",
        hint: `${MOD_KEY}+A`,
        keywords: "select all",
        icon: CheckCircleIcon,
        command: "select-all",
      },
      {
        id: "refresh",
        label: "刷新",
        keywords: "refresh reload",
        icon: ArrowClockwiseIcon,
        command: "refresh",
      },
      {
        id: "go-back",
        label: "后退",
        keywords: "back history navigate",
        icon: ArrowLeftIcon,
        command: "go-back",
      },
      {
        id: "go-forward",
        label: "前进",
        keywords: "forward history navigate",
        icon: ArrowRightIcon,
        command: "go-forward",
      },
      {
        id: "go-up",
        label: "上一级",
        keywords: "up parent navigate",
        icon: ArrowUpIcon,
        command: "go-up",
      },
      {
        id: "toggle-favorite",
        label: "收藏或取消收藏当前文件夹",
        keywords: "favorite star toggle folder",
        icon: StarIcon,
        command: "toggle-favorite",
      },
    ];

    const explorerCommandItems: CommandItem[] = folderActive
      ? explorerCommands.map((entry) => ({
          id: `cmd:${entry.id}`,
          group: "文件",
          label: entry.label,
          hint: entry.hint,
          keywords: entry.keywords,
          icon: entry.icon,
          run: () => dispatchExplorerCommand(entry.command),
        }))
      : [];

    const viewItems: CommandItem[] = [
      {
        id: "view:list",
        group: "视图",
        label: "切换到列表视图",
        keywords: "view list mode",
        icon: ListIcon,
        run: () => setViewMode("list"),
      },
      {
        id: "view:grid",
        group: "视图",
        label: "切换到网格视图",
        keywords: "view grid mode",
        icon: SquaresFourIcon,
        run: () => setViewMode("grid"),
      },
      {
        id: "view:column",
        group: "视图",
        label: "切换到分栏视图",
        keywords: "view column miller mode",
        icon: ColumnsIcon,
        run: () => setViewMode("column"),
      },
      ...(
        [
          { key: "name", label: "按名称排序", icon: TextAaIcon },
          { key: "modified", label: "按修改日期排序", icon: CalendarIcon },
          { key: "type", label: "按类型排序", icon: FileIcon },
          { key: "size", label: "按大小排序", icon: ArrowsDownUpIcon },
        ] as ReadonlyArray<{ icon: ComponentType<{ className?: string }>; key: ExplorerSortKey; label: string }>
      ).map<CommandItem>((entry) => ({
        id: `sort:${entry.key}`,
        group: "视图",
        label: entry.label,
        keywords: "sort order arrange",
        icon: entry.icon,
        run: () => {
          setSortKey(entry.key);
          setSortOrder(DEFAULT_SORT_ORDER[entry.key]);
        },
      })),
      {
        id: "sort:toggle-order",
        group: "视图",
        label: "切换升序/降序",
        keywords: "sort order ascending descending toggle",
        icon: ArrowsDownUpIcon,
        run: () => setSortOrder((order) => (order === "asc" ? "desc" : "asc")),
      },
      ...(
        [
          { value: "all", label: "类型过滤：全部" },
          { value: "folders", label: "类型过滤：仅文件夹" },
          { value: "files", label: "类型过滤：仅文件" },
          { value: "images", label: "类型过滤：仅图片" },
        ] as ReadonlyArray<{ label: string; value: ExplorerKindFilter }>
      ).map<CommandItem>((entry) => ({
        id: `filter:kind:${entry.value}`,
        group: "视图",
        label: entry.label,
        keywords: "filter kind type",
        icon: FunnelIcon,
        run: () => setEntryFilters((filters) => ({ ...filters, kind: entry.value })),
      })),
      {
        id: "filter:clear",
        group: "视图",
        label: "清除所有过滤器",
        keywords: "filter clear reset",
        icon: FunnelIcon,
        run: () => setEntryFilters(DEFAULT_ENTRY_FILTERS),
      },
      ...(
        [
          { icon: SunIcon, label: "主题：浅色", value: "light" },
          { icon: MoonIcon, label: "主题：深色", value: "dark" },
          { icon: MonitorIcon, label: "主题：跟随系统", value: "system" },
        ] as ReadonlyArray<{
          icon: ComponentType<{ className?: string }>;
          label: string;
          value: ThemePreference;
        }>
      ).map<CommandItem>((entry) => ({
        id: `theme:${entry.value}`,
        group: "视图",
        label: entry.label,
        keywords: "theme appearance light dark system",
        icon: entry.icon,
        run: () => setThemePreference(entry.value),
      })),
      {
        id: "density:compact",
        group: "视图",
        label: "紧凑密度",
        keywords: "density compact rows",
        icon: RowsIcon,
        run: () => setDensity("compact" satisfies ExplorerDensity),
      },
      {
        id: "density:comfortable",
        group: "视图",
        label: "舒适密度",
        keywords: "density comfortable rows",
        icon: RowsIcon,
        run: () => setDensity("comfortable" satisfies ExplorerDensity),
      },
      {
        id: "density:spacious",
        group: "视图",
        label: "宽松密度",
        keywords: "density spacious rows",
        icon: RowsIcon,
        run: () => setDensity("spacious" satisfies ExplorerDensity),
      },
    ];

    return [
      ...surfaceItems,
      ...spaceItems,
      ...favoriteItems,
      ...recentItems,
      ...explorerCommandItems,
      ...viewItems,
    ];
  }, [
    favorites,
    folderActive,
    navigateToFolder,
    openSurface,
    recents,
    setDensity,
    setEntryFilters,
    setSortKey,
    setSortOrder,
    setViewMode,
    spaces,
  ]);

  const trimmedQuery = query.trim();

  const fileResultItems = useMemo<CommandItem[]>(() => {
    const openSearchEntry = (entry: SearchEntry) => {
      recordRecentItem(entry.path, entry.kind, "opened");
      if (entry.kind === "directory") {
        navigateToFolder(entry.path);
        return;
      }

      void openPath(entry.path).catch((error) => {
        console.warn(`Unable to open ${entry.path}`, error);
      });
    };

    return fileResults.slice(0, MAX_FILE_RESULTS).map((entry) => ({
      id: `file:${entry.path}`,
      group: "搜索",
      label: entry.name,
      hint: entry.relativePath,
      keywords: "file search",
      icon: entry.kind === "directory" ? FolderIcon : FileIcon,
      run: () => openSearchEntry(entry),
    }));
  }, [fileResults, navigateToFolder]);

  const results = useMemo<RankedResult<CommandItem>[]>(
    () =>
      rankByFuzzy(trimmedQuery, [...items, ...fileResultItems], (item) =>
        `${item.label} ${item.keywords ?? ""} ${item.hint ?? ""}`.trim(),
      ),
    [fileResultItems, items, trimmedQuery],
  );

  // Grouped sections for the unfiltered list; the flattened order still
  // drives keyboard navigation.
  const renderGroups = useMemo(() => {
    if (trimmedQuery) return null;

    const groups: {
      group: CommandGroup;
      startIndex: number;
      entries: RankedResult<CommandItem>[];
    }[] = [];

    results.forEach((result, index) => {
      const last = groups.at(-1);
      if (last?.group === result.item.group) {
        last.entries.push(result);
      } else {
        groups.push({ group: result.item.group, startIndex: index, entries: [result] });
      }
    });

    return groups;
  }, [results, trimmedQuery]);

  const currentIndex = Math.min(activeIndex, Math.max(results.length - 1, 0));

  useEffect(() => {
    setActiveIndex(0);
  }, [trimmedQuery]);

  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-command-index="${currentIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [currentIndex, results.length]);

  const runCommand = (item: CommandItem) => {
    setOpen(false);
    item.run();
  };

  const handleInputKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (results.length === 0) return;

      const delta = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + delta + results.length) % results.length);
      return;
    }

    if (event.key === "Home") {
      event.preventDefault();
      setActiveIndex(0);
      return;
    }

    if (event.key === "End") {
      event.preventDefault();
      setActiveIndex(results.length - 1);
      return;
    }

    if (event.key === "Enter") {
      event.preventDefault();
      const entry = results[currentIndex];
      if (entry) runCommand(entry.item);
    }
  };

  return (
    <Dialog onOpenChange={setOpen} open={open}>
      <DialogContent
        className="top-20 w-full min-w-0 max-w-[min(36rem,calc(100%-2rem))] translate-y-0 gap-0 overflow-hidden rounded-2xl p-0 sm:max-w-[min(36rem,calc(100%-2rem))]"
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">命令栏</DialogTitle>
        <div className="flex items-center gap-2.5 border-b px-3.5">
          <MagnifyingGlassIcon className="size-4 shrink-0 text-muted-foreground" />
          <input
            aria-activedescendant={results.length > 0 ? `command-item-${currentIndex}` : undefined}
            aria-autocomplete="list"
            aria-controls="command-bar-results"
            aria-expanded="true"
            aria-label="搜索命令与位置"
            autoComplete="off"
            className="h-11 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            id="command-bar-input"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder="搜索命令、空间、收藏或位置…"
            ref={inputRef}
            role="combobox"
            spellCheck={false}
            type="text"
            value={query}
          />
          <kbd className="shrink-0 rounded-xs border bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
            {MOD_KEY} K
          </kbd>
        </div>
        <div
          aria-label="命令结果"
          className="max-h-80 overflow-y-auto overscroll-contain p-1.5"
          id="command-bar-results"
          ref={listRef}
          role="listbox"
        >
          {results.length === 0 ? (
            isSearchingFiles ? (
              <p className="px-2.5 py-6 text-center text-[13px] text-muted-foreground">
                正在搜索文件…
              </p>
            ) : (
              <p className="px-2.5 py-6 text-center text-[13px] text-muted-foreground">
                没有匹配“{trimmedQuery}”的命令或位置
              </p>
            )
          ) : renderGroups ? (
            renderGroups.map((section) => (
              <div key={section.group}>
                <p
                  aria-hidden="true"
                  className="px-3.5 pt-2 pb-1 text-xs font-medium text-muted-foreground select-none"
                >
                  {section.group}
                </p>
                {section.entries.map((entry, localIndex) => (
                  <CommandResultRow
                    dataIndex={section.startIndex + localIndex}
                    isActive={section.startIndex + localIndex === currentIndex}
                    item={entry.item}
                    key={entry.item.id}
                    matchedIndices={entry.matchedIndices}
                    onSelect={() => runCommand(entry.item)}
                  />
                ))}
              </div>
            ))
          ) : (
            results.map((entry, index) => (
              <CommandResultRow
                dataIndex={index}
                isActive={index === currentIndex}
                item={entry.item}
                key={entry.item.id}
                matchedIndices={entry.matchedIndices}
                onSelect={() => runCommand(entry.item)}
              />
            ))
          )}
        </div>
        <footer className="flex h-8 shrink-0 items-center justify-between border-t px-3.5 text-xs text-muted-foreground select-none">
          <span className="flex items-center gap-1.5">
            {isSearchingFiles && <CircleNotchIcon className="size-3 animate-spin" />}
            ↑↓ 选择
          </span>
          <span>Enter 执行 · Esc 关闭</span>
        </footer>
      </DialogContent>
    </Dialog>
  );
}

function CommandResultRow({
  dataIndex,
  isActive,
  item,
  matchedIndices,
  onSelect,
}: {
  dataIndex: number;
  isActive: boolean;
  item: CommandItem;
  matchedIndices: number[];
  onSelect: () => void;
}) {
  return (
    <button
      aria-selected={isActive}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-full px-3 py-2 text-left text-[13px] transition-colors outline-none",
        isActive ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
      )}
      data-command-index={dataIndex}
      id={`command-item-${dataIndex}`}
      onClick={onSelect}
      role="option"
      tabIndex={-1}
      type="button"
    >
      <item.icon className="size-4 shrink-0 text-muted-foreground" />
      <HighlightedLabel label={item.label} matchedIndices={matchedIndices} />
      {item.hint && (
        <span
          className="max-w-[45%] shrink-0 truncate text-xs text-muted-foreground"
          title={item.hint}
        >
          {item.hint}
        </span>
      )}
    </button>
  );
}

function HighlightedLabel({ label, matchedIndices }: { label: string; matchedIndices: number[] }) {
  if (matchedIndices.length === 0) {
    return <span className="min-w-0 flex-1 truncate">{label}</span>;
  }

  const matches = new Set(matchedIndices);

  return (
    <span className="min-w-0 flex-1 truncate">
      {Array.from(label, (char, index) =>
        matches.has(index) ? (
          <mark className="bg-transparent font-semibold text-foreground" key={index}>
            {char}
          </mark>
        ) : (
          <span key={index}>{char}</span>
        ),
      )}
    </span>
  );
}
