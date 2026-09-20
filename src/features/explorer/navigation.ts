import { commands } from "@/bindings";
import { recordRecentItem } from "@/features/workspace/recents-atoms";

import { isSameListing, openDirectoryListing, type DirectoryListing } from "./directory-listing";
import type { Breadcrumb, DirectoryEntry, DirectoryView, FileSystemError } from "./types";

export type ExplorerStatus = "idle" | "loading" | "ready" | "error";

export interface ExplorerState {
  status: ExplorerStatus;
  directory: DirectoryView | null;
  pendingPath: string | null;
  error: FileSystemError | null;
  history: string[];
  historyIndex: number;
}

/** Serializable state transferred when a tab moves to another webview. */
export interface ExplorerNavigatorSnapshot {
  state: ExplorerState;
  scrollOffsets: [string, number][];
}

export type ExplorerListener = () => void;

type NavigationMode = { type: "push" } | { type: "replace" } | { type: "history"; index: number };

const initialState: ExplorerState = {
  status: "idle",
  directory: null,
  pendingPath: null,
  error: null,
  history: [],
  historyIndex: -1,
};

const fileSystemErrorKinds = new Set<FileSystemError["kind"]>([
  "not_found",
  "permission_denied",
  "not_directory",
  "io",
  "internal",
]);

export class ExplorerNavigator {
  private state = initialState;
  private requestVersion = 0;
  /** Listing whose remaining batches are still streaming into the state. */
  private listing: DirectoryListing | null = null;
  /** Releases a settling read that is still waiting on its listing. */
  private cancelSettle: (() => void) | null = null;
  private readonly scrollOffsets = new Map<string, number>();
  private readonly listeners = new Set<ExplorerListener>();

  constructor(private readonly api = commands) {}

  getSnapshot = (): ExplorerState => this.state;

  getScrollOffset(path: string): number {
    return this.scrollOffsets.get(path) ?? 0;
  }

  setScrollOffset(path: string, offset: number): void {
    this.scrollOffsets.set(path, offset);
  }

  /** Captures the complete navigation model without its non-serializable listeners. */
  createSnapshot(): ExplorerNavigatorSnapshot {
    return {
      state: this.state,
      scrollOffsets: [...this.scrollOffsets],
    };
  }

  /** Restores a snapshot before the destination window's first React render. */
  restoreSnapshot(snapshot: ExplorerNavigatorSnapshot): void {
    ++this.requestVersion;
    this.scrollOffsets.clear();
    snapshot.scrollOffsets.forEach(([path, offset]) => this.scrollOffsets.set(path, offset));

    const pendingPath = snapshot.state.status === "loading" ? snapshot.state.pendingPath : null;
    const restoredState: ExplorerState =
      snapshot.state.status === "loading"
        ? snapshot.state.directory
          ? {
              ...snapshot.state,
              status: "ready",
              pendingPath: null,
              error: null,
            }
          : {
              ...initialState,
              history: snapshot.state.history,
              historyIndex: snapshot.state.historyIndex,
            }
        : snapshot.state;

    this.setState(restoredState);

    // A navigation request in the source webview cannot continue after the
    // handoff. Resume it here instead of leaving the destination permanently
    // in a loading state.
    if (pendingPath) void this.navigate(pendingPath);
  }

  subscribe = (listener: ExplorerListener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async initialize(): Promise<DirectoryView | undefined> {
    const requestVersion = ++this.requestVersion;
    this.setState({ ...this.state, status: "loading", pendingPath: null, error: null });

    try {
      const homeDirectory = await this.api.getHomeDirectory();

      if (requestVersion !== this.requestVersion) {
        return undefined;
      }

      return this.load(homeDirectory, { type: "replace" });
    } catch (error) {
      if (requestVersion === this.requestVersion) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  navigate(path: string): Promise<DirectoryView | undefined> {
    return this.load(path, { type: "push" });
  }

  /**
   * Re-reads the displayed directory without disturbing it.
   *
   * Refreshes are watcher-driven and mostly consecutive, so the cheap outcome
   * is the common one: a settling read whose entries match what is on screen
   * leaves the state object untouched. That matters more than it sounds — the
   * sort, the filters, the selection reconciliation and the row count are all
   * keyed on `directory`, so replacing it costs a re-sort of the whole listing
   * and a re-render of it, for a list that did not change.
   */
  async refresh(path = this.state.directory?.path): Promise<DirectoryView | undefined> {
    if (!path || this.state.directory?.path !== path || this.state.status === "loading") {
      return undefined;
    }

    const requestVersion = ++this.requestVersion;

    try {
      const directory = await this.readListing(path, requestVersion, {
        settle: true,
        watch: false,
      });

      if (
        directory === null ||
        requestVersion !== this.requestVersion ||
        this.state.directory?.path !== path
      ) {
        return undefined;
      }

      const displayed = this.state.directory;

      if (isSameListing(displayed.entries, directory.entries)) {
        return displayed;
      }

      this.setState({
        ...this.state,
        status: "ready",
        directory,
        pendingPath: null,
        error: null,
      });

      return directory;
    } catch (error) {
      if (requestVersion === this.requestVersion && this.state.directory?.path === path) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  private async load(path: string, mode: NavigationMode): Promise<DirectoryView | undefined> {
    const requestVersion = ++this.requestVersion;
    this.setState({ ...this.state, status: "loading", pendingPath: path, error: null });

    try {
      const directory = await this.readListing(path, requestVersion, {
        settle: false,
        watch: true,
      });

      if (directory === null || requestVersion !== this.requestVersion) {
        return undefined;
      }

      const history = this.updateHistory(directory.path, mode);
      this.setState({
        status: "ready",
        directory,
        pendingPath: null,
        error: null,
        ...history,
      });

      // Every successful navigation counts as a directory visit, so the
      // workspace Recents surface reflects where the user actually went.
      recordRecentItem(directory.path, "directory", "visited");

      return directory;
    } catch (error) {
      if (requestVersion === this.requestVersion) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  /**
   * Reads `path` and folds the batches that follow the first one into the
   * displayed directory, so a large folder paints as soon as its first batch
   * is in instead of waiting for the whole walk.
   *
   * `settle` inverts that, for a refresh. The explorer already shows a complete
   * listing there, and publishing the batches would first shrink it back to the
   * head — 512 entries — before growing it again, which clamps the scroll
   * offset to a list that is suddenly 70× shorter. A settling read keeps the
   * current listing on screen and resolves once the walk is finished.
   *
   * `options.watch` arms the backend's directory watcher as part of the read.
   * Only the read that establishes a view asks for it: the watcher has to be
   * armed before the directory is read for the listing and the events to cover
   * every change between them, and a refresh happens *because* an event
   * arrived, so the watcher for the directory on screen is already in place.
   *
   * Any listing that is still streaming is dropped first: a newer read (a
   * navigation, a watcher refresh) always wins over the one it replaces.
   */
  private readListing(
    path: string,
    requestVersion: number,
    options: { settle: boolean; watch: boolean },
  ): Promise<DirectoryView | null> {
    this.cancelListing();

    // The head is kept here rather than read back from the state: a batch can
    // beat `load`/`refresh` to the state update, and it still has to render
    // against its own head.
    let head: DirectoryView | null = null;
    let entries: DirectoryEntry[] | null = null;
    let resolveSettled: (() => void) | null = null;
    const settled = options.settle
      ? new Promise<void>((resolve) => {
          resolveSettled = resolve;
        })
      : null;

    const listing = openDirectoryListing(
      path,
      {
        onHead: (view) => {
          head = view;
        },
        onEntries: (latest) => {
          entries = latest;
          if (options.settle || requestVersion !== this.requestVersion || !head) return;
          this.setState({ ...this.state, directory: { ...head, entries: latest } });
        },
        onDone: () => resolveSettled?.(),
      },
      this.api,
      options.watch,
    );

    this.listing = listing;
    // A settling read that is superseded has to stop waiting; the caller
    // discards its result through the request version either way.
    this.cancelSettle = () => resolveSettled?.();

    return listing.head.then(async (view) => {
      if (settled === null) {
        // Whatever was published while the head was in flight rides along, so
        // the caller's state update cannot drop it.
        return { ...view, entries: entries ?? view.entries };
      }

      await settled;
      if (requestVersion !== this.requestVersion) {
        return null;
      }
      return entries === null ? null : { ...view, entries };
    });
  }

  /** Drops the active listing and releases a settling read waiting on it. */
  private cancelListing(): void {
    this.listing?.dispose();
    this.listing = null;
    this.cancelSettle?.();
    this.cancelSettle = null;
  }

  /** Stops the active listing; the navigator is not used afterwards. */
  dispose(): void {
    ++this.requestVersion;
    this.cancelListing();
    this.listeners.clear();
  }

  navigateBreadcrumb(breadcrumb: Breadcrumb): Promise<DirectoryView | undefined> {
    return this.navigate(breadcrumb.path);
  }

  goUp(): Promise<DirectoryView | undefined> {
    const parent = this.state.directory?.breadcrumbs.at(-2);
    return parent ? this.navigateBreadcrumb(parent) : Promise.resolve(undefined);
  }

  goBack(): Promise<DirectoryView | undefined> {
    const previousIndex = this.state.historyIndex - 1;
    return previousIndex >= 0
      ? this.load(this.state.history[previousIndex], { type: "history", index: previousIndex })
      : Promise.resolve(undefined);
  }

  goForward(): Promise<DirectoryView | undefined> {
    const nextIndex = this.state.historyIndex + 1;
    return nextIndex < this.state.history.length
      ? this.load(this.state.history[nextIndex], { type: "history", index: nextIndex })
      : Promise.resolve(undefined);
  }

  private updateHistory(
    path: string,
    mode: NavigationMode,
  ): Pick<ExplorerState, "history" | "historyIndex"> {
    if (mode.type === "replace") {
      return { history: [path], historyIndex: 0 };
    }

    if (mode.type === "history") {
      const history = [...this.state.history];
      history[mode.index] = path;
      return { history, historyIndex: mode.index };
    }

    const history = this.state.history.slice(0, this.state.historyIndex + 1);

    if (history.at(-1) === path) {
      return { history, historyIndex: history.length - 1 };
    }

    history.push(path);
    return { history, historyIndex: history.length - 1 };
  }

  private setState(state: ExplorerState): void {
    this.state = state;
    this.listeners.forEach((listener) => listener());
  }
}

/** Snapshot of a pane that has never navigated. */
function idleSnapshot(): ExplorerNavigatorSnapshot {
  return { state: initialState, scrollOffsets: [] };
}

/**
 * Snapshots for a tab that must land on `path` the first time it renders.
 *
 * The primary pane is seeded as a *pending* navigation rather than as a
 * resolved listing: a window opened straight onto a folder has no directory
 * view of its own to ship, and `restoreSnapshot` resumes the pending read on
 * the destination side instead of carrying one across. The split pane stays
 * idle — a handoff never opens dual-pane.
 */
export function createFolderNavigationSnapshot(path: string): {
  primary: ExplorerNavigatorSnapshot;
  split: ExplorerNavigatorSnapshot;
} {
  return {
    primary: {
      state: {
        status: "loading",
        directory: null,
        pendingPath: path,
        error: null,
        history: [path],
        historyIndex: 0,
      },
      scrollOffsets: [],
    },
    split: idleSnapshot(),
  };
}

function toFileSystemError(error: unknown): FileSystemError {
  if (isFileSystemError(error)) {
    return error;
  }

  return {
    kind: "internal",
    message: error instanceof Error ? error.message : String(error),
  };
}

function isFileSystemError(value: unknown): value is FileSystemError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    "message" in value &&
    typeof value.kind === "string" &&
    fileSystemErrorKinds.has(value.kind as FileSystemError["kind"]) &&
    typeof value.message === "string"
  );
}
