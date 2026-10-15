/**
 * What a listing-wide scan costs on a byte-backed view.
 *
 * The explorer's render pipeline is written against `ListingView` accessors,
 * and a few of its consumers scan the *whole* listing: the type-ahead name
 * table (`allNames`, `file-list.tsx`) and the selection prune's available-path
 * set (`allPaths`, `use-explorer-selection.ts`) both rebuild an array of one
 * string per row on every published snapshot — which, while a listing streams,
 * is once per animation frame.
 *
 * This measures exactly that: `nameAt` / `pathAt` over every row of a packed
 * listing, plus the cost those scans add up to across one streamed listing
 * (the batches double from `FIRST_BATCH_SIZE`, so the scan is repeated over
 * every prefix).
 *
 * It is not part of the app and nothing imports it. Run it with Bun, from the
 * repository root:
 *
 * ```bash
 * bun bench/frontend-scans/scan-cost.ts
 * bun bench/frontend-scans/scan-cost.ts --entries 120000
 * ```
 *
 * The numbers land in `docs/performance-opportunities.md`. They are
 * per-machine: compare this harness against itself, before and after a change,
 * and never against a number from another machine.
 */

import { packEntries } from "../../src/features/explorer/entry-codec.ts";
import {
  allNames,
  allPaths,
  directoryPathSet,
  packetStreamView,
  prefixListingView,
  presentPaths,
  type ListingView,
} from "../../src/features/explorer/listing-view.ts";

/** The size the rest of the repository's notes are written against. */
const DEFAULT_ENTRIES = 35_803;

/** The batches `read_remaining_batches` sends, after the head the command answers with. */
const HEAD_SIZE = 512;
const BATCH_SIZES = [1024, 2048, 4096, 8192, 8192, 8192, 8192];

/**
 * One row in this many is a directory, for the scans that need rows to collect.
 * A real folder of 35,803 entries is not all files, and `directoryPathSet` only
 * reads a row's path when its kind says directory.
 */
const DIRECTORY_EVERY = 8;

function entryCount(): number {
  const index = process.argv.indexOf("--entries");
  if (index < 0) return DEFAULT_ENTRIES;

  const parsed = Number(process.argv[index + 1]);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : DEFAULT_ENTRIES;
}

/**
 * The rows of one listing. `directoryEvery` 0 means every row is a file, which
 * is what the type-ahead and selection scans want; the drag scan needs the
 * directories a real listing has among them.
 */
function dataset(count: number, directoryEvery = 0) {
  const base = "K:\\dae-listing-bench\\big";
  const rows = Array.from({ length: count }, (_, index) => ({
    hidden: false,
    kind:
      directoryEvery > 0 && index % directoryEvery === 0
        ? ("directory" as const)
        : ("file" as const),
    modifiedAt: 1_700_000_000_000 + index,
    name: `entry-${index}.txt`,
    path: `${base}\\entry-${index}.txt`,
    readOnly: false,
    size: 1234 + index,
  }));

  return { base, rows };
}

/** Median of `passes` runs, in milliseconds; `fn` must return a value the
 *  engine cannot drop (every scan below keeps its array). The `warmup` passes
 *  are discarded — pass `0` when the point is the *first* call. */
function median(fn: () => unknown, passes = 7, warmup = 2): number {
  const samples: number[] = [];
  for (let pass = 0; pass < passes + warmup; pass += 1) {
    const started = performance.now();
    fn();
    const elapsed = performance.now() - started;
    if (pass >= warmup) samples.push(elapsed);
  }
  samples.sort((left, right) => left - right);
  return samples[Math.floor(samples.length / 2)];
}

/** `allNames`: one decoded name per row, kept. */
function scanNames(packet: ListingPacket, rows: number): number {
  const names: string[] = [];
  for (let index = 0; index < rows; index += 1) {
    names.push(packet.name(index) ?? "");
  }
  return names.length;
}

/** `allPaths`: one spelled-out path per row, kept. */
function scanPaths(packet: ListingPacket, rows: number): number {
  const paths: string[] = [];
  for (let index = 0; index < rows; index += 1) {
    paths.push(packet.path(index) ?? "");
  }
  return paths.length;
}

/**
 * The selection prune: which of `selected` the listing still holds — the
 * shipped `presentPaths`, so what is measured here is what runs.
 *
 * `selected` is a set of paths, so the walk stops at the *last* of them; a
 * selection whose paths are not in the listing at all (the deleted-file case
 * the prune exists for) reads every row, exactly as the scan it replaced did.
 */
function scanForSelection(view: ListingView, selected: string[]): number {
  return presentPaths(view, selected).size;
}

/** The drag path: which of the dragged paths this listing still sees as a
 *  directory. One `kindAt` per row, plus a `pathAt` for the rows that turn out
 *  to be directories — the shipped `directoryPathSet`, cached on the view. */
function scanDirectories(view: ListingView): number {
  return directoryPathSet(view).size;
}

/** One streamed listing, as the frontend receives it: the JSON head, then the
 *  packets. Returns the cumulative row count a full scan of each published
 *  snapshot would touch, in publish order. */
function streamPrefixes(rows: number): number[] {
  const counts: number[] = [Math.min(rows, HEAD_SIZE)];
  let seen = counts[0];

  for (const size of BATCH_SIZES) {
    if (seen >= rows) break;
    seen = Math.min(rows, seen + size);
    counts.push(seen);
  }

  return counts;
}

function main(): void {
  const count = entryCount();
  const { base, rows } = dataset(count);

  const packStarted = performance.now();
  const packet = packEntries(base, rows);
  const packElapsed = performance.now() - packStarted;

  console.log(`entries                     ${count}`);
  console.log(`packet bytes                ${packet.byteLen}`);
  console.log(`pack (one batch of ${count})   ${packElapsed.toFixed(2)} ms`);
  console.log("");

  // The head arrives as JSON and its names are already strings, so only the
  // packet rows cost a decode. Scanning the whole packet is therefore the
  // worst case for one scan, and the per-row cost the prefixes below reuse.
  const namesScan = median(() => scanNames(packet, packet.count));
  const pathsScan = median(() => scanPaths(packet, packet.count));
  console.log(`allNames full scan          ${namesScan.toFixed(2)} ms`);
  console.log(`allPaths full scan          ${pathsScan.toFixed(2)} ms`);
  console.log("");

  const prefixes = streamPrefixes(count);
  const packedRows = packet.count;

  let cumulativeNames = 0;
  let cumulativePaths = 0;
  for (const rowsSoFar of prefixes) {
    const packetRows = Math.max(0, Math.min(packedRows, rowsSoFar - HEAD_SIZE));
    cumulativeNames += median(() => scanNames(packet, packetRows), 3);
    cumulativePaths += median(() => scanPaths(packet, packetRows), 3);
  }

  console.log(`publishes per listing       ${prefixes.length} (${prefixes.join(", ")} rows)`);
  console.log(`cumulative allNames         ${cumulativeNames.toFixed(2)} ms`);
  console.log(`cumulative allPaths         ${cumulativePaths.toFixed(2)} ms`);
  console.log(`cumulative both             ${(cumulativeNames + cumulativePaths).toFixed(2)} ms`);
  console.log("");

  // The two consumers that used to run on every publish, and what each costs
  // now that it runs only when something asks it to.
  const view = packetStreamView([], [packet]);
  const pathAt = (index: number) => view.pathAt(index) ?? "";

  console.log("selection prune");
  // What this replaced, kept as the baseline: every path built, then a set of
  // them, once per published snapshot while anything was selected.
  const replaced = median(() => new Set(allPaths(view)).size, 3);
  console.log(`  ${"(before) whole listing".padEnd(24)} ${replaced.toFixed(3)} ms`);
  const selections: [string, string[]][] = [
    ["selected row is first", [pathAt(0)]],
    ["selected row is middle", [pathAt(Math.floor(packet.count / 2))]],
    ["selected row is last", [pathAt(packet.count - 1)]],
    ["selected path is gone", [`${base}\\not-in-this-listing.txt`]],
  ];
  for (const [label, selected] of selections) {
    const elapsed = median(() => scanForSelection(view, selected));
    console.log(`  ${label.padEnd(24)} ${elapsed.toFixed(3)} ms`);
  }
  console.log("");

  // The type-ahead table, which is now built by the keystroke that needs it and
  // cached on the view rather than rebuilt for every publish. `packetStreamView`
  // memoises on the packets array, and that array is fresh here, so every cold
  // sample builds the table from scratch while every warm sample reads the one
  // the first call cached.
  const cold = median(() => allNames(packetStreamView([], [packet])).length, 9, 0);
  const warm = median(() => allNames(view).length);
  console.log("type-ahead table (what a keystroke pays)");
  console.log(`  cold (builds the table)   ${cold.toFixed(2)} ms`);
  console.log(`  warm (cached on the view) ${warm.toFixed(3)} ms`);
  console.log("");

  // The drag path. `file-list.tsx` asks for this from `pointermove`, and the
  // answer is cached on the view — so a drag pays one scan per *published
  // snapshot* rather than one per move event. A snapshot is one animation frame
  // while a listing streams, and a new batch is a new view, which is the case
  // the cache cannot cover by itself: it is the one a drag in flight over a
  // still-streaming directory hits.
  //
  // The cold samples wrap a fresh packets array, so `packetStreamView` hands
  // back a view this cache has never seen; the cumulative ones run over
  // `prefixListingView`, which is the mapped shape the displayed (sorted or
  // filtered) listing has — one extra indirection per row, which the raw view
  // above does not pay.
  const drag = dataset(count, DIRECTORY_EVERY);
  const dragPacket = packEntries(drag.base, drag.rows);
  const dragView = packetStreamView([], [dragPacket]);
  const dragCold = median(() => scanDirectories(packetStreamView([], [dragPacket])), 9, 0);
  const dragWarm = median(() => scanDirectories(dragView));

  let cumulativeDrag = 0;
  for (const rowsSoFar of prefixes) {
    cumulativeDrag += median(() => scanDirectories(prefixListingView(dragView, rowsSoFar)), 3);
  }

  console.log(`drag: directory set (one row in ${DIRECTORY_EVERY} a directory)`);
  console.log(`  rows / directories        ${count} / ${directoryPathSet(dragView).size}`);
  console.log(`  cold (one snapshot)       ${dragCold.toFixed(3)} ms`);
  console.log(`  warm (cached on the view) ${dragWarm.toFixed(3)} ms`);
  console.log(
    `  cumulative over a stream  ${cumulativeDrag.toFixed(3)} ms  (${prefixes.length} publishes)`,
  );
}

main();
