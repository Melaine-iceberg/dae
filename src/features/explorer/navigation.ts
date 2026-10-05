import { commands } from "@/bindings";
import { recordRecentItem } from "@/features/workspace/recents-atoms";

import {
  invalidateCachedListing,
  isSameListing,
  openPacketDirectoryListing,
  type DirectoryListing,
} from "./directory-listing";
import {
  indicesOfNames,
  listingRowMatches,
  listingViewOf,
  patchedListingView,
  type ListingPatchChange,
  type ListingView,
} from "./listing-view";
import type { Breadcrumb, DirectoryEntry, DirectoryView, FileSystemError } from "./types";

export type ExplorerStatus = "idle" | "loading" | "ready" | "error";

export interface ExplorerState {
  status: ExplorerStatus;
  /**
   * The directory's metadata and its first batch. `entries` here is the head
   * only once a listing streams — `listing` is what holds the whole thing, and
   * it is what every consumer of the entries reads.
   */
  directory: DirectoryView | null;
  /**
   * The readable listing behind `directory`.
   *
   * Not part of `ExplorerNavigatorSnapshot`, and deliberately so: it is either
   * an array of objects or a set of packed buffers, and a tab handoff goes
   * through `JSON.stringify`. A restored pane therefore starts without one and
   * re-reads (see `restoreSnapshot`), which is also what the snapshot did
   * before for a listing that had only partly streamed.
   */
  listing: ListingView | null;
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

/** One completed read: the backend's view, and the listing that reads it. */
interface ReadListing {
  directory: DirectoryView;
  listing: ListingView;
}

export type ExplorerListener = () => void;

type NavigationMode = { type: "push" } | { type: "replace" } | { type: "history"; index: number };

const initialState: ExplorerState = {
  status: "idle",
  directory: null,
  listing: null,
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
  /**
   * This view's identity in the backend's watcher registry. One id per
   * navigator, for its whole life: navigation re-targets the watch rather than
   * minting a new one, so a stale read can never arm a directory this view is
   * no longer showing (see `arm_local_watcher`), and every read of the
   * displayed directory can safely (re-)arm it.
   */
  private readonly watcherId = crypto.randomUUID();
  /** Listing whose remaining batches are still streaming into the state. */
  private listing: DirectoryListing | null = null;
  /**
   * Whether the listing on screen has every batch.
   *
   * Only a complete one can be patched: a patch appends rows to the end of an
   * index space that is still growing, and a batch that lands afterwards would
   * describe a directory as of *before* the patch.
   */
  private listingComplete = false;
  /**
   * The tail of the patch chain; see `patch`. Resolved rather than `null` so the
   * first patch needs no special case.
   */
  private pendingPatch: Promise<void> = Promise.resolve();
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

  /**
   * Captures the complete navigation model without its non-serializable
   * listeners — and without its listing, which is either a set of packet
   * buffers or a row cache. The destination re-reads instead (see
   * `restoreSnapshot`). Carrying a listing across was never lossless anyway: a
   * snapshot taken mid-stream only ever held the batches that had arrived.
   */
  createSnapshot(): ExplorerNavigatorSnapshot {
    return {
      state: { ...this.state, listing: null },
      scrollOffsets: [...this.scrollOffsets],
    };
  }

  /** Restores a snapshot before the destination window's first React render. */
  restoreSnapshot(snapshot: ExplorerNavigatorSnapshot): void {
    ++this.requestVersion;
    this.scrollOffsets.clear();
    snapshot.scrollOffsets.forEach(([path, offset]) => this.scrollOffsets.set(path, offset));

    const pendingPath = snapshot.state.status === "loading" ? snapshot.state.pendingPath : null;
    // A snapshot never carries a listing (see `createSnapshot`), so a directory
    // that arrives with one is shown as its head and re-read to completion
    // below instead of being trusted as complete.
    const carried = snapshot.state.directory;
    const staleListing = carried !== null && snapshot.state.status === "ready";

    const restoredState: ExplorerState =
      snapshot.state.status === "loading"
        ? carried
          ? {
              ...snapshot.state,
              listing: null,
              status: "ready",
              pendingPath: null,
              error: null,
            }
          : {
              ...initialState,
              history: snapshot.state.history,
              historyIndex: snapshot.state.historyIndex,
            }
        : { ...snapshot.state, listing: null };

    this.setState(restoredState);

    // A navigation request in the source webview cannot continue after the
    // handoff. Resume it here instead of leaving the destination permanently
    // in a loading state.
    if (pendingPath) {
      void this.navigate(pendingPath);
    } else if (staleListing) {
      // The pane paints the head it was handed and fills in behind it.
      void this.refresh(carried.path);
    }
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
      const read = await this.readListing(path, requestVersion, { settle: true });

      if (
        read === null ||
        requestVersion !== this.requestVersion ||
        this.state.directory?.path !== path
      ) {
        return undefined;
      }

      const displayed = this.state.directory;

      // A restored pane has no listing of its own (see `ExplorerState`), so the
      // head it carries stands in for one. That comparison is the conservative
      // direction: a head against a complete listing reads as changed.
      const previous = this.state.listing ?? listingViewOf(displayed.entries);

      if (isSameListing(previous, read.listing)) {
        return displayed;
      }

      this.setState({
        ...this.state,
        status: "ready",
        directory: read.directory,
        listing: read.listing,
        pendingPath: null,
        error: null,
      });
      // The directory is known to differ from the listing another pane may still
      // be painting from the cache.
      invalidateCachedListing(path);

      return read.directory;
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

  /**
   * Repairs the displayed listing from the children the watcher named, instead of
   * reading the directory again.
   *
   * A change to one file in a 35,803 entry folder is not a reason to enumerate
   * the folder: this asks the backend to stat the names that changed and rewrites
   * those rows in the listing already on screen. The re-read it replaces measured
   * ~12.3 ms of enumeration, 5.76 MB of packed bytes across IPC, a full
   * `isSameListing` scan, and a sort the worker starts over because a fresh head
   * array is not the one it ordered; the patch is one `stat` per name, a scan of
   * the listing's names to locate those rows, and — for a change that only adds
   * rows — a sort that folds the new rows into the order it holds.
   *
   * `refresh` is the answer whenever the patch cannot be trusted: the change is
   * not attributable to a row, the listing is still streaming, the stats failed,
   * the listing on screen is not one this class could patch, or the pane moved
   * while the stats were in flight. It is also what a patch *falls back to*, so a
   * caller can hand every notification here and let the guards decide.
   *
   * Patches run one at a time. A burst can name the same child twice across two
   * windows, and the second has to be computed against the listing the first
   * published — otherwise it addresses rows by the index the first patch
   * replaced, and its only safe outcome is to be thrown away.
   */
  patch(path: string, names: readonly string[]): Promise<DirectoryView | undefined> {
    const run = this.pendingPatch.then(() => this.applyPatch(path, names));
    // The chain keeps going whatever this one did.
    this.pendingPatch = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async applyPatch(
    path: string,
    names: readonly string[],
  ): Promise<DirectoryView | undefined> {
    const listing = this.state.listing;

    if (
      listing === null ||
      !this.listingComplete ||
      this.state.status !== "ready" ||
      this.state.directory?.path !== path ||
      names.length === 0
    ) {
      return this.refresh(path);
    }

    // Not bumped: a patch does not invalidate a read, it only refuses to race
    // one. Whatever version this captures is the one whose listing is on screen.
    const requestVersion = this.requestVersion;

    let resolved: (DirectoryEntry | null)[];
    try {
      resolved = await this.api.readDirectoryChanges(path, [...names]);
    } catch {
      // A stat that failed says nothing about which rows changed, and the
      // re-read reports the error itself if the directory stays unreadable.
      return this.refresh(path);
    }

    // The listing this patch was computed against is no longer the one on screen.
    if (requestVersion !== this.requestVersion || this.state.listing !== listing) {
      return undefined;
    }

    // One resolution per name: a burst reports the same child repeatedly, and a
    // row cannot be both rewritten and removed by the same patch.
    const wanted = new Map<string, DirectoryEntry | null>();
    for (let ordinal = 0; ordinal < names.length; ordinal += 1) {
      wanted.set(names[ordinal], resolved[ordinal]);
    }

    const at = indicesOfNames(listing, new Set(wanted.keys()));
    const changes: ListingPatchChange[] = [];
    for (const [name, entry] of wanted) {
      const index = at.get(name) ?? -1;
      // A name that is not on disk and was never on screen needs no row, and a
      // row that already says what the disk says needs no rewrite. Both are the
      // watcher reporting something a listing does not show.
      if (entry === null && index < 0) continue;
      if (entry !== null && index >= 0 && listingRowMatches(listing, index, entry)) continue;
      changes.push({ entry, index });
    }

    if (changes.length === 0) return this.state.directory ?? undefined;

    const patched = patchedListingView(listing, changes);
    if (patched === null) return this.refresh(path);
    if (patched === listing) return this.state.directory ?? undefined;

    // `directory` keeps its identity: only its listing changed, and a row that
    // appeared or disappeared is not a reason to re-key the pane's filters.
    this.setState({ ...this.state, listing: patched });
    invalidateCachedListing(path);

    return this.state.directory ?? undefined;
  }

  private async load(path: string, mode: NavigationMode): Promise<DirectoryView | undefined> {
    const requestVersion = ++this.requestVersion;
    this.setState({ ...this.state, status: "loading", pendingPath: path, error: null });

    try {
      const read = await this.readListing(path, requestVersion, { settle: false });

      if (read === null || requestVersion !== this.requestVersion) {
        return undefined;
      }

      const history = this.updateHistory(read.directory.path, mode);
      this.setState({
        status: "ready",
        directory: read.directory,
        listing: read.listing,
        pendingPath: null,
        error: null,
        ...history,
      });

      // Every successful navigation counts as a directory visit, so the
      // workspace Recents surface reflects where the user actually went.
      recordRecentItem(read.directory.path, "directory", "visited");

      return read.directory;
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
   * Every read arms this view's watcher for the directory being read (see
   * `watcherId`). Arming it before the first entry is read is what makes the
   * listing and the events cover every change between them; re-arming on a
   * refresh is free when the view already watches the same directory — the
   * backend skips that — and is what restores the watch after the pane
   * unmounted and released it (see `releaseWatcher`).
   *
   * Any listing that is still streaming is dropped first: a newer read (a
   * navigation, a watcher refresh) always wins over the one it replaces.
   */
  private readListing(
    path: string,
    requestVersion: number,
    options: { settle: boolean },
  ): Promise<ReadListing | null> {
    this.cancelListing();
    this.listingComplete = false;

    // The head is kept here rather than read back from the state: a batch can
    // beat `load`/`refresh` to the state update, and it still has to render
    // against its own head.
    let head: DirectoryView | null = null;
    let listing: ListingView | null = null;
    let resolveSettled: (() => void) | null = null;
    const settled = options.settle
      ? new Promise<void>((resolve) => {
          resolveSettled = resolve;
        })
      : null;

    // One opener for both transports: the batches that follow the head arrive
    // as columnar packets, the head itself as the JSON view the rest of the app
    // already reads.
    const stream = openPacketDirectoryListing(
      path,
      {
        onHead: (view) => {
          head = view;
        },
        onListing: (latest) => {
          listing = latest;
          if (options.settle || requestVersion !== this.requestVersion || !head) return;
          this.setState({ ...this.state, directory: head, listing: latest });
        },
        onDone: () => {
          // A read that a newer one superseded was disposed before it could get
          // here, so this is normally the read that owns the state — the version
          // check is for a navigation that started while the last batch was in
          // flight, whose head-only listing is not complete.
          if (requestVersion === this.requestVersion) this.listingComplete = true;
          resolveSettled?.();
        },
      },
      this.api,
      this.watcherId,
    );

    this.listing = stream;
    // A settling read that is superseded has to stop waiting; the caller
    // discards its result through the request version either way.
    this.cancelSettle = () => resolveSettled?.();

    return stream.head.then(async (view) => {
      if (settled === null) {
        // Whatever was published while the head was in flight rides along, so
        // the caller's state update cannot drop it.
        return { directory: view, listing: listing ?? listingViewOf(view.entries) };
      }

      await settled;
      if (requestVersion !== this.requestVersion) {
        return null;
      }
      return listing === null ? null : { directory: view, listing };
    });
  }

  /** Drops the active listing and releases a settling read waiting on it. */
  private cancelListing(): void {
    this.listing?.dispose();
    this.listing = null;
    this.cancelSettle?.();
    this.cancelSettle = null;
  }

  /**
   * Releases this view's directory watch on the backend. Called when the pane
   * displaying the view goes away (a tab switched away from, a split pane
   * closed); a view that comes back re-arms on its next read. Safe to call
   * more than once, and while a read is still in flight — a late arm for a
   * released id simply installs a watcher that the next release stops.
   */
  releaseWatcher(): void {
    void this.api.unwatchDirectory(this.watcherId).catch(() => {});
  }

  /** Stops the active listing; the navigator is not used afterwards. */
  dispose(): void {
    ++this.requestVersion;
    this.cancelListing();
    this.releaseWatcher();
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
        listing: null,
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
