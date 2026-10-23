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
 * - `inspector-panel.tsx`      — the window's inspector column, which the pane
 *                                publishes into (see `inspector-atom.ts`)
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
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";

import { tabPerfMark } from "@/lib/tab-perf";

import { commands, type ArchiveFormat, type UndoRedoOutcome } from "@/bindings";

import { formatBinding } from "@/features/settings/shortcut-registry";
import { hotkeysPausedAtom, useBinding } from "@/features/settings/settings-atoms";
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
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { copyWithNotice, notify } from "@/lib/notifications";
import { isWindowsPlatform } from "@/lib/platform";

import { ContentSearchResults, ContentSearchToolbar, useContentSearch } from "./content-search";
import { ContextualActionBar } from "./contextual-action-bar";
import { ArchivePasswordDialog } from "./archive-password-dialog";
import { BulkRenameDialog } from "./bulk-rename";
import { useDirectorySearch, type ExplorerSearchMode } from "./directory-search";
import { isLocalExplorerPath } from "./drag-drop";
import { isArchiveFile } from "./entry-context-menu";
import { displayNameOfPath, isWrongPasswordError } from "./explorer-errors";
import {
  DeleteDialog,
  ExplorerErrorAlert,
  RenameDialog,
  CreateEntryDialog,
} from "./explorer-dialogs";
import { ExplorerToolbar } from "./explorer-toolbar";
import { FileList, FileListSkeleton } from "./file-list";
import { useGitStatus } from "./git-status";
import { inspectorAtom } from "./inspector-atom";
import { preloadInspectorPanel } from "./inspector-panel";
import { listingViewOf, allNames } from "./listing-view";
import type { ExplorerNavigator } from "./navigation";
import { explorerStatusAtom } from "./status-strip-atom";
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
  // The inspector is the reference's canonical layout — list on the left, the
  // facts about the selection on the right — so a single-pane folder opens
  // with it. A split pane does not: 300px of inspector out of ~600 is not an
  // inspector, it is a second column of chrome. Neither does a window narrower
  // than 1180px, where the panel would leave the listing under 400px; there
  // the user can still ask for it by hand. `splitEnabled` is only read for the
  // initial value; after that the toggle belongs to the user.
  const [isPreviewOpen, setIsPreviewOpen] = useState(
    () =>
      !splitEnabled &&
      (typeof window === "undefined" || !window.matchMedia("(max-width: 1180px)").matches),
  );
  const [searchMode, setSearchMode] = useState<ExplorerSearchMode>("name");
  /** Bumped by Ctrl+L / Alt+D; the path bar edits on every increment. The
   *  signal is pane-local, so in a split only the active pane's bar reacts. */
  const [pathEditSignal, setPathEditSignal] = useState(0);
  // Live bindings for the operations a notification offers to repeat. They used
  // to be baked into the translation strings ("收起预览面板 (Space)"), which meant
  // a rebind left the message teaching a key that no longer did anything.
  const undoBinding = formatBinding(useBinding("explorer.undo"));
  const redoBinding = formatBinding(useBinding("explorer.redo"));

  // One notification slot per channel, so a pane's messages REPLACE their own
  // predecessors instead of stacking: two deletes in a row must not leave an
  // offer to undo the first one standing under a message about the second, and
  // two failures of the same operation are one complaint. `useId` keeps the two
  // panes of a split out of each other's slots.
  const paneNotificationId = useId();
  // The status strip is drawn by the window (below the terminal drawer), so the
  // pane publishes its facts instead of painting a row of its own. This is the
  // pane's identity in that atom: a pane may only clear the strip while the
  // strip is still its own.
  const statusOwner = useId();
  const setExplorerStatus = useSetAtom(explorerStatusAtom);
  // The inspector is the window's third column too (right of the terminal
  // drawer and the status strip, not above them), so the pane publishes the
  // panel instead of painting it beside its listing. Same ownership rule: a
  // pane may only clear the column while the column is still its own, which is
  // what makes a split show the focused pane's inspector rather than a race
  // between the two.
  const inspectorOwner = useId();
  const setInspector = useSetAtom(inspectorAtom);
  const notificationIds = useMemo(
    () => ({
      operationError: `explorer-operation-error-${paneNotificationId}`,
      undoRedo: `explorer-undo-redo-${paneNotificationId}`,
    }),
    [paneNotificationId],
  );

  // Every file-operation failure in this pane reports through one port, so a
  // failure is surfaced the same way whichever flow raised it — the transfer
  // pipeline, a dialog submission, an entry action. The port keeps the shape it
  // had when it drove a banner (set a message, clear with `null`): the flows
  // genuinely use both halves, and a cleared error is a notification the user
  // has already moved past — a new directory, a second attempt.
  const setOperationError = useCallback(
    (error: string | null) => {
      if (error === null) {
        notify.dismiss(notificationIds.operationError);
        return;
      }

      notify.error(error, {
        id: notificationIds.operationError,
        title: t("explorer:errors.operationFailedTitle"),
      });
    },
    [notificationIds.operationError, t],
  );

  // The undo and redo notifications offer each other, and both self-gate on the
  // backend's stack — which only hears about an operation once it has reported
  // back. Capturing a handler when the message is raised would therefore freeze
  // the stack state from that moment: a redo button that had not yet been told
  // it could redo. The buttons read the current render's handler instead.
  const stepHistoryRef = useRef<(direction: "undo" | "redo") => void>(() => undefined);

  // A notification is mounted at the window rather than in the pane, but
  // everything this pane offers to do acts on this pane's directory and
  // selection. Closing the tab or collapsing the split therefore takes its
  // notifications with it, instead of leaving an offer that would act on a
  // folder nobody is looking at. The stack itself survives both — the toolbar
  // and Ctrl+Z still reach it.
  useEffect(
    () => () => {
      notify.dismiss(notificationIds.operationError);
      notify.dismiss(notificationIds.undoRedo);
    },
    [notificationIds],
  );

  const directory = state.directory;
  const listing = state.listing;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  // The listing is what gates the first paint; git status arrives on its own and
  // the list only reads it for a badge. Marking the moment the navigator stops
  // loading is what tells the two apart in an adoption, where both are IPC calls
  // that settle within a few milliseconds of each other.
  useEffect(() => {
    if (!isLoading) tabPerfMark("listing");
  }, [isLoading]);
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;

  const search = useDirectorySearch(directoryPath ?? null, directory, searchMode === "name");
  const contentSearch = useContentSearch(
    directoryPath ?? null,
    directory,
    searchMode === "content",
  );
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

  const { selectedPaths, setSelectedPaths, selectedEntries, selectAll } = useExplorerSelection({
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
  const { performFileOperation, fileOperationProgress, isOperationPending } = useFileOperations({
    directoryPath,
    refresh,
  });

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

  // The preview's own chunk is warmed a frame into the explorer's life: the
  // listing's first read has the disk to itself first, and the panel is a
  // toggle away at the earliest. Deferred so long as the user never opens it,
  // a window sitting on the Overview never asks for it at all.
  useEffect(() => {
    const frame = requestAnimationFrame(preloadInspectorPanel);
    return () => cancelAnimationFrame(frame);
  }, []);

  // Crossing into a window too narrow for the canonical three-column layout
  // closes the inspector. It never re-opens itself: re-opening on the way back
  // up would fight a user who deliberately closed it at a wide size.
  useEffect(() => {
    const query = window.matchMedia("(max-width: 1180px)");
    const onChange = (event: MediaQueryListEvent) => {
      if (event.matches) setIsPreviewOpen(false);
    };
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  // A new directory/query resets the error banner. The preview pane is not
  // reset with it any more: the inspector is a *panel* the user opened, and
  // collapsing it on every navigation made the reference's canonical
  // list-plus-inspector layout impossible to keep — every folder click threw
  // it away, including the very first one after the window opened.
  useEffect(() => {
    setOperationError(null);
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

    // One newline-joined string as far as the clipboard is concerned; the count
    // is how many paths went into it, which is what the notification reports.
    void copyWithNotice(
      selectedEntries.map((entry) => entry.path).join("\n"),
      selectedEntries.length,
    );
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

  /** Space toggles the preview surface for the current selection. */
  // No open animation on purpose. The shared-element morph this used to run
  // (and, before it, the panel's own entrance fade) read as a window-wide
  // flicker rather than as motion: when a material is active the window canvas
  // is translucent, so anything that animates compositing at the root level
  // spends the duration below `--pane-alpha` and the desktop shows through.
  const togglePreview = useCallback(() => {
    setIsPreviewOpen((open) => !open);
  }, []);

  /** The collapse control in the window's inspector column: the flag lives in
   *  the pane, so the panel reaches it through the published snapshot. */
  const closePreview = useCallback(() => {
    setIsPreviewOpen(false);
  }, []);

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
    [
      dialogs.openArchivePassword,
      directoryPath,
      performFileOperation,
      selectedEntries,
      setOperationError,
    ],
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
      // The offer is the point of the message: a delete that can still be taken
      // back says so, and says which key does it.
      notify.withAction(
        t("explorer:undoRedo.toast_trash", { count: paths.length }),
        {
          hint: undoBinding,
          label: t("explorer:actions.undo"),
          onClick: () => stepHistoryRef.current("undo"),
        },
        notificationIds.undoRedo,
      );
    });
  }, [
    notificationIds.undoRedo,
    performFileOperation,
    selectedEntries,
    setOperationError,
    setSelectedPaths,
    t,
    undoBinding,
  ]);

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

  /**
   * Steps the backend's history stack in one direction and announces the result
   * with the offer to step back the other way.
   *
   * The two directions are one function rather than two because of that offer:
   * an undo announces a redo and a redo announces an undo, so two functions
   * would have to reference each other, and every callback that names the other
   * is recreated whenever the other is — a cycle that never settles. The
   * direction is therefore a parameter, and the notification's button is the
   * only place it is read from.
   */
  const stepHistory = useCallback(
    (direction: "undo" | "redo") => {
      const canStep = direction === "undo" ? undoRedo.canUndo : undoRedo.canRedo;
      if (!canStep || isOperationPending) return;

      notify.dismiss(notificationIds.undoRedo);
      setOperationError(null);
      let outcome: UndoRedoOutcome | null = null;
      void performFileOperation(async (operationId) => {
        outcome =
          direction === "undo"
            ? await commands.undoOperation(operationId!)
            : await commands.redoOperation(operationId!);
      }, "auto").then((result) => {
        if (!result.ok) {
          setOperationError(
            t(
              direction === "undo"
                ? "explorer:undoRedo.failedUndo"
                : "explorer:undoRedo.failedRedo",
              { detail: result.error },
            ),
          );
          return;
        }
        if (!outcome) return;

        // The step just taken is the one the message reports; the step still
        // available is its opposite, and that is what the button offers.
        const next = direction === "undo" ? "redo" : "undo";
        notify.withAction(
          t(`explorer:undoRedo.toast_${outcome.action}`, {
            count: outcome.count,
            op: t(`explorer:undoRedo.op_${outcome.op}`),
          }),
          {
            hint: next === "undo" ? undoBinding : redoBinding,
            label: t(next === "undo" ? "explorer:actions.undo" : "explorer:actions.redo"),
            onClick: () => stepHistoryRef.current(next),
          },
          notificationIds.undoRedo,
        );
      });
    },
    [
      isOperationPending,
      notificationIds.undoRedo,
      performFileOperation,
      redoBinding,
      setOperationError,
      t,
      undoBinding,
      undoRedo.canRedo,
      undoRedo.canUndo,
    ],
  );

  useEffect(() => {
    stepHistoryRef.current = stepHistory;
  }, [stepHistory]);

  /** Reverts the most recent recorded operation (move, rename, copy, trash,
   *  create, duplicate) through the backend history stack. */
  const undoLastOperation = useCallback(() => stepHistory("undo"), [stepHistory]);

  /** Re-applies the most recently undone operation. */
  const redoLastOperation = useCallback(() => stepHistory("redo"), [stepHistory]);

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

  // Publish this pane's foot facts for the window's status strip. Only the
  // focused pane speaks: in a split, the pane that loses focus clears its own
  // claim (and only its own), so the strip follows focus instead of freezing on
  // whichever pane published last.
  //
  // This lives here, in the pane, because the window has no way to know the
  // count of a directory it did not read. It is a passive effect rather than a
  // render-time write so a large listing scan does not put a second setState
  // into its own commit.
  useEffect(() => {
    if (!isActivePane) {
      setExplorerStatus((current) => (current?.owner === statusOwner ? null : current));
      return;
    }

    setExplorerStatus({
      owner: statusOwner,
      progress: fileOperationProgress,
      isLoading,
      isSearching: search.isSearching || contentSearch.isSearching,
      itemCount: listingCount,
      path: directory?.path ?? null,
      selectedCount: selectedPaths.length,
      git: gitStatus ? { branch: gitStatus.branch, root: gitStatus.root } : null,
    });
  }, [
    contentSearch.isSearching,
    directory?.path,
    fileOperationProgress,
    gitStatus?.branch,
    gitStatus?.root,
    isActivePane,
    isLoading,
    listingCount,
    search.isSearching,
    selectedPaths.length,
    setExplorerStatus,
    statusOwner,
  ]);

  // A pane that unmounts (a tab closed, a split collapsed) must not leave a
  // strip describing a folder nobody is looking at.
  useEffect(
    () => () => {
      setExplorerStatus((current) => (current?.owner === statusOwner ? null : current));
    },
    [setExplorerStatus, statusOwner],
  );

  // Publish the inspector for the window's third column, under the same rules
  // as the strip above: only the focused pane speaks, a pane that loses focus
  // clears only its own claim, and a selection change republishes. The panel
  // and the actions its buttons run belong to this pane — the window only owns
  // the 300px it is painted in.
  useEffect(() => {
    if (!isActivePane) {
      setInspector((current) => (current?.owner === inspectorOwner ? null : current));
      return;
    }

    setInspector({
      owner: inspectorOwner,
      isOpen: isPreviewOpen,
      entries: selectedEntries,
      onClose: closePreview,
      onOpen: openSelectedEntries,
    });
  }, [
    closePreview,
    inspectorOwner,
    isPreviewOpen,
    openSelectedEntries,
    selectedEntries,
    setInspector,
  ]);

  // Same teardown rule: a collapsed split or a closed tab takes its panel with
  // it instead of leaving the last selection up beside another pane.
  useEffect(
    () => () => {
      setInspector((current) => (current?.owner === inspectorOwner ? null : current));
    },
    [inspectorOwner, setInspector],
  );

  return (
    <main className="h-full" data-explorer-container="true">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <ExplorerToolbar
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          canGoUp={canGoUp}
          contentSearch={contentSearch}
          directory={directory}
          isActivePane={isActivePane}
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

        {/* File-operation failures, and the "this app would not start" report a
            shell command gives back, are notifications now — they carry the
            same title/detail split, outlive the menu that raised them, and are
            dismissed by the notification host rather than by a control inside
            the pane. The pane keeps only the failure that leaves it with
            nothing to show: an unreadable directory, below. */}

        {directory ? (
          // The listing stopped being a row of two columns: the inspector is now
          // the window's third column (`inspector-panel.tsx`), so nothing here
          // competes with the listing for width. The pane publishes the panel
          // instead of painting one.
          <div className="relative flex min-h-0 flex-1 flex-col">
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
                  onClearSearch={() => search.setQuery("")}
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
          </div>
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
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
