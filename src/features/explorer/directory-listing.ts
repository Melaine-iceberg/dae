import { useEffect, useRef, useState } from "react";

import { commands, events, type DirectoryEntry, type DirectoryView } from "@/bindings";

/**
 * Directory listings arrive in batches: `readDirectory` answers with the first
 * one, and the backend pushes the rest as `explorer-directory-entries` events
 * under the id the caller passed in.
 *
 * The id has to be minted here rather than by the backend because the batches
 * are fire-and-forget events. The backend starts pushing them as soon as the
 * first batch is on its way, so a listener attached *after* the command
 * resolves would miss the early ones. Callers therefore subscribe first and
 * only then invoke the command.
 */

type DirectoryListingApi = Pick<typeof commands, "readDirectory" | "cancelDirectoryListing">;

export interface DirectoryListingListener {
  /** Called once, when the first batch lands. */
  onHead: (view: DirectoryView) => void;
  /**
   * Called with the complete entry list so far, coalesced to one call per
   * animation frame while the listing streams — the backend can push several
   * batches per frame, and publishing each one would cost a full render and a
   * fresh array copy apiece. A trailing frame owed when the stream finishes is
   * flushed before `onDone`, so the listener never ends a frame behind.
   */
  onEntries: (entries: DirectoryEntry[]) => void;
  /** Called once, when the listing is complete (not on disposal). */
  onDone?: () => void;
  /** Called when the read failed (batches never fail individually). */
  onError?: (error: unknown) => void;
}

export interface DirectoryListing {
  /** Resolves with the first batch, or rejects with the backend error. */
  head: Promise<DirectoryView>;
  /** Stops the backend read and detaches the listener. Safe to call twice. */
  dispose: () => void;
}

/** Opens one directory listing, streaming its remaining batches into `listener`. */
export function openDirectoryListing(
  path: string,
  listener: DirectoryListingListener,
  api: DirectoryListingApi = commands,
): DirectoryListing {
  const streamId = crypto.randomUUID();
  let stopped = false;
  let complete = false;
  let detached = false;
  let head: DirectoryView | null = null;
  let streamed: DirectoryEntry[] = [];
  /** How much of `streamed` the last publish already included. */
  let publishedCount = 0;
  let flushHandle: number | null = null;

  // Publishes the accumulated batches, at most once per frame. The head is
  // published synchronously by the request handler below; this only carries
  // the batches that follow it.
  const flush = () => {
    flushHandle = null;
    if (stopped || !head || streamed.length === publishedCount) return;
    publishedCount = streamed.length;
    listener.onEntries([...head.entries, ...streamed]);
  };

  const scheduleFlush = () => {
    if (flushHandle !== null) return;
    flushHandle = requestAnimationFrame(flush);
  };

  // Batches that beat the head here are accumulated and published together
  // with it, so the list is always head-then-batches regardless of arrival
  // order.
  const unlisten = events.explorerDirectoryEntries.listen(({ payload }) => {
    if (stopped || payload.streamId !== streamId) return;

    if (payload.entries.length > 0) {
      streamed.push(...payload.entries);
      scheduleFlush();
    }

    if (payload.done) finish();
  });

  const detach = () => {
    if (detached) return;
    detached = true;
    if (flushHandle !== null) {
      cancelAnimationFrame(flushHandle);
      flushHandle = null;
    }
    void unlisten.then((stop) => stop());
  };

  const finish = () => {
    if (complete || stopped) return;
    complete = true;
    // Publish the tail synchronously so `onDone` never reports completion for
    // a listing the listener has not fully seen.
    flush();
    // A finished listing streams nothing more; the listener would only leak.
    detach();
    listener.onDone?.();
  };

  const headRequest = api
    .readDirectory(path, streamId)
    .then((view) => {
      if (stopped) return view;

      head = view;
      listener.onHead(view);
      publishedCount = streamed.length;
      listener.onEntries([...view.entries, ...streamed]);

      // A view without a stream id fits in one batch: no event is coming.
      if (view.streamId === null) finish();

      return view;
    })
    .catch((error: unknown) => {
      if (!stopped) {
        detach();
        listener.onError?.(error);
      }
      throw error;
    });

  return {
    head: headRequest,
    dispose: () => {
      if (stopped) return;
      stopped = true;
      detach();
      // Ids the backend already finished with are ignored there.
      void api.cancelDirectoryListing(streamId).catch(() => {});
    },
  };
}

/**
 * Completed listings by path, so re-opening a folder (drilling back out of a
 * Miller column, expanding a section again) paints immediately instead of
 * flashing an empty pane while the read runs.
 */
const completedListings = new Map<string, DirectoryEntry[]>();

/** Cap on cached listings; tabs rarely revisit more paths than this at once. */
const MAX_CACHED_LISTINGS = 16;

export interface DirectoryEntriesState {
  /** Every entry known so far; grows while the listing streams. */
  entries: DirectoryEntry[];
  isError: boolean;
  /** True while the listing is still streaming (cached entries stay visible). */
  isLoading: boolean;
}

/**
 * Lists `path`, folding the streamed batches in as they arrive. The previous
 * listing for the same path renders until the fresh read produces its first
 * batch.
 */
export function useDirectoryEntries(path: string): DirectoryEntriesState {
  const [state, setState] = useState<DirectoryEntriesState>(() => cachedState(path));
  const mountedPath = useRef(path);

  useEffect(() => {
    // Only a path change needs priming: the initializer already served the
    // first render.
    if (mountedPath.current !== path) {
      mountedPath.current = path;
      setState(cachedState(path));
    }

    let latest: DirectoryEntry[] = [];
    const listing = openDirectoryListing(path, {
      onHead: (view) => {
        latest = view.entries;
        setState({ entries: view.entries, isError: false, isLoading: true });
      },
      onEntries: (entries) => {
        latest = entries;
        setState({ entries, isError: false, isLoading: true });
      },
      onDone: () => {
        rememberListing(path, latest);
        setState((current) => ({ ...current, isLoading: false }));
      },
      onError: () => setState({ entries: [], isError: true, isLoading: false }),
    });

    // Reported through the listener above; this only keeps the rejection from
    // becoming an unhandled one.
    void listing.head.catch(() => {});

    return () => listing.dispose();
  }, [path]);

  return state;
}

function cachedState(path: string): DirectoryEntriesState {
  const cached = completedListings.get(path);
  return { entries: cached ?? [], isError: false, isLoading: cached === undefined };
}

/**
 * Whether two listings of a directory describe the same entries.
 *
 * A re-read that reports nothing new can be dropped instead of replacing the
 * displayed directory — see `ExplorerNavigator.refresh`. Field-by-field rather
 * than a rolling hash, because a hash errs in the dangerous direction:
 * "unchanged" for a listing that did change leaves the explorer stale until the
 * next change. Walking 35k entries costs a couple of milliseconds against the
 * ~123 ms sort and the full list re-render it lets the caller skip.
 *
 * The comparison is order-sensitive. The backend sorts every batch by the same
 * total order, so an unchanged directory comes back in the same order; a
 * backend that returned the same entries in a different batch order simply
 * reads as "changed" and gets the full refresh, which is the safe way to be
 * wrong.
 */
export function isSameListing(
  previous: readonly DirectoryEntry[],
  next: readonly DirectoryEntry[],
): boolean {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;

  for (let index = 0; index < previous.length; index += 1) {
    const before = previous[index];
    const after = next[index];
    if (before === after) continue;

    if (
      before.name !== after.name ||
      before.path !== after.path ||
      before.kind !== after.kind ||
      before.modifiedAt !== after.modifiedAt ||
      before.size !== after.size ||
      before.hidden !== after.hidden ||
      before.readOnly !== after.readOnly
    ) {
      return false;
    }
  }

  return true;
}

function rememberListing(path: string, entries: DirectoryEntry[]): void {
  completedListings.delete(path);
  completedListings.set(path, entries);

  while (completedListings.size > MAX_CACHED_LISTINGS) {
    const oldest = completedListings.keys().next().value;
    if (oldest === undefined) return;
    completedListings.delete(oldest);
  }
}
