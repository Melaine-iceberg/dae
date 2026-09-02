import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
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
  EyeIcon,
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
  TerminalIcon,
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
  foldersFirstAtom,
  showHiddenFilesAtom,
  sortKeyAtom,
  sortOrderAtom,
  viewModeAtom,
  type ExplorerDensity,
  type ExplorerKindFilter,
  type ExplorerSortKey,
} from "@/features/explorer/preferences";
import { activePaneNavigatorAtom } from "@/features/explorer/tabs";
import type { ExplorerNavigator } from "@/features/explorer/navigation";
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
import { getSpaceDisplayName } from "@/features/workspace/types";
import {
  activeSurfaceAtom,
  navigateToFolderAtom,
  openSurfaceAtom,
} from "@/features/workspace/workspace-atoms";
import { fuzzyMatch, rankByFuzzy, type RankedResult } from "@/lib/fuzzy";
import { MOD_KEY } from "@/lib/platform";
import { setThemePreference, type ThemePreference } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { commandBarModeAtom, commandBarOpenAtom } from "@/features/workspace/command-bar-atoms";

const MAX_RECENT_ITEMS = 8;
const MAX_PATH_RECENT_ITEMS = 12;
const MAX_FILE_RESULTS = 12;
const FILE_SEARCH_DEBOUNCE_MS = 220;
const MIN_FILE_QUERY_LENGTH = 2;

/** Absolute-path shapes: drive letter, home alias, UNC share, POSIX root. */
const PATH_LIKE_PATTERN = /^([a-zA-Z]:[\\/]|~(?=$|[\\/])|\\\\|\/)/;

const GROUP_ORDER = [
  "path",
  "search",
  "navigation",
  "spaces",
  "favorites",
  "recents",
  "files",
  "view",
] as const;
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

/** Reads the focused pane's current directory; null when unavailable. */
function getActiveFolderScope(navigator: ExplorerNavigator): string | null {
  try {
    return navigator.getSnapshot().directory?.path ?? null;
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
  const { t } = useTranslation("workspace");
  const [open, setOpen] = useAtom(commandBarOpenAtom);
  const mode = useAtomValue(commandBarModeAtom);
  const pathMode = mode === "path";
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
  const scopeNavigator = useAtomValue(activePaneNavigatorAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const ensureRecentsLoaded = useSetAtom(ensureRecentsLoadedAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openSurface = useSetAtom(openSurfaceAtom);
  const setDensity = useSetAtom(densityAtom);
  const setViewMode = useSetAtom(viewModeAtom);
  const setSortKey = useSetAtom(sortKeyAtom);
  const setSortOrder = useSetAtom(sortOrderAtom);
  const setFoldersFirst = useSetAtom(foldersFirstAtom);
  const setShowHiddenFiles = useSetAtom(showHiddenFilesAtom);
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
  }, [ensureFavoritesLoaded, ensureRecentsLoaded, ensureSpacesLoaded, mode, open]);

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
    // Path-like input in path mode is a jump target, not a name query —
    // skip the traversal entirely.
    if (
      !open ||
      query.trim().length < MIN_FILE_QUERY_LENGTH ||
      (pathMode && PATH_LIKE_PATTERN.test(query.trim()))
    ) {
      setFileResults([]);
      setIsSearchingFiles(false);
      return;
    }

    let cancelled = false;
    const trimmedQuery = query.trim();

    const resolveScope = folderActive
      ? Promise.resolve(getActiveFolderScope(scopeNavigator))
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
  }, [folderActive, open, pathMode, query, scopeNavigator]);

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
        group: "navigation",
        label: t("commandBar.navigation.overview"),
        keywords: "overview home start",
        icon: HouseIcon,
        run: () => openSurface({ kind: "overview" }),
      },
      {
        id: "surface:recents",
        group: "navigation",
        label: t("commandBar.navigation.recents"),
        keywords: "recent history",
        icon: ClockCounterClockwiseIcon,
        run: () => openSurface({ kind: "recents" }),
      },
      {
        id: "surface:favorites",
        group: "navigation",
        label: t("commandBar.navigation.favorites"),
        keywords: "favorites starred",
        icon: StarIcon,
        run: () => openSurface({ kind: "favorites" }),
      },
      {
        id: "surface:trash",
        group: "navigation",
        label: t("commandBar.navigation.trash"),
        keywords: "trash recycle bin deleted restore",
        icon: TrashIcon,
        run: () => openSurface({ kind: "trash" }),
      },
    ];

    const spaceItems: CommandItem[] = spaces.map((space) => ({
      id: `space:${space.id}`,
      group: "spaces",
      label: t("commandBar.openSpace", { name: getSpaceDisplayName(space) }),
      keywords: "space workspace open",
      icon: SquaresFourIcon,
      run: () => openSurface({ kind: "space", spaceId: space.id }),
    }));

    const favoriteItems: CommandItem[] = favorites.map((favorite) => ({
      id: `favorite:${favorite.path}`,
      group: "favorites",
      label: favorite.name,
      hint: favorite.path,
      keywords: "favorite open folder",
      icon: FolderIcon,
      run: () => navigateToFolder(favorite.path),
    }));

    const recentItems: CommandItem[] = recents
      // Path mode jumps to folders; files are reachable through file search.
      .filter((recent) => !pathMode || recent.kind === "directory")
      .slice(0, pathMode ? MAX_PATH_RECENT_ITEMS : MAX_RECENT_ITEMS)
      .map((recent) => ({
        id: `recent:${recent.path}`,
        group: "recents",
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
        label: t("commandBar.commands.createFolder"),
        keywords: "new create folder directory",
        icon: FolderPlusIcon,
        command: "create-folder",
      },
      {
        id: "create-file",
        label: t("commandBar.commands.createFile"),
        keywords: "new create file",
        icon: FilePlusIcon,
        command: "create-file",
      },
      {
        id: "rename",
        label: t("commandBar.commands.rename"),
        hint: "F2",
        keywords: "rename",
        icon: PencilIcon,
        command: "rename",
      },
      {
        id: "delete",
        label: t("commandBar.commands.delete"),
        hint: "Delete",
        keywords: "delete remove trash",
        icon: TrashIcon,
        command: "delete",
      },
      {
        id: "copy",
        label: t("commandBar.commands.copy"),
        hint: `${MOD_KEY}+C`,
        keywords: "copy",
        icon: CopyIcon,
        command: "copy",
      },
      {
        id: "cut",
        label: t("commandBar.commands.cut"),
        hint: `${MOD_KEY}+X`,
        keywords: "cut move",
        icon: ScissorsIcon,
        command: "cut",
      },
      {
        id: "paste",
        label: t("commandBar.commands.paste"),
        hint: `${MOD_KEY}+V`,
        keywords: "paste",
        icon: ClipboardIcon,
        command: "paste",
      },
      {
        id: "copy-paths",
        label: t("commandBar.commands.copyPaths"),
        keywords: "copy path clipboard location",
        icon: ClipboardTextIcon,
        command: "copy-paths",
      },
      {
        id: "select-all",
        label: t("commandBar.commands.selectAll"),
        hint: `${MOD_KEY}+A`,
        keywords: "select all",
        icon: CheckCircleIcon,
        command: "select-all",
      },
      {
        id: "refresh",
        label: t("commandBar.commands.refresh"),
        keywords: "refresh reload",
        icon: ArrowClockwiseIcon,
        command: "refresh",
      },
      {
        id: "go-back",
        label: t("commandBar.commands.goBack"),
        keywords: "back history navigate",
        icon: ArrowLeftIcon,
        command: "go-back",
      },
      {
        id: "go-forward",
        label: t("commandBar.commands.goForward"),
        keywords: "forward history navigate",
        icon: ArrowRightIcon,
        command: "go-forward",
      },
      {
        id: "go-up",
        label: t("commandBar.commands.goUp"),
        keywords: "up parent navigate",
        icon: ArrowUpIcon,
        command: "go-up",
      },
      {
        id: "open-terminal",
        label: t("commandBar.commands.openTerminal"),
        hint: `${MOD_KEY}+\``,
        keywords: "terminal shell console open external",
        icon: TerminalIcon,
        command: "open-terminal",
      },
      {
        id: "toggle-favorite",
        label: t("commandBar.commands.toggleFavorite"),
        keywords: "favorite star toggle folder",
        icon: StarIcon,
        command: "toggle-favorite",
      },
      {
        id: "toggle-split",
        label: t("commandBar.commands.toggleSplitView"),
        hint: "F6",
        keywords: "split dual pane panel column view",
        icon: ColumnsIcon,
        command: "toggle-split",
      },
    ];

    const explorerCommandItems: CommandItem[] = folderActive
      ? explorerCommands.map((entry) => ({
          id: `cmd:${entry.id}`,
          group: "files",
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
        group: "view",
        label: t("commandBar.view.switchToList"),
        keywords: "view list mode",
        icon: ListIcon,
        run: () => setViewMode("list"),
      },
      {
        id: "view:grid",
        group: "view",
        label: t("commandBar.view.switchToGrid"),
        keywords: "view grid mode",
        icon: SquaresFourIcon,
        run: () => setViewMode("grid"),
      },
      {
        id: "view:column",
        group: "view",
        label: t("commandBar.view.switchToColumn"),
        keywords: "view column miller mode",
        icon: ColumnsIcon,
        run: () => setViewMode("column"),
      },
      ...(
        [
          { key: "name", label: t("commandBar.view.sortByName"), icon: TextAaIcon },
          { key: "modified", label: t("commandBar.view.sortByModified"), icon: CalendarIcon },
          { key: "type", label: t("commandBar.view.sortByType"), icon: FileIcon },
          { key: "size", label: t("commandBar.view.sortBySize"), icon: ArrowsDownUpIcon },
        ] as ReadonlyArray<{
          icon: ComponentType<{ className?: string }>;
          key: ExplorerSortKey;
          label: string;
        }>
      ).map<CommandItem>((entry) => ({
        id: `sort:${entry.key}`,
        group: "view",
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
        group: "view",
        label: t("commandBar.view.toggleSortOrder"),
        keywords: "sort order ascending descending toggle",
        icon: ArrowsDownUpIcon,
        run: () => setSortOrder((order) => (order === "asc" ? "desc" : "asc")),
      },
      {
        id: "sort:toggle-folders-first",
        group: "view",
        label: t("commandBar.view.toggleFoldersFirst"),
        keywords: "sort folders first directories group top",
        icon: FolderIcon,
        run: () => setFoldersFirst((enabled) => !enabled),
      },
      {
        id: "view:toggle-hidden-files",
        group: "view",
        label: t("commandBar.view.toggleHiddenFiles"),
        keywords: "hidden files dotfiles visibility toggle",
        icon: EyeIcon,
        run: () => setShowHiddenFiles((visible) => !visible),
      },
      ...(
        [
          { value: "all", label: t("commandBar.view.filterAll") },
          { value: "folders", label: t("commandBar.view.filterFolders") },
          { value: "files", label: t("commandBar.view.filterFiles") },
          { value: "images", label: t("commandBar.view.filterImages") },
        ] as ReadonlyArray<{ label: string; value: ExplorerKindFilter }>
      ).map<CommandItem>((entry) => ({
        id: `filter:kind:${entry.value}`,
        group: "view",
        label: entry.label,
        keywords: "filter kind type",
        icon: FunnelIcon,
        run: () => setEntryFilters((filters) => ({ ...filters, kind: entry.value })),
      })),
      {
        id: "filter:clear",
        group: "view",
        label: t("commandBar.view.clearFilters"),
        keywords: "filter clear reset",
        icon: FunnelIcon,
        run: () => setEntryFilters(DEFAULT_ENTRY_FILTERS),
      },
      ...(
        [
          { icon: SunIcon, label: t("commandBar.view.themeLight"), value: "light" },
          { icon: MoonIcon, label: t("commandBar.view.themeDark"), value: "dark" },
          { icon: MonitorIcon, label: t("commandBar.view.themeSystem"), value: "system" },
        ] as ReadonlyArray<{
          icon: ComponentType<{ className?: string }>;
          label: string;
          value: ThemePreference;
        }>
      ).map<CommandItem>((entry) => ({
        id: `theme:${entry.value}`,
        group: "view",
        label: entry.label,
        keywords: "theme appearance light dark system",
        icon: entry.icon,
        run: () => setThemePreference(entry.value),
      })),
      {
        id: "density:compact",
        group: "view",
        label: t("commandBar.view.densityCompact"),
        keywords: "density compact rows",
        icon: RowsIcon,
        run: () => setDensity("compact" satisfies ExplorerDensity),
      },
      {
        id: "density:comfortable",
        group: "view",
        label: t("commandBar.view.densityComfortable"),
        keywords: "density comfortable rows",
        icon: RowsIcon,
        run: () => setDensity("comfortable" satisfies ExplorerDensity),
      },
      {
        id: "density:spacious",
        group: "view",
        label: t("commandBar.view.densitySpacious"),
        keywords: "density spacious rows",
        icon: RowsIcon,
        run: () => setDensity("spacious" satisfies ExplorerDensity),
      },
    ];

    // Path mode is a jump list: favorites + recent directories only. Commands,
    // surfaces and view toggles stay in the Ctrl/Cmd+K mode.
    if (pathMode) {
      return [...favoriteItems, ...recentItems];
    }

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
    pathMode,
    recents,
    setDensity,
    setEntryFilters,
    setFoldersFirst,
    setShowHiddenFiles,
    setSortKey,
    setSortOrder,
    setViewMode,
    spaces,
    t,
  ]);

  const trimmedQuery = query.trim();

  // Path mode's headline feature: an absolute path in the input becomes a
  // direct jump target, with `~` expanded against the home directory.
  const directPathItem = useMemo<CommandItem | null>(() => {
    if (!pathMode || !PATH_LIKE_PATTERN.test(trimmedQuery)) return null;

    const target = trimmedQuery;
    return {
      id: `path:${target}`,
      group: "path",
      label: t("commandBar.jumpToPath", { path: target }),
      hint: target,
      icon: ArrowRightIcon,
      run: () => {
        void (async () => {
          let resolved = target;
          if (resolved.startsWith("~")) {
            const home = await commands.getHomeDirectory().catch(() => null);
            if (!home) return;
            // Expand `~` against the home directory and normalize separators
            // to the platform's own (`~/project` → `C:\Users\me\project`).
            const separator = home.includes("\\") ? "\\" : "/";
            resolved = home + resolved.slice(1).replace(/[\\/]+/g, separator);
          }
          recordRecentItem(resolved, "directory", "opened");
          navigateToFolder(resolved);
        })();
      },
    };
  }, [navigateToFolder, pathMode, t, trimmedQuery]);

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

    return fileResults
      .filter((entry) => !pathMode || entry.kind === "directory")
      .slice(0, MAX_FILE_RESULTS)
      .map((entry) => ({
        id: `file:${entry.path}`,
        group: "search",
        label: entry.name,
        hint: entry.relativePath,
        keywords: "file search",
        icon: entry.kind === "directory" ? FolderIcon : FileIcon,
        run: () => openSearchEntry(entry),
      }));
  }, [fileResults, navigateToFolder, pathMode]);

  const results = useMemo<RankedResult<CommandItem>[]>(() => {
    const ranked = rankByFuzzy(trimmedQuery, [...items, ...fileResultItems], (item) =>
      `${item.label} ${item.keywords ?? ""} ${item.hint ?? ""}`.trim(),
    );

    // The direct jump target always leads the list; everything else ranks
    // fuzzily beneath it.
    if (!directPathItem) return ranked;

    const jumpMatch = fuzzyMatch(trimmedQuery, directPathItem.label);
    return [
      {
        item: directPathItem,
        score: Number.POSITIVE_INFINITY,
        matchedIndices: jumpMatch?.matchedIndices ?? [],
      },
      ...ranked,
    ];
  }, [directPathItem, fileResultItems, items, trimmedQuery]);

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

  const groupLabels: Record<CommandGroup, string> = {
    path: t("commandBar.groups.path"),
    search: t("commandBar.groups.search"),
    navigation: t("commandBar.groups.navigation"),
    spaces: t("commandBar.groups.spaces"),
    favorites: t("commandBar.groups.favorites"),
    recents: t("commandBar.groups.recents"),
    files: t("commandBar.groups.files"),
    view: t("commandBar.groups.view"),
  };

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
        className="top-20 w-full min-w-0 max-w-[min(36rem,calc(100%-2rem))] translate-y-0 gap-0 overflow-hidden rounded-xl p-0 sm:max-w-[min(36rem,calc(100%-2rem))]"
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">{t("commandBar.title")}</DialogTitle>
        <div className="flex items-center gap-2.5 border-b px-3.5">
          <MagnifyingGlassIcon className="size-4 shrink-0 text-muted-foreground" />
          <input
            aria-activedescendant={results.length > 0 ? `command-item-${currentIndex}` : undefined}
            aria-autocomplete="list"
            aria-controls="command-bar-results"
            aria-expanded="true"
            aria-label={t("commandBar.inputAriaLabel")}
            autoComplete="off"
            className="h-11 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            id="command-bar-input"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder={pathMode ? t("commandBar.pathPlaceholder") : t("commandBar.placeholder")}
            ref={inputRef}
            role="combobox"
            spellCheck={false}
            type="text"
            value={query}
          />
          <kbd className="shrink-0 rounded-xs border bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
            {MOD_KEY} {pathMode ? "P" : "K"}
          </kbd>
        </div>
        <div
          aria-label={t("commandBar.resultsAriaLabel")}
          className="max-h-80 overflow-y-auto overscroll-contain p-1.5"
          id="command-bar-results"
          ref={listRef}
          role="listbox"
        >
          {results.length === 0 ? (
            isSearchingFiles ? (
              <p className="px-2.5 py-6 text-center text-[13px] text-muted-foreground">
                {pathMode ? t("commandBar.searchingFolders") : t("commandBar.searchingFiles")}
              </p>
            ) : pathMode && !trimmedQuery ? (
              <p className="px-2.5 py-6 text-center text-[13px] text-muted-foreground">
                {t("commandBar.noPathLocations")}
              </p>
            ) : (
              <p className="px-2.5 py-6 text-center text-[13px] text-muted-foreground">
                {pathMode
                  ? t("commandBar.noPathResults", { query: trimmedQuery })
                  : t("commandBar.noResults", { query: trimmedQuery })}
              </p>
            )
          ) : renderGroups ? (
            renderGroups.map((section) => (
              <div key={section.group}>
                <p
                  aria-hidden="true"
                  className="px-3.5 pt-2 pb-1 text-label text-muted-foreground select-none"
                >
                  {groupLabels[section.group]}
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
            {t("commandBar.footerNavigateHint")}
          </span>
          <span>{t("commandBar.footerExecuteHint")}</span>
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
        "flex w-full items-center gap-2.5 rounded-sm px-3 py-2 text-left text-[13px] transition-colors outline-none",
        isActive ? "bg-selection font-medium text-accent-foreground" : "hover:bg-accent/60",
      )}
      data-command-index={dataIndex}
      id={`command-item-${dataIndex}`}
      onClick={onSelect}
      role="option"
      tabIndex={-1}
      type="button"
    >
      <item.icon
        className={cn("size-4 shrink-0", isActive ? "text-primary" : "text-muted-foreground")}
      />
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
