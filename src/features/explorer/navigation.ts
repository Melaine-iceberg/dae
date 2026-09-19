import { commands } from "@/bindings";
import { recordRecentItem } from "@/features/workspace/recents-atoms";

import { openDirectoryListing, type DirectoryListing } from "./directory-listing";
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

  async refresh(path = this.state.directory?.path): Promise<DirectoryView | undefined> {
    if (!path || this.state.directory?.path !== path || this.state.status === "loading") {
      return undefined;
    }

    const requestVersion = ++this.requestVersion;

    try {
      const directory = await this.readListing(path, requestVersion);

      if (requestVersion !== this.requestVersion || this.state.directory?.path !== path) {
        return undefined;
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
      const directory = await this.readListing(path, requestVersion);

      if (requestVersion !== this.requestVersion) {
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
   * Any listing that is still streaming is dropped first: a newer read (a
   * navigation, a watcher refresh) always wins over the one it replaces.
   */
  private readListing(path: string, requestVersion: number): Promise<DirectoryView> {
    this.listing?.dispose();
    this.listing = null;

    // The head is kept here rather than read back from the state: a batch can
    // beat `load`/`refresh` to the state update, and it still has to render
    // against its own head.
    let head: DirectoryView | null = null;
    let entries: DirectoryEntry[] | null = null;

    const listing = openDirectoryListing(
      path,
      {
        onHead: (view) => {
          head = view;
        },
        onEntries: (latest) => {
          entries = latest;
          if (requestVersion !== this.requestVersion || !head) return;
          this.setState({ ...this.state, directory: { ...head, entries: latest } });
        },
      },
      this.api,
    );

    this.listing = listing;
    // Whatever was published while the head was in flight rides along, so the
    // caller's state update cannot drop it.
    return listing.head.then((view) => ({ ...view, entries: entries ?? view.entries }));
  }

  /** Stops the active listing; the navigator is not used afterwards. */
  dispose(): void {
    ++this.requestVersion;
    this.listing?.dispose();
    this.listing = null;
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
