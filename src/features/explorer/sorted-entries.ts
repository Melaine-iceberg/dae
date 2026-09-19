import { useEffect, useMemo, useRef, useState } from "react";

import { i18n } from "@/i18n";

import { orderSignature, type ExplorerSortKey, type ExplorerSortOrder } from "./entry-order";
import type { SortRequest, SortResponse } from "./entry-sort-worker";
import { collectSortPrimitives, sortEntries } from "./preferences";
import type { DirectoryEntry } from "./types";

/**
 * Orders streamed directory listings without blocking the UI thread.
 *
 * A big directory reaches the explorer as growing snapshots — `readDirectory`
 * answers with 512 entries, then events push the rest in batches that double
 * up to 8,192. Ordering each snapshot from scratch cost ~123 ms of main-thread
 * time for a 35,803 entry directory (measured), all of it `Intl.Collator`
 * comparisons.
 *
 * That ordering cannot be made cheaper without changing what the user sees: a
 * hand-built sort key reproduces American-English order but not ICU's, which
 * for `zh-CN` sorts CJK ahead of Latin. So the comparisons keep their exact
 * semantics and move to a worker instead. Each batch ships only its own tail,
 * and the reply is a permutation transferred as an `Int32Array` — for the same
 * directory that is ~5 ms of main-thread work in total, mostly structured
 * clone of the names.
 *
 * Lists at or below `SYNC_SORT_LIMIT` are ordered inline: they are what the
 * explorer hits on almost every navigation, and one sort of a few hundred
 * entries is under a millisecond, so spawning and feeding a worker would cost
 * more than it saves.
 */

/** Above this many entries, ordering moves to the worker. */
const SYNC_SORT_LIMIT = 1000;

const NO_ENTRIES: DirectoryEntry[] = [];

/** Set once a worker fails to start, so later renders stop trying. */
let workerUnavailable = false;

interface OrderResult {
  /** Ordering options the result was produced for. */
  signature: string;
  /** The snapshot the permutation indexes into. */
  source: readonly DirectoryEntry[];
  ordered: DirectoryEntry[];
}

interface Session {
  worker: Worker;
  requestId: number;
  /** What the worker's accumulated state corresponds to; `null` forces a reset. */
  source: readonly DirectoryEntry[] | null;
  signature: string;
  /** The request whose reply is still worth applying. */
  pending: { requestId: number; signature: string; source: readonly DirectoryEntry[] } | null;
}

function spawnWorker(): Worker | null {
  if (workerUnavailable || typeof Worker === "undefined") {
    return null;
  }

  try {
    return new Worker(new URL("./entry-sort-worker.ts", import.meta.url), { type: "module" });
  } catch {
    workerUnavailable = true;
    return null;
  }
}

/**
 * Whether `previous` is an unchanged prefix of `next`, which is what a
 * streamed batch looks like: the same entries in the same order, plus a tail.
 * A pointer comparison per entry is enough — the listings reuse the entry
 * objects they were built from.
 */
function extendsSource(
  previous: readonly DirectoryEntry[],
  next: readonly DirectoryEntry[],
): boolean {
  if (previous.length > next.length) {
    return false;
  }

  for (let index = 0; index < previous.length; index++) {
    if (previous[index] !== next[index]) {
      return false;
    }
  }

  return true;
}

/**
 * Orders `entries` for display, keeping the previous result visible while the
 * worker catches up with a new snapshot or a new sort key.
 */
export function useSortedEntries(
  entries: readonly DirectoryEntry[],
  sortKey: ExplorerSortKey,
  sortOrder: ExplorerSortOrder,
  foldersFirst: boolean,
): DirectoryEntry[] {
  // The locale is part of the signature: it decides how names collate, so a
  // language switch has to reorder from scratch rather than merge.
  const signature = `${orderSignature({ foldersFirst, sortKey, sortOrder })}|${i18n.language}`;
  const sessionRef = useRef<Session | null>(null);
  const [streamed, setStreamed] = useState<OrderResult | null>(null);
  // Mirrors "a worker is alive and will answer"; state rather than a ref read
  // so the memo below stays reactive.
  const [canStream, setCanStream] = useState(false);

  useEffect(() => {
    if (entries.length <= SYNC_SORT_LIMIT) {
      // Nothing worth offloading. Forgetting the source makes the next large
      // listing reset instead of appending to a prefix it no longer shares.
      if (sessionRef.current) {
        sessionRef.current.source = null;
      }
      setStreamed(null);
      return;
    }

    let session = sessionRef.current;
    if (!session) {
      const worker = spawnWorker();
      if (!worker) {
        return;
      }

      const created: Session = { pending: null, requestId: 0, signature: "", source: null, worker };
      session = created;
      sessionRef.current = created;
      setCanStream(true);

      // The session object is stable for the worker's lifetime and
      // `setStreamed` is stable across renders, so neither closure goes stale.
      worker.onmessage = (event: MessageEvent<SortResponse>) => {
        const { count, order, requestId } = event.data;
        const pending = created.pending;

        // A superseded reply describes a snapshot the explorer has moved past.
        if (!pending || pending.requestId !== requestId || count !== pending.source.length) {
          return;
        }

        const source = pending.source;
        const ordered: DirectoryEntry[] = [];
        for (let index = 0; index < count; index++) {
          ordered.push(source[order[index]]);
        }

        setStreamed({ ordered, signature: pending.signature, source });
      };

      worker.onerror = () => {
        workerUnavailable = true;
        worker.terminate();
        if (sessionRef.current === created) {
          sessionRef.current = null;
        }
        // Dropping the result sends the next render down the inline path.
        setCanStream(false);
        setStreamed(null);
      };
    }

    // Messages reach a worker in order, so recording the optimistic state
    // before the reply is what makes the next batch's prefix check valid.
    const previous = session.source;
    const append =
      previous !== null && session.signature === signature && extendsSource(previous, entries);
    const base = append && previous !== null ? previous.length : 0;
    const primitives = collectSortPrimitives(base === 0 ? entries : entries.slice(base), sortKey);

    const requestId = ++session.requestId;
    session.source = entries;
    session.signature = signature;
    session.pending = { requestId, signature, source: entries };

    session.worker.postMessage({
      directoryFlags: primitives.directoryFlags,
      locale: i18n.language || undefined,
      names: primitives.names,
      options: { foldersFirst, sortKey, sortOrder },
      primaries: primitives.primaries,
      requestId,
      type: append ? "append" : "reset",
    } satisfies SortRequest);
  }, [entries, foldersFirst, signature, sortKey, sortOrder]);

  useEffect(
    () => () => {
      sessionRef.current?.worker.terminate();
      sessionRef.current = null;
    },
    [],
  );

  return useMemo(() => {
    if (entries.length === 0) {
      return NO_ENTRIES;
    }

    if (entries.length <= SYNC_SORT_LIMIT) {
      return sortEntries(entries, sortKey, sortOrder, foldersFirst);
    }

    // A result for an earlier prefix of this same listing is worth showing:
    // it is at most one batch behind, and it usually carries the right order
    // already. That covers a sort key change too, where waiting the few
    // milliseconds for the worker beats re-sorting a huge list inline.
    if (streamed && canStream && extendsSource(streamed.source, entries)) {
      return streamed.ordered;
    }

    // First snapshot past the limit, or a directory switch while the worker is
    // still busy. Ordered inline so the list is never painted in read order.
    return sortEntries(entries, sortKey, sortOrder, foldersFirst);
  }, [canStream, entries, foldersFirst, sortKey, sortOrder, streamed]);
}
