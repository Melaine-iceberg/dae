import {
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  AddFolderIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  CalendarIcon,
  CheckCircleIcon,
  ClipboardIcon,
  ClipboardListIcon,
  Columns3Icon,
  CopyIcon,
  EyeIcon,
  FileAddIcon,
  FileIcon,
  FilterIcon,
  FolderIcon,
  HistoryIcon,
  HomeIcon,
  ListIcon,
  LoaderIcon,
  MagnifierIcon,
  MonitorIcon,
  MoonIcon,
  PenIcon,
  ProgrammingIcon,
  RefreshIcon,
  Rows3Icon,
  ScissorsIcon,
  SettingsIcon,
  SortVerticalIcon,
  StarIcon,
  SunIcon,
  TextFieldIcon,
  TrashBinTrashIcon,
  WidgetIcon,
} from "@solar-icons/react/line-duotone";

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
import { setThemePreference, type ThemePreference } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { commandBarModeAtom, commandBarOpenAtom } from "@/features/workspace/command-bar-atoms";
import { appSettingsAtom, settingsOpenAtom } from "@/features/settings/settings-atoms";
import { formatBinding, resolveBinding } from "@/features/settings/shortcut-registry";

import { buildPaletteRows, rankIntoGroups, rowIndicesByResult } from "./palette-rows";

const MAX_RECENT_ITEMS = 8;
const MAX_PATH_RECENT_ITEMS = 12;
const MAX_FILE_RESULTS = 12;
const FILE_SEARCH_DEBOUNCE_MS = 220;
const MIN_FILE_QUERY_LENGTH = 2;

/** Row metrics of the palette list, in px. Both heights are fixed, which is
 *  what lets the virtualizer position rows without measuring them. */
const RESULT_ROW_HEIGHT_PX = 36;
const SECTION_HEADER_HEIGHT_PX = 34;

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
  /** How the right-hand hint reads: a filesystem path (head-ellipsized, so the
   *  filename stays visible) or a key binding (never ellipsized, so the whole
   *  chord shows). The two cannot be told apart from the string alone. */
  hintKind?: "path" | "keys";
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
  const setSettingsOpen = useSetAtom(settingsOpenAtom);
  const shortcuts = useAtomValue(appSettingsAtom)?.shortcuts;

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
        icon: HomeIcon,
        run: () => openSurface({ kind: "overview" }),
      },
      {
        id: "surface:recents",
        group: "navigation",
        label: t("commandBar.navigation.recents"),
        keywords: "recent history",
        icon: HistoryIcon,
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
        icon: TrashBinTrashIcon,
        run: () => openSurface({ kind: "trash" }),
      },
    ];

    const spaceItems: CommandItem[] = spaces.map((space) => ({
      id: `space:${space.id}`,
      group: "spaces",
      label: t("commandBar.openSpace", { name: getSpaceDisplayName(space) }),
      keywords: "space workspace open",
      icon: WidgetIcon,
      run: () => openSurface({ kind: "space", spaceId: space.id }),
    }));

    const favoriteItems: CommandItem[] = favorites.map((favorite) => ({
      id: `favorite:${favorite.path}`,
      group: "favorites",
      label: favorite.name,
      hint: favorite.path,
      hintKind: "path",
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
        hintKind: "path",
        keywords: "recent open",
        icon: recent.kind === "directory" ? FolderIcon : FileIcon,
        run: () => openRecentItem(recent),
      }));

    const explorerCommands: ReadonlyArray<{
      id: string;
      label: string;
      hint?: string;
      hintKind?: "path" | "keys";
      keywords: string;
      icon: ComponentType<{ className?: string }>;
      command: ExplorerCommandId;
    }> = [
      {
        id: "create-folder",
        label: t("commandBar.commands.createFolder"),
        keywords: "new create folder directory",
        icon: AddFolderIcon,
        command: "create-folder",
      },
      {
        id: "create-file",
        label: t("commandBar.commands.createFile"),
        keywords: "new create file",
        icon: FileAddIcon,
        command: "create-file",
      },
      {
        id: "rename",
        label: t("commandBar.commands.rename"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.rename")),
        hintKind: "keys",
        keywords: "rename",
        icon: PenIcon,
        command: "rename",
      },
      {
        id: "delete",
        label: t("commandBar.commands.delete"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.trash")),
        hintKind: "keys",
        keywords: "delete remove trash",
        icon: TrashBinTrashIcon,
        command: "delete",
      },
      {
        id: "copy",
        label: t("commandBar.commands.copy"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.copy")),
        hintKind: "keys",
        keywords: "copy",
        icon: CopyIcon,
        command: "copy",
      },
      {
        id: "cut",
        label: t("commandBar.commands.cut"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.cut")),
        hintKind: "keys",
        keywords: "cut move",
        icon: ScissorsIcon,
        command: "cut",
      },
      {
        id: "paste",
        label: t("commandBar.commands.paste"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.paste")),
        hintKind: "keys",
        keywords: "paste",
        icon: ClipboardIcon,
        command: "paste",
      },
      {
        id: "copy-paths",
        label: t("commandBar.commands.copyPaths"),
        keywords: "copy path clipboard location",
        icon: ClipboardListIcon,
        command: "copy-paths",
      },
      {
        id: "select-all",
        label: t("commandBar.commands.selectAll"),
        hint: formatBinding(resolveBinding(shortcuts, "explorer.selectAll")),
        hintKind: "keys",
        keywords: "select all",
        icon: CheckCircleIcon,
        command: "select-all",
      },
      {
        id: "refresh",
        label: t("commandBar.commands.refresh"),
        keywords: "refresh reload",
        icon: RefreshIcon,
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
        hint: formatBinding(resolveBinding(shortcuts, "explorer.openSystemTerminal")),
        hintKind: "keys",
        keywords: "terminal shell console open external",
        icon: ProgrammingIcon,
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
        hint: formatBinding(resolveBinding(shortcuts, "explorer.switchPane")),
        hintKind: "keys",
        keywords: "split dual pane panel column view",
        icon: Columns3Icon,
        command: "toggle-split",
      },
    ];

    const explorerCommandItems: CommandItem[] = folderActive
      ? explorerCommands.map((entry) => ({
          id: `cmd:${entry.id}`,
          group: "files",
          label: entry.label,
          hint: entry.hint,
          hintKind: entry.hintKind,
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
        icon: WidgetIcon,
        run: () => setViewMode("grid"),
      },
      {
        id: "view:column",
        group: "view",
        label: t("commandBar.view.switchToColumn"),
        keywords: "view column miller mode",
        icon: Columns3Icon,
        run: () => setViewMode("column"),
      },
      ...(
        [
          { key: "name", label: t("commandBar.view.sortByName"), icon: TextFieldIcon },
          { key: "modified", label: t("commandBar.view.sortByModified"), icon: CalendarIcon },
          { key: "type", label: t("commandBar.view.sortByType"), icon: FileIcon },
          { key: "size", label: t("commandBar.view.sortBySize"), icon: SortVerticalIcon },
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
        icon: SortVerticalIcon,
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
        icon: FilterIcon,
        run: () => setEntryFilters((filters) => ({ ...filters, kind: entry.value })),
      })),
      {
        id: "filter:clear",
        group: "view",
        label: t("commandBar.view.clearFilters"),
        keywords: "filter clear reset",
        icon: FilterIcon,
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
        icon: Rows3Icon,
        run: () => setDensity("compact" satisfies ExplorerDensity),
      },
      {
        id: "density:comfortable",
        group: "view",
        label: t("commandBar.view.densityComfortable"),
        keywords: "density comfortable rows",
        icon: Rows3Icon,
        run: () => setDensity("comfortable" satisfies ExplorerDensity),
      },
      {
        id: "density:spacious",
        group: "view",
        label: t("commandBar.view.densitySpacious"),
        keywords: "density spacious rows",
        icon: Rows3Icon,
        run: () => setDensity("spacious" satisfies ExplorerDensity),
      },
      {
        id: "open-settings",
        group: "view",
        label: t("commandBar.commands.openSettings"),
        hint: formatBinding(resolveBinding(shortcuts, "app.openSettings")),
        hintKind: "keys",
        keywords: "settings preferences shortcuts keyboard terminal default file manager options",
        icon: SettingsIcon,
        run: () => setSettingsOpen(true),
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
    setSettingsOpen,
    setShowHiddenFiles,
    setSortKey,
    setSortOrder,
    setViewMode,
    shortcuts,
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
      hintKind: "path",
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
        hintKind: "path",
        keywords: "file search",
        icon: entry.kind === "directory" ? FolderIcon : FileIcon,
        run: () => openSearchEntry(entry),
      }));
  }, [fileResults, navigateToFolder, pathMode]);

  const rankedResults = useMemo<RankedResult<CommandItem>[]>(() => {
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

  // Sections, ordered by their best match — see `rankIntoGroups`. This is what
  // keeps the group headings on screen while a query is typed; previously the
  // palette gave up on them and rendered one flat list.
  const sections = useMemo(
    () => rankIntoGroups(rankedResults, (item) => item.group, GROUP_ORDER),
    [rankedResults],
  );

  /** Flat result order — the index space ArrowUp/ArrowDown and Enter move in. */
  const results = useMemo(() => sections.flatMap((section) => section.entries), [sections]);

  const rows = useMemo(() => buildPaletteRows(sections), [sections]);

  /** Result index -> virtualizer row index (sections cost a header each). */
  const rowIndices = useMemo(() => rowIndicesByResult(rows), [rows]);

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

  const resultsVirtualizer = useVirtualizer({
    count: rows.length,
    estimateSize: (rowIndex) =>
      rows[rowIndex]?.kind === "header" ? SECTION_HEADER_HEIGHT_PX : RESULT_ROW_HEIGHT_PX,
    getScrollElement: () => listRef.current,
    overscan: 8,
  });

  useEffect(() => {
    setActiveIndex(0);
  }, [trimmedQuery]);

  // Keyboard navigation has to be able to reach a row that is not mounted:
  // `scrollToIndex` walks the scroll container, where scrolling to the DOM node
  // (what this did before the list was virtualized) only ever found rows the
  // viewport already contained.
  useEffect(() => {
    const rowIndex = rowIndices[currentIndex];
    if (rowIndex !== undefined) resultsVirtualizer.scrollToIndex(rowIndex, { align: "auto" });
  }, [currentIndex, resultsVirtualizer, rowIndices]);

  // The palette mounts inside the dialog's first frame, and until the
  // virtualizer has observed a *measured* scroll element it renders zero rows
  // out of a correctly sized track — an empty box with a scrollbar on first
  // open that a later state change (typing a query) would repaint. The
  // observer's own notification never reaches a re-render here, so re-measure
  // explicitly and force one render one frame later, once the dialog has laid
  // itself out. The reducer tick is deliberately not read: dispatching it is
  // the whole point.
  const [, renderResultRows] = useReducer((tick: number) => tick + 1, 0);
  useEffect(() => {
    if (!open) return;
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      resultsVirtualizer.measure();
      secondFrame = requestAnimationFrame(renderResultRows);
    });
    return () => {
      window.cancelAnimationFrame(firstFrame);
      window.cancelAnimationFrame(secondFrame);
    };
  }, [open, renderResultRows, resultsVirtualizer]);

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
        className="top-[15%] w-[calc(100%-2rem)] max-w-command-bar translate-y-0 gap-0 overflow-hidden rounded-xl p-0 shadow-ambient-lg"
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">{t("commandBar.title")}</DialogTitle>
        {/* No rule under the field: the input and the results are one surface,
            and a line across it drew a boundary inside a single plate. The gap
            does the separating. */}
        <div className="flex items-center gap-2.5 px-3.5 py-1">
          <MagnifierIcon className="size-4 shrink-0 text-foreground/72" />
          <input
            aria-activedescendant={results.length > 0 ? `command-item-${currentIndex}` : undefined}
            aria-autocomplete="list"
            aria-controls="command-bar-results"
            aria-expanded="true"
            aria-label={t("commandBar.inputAriaLabel")}
            autoComplete="off"
            className="h-12 min-w-0 flex-1 bg-transparent text-lead outline-none placeholder:text-muted-foreground"
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
          {/* The binding as plain text, not as keycap chips. A chip is a
              picture of a physical key, and inside a surface that is otherwise
              paper and grey wash it is the one thing that insists this is a
              developer tool. The hint stays — it is the only place the
              sibling palette's binding is discoverable — it just stops being
              drawn. */}
          <span className="shrink-0 text-micro text-muted-foreground tabular-nums select-none">
            {formatBinding(resolveBinding(shortcuts, pathMode ? "app.pathJump" : "app.commandBar"))}
          </span>
        </div>
        <div
          aria-label={t("commandBar.resultsAriaLabel")}
          className="max-h-[21rem] overflow-y-auto overscroll-contain p-1"
          id="command-bar-results"
          ref={listRef}
          role="listbox"
        >
          {rows.length === 0 ? (
            isSearchingFiles ? (
              <p className="px-2.5 py-6 text-center text-body text-muted-foreground">
                {pathMode ? t("commandBar.searchingFolders") : t("commandBar.searchingFiles")}
              </p>
            ) : pathMode && !trimmedQuery ? (
              <p className="px-2.5 py-6 text-center text-body text-muted-foreground">
                {t("commandBar.noPathLocations")}
              </p>
            ) : (
              <p className="px-2.5 py-6 text-center text-body text-muted-foreground">
                {pathMode
                  ? t("commandBar.noPathResults", { query: trimmedQuery })
                  : t("commandBar.noResults", { query: trimmedQuery })}
              </p>
            )
          ) : (
            <div className="relative" style={{ height: resultsVirtualizer.getTotalSize() }}>
              {resultsVirtualizer.getVirtualItems().map((virtualRow) => {
                const row = rows[virtualRow.index];
                return (
                  <div
                    className="absolute top-0 left-0 w-full"
                    key={row.key}
                    style={{
                      height: virtualRow.size,
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    {row.kind === "header" ? (
                      <p
                        aria-hidden="true"
                        className="flex h-full items-end px-2 pb-1 text-label text-muted-foreground select-none"
                      >
                        {groupLabels[row.group]}
                      </p>
                    ) : (
                      <CommandResultRow
                        dataIndex={row.resultIndex}
                        isActive={row.resultIndex === currentIndex}
                        item={row.entry.item}
                        matchedIndices={row.entry.matchedIndices}
                        onSelect={() => runCommand(row.entry.item)}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
        {/* A quiet line of hints on the plate itself: no rule above it and no
            tinted band under it. Both marked the footer off as a status strip,
            which is the chrome of a terminal rather than of a document — here
            the text is simply the smallest and greyest thing on the surface,
            which is enough to read as a hint without becoming a second pane.
            The bindings lost their keycap chips for the same reason the input's
            did; the arrow and return glyphs stay as plain characters, so the
            chord still reads without being drawn as a picture of a key. */}
        <footer className="flex h-9 shrink-0 items-center justify-between gap-3 px-3.5 pb-1 text-micro text-muted-foreground select-none">
          <span className="flex min-w-0 items-center gap-3">
            <span className="flex shrink-0 items-center gap-1.5">
              {isSearchingFiles && <LoaderIcon className="size-3 shrink-0 animate-spin" />}
              <span aria-hidden="true">↑↓</span>
              <span className="truncate">{t("commandBar.footerNavigateHint")}</span>
            </span>
            {/* The sibling palette's entry point, shown only while a palette is
                already open. This used to be a permanent hint strip in the window
                chrome, where it taught the shortcut to people who had already
                learnt it and lost to the view controls on narrow windows; here it
                is visible exactly when the user is looking at the surface it
                switches to. */}
            <span className="hidden shrink-0 items-center gap-1.5 sm:flex">
              <span aria-hidden="true">
                {formatBinding(
                  resolveBinding(shortcuts, pathMode ? "app.commandBar" : "app.pathJump"),
                )}
              </span>
              <span className="truncate">
                {pathMode ? t("commandBar.title") : t("commandBar.pathTitle")}
              </span>
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-3">
            <span className="flex items-center gap-1.5">
              <span aria-hidden="true">↵</span>
              <span>{t("commandBar.footerExecuteHint")}</span>
            </span>
            <span className="flex items-center gap-1.5">
              <span aria-hidden="true">Esc</span>
              <span>{t("commandBar.footerCloseHint")}</span>
            </span>
          </span>
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
        // Palette row: 32px, quiet control radius, flat selection. The active
        // row takes the same `row-active` plate the sidebar's current location
        // does — one wash over the fill, nothing else. It used to add a hairline
        // ring and a glow on the theory that a fill alone reads as a flat bar
        // between identical rows, but the ring is what made the row look like a
        // focused control rather than a place; the row is not the only surface
        // that has to say "this one" without a line around it.
        "group/command-row flex h-8 w-full items-center gap-2.5 rounded-md px-3 text-left text-body transition-[background-color,color,box-shadow] duration-fast ease-standard outline-none",
        isActive ? "row-active bg-accent text-foreground" : "hover:bg-accent",
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
      {item.hint &&
        (item.hintKind === "keys" ? (
          <span className="ml-auto shrink-0 text-caption text-muted-foreground tabular-nums">
            {item.hint}
          </span>
        ) : (
          <span
            className={cn(
              "path-ellipsis ml-auto max-w-row-meta shrink-0 truncate text-caption text-muted-foreground",
            )}
            title={item.hint}
          >
            {item.hint}
          </span>
        ))}
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
