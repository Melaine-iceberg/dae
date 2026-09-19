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
  /** Called with the complete entry list so far, after every batch. */
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

  // Batches that beat the head here are accumulated and published together
  // with it, so the list is always head-then-batches regardless of arrival
  // order.
  const unlisten = events.explorerDirectoryEntries.listen(({ payload }) => {
    if (stopped || payload.streamId !== streamId) return;

    if (payload.entries.length > 0) {
      streamed = streamed.concat(payload.entries);
      if (head) listener.onEntries([...head.entries, ...streamed]);
    }

    if (payload.done) finish();
  });

  const detach = () => {
    if (detached) return;
    detached = true;
    void unlisten.then((stop) => stop());
  };

  const finish = () => {
    if (complete || stopped) return;
    complete = true;
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

function rememberListing(path: string, entries: DirectoryEntry[]): void {
  completedListings.delete(path);
  completedListings.set(path, entries);

  while (completedListings.size > MAX_CACHED_LISTINGS) {
    const oldest = completedListings.keys().next().value;
    if (oldest === undefined) return;
    completedListings.delete(oldest);
  }
}
