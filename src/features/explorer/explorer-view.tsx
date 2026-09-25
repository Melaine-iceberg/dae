/**
 * The explorer pane: navigation, listing, selection and every file operation
 * the user can start from it.
 *
 * This file is the orchestration layer only. The pieces it used to contain
 * now live behind explicit boundaries:
 *
 * - `explorer-toolbar.tsx`      — the toolbar's markup (props in, buttons out)
 * - `explorer-dialogs.tsx`      — presentational rename/create/delete/error dialogs
 * - `explorer-chrome.tsx`       — listing stats, terminal toggle, progress strip
 * - `use-entry-dialogs.ts`      — dialog state + submission flows
 * - `use-file-operations.ts`    — the operation runner (progress/pending/error)
 * - `use-transfers.ts`          — copy/move pipeline + conflict resolution
 * - `use-explorer-clipboard.ts` — copy/cut/paste incl. the system-clipboard mirror
 * - `use-explorer-selection.ts` — selection state, pruning and resets
 * - `use-explorer-events.ts`    — directory refresh + external drag-drop
 * - `use-pending-explorer-command.ts` — the command-bus consumer
 *
 * What remains here is the listing pipeline, the entry actions that compose
 * the pieces above (delete/archive/undo/redo/open-with…), and the layout.
 */
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { RotateCw, RotateCcw, TriangleAlert, X } from "lucide-react";

import { commands, type ArchiveFormat, type UndoRedoOutcome } from "@/bindings";

import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { formatBinding } from "@/features/settings/shortcut-registry";
import {
  hotkeysPausedAtom,
  useBinding,
} from "@/features/settings/settings-atoms";
import {
  HOTKEY_COMMON_OPTIONS,
  asHotkey,
  guardedBackgroundAction,
} from "@/features/settings/hotkeys";
import { useHotkeys } from "@tanstack/react-hotkeys";
import {
  addFavoritePathsAtom,
  favoritesAtom,
  sidebarVisibleAtom,
  toggleFavoriteAtom,
} from "@/features/sidebar/sidebar-atoms";
import type { Favorite } from "@/features/sidebar/types";
import { addItemsToSpace } from "@/features/workspace/spaces-atoms";
import { recordRecentItem } from "@/features/workspace/recents-atoms";
import { shellCommandErrorAtom } from "@/features/shell-commands/shell-commands-atoms";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { isWindowsPlatform } from "@/lib/platform";
import { findEntryVisual, withSharedElement } from "@/lib/view-transition";

import { ContentSearchResults, ContentSearchToolbar, useContentSearch } from "./content-search";
import { ContextualActionBar } from "./contextual-action-bar";
import { ArchivePasswordDialog } from "./archive-password-dialog";
import { BulkRenameDialog } from "./bulk-rename";
import { useDirectorySearch, type ExplorerSearchMode } from "./directory-search";
import { isLocalExplorerPath } from "./drag-drop";
import { EntryPreview } from "./entry-preview";
import { isArchiveFile } from "./entry-context-menu";
import { displayNameOfPath, isWrongPasswordError } from "./explorer-errors";
import { DeleteDialog, ExplorerErrorAlert, RenameDialog, CreateEntryDialog } from "./explorer-dialogs";
import { FileOperationStatusBar } from "./explorer-chrome";
import { ExplorerToolbar } from "./explorer-toolbar";
import { FileList, FileListSkeleton } from "./file-list";
import { useGitStatus } from "./git-status";
import { listingViewOf, allNames } from "./listing-view";
import type { ExplorerNavigator } from "./navigation";
import { OpenWithDialog } from "./open-with-dialog";
import { useSortedListingView } from "./sorted-entries";
import { TransferConflictDialog } from "./transfer-conflict-dialog";
import {
  applyEntryFilters,
  entryFiltersAtom,
  filterHidden,
  foldersFirstAtom,
  showHiddenFilesAtom,
  sortKeyAtom,
  sortOrderAtom,
} from "./preferences";
import { undoRedoAtom } from "./tabs";
import { useEntryDialogs } from "./use-entry-dialogs";
import { useFileOperations } from "./use-file-operations";
import { useTransfers } from "./use-transfers";
import { useExplorerClipboard } from "./use-explorer-clipboard";
import { NO_ENTRIES, useExplorerSelection } from "./use-explorer-selection";
import { useDirectoryRefresh, useExternalDrop } from "./use-explorer-events";
import { usePendingExplorerCommand } from "./use-pending-explorer-command";

const UNDO_TOAST_DISMISS_MS = 6000;
/** Same deal as `NO_ENTRIES` in the selection hook: unloaded favorites need a
 *  stable identity so memoization downstream doesn't churn on every render. */
const NO_FAVORITES: Favorite[] = [];
/** The name list the bulk-rename dialog reads, which is only worth collecting
 *  while that dialog is open. */
const NO_NAMES: string[] = [];

interface ExplorerViewProps {
  navigator: ExplorerNavigator;
  /** Only the focused pane owns window-level shortcuts and command-bar
   *  intents; in the single-pane layout this stays true. It also arbitrates
   *  external drops that land outside any explorer surface (sidebar, tab
   *  strip): those go to the active pane rather than every mounted pane. */
  isActivePane?: boolean;
  /** Whether the dual-pane layout is currently up for this tab; drives the
   *  split toggle's state. */
  splitEnabled?: boolean;
  /** Toggles the dual-pane layout; the toolbar button is hidden without
   *  it. */
  onToggleSplit?: () => void;
}

/** Floating hint after an undoable operation or an undo/redo step. */
type UndoRedoToast = {
  outcome: { action: string; count: number; op: string };
  /** Follow-up action offered on the toast. */
  action: "undo" | "redo";
};

export function ExplorerView({
  navigator,
  isActivePane = true,
  splitEnabled = false,
  onToggleSplit,
}: ExplorerViewProps) {
  const { t } = useTranslation("explorer");
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const undoRedo = useAtomValue(undoRedoAtom);
  const favorites = useAtomValue(favoritesAtom) ?? NO_FAVORITES;
  const toggleFavorite = useSetAtom(toggleFavoriteAtom);
  const addFavoritePaths = useSetAtom(addFavoritePathsAtom);
  const [sidebarVisible, setSidebarVisible] = useAtom(sidebarVisibleAtom);
  // A shell command reports back only whether the start itself succeeded; the
  // app it launched says nothing. A failure belongs in the banner above the
  // list rather than in the menu that has already closed.
  const shellCommandError = useAtomValue(shellCommandErrorAtom);
  const setShellCommandError = useSetAtom(shellCommandErrorAtom);
  const [isPreviewOpen, setIsPreviewOpen] = useState(false);
  const [searchMode, setSearchMode] = useState<ExplorerSearchMode>("name");
  const [undoRedoToast, setUndoRedoToast] = useState<UndoRedoToast | null>(null);
  /** Bumped by Ctrl+L / Alt+D; the path bar edits on every increment. The
   *  signal is pane-local, so in a split only the active pane's bar reacts. */
  const [pathEditSignal, setPathEditSignal] = useState(0);
  // Live bindings for the undo toast's buttons. They used to be baked into
  // the translation strings ("收起预览面板 (Space)"), which meant a rebind left
  // the tooltip teaching a key that no longer did anything.
  const undoBinding = formatBinding(useBinding("explorer.undo"));
  const redoBinding = formatBinding(useBinding("explorer.redo"));

  const directory = state.directory;
  const listing = state.listing;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;

  const search = useDirectorySearch(directoryPath ?? null, directory, searchMode === "name");
  const contentSearch = useContentSearch(directoryPath ?? null, directory, searchMode === "content");
  const gitStatus = useGitStatus(directoryPath ?? null);
  const isContentSearchActive = searchMode === "content" && contentSearch.isActive;

  const sortKey = useAtomValue(sortKeyAtom);
  const sortOrder = useAtomValue(sortOrderAtom);
  const foldersFirst = useAtomValue(foldersFirstAtom);
  const showHiddenFiles = useAtomValue(showHiddenFilesAtom);
  const entryFilters = useAtomValue(entryFiltersAtom);

  // The listing the pane reads. A search replaces the directory's listing with
  // its own results; otherwise it is the one the navigator holds — and while a
  // restored pane is still re-reading, the head that came with the handoff.
  const sourceListing = search.isActive
    ? listingViewOf(search.response?.entries ?? NO_ENTRIES)
    : (listing ?? listingViewOf(directory?.entries ?? NO_ENTRIES));
  // Filtering keeps the source view's identity when nothing is filtered, so a
  // streamed batch reaches the ordering hook as a plain append.
  const filteredListing = useMemo(
    () => applyEntryFilters(filterHidden(sourceListing, showHiddenFiles), entryFilters),
    [entryFilters, showHiddenFiles, sourceListing],
  );
  const displayedListing = useSortedListingView(filteredListing, sortKey, sortOrder, foldersFirst);

  const {
    selectedPaths,
    setSelectedPaths,
    selectedEntries,
    selectAll,
  } = useExplorerSelection({
    displayedListing,
    isContentSearchActive,
    directoryPath,
    searchQuery: search.query,
  });
  const clearSelection = useCallback(() => setSelectedPaths([]), [setSelectedPaths]);

  const refresh = useCallback((path: string) => navigator.refresh(path), [navigator]);

  const hotkeysPaused = useAtomValue(hotkeysPausedAtom);
  // Pane-local navigation keys: the platform file-manager chords that never
  // made it into the window before. They are fixed rather than rebindable —
  // the registry mirrors the Rust defaults one-for-one, and Backspace/Alt+arrow
  // are OS conventions rather than dae bindings. Each handler self-gates on its
  // own pane (`isActivePane`) the same way the file list's registrations do.
  useHotkeys(
    [
      {
        hotkey: asHotkey("Backspace"),
        callback: guardedBackgroundAction(() => void navigator.goUp()),
        options: { enabled: isActivePane && !hotkeysPaused && canGoUp },
      },
      {
        hotkey: asHotkey("Alt+ArrowLeft"),
        callback: guardedBackgroundAction(() => void navigator.goBack()),
        options: { enabled: isActivePane && !hotkeysPaused && canGoBack },
      },
      {
        hotkey: asHotkey("Alt+ArrowRight"),
        callback: guardedBackgroundAction(() => void navigator.goForward()),
        options: { enabled: isActivePane && !hotkeysPaused && canGoForward },
      },
      {
        hotkey: asHotkey("Alt+ArrowUp"),
        callback: guardedBackgroundAction(() => void navigator.goUp()),
        options: { enabled: isActivePane && !hotkeysPaused && canGoUp },
      },
      {
        // F5 is Explorer's refresh; Ctrl+R is the browser/webview habit that
        // would otherwise reload the whole app out from under the user.
        hotkey: asHotkey("F5"),
        callback: guardedBackgroundAction(() =>
          directory ? void navigator.navigate(directory.path) : undefined,
        ),
        options: { enabled: isActivePane && !hotkeysPaused && directory !== null },
      },
      {
        hotkey: asHotkey("Control+R"),
        callback: guardedBackgroundAction(() =>
          directory ? void navigator.navigate(directory.path) : undefined,
        ),
        options: { enabled: isActivePane && !hotkeysPaused && directory !== null },
      },
      {
        // Ctrl+L (Chrome) and Alt+D (Explorer) both put the caret in the
        // address/path bar — the two conventions users bring with them.
        hotkey: asHotkey("Control+L"),
        callback: guardedBackgroundAction(() => setPathEditSignal((signal) => signal + 1)),
        options: { enabled: isActivePane && !hotkeysPaused },
      },
      {
        hotkey: asHotkey("Alt+D"),
        callback: guardedBackgroundAction(() => setPathEditSignal((signal) => signal + 1)),
        options: { enabled: isActivePane && !hotkeysPaused },
      },
    ],
    { ...HOTKEY_COMMON_OPTIONS },
  );
  const {
    performFileOperation,
    fileOperationProgress,
    isOperationPending,
    operationError,
    setOperationError,
  } = useFileOperations({ directoryPath, refresh });

  const {
    pendingTransfer,
    startTransfer,
    transferEntries,
    dropExternalEntries,
    createShortcutsEntries,
    resolveTransferConflicts,
    cancelTransferConflicts,
  } = useTransfers({ performFileOperation, setOperationError, clearSelection });

  const { copySelection, cutSelection, pasteClipboard } = useExplorerClipboard({
    selectedEntries,
    directoryPath,
    startTransfer,
    setOperationError,
    clearSelection,
  });

  const dialogs = useEntryDialogs({
    performFileOperation,
    setOperationError,
    isOperationPending,
    selectedEntries,
    directoryPath,
    searchActive: search.isActive,
    setSelectedPaths,
    searchQuery: search.query,
  });

  const { externalDrop } = useExternalDrop({
    directoryPath,
    searchQuery: search.query,
    isActivePane,
    onDropPaths: dropExternalEntries,
  });
  useDirectoryRefresh(navigator);

  useEffect(() => {
    if (navigator.getSnapshot().status === "idle") {
      void navigator.initialize();
    }
  }, [navigator]);

  // A new directory/query also resets the view-local surfaces that neither
  // the dialog hook nor the selection hook owns: the error banner and the
  // preview pane.
  useEffect(() => {
    setOperationError(null);
    setIsPreviewOpen(false);
  }, [directoryPath, search.query, setOperationError]);

  const retry = () => {
    if (directory) {
      void navigator.navigate(directory.path);
      return;
    }

    void navigator.initialize();
  };

  const navigateToPath = useCallback(
    (path: string) => navigator.navigate(path).then((result) => result !== undefined),
    [navigator],
  );

  const addToSpace = useCallback((spaceId: string, paths: string[]) => {
    void addItemsToSpace(spaceId, paths);
  }, []);

  const copySelectedPaths = useCallback(() => {
    if (selectedEntries.length === 0) return;

    void writeText(selectedEntries.map((entry) => entry.path).join("\n")).catch((error) => {
      console.warn("Unable to copy paths to clipboard", error);
    });
  }, [selectedEntries]);

  /** Opens the system default terminal at a directory (Windows Terminal,
   *  Terminal.app, or the desktop's default terminal on Linux). */
  const openTerminalHere = useCallback(
    (path: string) => {
      setOperationError(null);
      void commands.openTerminal(path).catch((error: unknown) => {
        setOperationError(
          t("explorer:terminalOpenFailed", { detail: getFileOperationErrorMessage(error) }),
        );
      });
    },
    [setOperationError, t],
  );

  /** Opens the "Open With" picker for a local file or folder: the native
   *  SHOpenWithDialog on Windows, the in-app picker on macOS/Linux. */
  const openWithHere = useCallback(
    (path: string) => {
      setOperationError(null);
      if (isWindowsPlatform) {
        void commands.openWith(path).catch((error: unknown) => {
          setOperationError(
            t("explorer:openWithFailed", { detail: getFileOperationErrorMessage(error) }),
          );
        });
        return;
      }
      dialogs.setOpenWithTarget(path);
    },
    [dialogs.setOpenWithTarget, setOperationError, t],
  );

  /** Opens every selected file and navigates into the first selected folder. */
  const openSelectedEntries = useCallback(() => {
    if (selectedEntries.length === 0 || isOperationPending) return;

    const firstDirectory = selectedEntries.find((entry) => entry.kind === "directory");
    if (firstDirectory) {
      void navigator.navigate(firstDirectory.path);
    }

    for (const entry of selectedEntries) {
      if (entry.kind === "directory") continue;

      recordRecentItem(entry.path, "file", "opened");
      void openPath(entry.path).catch((error) => {
        console.warn(`Unable to open ${entry.path}`, error);
      });
    }
  }, [isOperationPending, navigator, selectedEntries]);

  /** Space toggles the preview surface for the first selected entry. */
  const togglePreview = useCallback(() => {
    if (isPreviewOpen) {
      setIsPreviewOpen(false);
      return;
    }
    // Quick Look (src/lib/view-transition.ts): the entry's visual grows into
    // the preview's hero. `findEntryVisual` hands back null when the
    // virtualized listing has scrolled that entry out of the DOM — then this
    // is the plain open it was before, under the panel's own fade.
    withSharedElement(findEntryVisual(selectedEntries[0]?.path ?? null), () => {
      setIsPreviewOpen(true);
    });
  }, [isPreviewOpen, selectedEntries]);

  /** Duplicates the selection in place; the backend picks unique "副本" names. */
  const duplicateSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    setOperationError(null);
    void performFileOperation(
      (operationId) =>
        commands.duplicateEntries(
          selectedEntries.map((entry) => entry.path),
          operationId!,
        ),
      "copy",
    ).then((result) => {
      if (!result.ok) setOperationError(result.error);
    });
  }, [performFileOperation, selectedEntries, setOperationError]);

  /** Compresses the selection into a unique archive next to the entries.
   *  Encrypted requests go through the password dialog first. */
  const compressSelection = useCallback(
    (format: ArchiveFormat, encrypted: boolean) => {
      if (selectedEntries.length === 0 || !directoryPath) return;

      setOperationError(null);
      if (encrypted) {
        dialogs.openArchivePassword({ mode: "compress" });
        return;
      }

      void performFileOperation(
        (operationId) =>
          commands.compressEntries(
            selectedEntries.map((entry) => entry.path),
            directoryPath,
            format,
            null,
            operationId!,
          ),
        "compress",
      ).then((result) => {
        if (!result.ok) setOperationError(result.error);
      });
    },
    [dialogs.openArchivePassword, directoryPath, performFileOperation, selectedEntries, setOperationError],
  );

  /** Extracts an archive into a fresh folder next to it. Encrypted archives
   *  answer with a wrong-password error, which opens the password dialog. */
  const extractSelection = useCallback(
    (archivePath: string) => {
      setOperationError(null);
      void performFileOperation(
        (operationId) => commands.extractArchive(archivePath, null, null, operationId!),
        "extract",
      ).then((result) => {
        if (result.ok) return;

        if (isWrongPasswordError(result.rawError)) {
          dialogs.openArchivePassword({ archivePath, mode: "extract" });
          return;
        }
        setOperationError(result.error);
      });
    },
    [dialogs.openArchivePassword, performFileOperation, setOperationError],
  );

  /** Native cross-platform folder picker feeding the existing move pipeline. */
  const moveSelectionTo = useCallback(() => {
    if (selectedEntries.length === 0 || !directoryPath) return;

    const sourcePaths = selectedEntries.map((entry) => entry.path);
    setOperationError(null);

    void openDialog({
      defaultPath: directoryPath,
      directory: true,
      multiple: false,
      title: t("explorer:moveTo.dialogTitle"),
    })
      .then((destination) => {
        if (typeof destination !== "string" || !destination || destination === directoryPath) {
          return;
        }

        transferEntries(sourcePaths, destination, "move");
      })
      .catch((error: unknown) => {
        console.warn("Unable to open destination picker", error);
      });
  }, [directoryPath, selectedEntries, setOperationError, t, transferEntries]);

  /** Moves the selection into the system trash; the batch stays undoable. */
  const trashSelection = useCallback(() => {
    if (selectedEntries.length === 0) return;

    const paths = selectedEntries.map((entry) => entry.path);
    setOperationError(null);
    setSelectedPaths([]);

    void performFileOperation(
      (operationId) => commands.trashEntries(paths, operationId!),
      "delete",
    ).then((result) => {
      if (!result.ok) {
        setOperationError(result.error);
        setSelectedPaths(paths);
        return;
      }
      setUndoRedoToast({
        outcome: { action: "trash", count: paths.length, op: "trash" },
        action: "undo",
      });
    });
  }, [performFileOperation, selectedEntries, setOperationError, setSelectedPaths]);

  /** Delete moves the selection to the trash when every entry is local;
   *  network locations have no recycle bin, so they keep the permanent-delete
   *  confirmation dialog. */
  const requestDelete = useCallback(() => {
    if (selectedEntries.length === 0) return;

    if (selectedEntries.every((entry) => isLocalExplorerPath(entry.path))) {
      trashSelection();
      return;
    }

    dialogs.openDeleteDialog(selectedEntries);
  }, [dialogs.openDeleteDialog, selectedEntries, trashSelection]);

  /** Shift+Delete bypasses the trash and asks for permanent deletion. */
  const requestPermanentDelete = useCallback(() => {
    if (selectedEntries.length === 0) return;

    dialogs.openDeleteDialog(selectedEntries);
  }, [dialogs.openDeleteDialog, selectedEntries]);

  /** Reverts the most recent recorded operation (move, rename, copy, trash,
   *  create, duplicate) through the backend history stack. */
  const undoLastOperation = useCallback(() => {
    if (!undoRedo.canUndo || isOperationPending) return;

    setUndoRedoToast(null);
    setOperationError(null);
    let outcome: UndoRedoOutcome | null = null;
    void performFileOperation(async (operationId) => {
      outcome = await commands.undoOperation(operationId!);
    }, "auto").then((result) => {
      if (!result.ok) {
        setOperationError(t("explorer:undoRedo.failedUndo", { detail: result.error }));
        return;
      }
      if (outcome) {
        setUndoRedoToast({ outcome, action: "redo" });
      }
    });
  }, [isOperationPending, performFileOperation, setOperationError, t, undoRedo.canUndo]);

  /** Re-applies the most recently undone operation. */
  const redoLastOperation = useCallback(() => {
    if (!undoRedo.canRedo || isOperationPending) return;

    setUndoRedoToast(null);
    setOperationError(null);
    let outcome: UndoRedoOutcome | null = null;
    void performFileOperation(async (operationId) => {
      outcome = await commands.redoOperation(operationId!);
    }, "auto").then((result) => {
      if (!result.ok) {
        setOperationError(t("explorer:undoRedo.failedRedo", { detail: result.error }));
        return;
      }
      if (outcome) {
        setUndoRedoToast({ outcome, action: "undo" });
      }
    });
  }, [isOperationPending, performFileOperation, setOperationError, t, undoRedo.canRedo]);

  // The undo toast auto-dismisses after a delay; hovering pauses the timer
  // so the pointer can reach the action button before the toast disappears.
  const [isUndoToastHovered, setIsUndoToastHovered] = useState(false);

  useEffect(() => {
    if (!undoRedoToast || isUndoToastHovered) return undefined;

    const timer = window.setTimeout(() => setUndoRedoToast(null), UNDO_TOAST_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [isUndoToastHovered, undoRedoToast]);

  usePendingExplorerCommand({
    isActivePane,
    navigator,
    directory,
    directoryPath,
    requestCreate: dialogs.requestCreate,
    requestRename: dialogs.requestRename,
    requestDelete,
    copySelection,
    cutSelection,
    pasteClipboard,
    copySelectedPaths,
    selectAll,
    openTerminalHere,
    onToggleSplit,
  });

  const isCurrentFavorited =
    directory !== null && favorites.some((favorite) => favorite.path === directory.path);

  // Listing status for the path bar's trailing edge. These used to be the
  // entire prop list of a dedicated status bar; they are computed here so the
  // count can sit beside the breadcrumbs that name the folder it counts.
  const listingLoading = isLoading || search.isSearching || contentSearch.isSearching;
  const listingCount = isContentSearchActive
    ? (contentSearch.response?.files.length ?? 0)
    : displayedListing.count;
  const listingQuery = isContentSearchActive
    ? contentSearch.query.trim()
    : search.isActive
      ? search.query.trim()
      : null;
  const listingError = isContentSearchActive
    ? contentSearch.error
    : search.isActive
      ? search.error
      : null;
  const listingTruncated = isContentSearchActive
    ? (contentSearch.response?.truncated ?? false)
    : (search.response?.truncated ?? false);

  return (
    <main className="h-full bg-card" data-explorer-container="true">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <ExplorerToolbar
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          canGoUp={canGoUp}
          contentSearch={contentSearch}
          directory={directory}
          gitStatus={gitStatus}
          isCurrentFavorited={isCurrentFavorited}
          isLoading={isLoading}
          isPreviewOpen={isPreviewOpen}
          onGoBack={() => void navigator.goBack()}
          onGoForward={() => void navigator.goForward()}
          onGoUp={() => void navigator.goUp()}
          onNavigateBreadcrumb={(breadcrumb) => void navigator.navigateBreadcrumb(breadcrumb)}
          onNavigatePath={navigateToPath}
          onRefresh={() => directory && void navigator.navigate(directory.path)}
          onSearchModeChange={setSearchMode}
          onToggleFavorite={() =>
            directory &&
            toggleFavorite({
              path: directory.path,
              name: directory.breadcrumbs.at(-1)?.name ?? directory.path,
            })
          }
          onTogglePreview={togglePreview}
          onToggleSidebar={() => setSidebarVisible(!sidebarVisible)}
          onToggleSplit={onToggleSplit}
          pathEditSignal={pathEditSignal}
          search={search}
          searchMode={searchMode}
          sidebarVisible={sidebarVisible}
          splitEnabled={splitEnabled}
          stats={{
            isLoading: listingLoading,
            itemCount: listingCount,
            searchError: listingError,
            searchQuery: listingQuery,
            selectedCount: selectedPaths.length,
            truncated: listingTruncated,
          }}
        />

        {isContentSearchActive && (
          <div className="shrink-0 border-b border-border px-2 py-1.5">
            <ContentSearchToolbar search={contentSearch} />
          </div>
        )}

        {state.error && directory && (
          <div className="shrink-0 p-3 pb-0">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        )}

        {operationError && (
          <div className="shrink-0 p-3 pb-0">
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>{t("explorer:errors.operationFailedTitle")}</AlertTitle>
              <AlertDescription>{operationError}</AlertDescription>
              <AlertAction>
                <Button
                  onClick={() => setOperationError(null)}
                  size="xs"
                  type="button"
                  variant="outline"
                >
                  {t("explorer:actions.close")}
                </Button>
              </AlertAction>
            </Alert>
          </div>
        )}

        {shellCommandError && (
          <div className="shrink-0 p-3 pb-0">
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>{t("explorer:shellCommands.launchFailedTitle")}</AlertTitle>
              <AlertDescription>{shellCommandError}</AlertDescription>
              <AlertAction>
                <Button
                  onClick={() => setShellCommandError(null)}
                  size="xs"
                  type="button"
                  variant="outline"
                >
                  {t("explorer:actions.close")}
                </Button>
              </AlertAction>
            </Alert>
          </div>
        )}

        {directory ? (
          <div className="flex min-h-0 flex-1">
            <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
              {isContentSearchActive ? (
                <ContentSearchResults
                  error={contentSearch.error}
                  isSearching={contentSearch.isSearching}
                  onOpenLocation={(location) => void navigator.navigate(location)}
                  query={contentSearch.query.trim()}
                  response={contentSearch.response}
                />
              ) : (
                <FileList
                  currentDirectoryPath={directory.path}
                  entries={displayedListing}
                  externalDropItemCount={externalDrop?.sourcePaths.length ?? 0}
                  externalDropOperation={externalDrop?.operation ?? null}
                  externalDropTargetPath={externalDrop?.targetPath ?? null}
                  gitStatus={gitStatus}
                  initialScrollOffset={
                    search.isActive ? 0 : navigator.getScrollOffset(directory.path)
                  }
                  isActivePane={isActivePane}
                  isLoading={isLoading}
                  isOperationPending={isOperationPending}
                  canRedo={undoRedo.canRedo}
                  canUndo={undoRedo.canUndo}
                  onAddToFavorites={addFavoritePaths}
                  onAddToSpace={addToSpace}
                  onCompress={compressSelection}
                  onCopy={copySelection}
                  onCreateDirectory={() => dialogs.requestCreate("directory")}
                  onCreateFile={() => dialogs.requestCreate("file")}
                  onCut={cutSelection}
                  onDelete={requestDelete}
                  onDeletePermanent={requestPermanentDelete}
                  onDuplicate={duplicateSelection}
                  onDropEntries={transferEntries}
                  onCreateShortcuts={createShortcutsEntries}
                  onExtract={extractSelection}
                  onMoveTo={moveSelectionTo}
                  onOpenDirectory={(path) => void navigator.navigate(path)}
                  onOpenTerminal={() => directory.path && openTerminalHere(directory.path)}
                  onOpenWith={() => directory.path && openWithHere(directory.path)}
                  onPaste={pasteClipboard}
                  onRename={dialogs.requestRename}
                  onRedo={redoLastOperation}
                  onUndo={undoLastOperation}
                  onScrollOffsetChange={
                    search.isActive
                      ? undefined
                      : (offset) => navigator.setScrollOffset(directory.path, offset)
                  }
                  onSelectAll={selectAll}
                  onSelectedPathsChange={setSelectedPaths}
                  onTogglePreview={togglePreview}
                  searchState={
                    search.isActive
                      ? {
                          error: search.error,
                          isSearching: search.isSearching,
                          query: search.query.trim(),
                          truncated: search.response?.truncated ?? false,
                        }
                      : undefined
                  }
                  selectedPaths={selectedPaths}
                  viewId={
                    search.isActive ? `${directory.path}::search::${search.query}` : directory.path
                  }
                />
              )}
              {selectedPaths.length > 0 && (
                <ContextualActionBar
                  archiveSelectionPath={
                    selectedEntries.length === 1 && isArchiveFile(selectedEntries[0])
                      ? selectedEntries[0].path
                      : null
                  }
                  hasDirectorySelection={selectedEntries.some(
                    (entry) => entry.kind === "directory",
                  )}
                  isActionDisabled={isOperationPending || isLoading}
                  onAddToSpace={(spaceId) =>
                    addToSpace(
                      spaceId,
                      selectedEntries
                        .filter((entry) => entry.kind === "directory")
                        .map((entry) => entry.path),
                    )
                  }
                  onClearSelection={clearSelection}
                  onCompress={compressSelection}
                  onCopy={copySelection}
                  onCopyPaths={copySelectedPaths}
                  onCut={cutSelection}
                  onDelete={requestDelete}
                  onDuplicate={duplicateSelection}
                  onExtract={extractSelection}
                  onMoveTo={moveSelectionTo}
                  onOpen={openSelectedEntries}
                  onRename={dialogs.requestRename}
                  selectedCount={selectedPaths.length}
                />
              )}
              {undoRedoToast && (
                <div
                  className="absolute bottom-4 left-1/2 z-40 -translate-x-1/2"
                  onPointerEnter={() => setIsUndoToastHovered(true)}
                  onPointerLeave={() => setIsUndoToastHovered(false)}
                >
                  <div className="animate-float-in flex items-center gap-2 rounded-lg border border-border bg-popover px-4 py-2 text-body text-popover-foreground shadow-ambient-lg">
                    {undoRedoToast.action === "redo" ? (
                      <RotateCw className="size-4 shrink-0 text-muted-foreground" />
                    ) : (
                      <RotateCcw className="size-4 shrink-0 text-muted-foreground" />
                    )}
                    <span className="whitespace-nowrap">
                      {t(`explorer:undoRedo.toast_${undoRedoToast.outcome.action}`, {
                        op: t(`explorer:undoRedo.op_${undoRedoToast.outcome.op}`),
                        count: undoRedoToast.outcome.count,
                      })}
                    </span>
                    {undoRedoToast.action === "redo" ? (
                      <Button onClick={redoLastOperation} size="xs" type="button" variant="outline">
                        {t("explorer:actions.redo")}
                        <Kbd className="h-4 px-1 text-nano">{redoBinding}</Kbd>
                      </Button>
                    ) : (
                      <Button onClick={undoLastOperation} size="xs" type="button" variant="outline">
                        {t("explorer:actions.undo")}
                        <Kbd className="h-4 px-1 text-nano">{undoBinding}</Kbd>
                      </Button>
                    )}
                    <Button
                      aria-label={t("explorer:undoRedo.closeToast")}
                      onClick={() => setUndoRedoToast(null)}
                      size="xs"
                      type="button"
                      variant="ghost"
                    >
                      <X />
                    </Button>
                  </div>
                </div>
              )}
            </div>
            {isPreviewOpen && (
              <EntryPreview
                entry={selectedEntries[0] ?? null}
                onClose={() => setIsPreviewOpen(false)}
                onOpen={() => openSelectedEntries()}
              />
            )}
          </div>
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
        {fileOperationProgress && <FileOperationStatusBar progress={fileOperationProgress} />}
      </section>

      <RenameDialog
        error={dialogs.renameError}
        isPending={isOperationPending}
        onClose={dialogs.closeRenameDialog}
        onOpenChange={(open) => {
          if (!open) dialogs.closeRenameDialog();
        }}
        onSubmit={dialogs.submitRename}
        onValueChange={dialogs.setRenameValue}
        target={dialogs.renameTarget}
        value={dialogs.renameValue}
      />
      <BulkRenameDialog
        applyError={dialogs.bulkRenameError}
        entries={selectedEntries}
        existingNames={dialogs.bulkRenameOpen ? allNames(displayedListing) : NO_NAMES}
        isPending={isOperationPending}
        onApply={dialogs.applyBulkRename}
        onClose={dialogs.closeBulkRename}
        onOpenChange={(open) => {
          if (!open) dialogs.closeBulkRename();
        }}
        open={dialogs.bulkRenameOpen}
      />
      <CreateEntryDialog
        error={dialogs.newEntryError}
        isPending={isOperationPending}
        kind={dialogs.newEntryKind}
        onClose={dialogs.closeCreateDialog}
        onOpenChange={(open) => {
          if (!open) dialogs.closeCreateDialog();
        }}
        onSubmit={dialogs.submitCreate}
        onValueChange={dialogs.setNewEntryValue}
        value={dialogs.newEntryValue}
      />
      <DeleteDialog
        entries={dialogs.deleteTargets}
        isPending={isOperationPending}
        onClose={dialogs.closeDeleteDialog}
        onConfirm={dialogs.confirmDelete}
        onOpenChange={(open) => {
          if (!open) dialogs.closeDeleteDialog();
        }}
      />
      <OpenWithDialog
        onClose={() => dialogs.setOpenWithTarget(null)}
        onOpenChange={(open) => {
          if (!open) dialogs.setOpenWithTarget(null);
        }}
        target={dialogs.openWithTarget}
      />
      <ArchivePasswordDialog
        archiveName={
          dialogs.archivePasswordRequest?.mode === "extract"
            ? displayNameOfPath(dialogs.archivePasswordRequest.archivePath)
            : ""
        }
        error={dialogs.archivePasswordError}
        isPending={dialogs.archivePasswordPending}
        mode={dialogs.archivePasswordRequest?.mode ?? "extract"}
        onOpenChange={(open) => {
          if (!open) dialogs.closeArchivePassword();
        }}
        onSubmit={dialogs.submitArchivePassword}
        open={dialogs.archivePasswordRequest !== null}
      />
      {pendingTransfer && (
        <TransferConflictDialog
          conflicts={pendingTransfer.conflicts}
          operation={pendingTransfer.operation}
          onCancel={cancelTransferConflicts}
          onResolve={resolveTransferConflicts}
        />
      )}
    </main>
  );
}
