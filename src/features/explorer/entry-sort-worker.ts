/**
 * Off-thread ordering for streamed directory listings.
 *
 * `Intl.Collator` comparisons are the whole cost of putting a large directory
 * in order, and they cannot be made cheaper without changing what the user
 * sees: a hand-built sort key reproduces American-English ordering but not
 * ICU's, which for `zh-CN` places CJK ahead of Latin (verified — a lowercase,
 * zero-padded key disagrees with the collator on 1.1% of realistic filename
 * pairs). So the comparisons stay exactly as they are and move here instead,
 * where a 35,803 entry listing costs the UI thread about 5 ms of shipping
 * names in and permutations out rather than ~123 ms of collating.
 *
 * The worker keeps the accumulated primitives, so a batch only ever ships its
 * own tail. The reply is the full permutation, transferred as an
 * `Int32Array` buffer — never the entries themselves.
 */

import {
  createComparator,
  mergeRuns,
  sortIndices,
  SORT_COLLATOR_OPTIONS,
  type OrderOptions,
  type SortPrimitives,
} from "./entry-order";

export interface SortRequest {
  requestId: number;
  /** `reset` replaces the accumulated state; `append` extends it. */
  type: "reset" | "append";
  primaries: string[] | Float64Array;
  names: string[];
  directoryFlags: Uint8Array;
  options: OrderOptions;
  /** The app's active locale, so the worker collates like the main thread. */
  locale: string | undefined;
}

export interface SortResponse {
  requestId: number;
  /** Total entries the ordering covers; the caller checks it still matches. */
  count: number;
  /** `order[displayIndex] = sourceIndex` over the accumulated listing. */
  order: Int32Array;
}

interface WorkerScope {
  addEventListener(
    type: "message",
    listener: (event: MessageEvent<SortRequest>) => void,
  ): void;
  postMessage(message: SortResponse, transfer: Transferable[]): void;
}

const scope = globalThis as unknown as WorkerScope;

let primitives: SortPrimitives = {
  primaries: [],
  names: [],
  directoryFlags: new Uint8Array(0),
};
let order: number[] = [];
let cachedCollator: Intl.Collator | null = null;
let cachedCollatorKey = "";

/**
 * Collators are keyed by locale alone: the option set is a compile-time
 * constant (see `SORT_COLLATOR_OPTIONS`), so a locale switch is the only thing
 * that can invalidate one.
 */
function collatorFor(locale: string | undefined): Intl.Collator {
  if (cachedCollator === null || cachedCollatorKey !== (locale ?? "")) {
    cachedCollator = new Intl.Collator(locale, SORT_COLLATOR_OPTIONS);
    cachedCollatorKey = locale ?? "";
  }

  return cachedCollator;
}

/**
 * Orders `[start, end)` against the assembled primitives. The comparator reads
 * the whole arrays, so the tail runs carry their absolute indices from the
 * start rather than being rebased.
 */
function sortRange(
  start: number,
  end: number,
  compare: (left: number, right: number) => number,
): number[] {
  if (start === 0) {
    return sortIndices(end, compare);
  }

  const tail: number[] = [];
  for (let index = start; index < end; index++) {
    tail.push(index);
  }
  tail.sort(compare);
  return tail;
}

/**
 * Appends one numeric primary run to another. Spelled out rather than using
 * `concat`, which does not spread a typed array — `[].concat(new
 * Float64Array(3))` yields a single element holding the array, not three
 * numbers.
 */
function appendFloat64(previous: Float64Array, added: Float64Array): Float64Array {
  const extended = new Float64Array(previous.length + added.length);
  extended.set(previous, 0);
  extended.set(added, previous.length);
  return extended;
}

function handle(request: SortRequest): SortResponse {
  const collator = collatorFor(request.locale);
  const added = request.names.length;
  // An append onto nothing would leave the accumulated state half-formed, so
  // it is treated as a reset. The caller always resets first, which is what
  // makes the `append` branch below safe to assume a matching primary type.
  const replace = request.type === "reset" || primitives.names.length === 0;
  const base = replace ? 0 : primitives.names.length;

  if (replace) {
    primitives = {
      primaries: request.primaries,
      names: request.names,
      directoryFlags: request.directoryFlags,
    };
  } else {
    const names = primitives.names.concat(request.names);
    const directoryFlags = new Uint8Array(names.length);
    directoryFlags.set(primitives.directoryFlags, 0);
    directoryFlags.set(request.directoryFlags, base);

    const primaries =
      request.primaries instanceof Float64Array
        ? appendFloat64(primitives.primaries as Float64Array, request.primaries)
        : (primitives.primaries as string[]).concat(request.primaries as string[]);

    primitives = { primaries, names, directoryFlags };
  }

  // Built after the arrays are assembled: the comparator closes over them, so
  // a comparator built against the pre-append arrays would read stale data.
  const compare = createComparator(primitives, request.options, collator);

  if (replace) {
    order = sortRange(0, added, compare);
  } else if (added > 0) {
    const tail = sortRange(base, base + added, compare);
    // `order` still holds only indices below `base`, and those entries are
    // untouched, so merging the sorted tail keeps the whole run ordered.
    order = mergeRuns(order, tail, compare);
  }

  return {
    requestId: request.requestId,
    count: primitives.names.length,
    order: Int32Array.from(order),
  };
}

scope.addEventListener("message", (event) => {
  const response = handle(event.data);
  scope.postMessage(response, [response.order.buffer]);
});
