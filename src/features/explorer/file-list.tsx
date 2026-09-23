import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useHotkeys } from "@tanstack/react-hotkeys";
import { openPath } from "@tauri-apps/plugin-opener";
import { commands, type ArchiveFormat } from "@/bindings";
import { i18n } from "@/i18n";
import { localeDateTimeFormat, localeNumber, localeNumberFormat } from "@/i18n/format";
import {
  AppWindow,
  BoxSelect,
  ChevronDown,
  ChevronUp,
  Clipboard,
  Copy,
  FilePlus,
  Folder,
  FolderPlus,
  Link,
  Redo2,
  Scissors,
  LayoutGrid,
  Star,
  SquareTerminal,
  TriangleAlert,
  Undo2,
} from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Kbd } from "@/components/ui/kbd";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";
import { isEditableElement } from "@/lib/dom";
import { jumpListIndex, stepListIndex, typeAheadIndex } from "@/lib/list-navigation";
import { useMeasuredWidth } from "@/lib/use-element-width";

import { recordRecentItem } from "@/features/workspace/recents-atoms";
import { appSettingsAtom, hotkeysPausedAtom } from "@/features/settings/settings-atoms";
import { formatBinding, resolveBinding } from "@/features/settings/shortcut-registry";
import { HOTKEY_COMMON_OPTIONS, asHotkey, guardedAction } from "@/features/settings/hotkeys";

import {
  canDropEntries,
  dragOperationFromModifiers,
  dragOutModeFromModifiers,
  getExplorerDropTargetAtPoint,
  getSidebarSpaceDropTargetAtPoint,
  isLocalExplorerPath,
  isOverSidebarFavoritesAtPoint,
  type FileTransferOperation,
  type TransferOperation,
} from "./drag-drop";
import { EntryIconFrame, HIDDEN_ENTRY_CLASS } from "./entry-badges";
import { EntryContextMenuContent } from "./entry-context-menu";
import { FileColumnView } from "./file-column-view";
import { getEntryPresentation } from "./file-icons";
import { FileGridView } from "./file-grid-view";
import { getEntryGitStatus, GitStatusBadge, type ExplorerGitStatus } from "./git-status";
import { TypeIconTile } from "./icon-tile";
import {
  allPaths,
  directoryPathSet,
  draggableDirectoryPaths,
  pathsInRange,
  type ListingView,
} from "./listing-view";
import { MarqueeOverlay, useMarqueeSelection, type MarqueeRect } from "./marquee";
import { isNativeIconSupported, NativeIconImage } from "./native-icon";
import {
  DEFAULT_SORT_ORDER,
  DENSITY_ROW_HEIGHT,
  densityAtom,
  showHiddenFilesAtom,
  sortKeyAtom,
  sortOrderAtom,
  viewModeAtom,
  type ExplorerSortKey,
} from "./preferences";
import type { DirectoryEntry } from "./types";

/** Shared stand-in for "not dragging": a fresh empty Set per render would
 *  change identity every time and break memoization of the grid/column views
 *  that receive it as a prop. */
const NO_DRAGGING_PATHS: Set<string> = new Set();

interface FileListProps {
  canRedo: boolean;
  canUndo: boolean;
  currentDirectoryPath: string;
  entries: ListingView;
  externalDropItemCount: number;
  externalDropTargetPath: string | null;
  gitStatus?: ExplorerGitStatus | null;
  initialScrollOffset?: number;
  /** Window-level shortcuts only fire for the focused pane of the
   *  dual-pane layout; always true in the single-pane layout. */
  isActivePane: boolean;
  isLoading: boolean;
  isOperationPending: boolean;
  onAddToFavorites: (paths: string[]) => void;
  onAddToSpace: (spaceId: string, paths: string[]) => void;
  onCompress: (format: ArchiveFormat, encrypted: boolean) => void;
  onCopy: () => void;
  onCreateDirectory: () => void;
  onCreateFile: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDeletePermanent: () => void;
  onDuplicate: () => void;
  onDropEntries: (
    sourcePaths: string[],
    destinationPath: string,
    operation: TransferOperation,
  ) => void;
  onCreateShortcuts: (sourcePaths: string[], destinationPath: string) => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpenDirectory: (path: string) => void;
  onOpenTerminal: () => void;
  onOpenWith: () => void;
  onPaste: () => void;
  onRedo: () => void;
  onRename: () => void;
  onScrollOffsetChange?: (offset: number) => void;
  onSelectAll: () => void;
  onSelectedPathsChange: (paths: string[]) => void;
  onTogglePreview: () => void;
  onUndo: () => void;
  searchState?: FileListSearchState;
  selectedPaths: string[];
  viewId: string;
}

interface FileListSearchState {
  error: string | null;
  isSearching: boolean;
  query: string;
  truncated: boolean;
}

/**
 * What a listing surface must provide for the shared keyboard navigation
 * (`FileList.handleNavKeyDown`). The detail list and the grid both mount
 * their own scroller and their own virtualizer, so they describe their
 * geometry here and get cursor movement, type-ahead, paging and focus back
 * for free.
 */
export interface ListingNavContext {
  /** The scroller the key event bubbled through; focus targets are searched
   *  within it so a split pane's twin list is never focused by mistake. */
  container: HTMLElement;
  /** 1 in the detail list (vertical only), the measured count in the grid. */
  columns: number;
  /** Distance between row tops — row height in the list, cell height + gap in
   *  the grid — so paging follows the real geometry. */
  rowStride: number;
  /** Scroll content above the first row (the list's sticky column header). */
  leadingOffset: number;
  /** Brings the target into view before focus moves: the list scrolls its
   *  row, the grid its whole virtual row. */
  scrollToEntry: (index: number) => void;
}

/** How many frames a focus request may wait for its virtualized row to mount.
 *  A second is the budget: a smooth scroll or a re-windowed virtual row can
 *  take several frames to land, and a focus that gives up early strands the
 *  keyboard on the previous row. */
const FOCUS_RETRY_FRAMES = 60;

/** How long a type-ahead buffer survives after the last keystroke. */
const TYPE_AHEAD_TIMEOUT_MS = 800;

const MODIFIED_DATE_FORMAT_OPTIONS: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
};

const FILE_SIZE_FORMAT_OPTIONS: Intl.NumberFormatOptions = {
  maximumFractionDigits: 1,
};

const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

const DRAG_START_DISTANCE_PX = 6;

/**
 * Column thresholds for the detail listing, measured against the pane the
 * listing is drawn in (see `useElementWidth`).
 *
 * The row used to be a fixed 58rem grid with a `min-w-160` floor on its
 * wrapper, so a pane narrower than that scrolled sideways and every row's
 * cells slid out from under their headers. Columns are now dropped instead, in
 * the order that costs the least: type first (it is already implied by the
 * icon), then size, then the modified date — which is the last column worth
 * losing, because the sort order may be keyed on it.
 *
 * The name column is `1fr`, so a wide pane gives the extra room to the one
 * column that can use it and the row never exceeds its container.
 */
const LIST_COLUMN_MIN_WIDTH_PX = { modified: 340, size: 480, type: 640 } as const;

/**
 * Where the last column ends, shared by the sticky header grid and every row
 * wrapper: 36.5rem of name plus the 9.5+6.5+5.5 rem the three metadata tracks
 * ask for.
 *
 * The name track is `1fr`, so this cap is what makes the two grids resolve
 * that track identically: a header free to span a wider pane would pour the
 * extra width into its own name column and slide the modified/type/size
 * headings out from under the values they label. Cap one without the other
 * and the columns drift by exactly `pane width - 58rem`.
 */
const LIST_CONTENT_MAX_WIDTH = "58rem";

interface ListColumns {
  modified: boolean;
  size: boolean;
  type: boolean;
}

const ALL_LIST_COLUMNS: ListColumns = { modified: true, size: true, type: true };

function listColumnsForWidth(width: number): ListColumns {
  if (width <= 0) return ALL_LIST_COLUMNS;
  return {
    modified: width >= LIST_COLUMN_MIN_WIDTH_PX.modified,
    size: width >= LIST_COLUMN_MIN_WIDTH_PX.size,
    type: width >= LIST_COLUMN_MIN_WIDTH_PX.type,
  };
}

/** Grid template matching `columns` cell-for-cell — the two must not drift. */
function listGridTemplate(columns: ListColumns): string {
  return [
    "minmax(0,1fr)",
    columns.modified ? "minmax(0,9.5rem)" : null,
    columns.type ? "minmax(0,6.5rem)" : null,
    columns.size ? "minmax(0,5.5rem)" : null,
  ]
    .filter((track): track is string => track !== null)
    .join(" ");
}
const LIST_HEADER_HEIGHT_PX = 28;

type InternalDragTarget =
  | { kind: "directory"; path: string }
  | { kind: "favorites" }
  | { kind: "space"; spaceId: string };

type InternalDragState = {
  operation: FileTransferOperation;
  pointerId: number;
  position: { x: number; y: number };
  sourcePaths: string[];
  target: InternalDragTarget | null;
};

type DragCandidate = {
  pointerId: number;
  startX: number;
  startY: number;
  sourcePaths: string[];
};

function resolveDragTarget(
  entries: ListingView,
  sourcePaths: string[],
  x: number,
  y: number,
): InternalDragTarget | null {
  const draggableDirectories = draggableDirectoryPaths(entries, sourcePaths);

  if (draggableDirectories.length > 0 && isOverSidebarFavoritesAtPoint(x, y)) {
    return { kind: "favorites" };
  }

  const spaceId = getSidebarSpaceDropTargetAtPoint(x, y);
  if (draggableDirectories.length > 0 && spaceId !== null) {
    return { kind: "space", spaceId };
  }

  const targetPath = getExplorerDropTargetAtPoint(x, y);
  if (targetPath && canDropEntries(sourcePaths, targetPath)) {
    return { kind: "directory", path: targetPath };
  }

  return null;
}

function targetsAreEqual(
  left: InternalDragTarget | null,
  right: InternalDragTarget | null,
): boolean {
  if (left === right) return true;
  if (!left || !right || left.kind !== right.kind) return false;

  if (left.kind === "favorites") return true;
  if (left.kind === "space") return left.spaceId === (right as { spaceId: string }).spaceId;

  return left.path === (right as { path: string }).path;
}

export function FileList({
  canRedo,
  canUndo,
  currentDirectoryPath,
  entries,
  externalDropItemCount,
  externalDropTargetPath,
  gitStatus,
  initialScrollOffset = 0,
  isActivePane,
  isLoading,
  isOperationPending,
  onAddToFavorites,
  onAddToSpace,
  onCompress,
  onCopy,
  onCreateDirectory,
  onCreateFile,
  onCut,
  onDelete,
  onDeletePermanent,
  onDuplicate,
  onDropEntries,
  onCreateShortcuts,
  onExtract,
  onMoveTo,
  onOpenDirectory,
  onOpenTerminal,
  onOpenWith,
  onPaste,
  onRedo,
  onRename,
  onScrollOffsetChange,
  onSelectAll,
  onSelectedPathsChange,
  onTogglePreview,
  onUndo,
  searchState,
  selectedPaths,
  viewId,
}: FileListProps) {
  const { t } = useTranslation("explorer");
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectionAnchorIndexRef = useRef<number | null>(null);
  /** Index of the row the keyboard is on — the roving tab stop and the range
   *  origin for Shift+arrows. Null until the first click or key press. */
  const [cursorIndex, setCursorIndex] = useState<number | null>(null);
  /** Latest focus request; a newer move cancels an older retry loop so two
   *  quick arrow presses cannot leave focus on the first target. */
  const focusRequestRef = useRef<string | null>(null);
  const typeAheadRef = useRef<{ buffer: string; timer: number | null }>({ buffer: "", timer: null });
  const dragCandidateRef = useRef<DragCandidate | null>(null);
  const internalDragRef = useRef<InternalDragState | null>(null);
  const suppressNextClickRef = useRef(false);
  const [internalDrag, setInternalDrag] = useState<InternalDragState | null>(null);
  const viewMode = useAtomValue(viewModeAtom);
  const density = useAtomValue(densityAtom);
  const [sortKey, setSortKey] = useAtom(sortKeyAtom);
  const [sortOrder, setSortOrder] = useAtom(sortOrderAtom);
  const setShowHiddenFiles = useSetAtom(showHiddenFilesAtom);
  const rowHeight = DENSITY_ROW_HEIGHT[density];
  const selectedPathSet = new Set(selectedPaths);
  const listIsLoading = isLoading || searchState?.isSearching === true;
  const actionsDisabled = listIsLoading || isOperationPending;
  const selectedCount = selectedPaths.length;
  const activeViewMode = viewMode === "column" && searchState ? "list" : viewMode;
  // Which detail columns fit in this pane. `0` (unmeasured, or the listing is
  // not mounted at all because the grid view is showing) reads as "all of
  // them", so the first paint is never missing a column it should have.
  const listColumns = listColumnsForWidth(useMeasuredWidth(scrollRef, activeViewMode));
  const listTemplate = listGridTemplate(listColumns);
  const virtualizer = useVirtualizer({
    count: entries.count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    initialOffset: initialScrollOffset,
    overscan: 10,
  });

  useEffect(() => {
    selectionAnchorIndexRef.current = null;
    setCursorIndex(null);
    focusRequestRef.current = null;
    if (scrollRef.current) {
      scrollRef.current.scrollTop = initialScrollOffset;
    }
  }, [initialScrollOffset, viewId]);

  // The cursor may not outlive its row: a filter, a re-sort or a delete
  // shortens the listing under the keyboard's feet, and an index past the end
  // would send the next arrow press out of range.
  useEffect(() => {
    setCursorIndex((current) => {
      if (current === null) return null;
      if (entries.count === 0) return null;
      return current < entries.count ? current : entries.count - 1;
    });
  }, [entries.count]);

  // Type-to-jump names, kept out of the key handler so one keystroke does not
  // rebuild the array for every row.
  const entryNames = useMemo(() => {
    const names: string[] = [];
    for (let index = 0; index < entries.count; index += 1) {
      const entry = entries.entryAt(index);
      if (entry) names.push(entry.name);
    }
    return names;
  }, [entries]);

  // Stops the buffered type-ahead timer when the tab (and this list) unmounts.
  useEffect(
    () => () => {
      if (typeAheadRef.current.timer !== null) {
        window.clearTimeout(typeAheadRef.current.timer);
      }
    },
    [],
  );

  const shortcuts = useAtomValue(appSettingsAtom)?.shortcuts;
  const hotkeysPaused = useAtomValue(hotkeysPausedAtom);
  // Empty folders teach the palette shortcut (Raycast convention): the one
  // moment there is nothing to click is the moment worth spending on a
  // keyboard affordance.
  const commandBarBinding = formatBinding(resolveBinding(shortcuts, "app.commandBar"));

  // Explorer keyboard shortcuts, migrated to user-configurable TanStack
  // Hotkeys. Every action is gated on this pane being the focused one (the
  // inactive split pane ignores them entirely) and on the recorder not
  // capturing a new binding; the per-action conditions mirror the previous
  // single handler exactly. Event-level guards — already-prevented, IME
  // composing, focus in an editable surface — and "swallow the key only when
  // the action runs" live in `guardedAction`, so a combo pressed with nothing
  // selected still falls through untouched.
  const hotkeysActive = isActivePane && !hotkeysPaused;
  const hasSelection = selectedCount > 0;
  const canModifySelection = hasSelection && !actionsDisabled;
  useHotkeys(
    [
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.clearSelection")),
        callback: guardedAction(() => onSelectedPathsChange([]), { preventDefault: false }),
        options: { enabled: hotkeysActive && hasSelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.copy")),
        callback: guardedAction(onCopy),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.cut")),
        callback: guardedAction(onCut),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.paste")),
        callback: guardedAction(onPaste),
        options: { enabled: hotkeysActive && !actionsDisabled },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.selectAll")),
        callback: guardedAction(() => onSelectedPathsChange(allPaths(entries))),
        options: { enabled: hotkeysActive && !listIsLoading },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.toggleHidden")),
        callback: guardedAction(() => setShowHiddenFiles((visible) => !visible)),
        options: { enabled: hotkeysActive },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.rename")),
        callback: guardedAction(onRename),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.preview")),
        callback: guardedAction(onTogglePreview),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.undo")),
        callback: guardedAction(onUndo),
        options: { enabled: hotkeysActive && canUndo && !actionsDisabled },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.redo")),
        callback: guardedAction(onRedo),
        options: { enabled: hotkeysActive && canRedo && !actionsDisabled },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.redoAlt")),
        callback: guardedAction(onRedo),
        options: { enabled: hotkeysActive && canRedo && !actionsDisabled },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.trash")),
        callback: guardedAction(onDelete),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.deletePermanent")),
        callback: guardedAction(onDeletePermanent),
        options: { enabled: hotkeysActive && canModifySelection },
      },
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.openSystemTerminal")),
        callback: guardedAction(onOpenTerminal),
        options: { enabled: hotkeysActive },
      },
      {
        // The creation shortcut every file manager has; the blank-area context
        // menu was its only entry point. Fixed like the list's other
        // non-rebindable keys (arrows, type-ahead) rather than registered —
        // the registry mirrors the Rust defaults one-for-one.
        hotkey: asHotkey("Mod+Shift+N"),
        callback: guardedAction(onCreateDirectory),
        options: { enabled: hotkeysActive && !actionsDisabled },
      },
    ],
    HOTKEY_COMMON_OPTIONS,
  );

  useEffect(() => {
    const updateDrag = (nextDrag: InternalDragState | null) => {
      internalDragRef.current = nextDrag;
      setInternalDrag(nextDrag);
    };

    const stopDragging = () => {
      dragCandidateRef.current = null;
      updateDrag(null);
    };

    const handlePointerMove = (event: PointerEvent) => {
      const candidate = dragCandidateRef.current;
      if (!candidate || candidate.pointerId !== event.pointerId) return;

      const distanceX = event.clientX - candidate.startX;
      const distanceY = event.clientY - candidate.startY;
      const isDragging = internalDragRef.current !== null;
      if (!isDragging && Math.hypot(distanceX, distanceY) < DRAG_START_DISTANCE_PX) return;

      // Chromium keeps delivering pointermove while the button is held, even
      // outside the webview, so an out-of-bounds position means the user is
      // dragging towards another app. Hand the gesture to a native OS drag
      // (OLE DoDragDrop); dropping back onto our own window still works via
      // the external-drop path.
      if (
        isDragging &&
        (event.clientX < 0 ||
          event.clientY < 0 ||
          event.clientX >= window.innerWidth ||
          event.clientY >= window.innerHeight)
      ) {
        const dragPaths = internalDragRef.current?.sourcePaths ?? [];
        suppressNextClickRef.current = true;
        stopDragging();

        const localPaths = dragPaths.filter(isLocalExplorerPath);
        if (localPaths.length > 0) {
          // Windows conventions: plain/Ctrl copies out, Shift moves,
          // Alt (or Ctrl+Shift) creates shortcuts at the drop target.
          const dragOutMode = dragOutModeFromModifiers(event);
          void commands.startDragOut(localPaths, dragOutMode).catch((error) => {
            console.warn("Unable to start the native drag-out", error);
          });
        }
        return;
      }

      // Windows conventions inside the window: Alt (or Ctrl+Shift) links,
      // Ctrl copies, plain/Shift moves (Explorer's same-volume default).
      const operation: FileTransferOperation = dragOperationFromModifiers(event);
      const nextTarget = resolveDragTarget(
        entries,
        candidate.sourcePaths,
        event.clientX,
        event.clientY,
      );
      const previousDrag = internalDragRef.current;

      if (
        previousDrag &&
        previousDrag.position.x === event.clientX &&
        previousDrag.position.y === event.clientY &&
        previousDrag.operation === operation &&
        targetsAreEqual(previousDrag.target, nextTarget)
      ) {
        return;
      }

      event.preventDefault();
      updateDrag({
        operation,
        pointerId: candidate.pointerId,
        position: { x: event.clientX, y: event.clientY },
        sourcePaths: candidate.sourcePaths,
        target: nextTarget,
      });
    };

    const handlePointerUp = (event: PointerEvent) => {
      const candidate = dragCandidateRef.current;
      const activeDrag = internalDragRef.current;
      if (!candidate || candidate.pointerId !== event.pointerId) return;

      if (activeDrag) {
        suppressNextClickRef.current = true;
        if (activeDrag.target?.kind === "directory") {
          if (activeDrag.operation === "link") {
            onCreateShortcuts(activeDrag.sourcePaths, activeDrag.target.path);
          } else {
            onDropEntries(activeDrag.sourcePaths, activeDrag.target.path, activeDrag.operation);
          }
        } else if (activeDrag.target?.kind === "favorites") {
          onAddToFavorites(draggableDirectoryPaths(entries, activeDrag.sourcePaths));
        } else if (activeDrag.target?.kind === "space") {
          onAddToSpace(
            activeDrag.target.spaceId,
            draggableDirectoryPaths(entries, activeDrag.sourcePaths),
          );
        }
      }

      stopDragging();
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", stopDragging);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", stopDragging);
    };
  }, [entries, onAddToFavorites, onAddToSpace, onCreateShortcuts, onDropEntries]);

  const selectEntry = (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => {
    if (actionsDisabled) return;

    const isToggleSelection = event.ctrlKey || event.metaKey;
    const anchorIndex = selectionAnchorIndexRef.current;

    if (event.shiftKey && anchorIndex !== null && index >= 0) {
      const [start, end] = [anchorIndex, index].sort((left, right) => left - right);
      const range = pathsInRange(entries, start, end + 1);
      const nextSelection = isToggleSelection
        ? new Set([...selectedPaths, ...range])
        : new Set(range);
      // The range origin stays where it was; only the cursor follows the
      // click, so a second Shift+click still extends from the same anchor.
      setCursorIndex(index);
      onSelectedPathsChange([...nextSelection]);
      return;
    }

    if (index >= 0) {
      selectionAnchorIndexRef.current = index;
      setCursorIndex(index);
    }

    if (isToggleSelection) {
      const nextSelection = new Set(selectedPaths);
      if (nextSelection.has(entry.path)) {
        nextSelection.delete(entry.path);
      } else {
        nextSelection.add(entry.path);
      }
      onSelectedPathsChange([...nextSelection]);
      return;
    }

    onSelectedPathsChange([entry.path]);
  };

  const selectForContextMenu = (entry: DirectoryEntry, index: number) => {
    if (actionsDisabled || selectedPathSet.has(entry.path)) return;

    if (index >= 0) {
      selectionAnchorIndexRef.current = index;
      setCursorIndex(index);
    }

    onSelectedPathsChange([entry.path]);
  };

  const prepareInternalDrag = (entry: DirectoryEntry, event: ReactPointerEvent) => {
    if (actionsDisabled || event.button !== 0 || event.shiftKey) {
      return;
    }

    const sourcePaths = selectedPathSet.has(entry.path) ? selectedPaths : [entry.path];
    if (!selectedPathSet.has(entry.path) && !event.ctrlKey && !event.metaKey) {
      onSelectedPathsChange(sourcePaths);
    }

    dragCandidateRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      sourcePaths,
    };
  };

  const openEntry = (entry: DirectoryEntry) => {
    if (actionsDisabled) return;

    if (entry.kind === "directory") {
      onOpenDirectory(entry.path);
      return;
    }

    void openFile(entry.path);
  };

  /**
   * Moves the keyboard cursor to `next`: the selection collapses to that row,
   * or — with Shift — extends to the range from the anchor. Scrolling and
   * focus are the caller's job through {@link ListingNavContext}, because the
   * detail list and the grid virtualize differently.
   */
  const moveCursorTo = (next: number, extend: boolean) => {
    if (actionsDisabled || next < 0 || next >= entries.count) return;
    const path = entries.pathAt(next);
    if (path === undefined) return;

    if (extend) {
      const anchor = selectionAnchorIndexRef.current ?? cursorIndex;
      if (anchor !== null && anchor >= 0 && anchor < entries.count) {
        const [start, end] = [anchor, next].sort((left, right) => left - right);
        onSelectedPathsChange(pathsInRange(entries, start, end + 1));
      } else {
        selectionAnchorIndexRef.current = next;
        onSelectedPathsChange([path]);
      }
    } else {
      selectionAnchorIndexRef.current = next;
      onSelectedPathsChange([path]);
    }
    setCursorIndex(next);
  };

  /**
   * Focuses the row for `path` once virtualization has mounted it. The scroll
   * happens first (each view already called its own `scrollToEntry`), so the
   * row usually exists on the first frame; the retries cover the frame the
   * virtualizer still needs to commit. A newer request cancels an older loop.
   */
  const focusEntryRow = (container: HTMLElement, path: string) => {
    focusRequestRef.current = path;
    let frames = 0;
    const attempt = () => {
      if (focusRequestRef.current !== path || !container.isConnected) return;
      const row = Array.from(container.querySelectorAll<HTMLElement>("[data-entry-path]")).find(
        (element) => element.dataset.entryPath === path,
      );
      if (row) {
        row.focus({ preventScroll: true });
        focusRequestRef.current = null;
        return;
      }
      if (frames < FOCUS_RETRY_FRAMES) {
        frames += 1;
        requestAnimationFrame(attempt);
      }
    };
    requestAnimationFrame(attempt);
  };

  /**
   * Shared keyboard model for the detail list and the grid: arrows, Home/End,
   * paging, Enter and Windows-style type-to-jump (SKILL.md: the explorer's
   * rows used to be pointer-only — arrows now move the selection and the
   * roving tab stop together, exactly like the Trash list already did).
   *
   * Modified keys stand down for the registered hotkeys, an editable target
   * (the inline rename cell, the path editor) types rather than navigates, and
   * Space is left to the preview binding — which is also why it never joins
   * the type-ahead buffer.
   */
  const handleNavKeyDown = (event: ReactKeyboardEvent<HTMLElement>, context: ListingNavContext) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (actionsDisabled) return;
    const target = event.target;
    if (target instanceof Element && isEditableElement(target)) return;

    const count = entries.count;
    if (count === 0) return;
    const current = cursorIndex ?? -1;
    const extend = event.shiftKey;

    const commit = (next: number) => {
      if (next < 0 || next >= count) return;
      moveCursorTo(next, extend);
      context.scrollToEntry(next);
      const path = entries.pathAt(next);
      if (path !== undefined) focusEntryRow(context.container, path);
    };

    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        commit(stepListIndex(current, count, context.columns));
        return;
      case "ArrowUp":
        event.preventDefault();
        commit(stepListIndex(current, count, -context.columns));
        return;
      case "ArrowLeft":
      case "ArrowRight": {
        // A vertical list has no horizontal neighbour to move to; the grid
        // moves linearly, so a press at a row's edge crosses into the adjacent
        // row the way Explorer's icon view does.
        if (context.columns <= 1) return;
        event.preventDefault();
        commit(stepListIndex(current, count, event.key === "ArrowRight" ? 1 : -1));
        return;
      }
      case "Home":
        event.preventDefault();
        commit(0);
        return;
      case "End":
        event.preventDefault();
        commit(count - 1);
        return;
      case "PageUp":
      case "PageDown": {
        event.preventDefault();
        const rowsPerPage = Math.max(
          1,
          Math.floor((context.container.clientHeight - context.leadingOffset) / context.rowStride) -
            1,
        );
        commit(
          jumpListIndex(
            current,
            count,
            event.key === "PageUp" ? "pageUp" : "pageDown",
            rowsPerPage * context.columns,
          ),
        );
        return;
      }
      case "Enter": {
        // A focused row handled this press first and marked it prevented;
        // this branch is for a press whose focus sat on the header or on a row
        // that virtualization has since unmounted — open the cursor's entry.
        if (event.defaultPrevented) return;
        const entry = current >= 0 ? entries.entryAt(current) : undefined;
        if (!entry) return;
        event.preventDefault();
        openEntry(entry);
        return;
      }
      default:
        break;
    }

    if (event.key.length === 1 && event.key !== " " && !event.repeat) {
      const state = typeAheadRef.current;
      if (state.timer !== null) window.clearTimeout(state.timer);
      const buffer = state.buffer + event.key;
      typeAheadRef.current = {
        buffer,
        timer: window.setTimeout(() => {
          typeAheadRef.current = { buffer: "", timer: null };
        }, TYPE_AHEAD_TIMEOUT_MS),
      };
      const match = typeAheadIndex(entryNames, buffer, current);
      if (match >= 0) commit(match);
    }
  };

  /**
   * Column view child panes render entries fetched on their own, so the
   * dragged entry may not be part of the root `entries`. Fall back to the
   * entry's own kind for those instead of silently dropping them.
   */
  const directoryPathsForEntry = (entry: DirectoryEntry): string[] => {
    const sourcePaths = selectedPathSet.has(entry.path) ? selectedPaths : [entry.path];
    const knownDirectories = directoryPathSet(entries);

    return sourcePaths.filter((path) =>
      path === entry.path ? entry.kind === "directory" : knownDirectories.has(path),
    );
  };

  const addEntryToFavorites = (entry: DirectoryEntry) => {
    onAddToFavorites(directoryPathsForEntry(entry));
  };

  const addEntryToSpace = (entry: DirectoryEntry, spaceId: string) => {
    onAddToSpace(spaceId, directoryPathsForEntry(entry));
  };

  /** Swallows the click that ends an internal drag before it changes selection. */
  const selectEntryIfNotDragging = (
    entry: DirectoryEntry,
    index: number,
    event: ReactMouseEvent,
  ) => {
    if (suppressNextClickRef.current) {
      suppressNextClickRef.current = false;
      return;
    }
    selectEntry(entry, index, event);
  };

  const draggingPaths = internalDrag
    ? new Set(internalDrag.sourcePaths)
    : NO_DRAGGING_PATHS;
  const internalDropTargetPath =
    internalDrag?.target?.kind === "directory" ? internalDrag.target.path : null;

  // Bulk menu action callbacks shared by every row (SKILL.md §10).
  const menuActions = {
    onCompress,
    onCopy,
    onCut,
    onDelete,
    onDeletePermanent,
    onDuplicate,
    onExtract,
    onMoveTo,
    onOpenWith,
    onRename,
    onTogglePreview,
  };

  const blankMenuDisabled = actionsDisabled || Boolean(searchState);

  // Rubber-band selection over uniform-height rows (SKILL.md §19).
  const listMarquee = useMarqueeSelection({
    enabled: !actionsDisabled && activeViewMode === "list",
    getBaseSelection: () => selectedPaths,
    hitTest: (rect: MarqueeRect) => {
      const container = scrollRef.current;
      if (!container) return [];

      // Windows behavior: rows end at the last column's edge, so the
      // marquee must cross that content box horizontally too — a band
      // drawn over the blank area right of the columns selects nothing.
      const rowElement = container.querySelector<HTMLElement>('[role="option"]');
      if (!rowElement) return [];
      const rowBounds = rowElement.getBoundingClientRect();
      if (rect.right <= rowBounds.left || rect.left >= rowBounds.right) return [];

      const bounds = container.getBoundingClientRect();
      const topContent = rect.top - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      const bottomContent = rect.bottom - bounds.top + container.scrollTop - LIST_HEADER_HEIGHT_PX;
      if (bottomContent <= 0) return [];

      const firstRow = Math.max(0, Math.floor(topContent / rowHeight));
      const lastRow = Math.min(entries.count - 1, Math.ceil(bottomContent / rowHeight) - 1);
      if (lastRow < firstRow) return [];

      return pathsInRange(entries, firstRow, lastRow + 1);
    },
    onSelectionChange: onSelectedPathsChange,
    scrollElementRef: scrollRef,
  });

  /** Clicking the active column toggles direction; a new column starts at its
   * default direction (SKILL.md §18). */
  const applySort = (key: ExplorerSortKey) => {
    if (key === sortKey) {
      setSortOrder(sortOrder === "asc" ? "desc" : "asc");
      return;
    }

    setSortKey(key);
    setSortOrder(DEFAULT_SORT_ORDER[key]);
  };

  // Shared control bundle for grid and column views; the Compiler memoizes
  // both the object and every child prop automatically.
  const viewControls = {
    actionsDisabled,
    draggingPaths,
    dropTargetPath: internalDropTargetPath ?? externalDropTargetPath,
    gitStatus,
    menuActions,
    onAddToFavorites: addEntryToFavorites,
    onAddToSpace: addEntryToSpace,
    onContextMenuEntry: selectForContextMenu,
    onOpenEntry: openEntry,
    onPointerDownEntry: prepareInternalDrag,
    onSelectEntry: selectEntryIfNotDragging,
    onSelectedPathsChange,
    selectedCount,
    selectedPathSet,
  };

  return (
    <ContextMenu disabled={blankMenuDisabled}>
      <ContextMenuTrigger
        onContextMenu={(event) => {
          // Entries own their context menu and set the selection themselves;
          // only a right-click on blank space should clear it.
          if (
            event.target instanceof Element &&
            event.target.closest('[role="option"], [role="columnheader"]')
          ) {
            return;
          }
          selectionAnchorIndexRef.current = null;
          onSelectedPathsChange([]);
        }}
        render={
          <section
            aria-label={t("explorer:list.ariaLabel")}
            className="relative flex min-h-0 flex-1 flex-col"
            data-explorer-drop-target={currentDirectoryPath}
          />
        }
      >
        {entries.count === 0 && !listIsLoading ? (
          // The empty-state anatomy, shared with every other surface: one icon
          // on a recessed tile, a title, one line of description, and the
          // single action that can change the outcome. An empty folder and a
          // failed search differ only in the glyph's hue.
          <Empty className="min-h-0 flex-1 select-none">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                {searchState?.error ? (
                  <TriangleAlert className="text-warning" />
                ) : (
                  <Folder className="text-folder" fill="currentColor" />
                )}
              </EmptyMedia>
              <EmptyTitle>
                {searchState
                  ? searchState.error
                    ? t("explorer:list.searchError", { error: searchState.error })
                    : t("explorer:list.searchEmpty", { query: searchState.query })
                  : t("explorer:list.emptyFolder")}
              </EmptyTitle>
              {!searchState && (
                <EmptyDescription className="flex items-center gap-1.5">
                  <Kbd>{commandBarBinding}</Kbd>
                  <span>{t("explorer:list.emptyHint")}</span>
                </EmptyDescription>
              )}
            </EmptyHeader>
          </Empty>
        ) : activeViewMode === "grid" ? (
          <FileGridView
            {...viewControls}
            cursorIndex={cursorIndex}
            entries={entries}
            onNavKeyDown={handleNavKeyDown}
          />
        ) : activeViewMode === "column" ? (
          <FileColumnView {...viewControls} rootEntries={entries} viewId={viewId} />
        ) : (
          <div
            ref={scrollRef}
            className="min-h-0 flex-1 overflow-auto"
            onKeyDown={(event) =>
              handleNavKeyDown(event, {
                container: event.currentTarget,
                columns: 1,
                leadingOffset: LIST_HEADER_HEIGHT_PX,
                rowStride: rowHeight,
                scrollToEntry: (index) => virtualizer.scrollToIndex(index, { align: "auto" }),
              })
            }
            onPointerDown={(event) => {
              if (
                event.target instanceof Element &&
                event.target.closest('[role="option"], [role="columnheader"]')
              ) {
                return;
              }
              listMarquee.beginMarquee(event);
            }}
            onScroll={(event) => onScrollOffsetChange?.(event.currentTarget.scrollTop)}
          >
            <div>
              {/*
                The bar itself stays full-width so its background and border
                still cover the rows scrolling underneath; only the column
                track is capped, to the same edge the rows stop at.
              */}
              <div className="sticky top-0 z-10 h-7 shrink-0 border-b border-border bg-card">
                <div
                  className="grid h-full items-center justify-start text-label whitespace-nowrap text-muted-foreground"
                  style={{ gridTemplateColumns: listTemplate, maxWidth: LIST_CONTENT_MAX_WIDTH }}
                >
                  <SortHeaderCell
                    active={sortKey === "name"}
                    inset="name"
                    label={t("explorer:columns.name")}
                    onSort={() => applySort("name")}
                    order={sortOrder}
                  />
                  {listColumns.modified && (
                    <SortHeaderCell
                      active={sortKey === "modified"}
                      label={t("explorer:columns.modified")}
                      onSort={() => applySort("modified")}
                      order={sortOrder}
                    />
                  )}
                  {listColumns.type && (
                    <SortHeaderCell
                      active={sortKey === "type"}
                      label={t("explorer:columns.type")}
                      onSort={() => applySort("type")}
                      order={sortOrder}
                    />
                  )}
                  {listColumns.size && (
                    <SortHeaderCell
                      active={sortKey === "size"}
                      align="right"
                      label={t("explorer:columns.size")}
                      onSort={() => applySort("size")}
                      order={sortOrder}
                    />
                  )}
                </div>
              </div>
              <div
                aria-multiselectable="true"
                className="relative"
                role="listbox"
                style={{ height: virtualizer.getTotalSize() }}
              >
                {virtualizer.getVirtualItems().map((virtualRow) => {
                  const entry = entries.entryAt(virtualRow.index);
                  if (!entry) return null;

                  // Windows detail-view geometry: rows stop at the size
                  // column's right edge instead of stretching across the
                  // window, so the area right of the columns stays blank
                  // background for clicks and marquee starts. The cap is
                  // shared with the header grid — see LIST_CONTENT_MAX_WIDTH.
                  return (
                    <div
                      key={entry.path}
                      className="absolute left-0 top-0 w-full"
                      style={{
                        maxWidth: LIST_CONTENT_MAX_WIDTH,
                        transform: `translateY(${virtualRow.start}px)`,
                      }}
                    >
                      <FileListRow
                        columns={listColumns}
                        densityRowHeight={rowHeight}
                        entry={entry}
                        gitStatus={gitStatus}
                        index={virtualRow.index}
                        isActionDisabled={actionsDisabled}
                        isCursor={virtualRow.index === (cursorIndex ?? 0)}
                        isDragging={draggingPaths.has(entry.path)}
                        isDropTarget={
                          internalDropTargetPath === entry.path ||
                          externalDropTargetPath === entry.path
                        }
                        isSelected={selectedPathSet.has(entry.path)}
                        menuActions={menuActions}
                        onAddEntryToFavorites={addEntryToFavorites}
                        onAddEntryToSpace={addEntryToSpace}
                        onContextMenuEntry={selectForContextMenu}
                        onOpenEntry={openEntry}
                        onPointerDownEntry={prepareInternalDrag}
                        onSelectEntry={selectEntryIfNotDragging}
                        selectedCount={selectedCount}
                        selectedPaths={selectedPaths}
                      />
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        )}
        {externalDropItemCount > 0 && (
          <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-lg border-2 border-dashed border-primary/50 bg-primary/5 text-body font-medium text-primary">
            {t("explorer:drag.dropToCopy", { count: externalDropItemCount })}
          </div>
        )}
        <MarqueeOverlay rect={listMarquee.rect} />
        {internalDrag && (
          <div
            aria-hidden="true"
            className="pointer-events-none fixed z-50 flex items-center gap-2 rounded-md border border-border bg-popover px-3 py-1.5 text-body text-popover-foreground shadow-ambient"
            style={{ left: internalDrag.position.x + 14, top: internalDrag.position.y + 14 }}
          >
            {internalDrag.target?.kind === "favorites" ? (
              <>
                <Star />
                {t("explorer:drag.addToFavorites", {
                  count: draggableDirectoryPaths(entries, internalDrag.sourcePaths).length,
                })}
              </>
            ) : internalDrag.target?.kind === "space" ? (
              <>
                <LayoutGrid />
                {t("explorer:drag.addToSpace", {
                  count: draggableDirectoryPaths(entries, internalDrag.sourcePaths).length,
                })}
              </>
            ) : (
              <>
                {internalDrag.operation === "copy" ? (
                  <Copy />
                ) : internalDrag.operation === "link" ? (
                  <Link />
                ) : (
                  <Scissors />
                )}
                {internalDrag.operation === "copy"
                  ? t("explorer:drag.opCopy")
                  : internalDrag.operation === "link"
                    ? t("explorer:drag.opLink")
                    : t("explorer:drag.opMove")}{" "}
                {t("explorer:drag.items", { count: internalDrag.sourcePaths.length })}
              </>
            )}
          </div>
        )}
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={onCreateFile}>
            <FilePlus />
            {t("explorer:contextMenu.newFile")}
          </ContextMenuItem>
          <ContextMenuItem onClick={onCreateDirectory}>
            <FolderPlus />
            {t("explorer:contextMenu.newFolder")}
            <ContextMenuShortcut>{formatBinding("Mod+Shift+N")}</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={onOpenTerminal}>
            <SquareTerminal />
            {t("explorer:contextMenu.openInTerminal")}
            <ContextMenuShortcut>
              {formatBinding(resolveBinding(shortcuts, "explorer.openSystemTerminal"))}
            </ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={blankMenuDisabled} onClick={onOpenWith}>
            <AppWindow />
            {t("explorer:contextMenu.openWithOtherApp")}
          </ContextMenuItem>
          <ContextMenuItem onClick={onPaste}>
            <Clipboard />
            {t("explorer:contextMenu.paste")}
            <ContextMenuShortcut>
              {formatBinding(resolveBinding(shortcuts, "explorer.paste"))}
            </ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
        {/* Selection and history: the three actions that only ever exist as
            keystrokes. Nothing in the window named them before, so they were
            discoverable only by trying keys at random. */}
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={onSelectAll}>
            <BoxSelect />
            {t("explorer:contextMenu.selectAll")}
            <ContextMenuShortcut>
              {formatBinding(resolveBinding(shortcuts, "explorer.selectAll"))}
            </ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={!canUndo} onClick={onUndo}>
            <Undo2 />
            {t("explorer:actions.undo")}
            <ContextMenuShortcut>
              {formatBinding(resolveBinding(shortcuts, "explorer.undo"))}
            </ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem disabled={!canRedo} onClick={onRedo}>
            <Redo2 />
            {t("explorer:actions.redo")}
            <ContextMenuShortcut>
              {formatBinding(resolveBinding(shortcuts, "explorer.redo"))}
            </ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function SortHeaderCell({
  active,
  align = "left",
  inset = "meta",
  label,
  onSort,
  order,
}: {
  active: boolean;
  align?: "left" | "right";
  /** Which row cell this heading labels — the two insets in `FileListRow`. */
  inset?: "meta" | "name";
  label: string;
  onSort: () => void;
  order: "asc" | "desc";
}) {
  const { t } = useTranslation("explorer");

  return (
    <div
      aria-sort={active ? (order === "asc" ? "ascending" : "descending") : "none"}
      className={cn("min-w-0", align === "right" && "text-right")}
      role="columnheader"
    >
      <button
        className={cn(
          // The horizontal inset is the row cell's, so a heading sits on the
          // same text edge as the values under it — `px-3` over the name
          // column (where the icon starts), `px-2.5` over the metadata ones.
          "flex min-w-0 items-center gap-1 rounded-sm text-left transition-colors duration-fast hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none",
          inset === "name" ? "px-3" : "px-2.5",
          active && "text-foreground",
          // `w-full` is load bearing, not cosmetic: a `<button>` shrink-wraps
          // its content even when it is a flex container, so a right-aligned
          // heading needs the box to reach the cell's right edge before
          // `flex-row-reverse` has anything to push the label against. Without
          // it the size heading sits at the column's left edge while every
          // value under it is flush right.
          align === "right" && "w-full flex-row-reverse",
        )}
        onClick={onSort}
        title={
          active
            ? t("explorer:sort.activeTitle", {
                column: label,
                direction:
                  order === "asc" ? t("explorer:sort.ascending") : t("explorer:sort.descending"),
              })
            : t("explorer:sort.inactiveTitle", { column: label })
        }
        type="button"
      >
        <span className="truncate">{label}</span>
        {active &&
          (order === "asc" ? (
            <ChevronUp className="size-3 shrink-0" />
          ) : (
            <ChevronDown className="size-3 shrink-0" />
          ))}
      </button>
    </div>
  );
}

/** Bulk action callbacks shared by every row's context menu. */
export interface MenuActions {
  onCompress: (format: ArchiveFormat, encrypted: boolean) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDeletePermanent: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpenWith: (path: string) => void;
  onRename: () => void;
  onTogglePreview: () => void;
}

/**
 * List row. The React Compiler memoizes this component's render output, so
 * scroll, selection, and drag churn only re-renders rows whose props
 * actually changed.
 */
function FileListRow({
  columns,
  densityRowHeight,
  entry,
  gitStatus,
  index,
  isActionDisabled,
  isCursor,
  isDragging,
  isDropTarget,
  isSelected,
  menuActions,
  onAddEntryToFavorites,
  onAddEntryToSpace,
  onContextMenuEntry,
  onOpenEntry,
  onPointerDownEntry,
  onSelectEntry,
  selectedCount,
  selectedPaths,
}: {
  /** Detail columns this pane has room for; the row renders exactly these and
   *  the grid template is derived from the same object. */
  columns: ListColumns;
  densityRowHeight: number;
  entry: DirectoryEntry;
  gitStatus?: ExplorerGitStatus | null;
  index: number;
  isActionDisabled: boolean;
  /** The roving tab stop: exactly one row answers Tab, and the keyboard's
   *  arrows move it together with the selection. */
  isCursor: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  menuActions: MenuActions;
  onAddEntryToFavorites: (entry: DirectoryEntry) => void;
  onAddEntryToSpace: (entry: DirectoryEntry, spaceId: string) => void;
  onContextMenuEntry: (entry: DirectoryEntry, index: number) => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onPointerDownEntry: (entry: DirectoryEntry, event: ReactPointerEvent) => void;
  onSelectEntry: (entry: DirectoryEntry, index: number, event: ReactMouseEvent) => void;
  selectedCount: number;
  selectedPaths: string[];
}) {
  const { t } = useTranslation("explorer");
  const presentation = getEntryPresentation(entry);
  const isDirectory = entry.kind === "directory";
  const entryStatus = getEntryGitStatus(gitStatus, entry);
  const displaySize = isDirectory ? null : entry.size;
  const handleSelect = (event: ReactMouseEvent) => onSelectEntry(entry, index, event);

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <div
          aria-selected={isSelected}
          className={cn(
            // Desktop row: 13px text, tonal hover, filled selection. Selection
            // never changes the text weight — a re-measuring label makes a
            // multi-select scan jumpy, and the fill already carries the state.
            // Focus is the rows' inset hairline (an outer ring would be clipped
            // by the virtual scroller); the drop target is the only state that
            // adds an accent ring.
            "render-contain state-layer grid cursor-grab items-center justify-start rounded-sm whitespace-nowrap transition-[background-color,box-shadow,opacity] duration-fast ease-standard select-none focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset",
            entry.hidden && HIDDEN_ENTRY_CLASS,
            isSelected && "bg-selection",
            isDragging && "cursor-grabbing opacity-50",
            isDropTarget && "bg-primary/10 ring-2 ring-primary ring-inset",
          )}
          data-entry-path={entry.path}
          data-explorer-directory-drop-target={isDirectory ? entry.path : undefined}
          onClick={handleSelect}
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
          tabIndex={isCursor ? 0 : -1}
          title={entry.path}
          style={{ gridTemplateColumns: listGridTemplate(columns), height: densityRowHeight }}
        >
          <div className="flex min-w-0 items-center gap-2.5 px-3">
            <EntryIconFrame entry={entry}>
              {isNativeIconSupported(entry) ? (
                <NativeIconImage
                  className="shrink-0"
                  entry={entry}
                  fallback={
                    <TypeIconTile
                      className="size-tile-list tile-radius"
                      iconSize={16}
                      presentation={presentation}
                    />
                  }
                  pixelSize={18}
                />
              ) : (
                <TypeIconTile
                  className="size-tile-list tile-radius"
                  iconSize={16}
                  presentation={presentation}
                />
              )}
            </EntryIconFrame>
            <span className="min-w-0 truncate text-body">{entry.name}</span>
            {entryStatus && <GitStatusBadge kind={entryStatus} />}
            {entry.relativePath && (
              <span
                className="ml-auto max-w-row-meta shrink-0 truncate text-caption text-muted-foreground"
                title={entry.relativePath}
              >
                {formatRelativeLocation(entry.relativePath)}
              </span>
            )}
          </div>
          {columns.modified && (
            <div className="px-2.5 text-caption text-muted-foreground tabular-nums">
              {formatModifiedAt(entry.modifiedAt)}
            </div>
          )}
          {columns.type && (
            <div className="px-2.5 text-caption text-muted-foreground">{presentation.label}</div>
          )}
          {columns.size && (
            <div
              className="px-2.5 text-right text-caption text-muted-foreground tabular-nums"
              title={
                displaySize === null
                  ? undefined
                  : t("explorer:list.bytesTitle", { size: localeNumber(displaySize) })
              }
            >
              {formatFileSize(displaySize)}
            </div>
          )}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <EntryContextMenuContent
          entry={entry}
          isActionDisabled={isActionDisabled}
          isSingleSelection={selectedCount === 1}
          onAddToFavorites={() => onAddEntryToFavorites(entry)}
          onAddToSpace={(spaceId) => onAddEntryToSpace(entry, spaceId)}
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
          selectedPaths={selectedPaths}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}

async function openFile(path: string): Promise<void> {
  recordRecentItem(path, "file", "opened");

  try {
    await openPath(path);
  } catch (error) {
    console.warn(`Unable to open ${path}`, error);
  }
}

function formatModifiedAt(modifiedAt: number | null): string {
  return modifiedAt === null
    ? "—"
    : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(modifiedAt);
}

function formatFileSize(size: number | null): string {
  if (size === null) {
    return "";
  }

  if (size === 0) {
    return "0 B";
  }

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  const value = size / 1024 ** unitIndex;

  return `${localeNumberFormat(FILE_SIZE_FORMAT_OPTIONS).format(value)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

function formatRelativeLocation(relativePath: string): string {
  const separatorIndex = Math.max(relativePath.lastIndexOf("/"), relativePath.lastIndexOf("\\"));
  if (separatorIndex < 0) {
    return i18n.t("explorer:list.currentFolder");
  }

  return relativePath.slice(0, separatorIndex).replaceAll(/[\\/]/g, " › ");
}

export function FileListSkeleton() {
  const { t } = useTranslation("explorer");

  return (
    <section
      aria-label={t("explorer:list.loadingAriaLabel")}
      className="flex min-h-0 flex-1 flex-col"
    >
      <div className="flex h-7 shrink-0 items-center justify-between border-b px-3">
        <Skeleton className="h-3.5 w-12" />
        <Skeleton className="h-3 w-16" />
      </div>
      <Table className="min-w-160 table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead>{t("explorer:columns.name")}</TableHead>
            <TableHead className="w-44">{t("explorer:columns.modified")}</TableHead>
            <TableHead className="w-28">{t("explorer:columns.type")}</TableHead>
            <TableHead className="w-24 text-right">{t("explorer:columns.size")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {Array.from({ length: 8 }, (_, index) => (
            <TableRow key={index}>
              <TableCell>
                <div className="flex h-8 items-center gap-2 px-2">
                  <Skeleton className="size-4" />
                  <Skeleton className={index % 3 === 0 ? "h-4 w-48" : "h-4 w-32"} />
                </div>
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-30" />
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-14" />
              </TableCell>
              <TableCell>
                <Skeleton className="ml-auto h-4 w-12" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}
