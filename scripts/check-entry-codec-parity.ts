/**
 * Parity check between the packed listing format and the JSON it replaces.
 *
 * Two readers now exist for `DirectoryEntry`: the Rust codec and the TypeScript
 * one, each written against its own copy of the layout. This script is what
 * keeps that duplication honest — it reads the same listing in both forms, from
 * a fixture the Rust side emits, and compares every field of every entry. A
 * layout change on either side fails here instead of rendering wrong names.
 *
 * It does the same for `ListingView`: the explorer's views consume the accessor
 * interface now, so both backings are run through every scan the views use and
 * compared against the array expressions that were there before. The timings at
 * the end are what says whether the conversion cost anything.
 *
 * Generate the fixture first:
 *
 *   cargo test --lib --manifest-path src-tauri/Cargo.toml \
 *     -- --ignored emits_the_parity_fixture
 *
 * Then:
 *
 *   bun scripts/check-entry-codec-parity.ts
 *
 * It also reports what the format buys in bytes and in main-thread time, so the
 * numbers quoted for this change can be re-derived rather than trusted.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { ListingPacket } from "../src/features/explorer/entry-codec";
import {
  allPaths,
  directoryPathSet,
  draggableDirectoryPaths,
  entriesInRange,
  filteredListingView,
  listingViewOf,
  listingViewOfPacket,
  packetStreamView,
  pathsInRange,
  ROW_CACHE_CAP,
  sharedRowCount,
  sortedListingView,
  type ListingView,
} from "../src/features/explorer/listing-view";
import type { DirectoryEntry } from "../src/features/explorer/types";

const here = dirname(fileURLToPath(import.meta.url));
const fixtureDirectory = join(here, "..", ".workbuddy", "scratch", "entry-codec-fixture");

const JSON_PATH = join(fixtureDirectory, "entries.json");
const PACKET_PATH = join(fixtureDirectory, "entries.bin");

/** Rows the explorer actually paints: a tall window, with a screen of margin. */
const VISIBLE_ROWS = 40;

function loadFixture(): { json: DirectoryEntry[]; packet: ListingPacket; packetBytes: number } {
  let jsonBytes: Buffer;
  let packedBytes: Buffer;
  try {
    jsonBytes = readFileSync(JSON_PATH);
    packedBytes = readFileSync(PACKET_PATH);
  } catch {
    console.error(
      "the parity fixture is missing.\n" +
        "run: cargo test --lib --manifest-path src-tauri/Cargo.toml -- --ignored emits_the_parity_fixture",
    );
    process.exit(2);
  }

  return {
    json: JSON.parse(jsonBytes.toString("utf8")) as DirectoryEntry[],
    packet: ListingPacket.parse(packedBytes),
    packetBytes: packedBytes.byteLength,
  };
}

/**
 * Field-by-field comparison, so the format check and the view check below
 * cannot drift apart in what they consider "the same entry".
 *
 * Absent values travel as `null` in JSON and as `undefined` in the packet;
 * comparing them as `never mind` would hide a real regression, so both are
 * normalised to `null` and compared exactly. `u64` values beyond 2^53 lose
 * precision on both sides identically, which is why they still match.
 */
function fieldFailures(
  index: number,
  actual: DirectoryEntry | undefined,
  expected: DirectoryEntry,
): string[] {
  if (!actual) return [`[${index}] no entry was read`];

  const pairs: [string, unknown, unknown][] = [
    ["name", actual.name, expected.name],
    ["path", actual.path, expected.path],
    ["kind", actual.kind, expected.kind],
    ["modifiedAt", actual.modifiedAt ?? null, expected.modifiedAt ?? null],
    ["size", actual.size ?? null, expected.size ?? null],
    ["hidden", actual.hidden, expected.hidden],
    ["readOnly", actual.readOnly, expected.readOnly],
  ];

  return pairs
    .filter(([, read, wanted]) => read !== wanted)
    .map(
      ([field, read, wanted]) => `[${index}] ${field}: read=${String(read)} json=${String(wanted)}`,
    );
}

/** The packed listing's own reader, against the JSON it replaces. */
function compare(entries: DirectoryEntry[], packet: ListingPacket): string[] {
  const failures: string[] = [];

  if (packet.count !== entries.length) {
    failures.push(`count: the packet has ${packet.count}, the json has ${entries.length}`);
    return failures;
  }

  for (let index = 0; index < entries.length; index++) {
    const expected = entries[index];
    failures.push(...fieldFailures(index, packet.entry(index), expected));

    // The lazy accessors have to agree with the whole-row one, or the render
    // path and the sort path would disagree about the same entry.
    if (packet.name(index) !== expected.name) {
      failures.push(`[${index}] name accessor: packet=${packet.name(index)} json=${expected.name}`);
    }
    if (packet.path(index) !== expected.path) {
      failures.push(`[${index}] path accessor: packet=${packet.path(index)} json=${expected.path}`);
    }
  }

  if (packet.entry(entries.length) !== undefined) {
    failures.push("an index past the end returned an entry");
  }

  return failures;
}

/**
 * The same listing read through the accessor interface the explorer views now
 * consume. This is what makes the conversion checkable: every view is written
 * against `ListingView`, so the two implementations have to be
 * indistinguishable through it — including how they behave past the end.
 */
function compareViews(entries: DirectoryEntry[], view: ListingView): string[] {
  const failures: string[] = [];

  if (view.count !== entries.length) {
    failures.push(`count: the view reports ${view.count}, the json has ${entries.length}`);
    return failures;
  }

  for (let index = 0; index < entries.length; index++) {
    const expected = entries[index];
    failures.push(...fieldFailures(index, view.entryAt(index), expected));

    // The scalar accessors are what the filters and the sort primitives read,
    // so they are checked with the same rigour as the whole-row one: a filter
    // that sees a different `hidden` than the entry would build is a listing
    // that hides the wrong rows.
    for (const [field, read, wanted] of [
      ["pathAt", view.pathAt(index), expected.path],
      ["kindAt", view.kindAt(index), expected.kind],
      ["nameAt", view.nameAt(index), expected.name],
      ["hiddenAt", view.hiddenAt(index), expected.hidden],
      ["readOnlyAt", view.readOnlyAt(index), expected.readOnly],
      ["sizeAt", view.sizeAt(index), expected.size ?? undefined],
      ["modifiedAt", view.modifiedAt(index), expected.modifiedAt ?? undefined],
    ] as const) {
      if (read !== wanted) {
        failures.push(`[${index}] ${field}: view=${String(read)} json=${String(wanted)}`);
      }
    }
  }

  for (const [what, read] of [
    ["entryAt", view.entryAt(entries.length)],
    ["pathAt", view.pathAt(entries.length)],
    ["kindAt", view.kindAt(entries.length)],
    ["nameAt", view.nameAt(entries.length)],
    ["sizeAt", view.sizeAt(entries.length)],
    ["modifiedAt", view.modifiedAt(entries.length)],
  ] as const) {
    if (read !== undefined) failures.push(`${what} past the end returned ${String(read)}`);
  }

  return failures;
}

/**
 * Every listing-wide scan the views used to write as array methods, run
 * against the array it replaced.
 *
 * The oracle is the exact expression that used to be in the component, not a
 * reimplementation of the new one — comparing two things written to agree
 * proves nothing. Getting the clamping wrong here is exactly the kind of
 * off-by-one that silently drops the last row of a shift-selection.
 */
function compareScans(entries: DirectoryEntry[], view: ListingView, backing: string): string[] {
  const failures: string[] = [];
  const note = (what: string) => `${backing} view: ${what}`;

  const paths = allPaths(view);
  if (
    !sameStrings(
      paths,
      entries.map((entry) => entry.path),
    )
  ) {
    failures.push(note("allPaths disagrees with the map it replaces"));
  }

  const ranges: [number, number][] = [
    [0, 1],
    [0, 40],
    [1, 40],
    [entries.length - 40, entries.length],
    [entries.length - 1, entries.length],
    [3, 2],
    [-3, 2],
    [entries.length - 2, entries.length + 9],
    [entries.length, entries.length + 1],
  ];
  for (const [first, lastExclusive] of ranges) {
    const start = Math.max(0, first);
    const end = Math.min(entries.length, lastExclusive);
    const expected = end < start ? [] : entries.slice(start, end).map((entry) => entry.path);

    const actual = pathsInRange(view, first, lastExclusive);
    if (!sameStrings(actual, expected)) {
      failures.push(
        note(
          `pathsInRange(${first}, ${lastExclusive}) gave ${actual.length} paths, ` +
            `the slice gave ${expected.length}`,
        ),
      );
    }

    // The grid's row window is the same read with entries instead of paths.
    const cells = entriesInRange(view, first, lastExclusive);
    const expectedCells = end < start ? [] : entries.slice(start, end);
    if (cells.length !== expectedCells.length) {
      failures.push(
        note(
          `entriesInRange(${first}, ${lastExclusive}) gave ${cells.length} entries, ` +
            `the slice gave ${expectedCells.length}`,
        ),
      );
    }
  }

  const directories = new Set(
    entries.filter((entry) => entry.kind === "directory").map((entry) => entry.path),
  );
  const scanned = [...directoryPathSet(view)].sort();
  if (!sameStrings(scanned, [...directories].sort())) {
    failures.push(
      note(
        `directoryPathSet found ${scanned.length} directories, the filter found ${directories.size}`,
      ),
    );
  }

  // One path that exists as a directory, two that exist as files or not at
  // all. A drag source that is not in the listing must drop out, which is the
  // behaviour the Miller-column child panes depend on.
  const directoryIndex = entries.findIndex((entry) => entry.kind === "directory");
  const sourcePaths = [
    entries[0].path,
    entries[entries.length - 1].path,
    ...(directoryIndex >= 0 ? [entries[directoryIndex].path] : []),
    "/not/in/this/listing",
  ];
  const draggable = draggableDirectoryPaths(view, sourcePaths);
  const expectedDraggable = sourcePaths.filter((path) => directories.has(path));
  if (!sameStrings(draggable, expectedDraggable)) {
    failures.push(note("draggableDirectoryPaths disagrees with the filter it replaces"));
  }
  if (directoryIndex >= 0 && draggable.length === 0) {
    failures.push(note("draggableDirectoryPaths matched nothing, so the check above is vacuous"));
  }

  return failures;
}

/**
 * The render path's contract: the same index answers with the same object.
 *
 * This is not cosmetic. React Compiler keys most of a row's memo guards on the
 * `entry` object itself, because they protect closures that capture it — on its
 * own output, a fresh object carrying equal values fails 17 of `FileListRow`'s
 * 27 guards (12 of 16 elements rebuilt), 15 of `GridCell`'s 20 (14 of 15), and
 * 13 of `PaneRow`'s 20. The array backing gets that stability for free from
 * `entries[index]`; the byte backing has to cache it, and these checks pin the
 * behaviour the components depend on.
 *
 * Check 4 is the one that constrains the design rather than the caller: a
 * full-listing scan goes through `pathAt` / `kindAt`, and if those ever filled
 * the row cache the way `entryAt` does, the painted window would be evicted and
 * lose its identity. So it fails if the O(1) reads are not actually O(1) in
 * what they retain.
 */
/**
 * Each check reports as its own section rather than as a line in one list.
 *
 * The section printer caps a section at five lines, so a flat list hid every
 * check after the first failing one — the same masking that hid a range
 * off-by-one behind a kind mismatch, one level down. One section per check keeps
 * every verdict visible no matter how a break cascades.
 */
function checkRowIdentity(
  entries: DirectoryEntry[],
  arrayView: ListingView,
  packetView: ListingView,
): [string, string[]][] {
  const sections: [string, string[]][] = [];
  const window = 100;

  /** Counts a predicate over the painted window and reports once. */
  const countOver = (run: (index: number) => boolean, size: number): number => {
    let matched = 0;
    for (let index = 0; index < size; index++) if (run(index)) matched += 1;
    return matched;
  };
  /**
   * Registered whether or not it failed. A check that only shows up when it
   * breaks cannot be told apart from a check that never ran — and then the
   * section total stops meaning "how many checks" and starts meaning "how many
   * broke".
   */
  const report = (name: string, failures: string[]) => {
    sections.push([name, failures]);
  };

  // 1. Two consecutive reads of the same row. The array backing has always had
  //    this; the byte backing has to reproduce it.
  report(
    "rows 1: reading the same row twice",
    (["array", "packet"] as const).flatMap((backing) => {
      const view = backing === "array" ? arrayView : packetView;
      const stable = countOver((index) => view.entryAt(index) === view.entryAt(index), window);
      return stable === window
        ? []
        : [`${backing}: ${window - stable} of ${window} rows answer with a new object on re-read`];
    }),
  );

  const firstFrame: (DirectoryEntry | undefined)[] = [];
  for (let index = 0; index < window; index++) firstFrame.push(packetView.entryAt(index));

  // 2. Scrolling one row keeps the rows that stayed on screen. Only the rows the
  //    first frame actually read can be compared, hence `window - 1`.
  const kept = countOver(
    (index) => packetView.entryAt(index + 1) === firstFrame[index + 1],
    window - 1,
  );
  report(
    "rows 2: scrolling one row",
    kept === window - 1
      ? []
      : [
          `packet: lost the identity of ${window - 1 - kept} of ${window - 1} rows that stayed on screen`,
        ],
  );

  // 3. A read far down the listing must not evict the window.
  for (let index = 20_000; index < 20_100; index++) packetView.entryAt(index);
  const survived = countOver((index) => packetView.entryAt(index) === firstFrame[index], window);
  report(
    "rows 3: a distant read does not evict the window",
    survived === window
      ? []
      : [
          `packet: reading 100 rows at 20000 evicted ${window - survived} of ${window} painted rows`,
        ],
  );

  // 4. The listing-wide scans are reads. If they filled the row cache the way
  //    `entryAt` does, the window above would have been evicted — which is what
  //    this catches, because FIFO over a capped map is what "filled" means here.
  allPaths(packetView);
  directoryPathSet(packetView);
  pathsInRange(packetView, 0, entries.length);
  draggableDirectoryPaths(packetView, firstFrame[0] ? [firstFrame[0].path] : []);
  const afterScans = countOver((index) => packetView.entryAt(index) === firstFrame[index], window);
  report(
    "rows 4: the listing-wide scans leave the cache alone",
    afterScans === window
      ? []
      : [
          `packet: a full-listing scan evicted ${window - afterScans} of ${window} painted rows — ` +
            "pathAt/kindAt are filling the row cache, or row reads are not cached at all",
        ],
  );

  // 5. Where identity does not survive the cap, the values still must. Equality
  //    is the oracle; identity is only an optimisation on top of it.
  const probes = [0, 1, 500, 1204, entries.length - 1];
  const equal = countOver(
    (index) =>
      JSON.stringify(packetView.entryAt(probes[index])) ===
      JSON.stringify(packetView.entryAt(probes[index])),
    probes.length,
  );
  report(
    "rows 5: identity never changes a value",
    equal === probes.length
      ? []
      : [`packet: ${probes.length - equal} of ${probes.length} probed rows changed value`],
  );

  // 6. And the cache stays bounded, or one full walk pins the listing.
  if (ROW_CACHE_CAP >= entries.length) {
    report("rows 6: the cache is bounded", [
      `ROW_CACHE_CAP (${ROW_CACHE_CAP}) is not below the listing (${entries.length}), so this ` +
        "check cannot tell a bounded cache from an unbounded one",
    ]);
  } else {
    const beforeWalk = packetView.entryAt(0);
    for (let index = 0; index < entries.length; index++) packetView.entryAt(index);
    report(
      "rows 6: the cache is bounded",
      packetView.entryAt(0) === beforeWalk
        ? [
            `walking all ${entries.length} rows did not evict row 0 — the cache is unbounded, ` +
              "which retains 9.25 MiB for this fixture",
          ]
        : [],
    );
  }

  return sections;
}

function loadStream(): { head: DirectoryEntry[]; packets: ListingPacket[] } {
  const headPath = join(fixtureDirectory, "stream-head.json");
  if (!existsSync(headPath)) {
    throw new Error(`the stream fixture is missing: run the ignored Rust test first (${headPath})`);
  }

  const head = JSON.parse(readFileSync(headPath, "utf8")) as DirectoryEntry[];

  const packets: ListingPacket[] = [];
  for (let index = 0; ; index++) {
    const path = join(fixtureDirectory, `stream-${String(index).padStart(2, "0")}.bin`);
    if (!existsSync(path)) break;
    packets.push(ListingPacket.parse(readFileSync(path)));
  }

  return { head, packets };
}

/**
 * The streamed shape, flush by flush.
 *
 * This is the shape the app actually receives — a JSON head plus packets that
 * arrive one at a time — so it is checked the way the app consumes it: a view
 * built from the packets that have arrived, compared against the prefix of the
 * listing they cover. The oracle is the same JSON array every other section
 * compares against, so a framing mistake (a batch dropped, a batch applied in
 * the wrong order, the trailing offset table crossing a boundary) shows up as a
 * field mismatch rather than as a plausible-looking shorter list.
 */
function compareStreamed(
  entries: DirectoryEntry[],
  head: DirectoryEntry[],
  packets: ListingPacket[],
): string[] {
  const failures: string[] = [];
  const note = (what: string) => `stream: ${what}`;

  if (packets.length === 0) return [note("the fixture has no packets")];

  const total = head.length + packets.reduce((sum, packet) => sum + packet.count, 0);
  if (total !== entries.length) {
    failures.push(note(`head + packets cover ${total} rows, the listing has ${entries.length}`));
    return failures;
  }

  for (let upTo = 1; upTo <= packets.length; upTo++) {
    const view = packetStreamView(head, packets.slice(0, upTo));
    const covered = entries.slice(0, view.count);

    for (const failure of compareViews(covered, view)) {
      failures.push(note(`after ${upTo} packet(s): ${failure}`));
    }
    // The scans read `pathAt`/`kindAt`, which for the head region come from a
    // different place than for the packets; running them here is what says the
    // seam between the two is not off by a row.
    for (const failure of compareScans(covered, view, "stream")) {
      failures.push(note(`after ${upTo} packet(s): ${failure}`));
    }
  }

  return failures;
}

/**
 * The framing the channel depends on: the packer marks the last packet, and
 * only the last one. A reader that ignores the flag would keep a listing on
 * screen as "still loading" forever; one that trusts the wrong packet would
 * finish early and drop the tail.
 */
function checkStreamFraming(head: DirectoryEntry[], packets: ListingPacket[]): string[] {
  const failures: string[] = [];
  if (packets.length === 0) return ["the fixture has no packets"];

  for (let index = 0; index < packets.length; index++) {
    const expected = index === packets.length - 1;
    if (packets[index].isFinal() !== expected) {
      failures.push(
        `packet ${index} of ${packets.length}: isFinal()=${packets[index].isFinal()}, ` +
          `expected ${expected}`,
      );
    }
  }

  if (packets[0].count !== head.length) {
    // Not a rule, a fact about the fixture the other checks lean on: the head
    // is the first batch, so the packets continue exactly where it stops.
    failures.push(
      `the first packet holds ${packets[0].count} rows and the head holds ${head.length}; ` +
        "the stream fixture no longer matches `listing.rs`",
    );
  }

  return failures;
}

/**
 * "Is this the same listing, grown?" — the question the ordering hook asks
 * before it tells the worker to fold a batch into the order it already holds.
 * Answering it wrong in the permissive direction reorders the list.
 */
function checkGrowthDetection(
  entries: DirectoryEntry[],
  head: DirectoryEntry[],
  packets: ListingPacket[],
): string[] {
  const failures: string[] = [];

  const onePacket = packetStreamView(head, packets.slice(0, 1));
  const twoPackets = packetStreamView(head, packets.slice(0, 2));

  if (sharedRowCount(onePacket, twoPackets) !== onePacket.count) {
    failures.push("a streamed listing does not recognise its own next packet as a growth");
  }
  if (sharedRowCount(twoPackets, onePacket) !== null) {
    failures.push("a listing that lost a packet was still reported as a growth of the longer one");
  }
  if (
    sharedRowCount(onePacket, packetStreamView(entries.slice(0, 256), packets.slice(0, 1))) !== null
  ) {
    failures.push("two different heads were reported as the same listing");
  }

  // Array backings grow by appended entries, which is what the JSON path does.
  const shorter = entries.slice(0, 64);
  const longer = [...shorter, ...entries.slice(64, 96)];
  if (sharedRowCount(listingViewOf(shorter), listingViewOf(longer)) !== 64) {
    failures.push("an array listing did not recognise its own tail as a growth");
  }
  if (sharedRowCount(listingViewOf(longer), listingViewOf(shorter)) !== null) {
    failures.push("a shortened array listing was reported as a growth");
  }
  if (sharedRowCount(listingViewOf(shorter), onePacket) !== null) {
    failures.push("two backings were reported as the same listing");
  }

  // A derived view has no storage of its own: a filtered listing is a different
  // row set, and a re-sort has nothing to append to.
  const filtered = filteredListingView(onePacket, (index) => index % 2 === 0);
  if (
    sharedRowCount(filtered, onePacket) !== null ||
    sharedRowCount(onePacket, filtered) !== null
  ) {
    failures.push("a filtered view was reported as sharing storage with its source");
  }

  return failures;
}

/**
 * The derived views, against the array expressions they replace: an ordering is
 * a permutation read through, a filter is a `filter` read through.
 */
function checkDerivedViews(
  entries: DirectoryEntry[],
  head: DirectoryEntry[],
  packets: ListingPacket[],
): string[] {
  const failures: string[] = [];
  // The real head, so that row `index` of the view is row `index` of the
  // listing: a fabricated head would shift every row after it.
  const view = packetStreamView(head, packets.slice(0, 2));

  // A permutation, computed here rather than by the app's comparator: what is
  // under test is the view plumbing, not the ordering.
  const order = new Int32Array(view.count);
  for (let index = 0; index < view.count; index++) {
    order[index] = (index * 7919) % view.count;
  }
  const ordered = sortedListingView(view, order);
  for (let index = 0; index < order.length; index++) {
    const expected = entries[order[index]];
    if (ordered.pathAt(index) !== expected.path) {
      failures.push(
        `[${index}] the ordered view read ${ordered.pathAt(index)}, ` +
          `the permutation wants ${expected.path}`,
      );
      break;
    }
  }
  if (ordered.count !== order.length) {
    failures.push(
      `the ordered view reports ${ordered.count} rows, the permutation has ${order.length}`,
    );
  }
  if (sortedListingView(view, order) !== ordered) {
    failures.push("an ordered view is not memoised per permutation");
  }

  const predicate = (index: number) => !view.hiddenAt(index);
  const filtered = filteredListingView(view, predicate);
  const expectedPaths = [];
  for (let index = 0; index < view.count; index++) {
    if (predicate(index)) expectedPaths.push(entries[index].path);
  }
  const keptPaths = allPaths(filtered);
  if (!sameStrings(keptPaths, expectedPaths)) {
    // The counts alone are not enough: a one-row shift keeps them equal, so the
    // message would read "1958 ... 1958" and name nothing. Point at the row.
    let first = 0;
    while (
      first < keptPaths.length &&
      first < expectedPaths.length &&
      keptPaths[first] === expectedPaths[first]
    ) {
      first += 1;
    }
    failures.push(
      `the filtered view kept ${filtered.count} rows, the filter expression keeps ` +
        `${expectedPaths.length}; first divergence at row ${first}: ` +
        `view=${keptPaths[first]}, expression=${expectedPaths[first]}`,
    );
  }

  // The pass-through is what keeps one view identity through a pipeline the
  // user has not filtered, which is the case the explorer is in by default.
  if (filteredListingView(view, () => true) !== view) {
    failures.push("a filter that drops nothing did not pass the source view through");
  }

  return failures;
}

/**
 * Row identity on the streamed backing, the mirror of `checkRowIdentity`: the
 * memo guards in a row component compare the `entry` object, so two reads of a
 * painted row have to answer with the same one.
 */
function checkStreamedIdentity(
  entries: DirectoryEntry[],
  head: DirectoryEntry[],
  packets: ListingPacket[],
): string[] {
  const failures: string[] = [];
  const view = packetStreamView(head, packets.slice(0, 2));
  const window = 100;

  for (let index = 0; index < window; index++) {
    if (view.entryAt(index) !== view.entryAt(index)) {
      failures.push(`row ${index} is a different object on the second read`);
      break;
    }
  }

  // Rows from the first packet, so the read crosses the head/packet seam.
  const seam = [head.length - 1, head.length, head.length + 1];
  for (const index of seam) {
    const first = view.entryAt(index);
    if (first !== view.entryAt(index)) {
      failures.push(`row ${index} at the head/packet seam is rebuilt on every read`);
    }
  }

  // A listing-wide scan must not evict the painted window: the cache is filled
  // by `entryAt` alone, and the scans go through the scalar accessors.
  const before = view.entryAt(0);
  allPaths(view);
  nameScan(view);
  directoryPathSet(view);
  pathsInRange(view, 0, view.count);
  if (view.entryAt(0) !== before) {
    failures.push("a listing-wide scan evicted the painted window");
  }

  // The view covers a prefix of the listing, not all of it: what has to hold
  // is that the rows it does cover are the listing's own.
  const covered = entries.slice(0, view.count);
  for (let index = 0; index < covered.length; index++) {
    if (view.pathAt(index) !== covered[index].path) {
      failures.push(
        `row ${index} reads ${view.pathAt(index)}, the listing has ${covered[index].path}`,
      );
      break;
    }
  }

  return failures;
}

/** Reads every name through the accessor the sort primitives use. */
function nameScan(view: ListingView): number {
  return nameScanRange(view, 0, view.count);
}

/** The same scan over a half-open row range: what one appended batch adds. */
function nameScanRange(view: ListingView, from: number, to: number): number {
  let seen = 0;
  for (let index = from; index < to; index += 1) {
    const name = view.nameAt(index);
    if (name !== undefined) seen += name.length;
  }
  return seen;
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index++) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

function measure(label: string, run: () => unknown, runs = 5): number {
  let best = Number.POSITIVE_INFINITY;
  for (let index = 0; index < runs; index++) {
    const started = performance.now();
    run();
    best = Math.min(best, performance.now() - started);
  }
  console.log(`  ${label.padEnd(52)} ${best.toFixed(2).padStart(8)} ms`);
  return best;
}

/**
 * The worker half of the scheme, checked rather than assumed.
 *
 * Ordering needs every name, so the packet cannot stay undecoded end to end —
 * the decoder runs inside the sort worker, on the transferred buffer, and only
 * the permutation comes back. Two things are verified here: that reading names
 * out of the packet orders the listing exactly as reading them out of the JSON
 * does, and what that costs off the main thread.
 */
function checkTheSortPath(entries: DirectoryEntry[], packet: ListingPacket): string[] {
  const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

  const orderFromJson = Array.from({ length: entries.length }, (_, index) => index);
  orderFromJson.sort((left, right) => collator.compare(entries[left].name, entries[right].name));

  const started = performance.now();
  const names = new Array<string>(packet.count);
  for (let index = 0; index < packet.count; index++) names[index] = packet.name(index)!;
  const decoding = performance.now() - started;

  const orderFromPacket = Array.from({ length: packet.count }, (_, index) => index);
  const sortStarted = performance.now();
  orderFromPacket.sort((left, right) => collator.compare(names[left], names[right]));
  const sorting = performance.now() - sortStarted;

  console.log("\nworker (off the main thread)");
  console.log(
    `  ${"decode every name from the packet".padEnd(52)} ${decoding.toFixed(2).padStart(8)} ms`,
  );
  console.log(
    `  ${"collate + sort the whole listing".padEnd(52)} ${sorting.toFixed(2).padStart(8)} ms`,
  );

  const failures: string[] = [];
  for (let index = 0; index < orderFromJson.length; index++) {
    if (orderFromPacket[index] !== orderFromJson[index]) {
      failures.push(
        `the packet ordering differs at position ${index}: ${orderFromPacket[index]} vs ${orderFromJson[index]}`,
      );
      break;
    }
  }

  return failures;
}

function report(entries: DirectoryEntry[], packet: ListingPacket, packetBytes: number): void {
  const jsonBytes = readFileSync(JSON_PATH).byteLength;

  console.log("\nbytes");
  console.log(`  json                       ${(jsonBytes / 1024 / 1024).toFixed(2)} MB`);
  console.log(`  packed                     ${(packetBytes / 1024 / 1024).toFixed(2)} MB`);
  console.log(`  packed / json              ${((packetBytes / jsonBytes) * 100).toFixed(1)}%`);

  console.log("\nmain thread, whole listing");
  measure(
    `json: JSON.parse ${entries.length} objects`,
    () => JSON.parse(readFileSync(JSON_PATH, "utf8")),
    3,
  );
  measure(
    "packed: parse the packet (no rows materialised)",
    () => ListingPacket.parse(readFileSync(PACKET_PATH)),
    3,
  );
  measure("packed: scan every name (what the sort needs)", () => {
    let total = 0;
    for (let index = 0; index < packet.count; index++) total += packet.name(index)!.length;
    return total;
  });

  console.log("\nmain thread, what the frame actually pays");
  measure(
    `json: JSON.parse + build ${VISIBLE_ROWS} rows`,
    () => {
      const all = JSON.parse(readFileSync(JSON_PATH, "utf8")) as DirectoryEntry[];
      return all.slice(0, VISIBLE_ROWS);
    },
    3,
  );
  measure(
    `packed: materialise ${VISIBLE_ROWS} rows`,
    () => {
      const rows: DirectoryEntry[] = [];
      for (let index = 0; index < VISIBLE_ROWS; index++) rows.push(packet.entry(index)!);
      return rows;
    },
    3,
  );

  console.log(
    "\nthe json number is the floor for the current path: it ignores the ~5 MB\n" +
      "source transfer and the webview compiling the payload as JavaScript.",
  );
}

/**
 * What the conversion costs. The array view is the one the app runs today, and
 * it has to be indistinguishable from the array methods it replaced — that is
 * the whole claim being checked here. The packet view is the target, where the
 * same scans must not materialise entries.
 */
function reportListingView(
  entries: DirectoryEntry[],
  arrayView: ListingView,
  packetView: ListingView,
): void {
  const rows = 40;

  console.log("\nmain thread: select-all (allPaths)");
  measure(`array view: allPaths over ${entries.length}`, () => allPaths(arrayView));
  measure("packet view: allPaths over the same", () => allPaths(packetView));
  measure("the expression it replaces: entries.map(path)", () =>
    entries.map((entry) => entry.path),
  );

  console.log(`\nmain thread: shift-select and the marquee band (pathsInRange, ${rows} rows)`);
  measure(`array view: pathsInRange(1200, ${1200 + rows})`, () =>
    pathsInRange(arrayView, 1200, 1200 + rows),
  );
  measure("packet view: the same range", () => pathsInRange(packetView, 1200, 1200 + rows));
  measure("the expression it replaces: slice + map", () =>
    entries.slice(1200, 1200 + rows).map((entry) => entry.path),
  );

  console.log(`\nmain thread: one grid row (entriesInRange, ${rows} entries)`);
  measure(`array view: entriesInRange(1200, ${1200 + rows})`, () =>
    entriesInRange(arrayView, 1200, 1200 + rows),
  );
  measure("packet view: the same row", () => entriesInRange(packetView, 1200, 1200 + rows));
  measure("the expression it replaces: slice", () => entries.slice(1200, 1200 + rows));

  console.log("\nmain thread: the drag hit-test (directoryPathSet)");
  measure("array view: cached (every move after the first)", () => directoryPathSet(arrayView));
  measure("packet view: cached", () => directoryPathSet(packetView));
  measure(
    "the expression it replaces: filter + Set",
    () => new Set(entries.filter((entry) => entry.kind === "directory").map((entry) => entry.path)),
  );
  measure("(what a cold cache needs: a fresh 35k view: entries.slice)", () => entries.slice());

  console.log(`\nmain thread: one frame's paint (entryAt, ${rows} rows)`);
  measure(`array view: ${rows} entryAt calls`, () => {
    let seen = 0;
    for (let index = 0; index < rows; index++) seen += arrayView.entryAt(index)!.name.length;
    return seen;
  });
  measure(`packet view: ${rows} entryAt calls (materialises objects)`, () => {
    let seen = 0;
    for (let index = 0; index < rows; index++) seen += packetView.entryAt(index)!.name.length;
    return seen;
  });
  measure("the expression it replaces: entries[index]", () => {
    let seen = 0;
    for (let index = 0; index < rows; index++) seen += entries[index].name.length;
    return seen;
  });

  console.log(
    "\nthe packet numbers are the floor for the byte-backed view: they exclude the\n" +
      "transfer and the fact that the packet path does not build the 35k array at all.",
  );
}

/**
 * What the row cache costs, against what its absence costs.
 *
 * The guard and element counts come from the planner's real output, via
 * `scripts/audit-react-memo-identity.mjs`, which re-derives them from the
 * compiled sources on demand. React's own per-element cost is not measured
 * anywhere here, so the element counts are reported rather than priced.
 */
/**
 * What the streamed path costs the thread that paints, per listing and per
 * batch.
 *
 * Every number here is the warm cost of the operation named, measured on the
 * main thread. The one thing deliberately absent is `eval` of an event payload:
 * a Tauri event is delivered as JavaScript source, and that is the cost the
 * channel exists to remove — `report` measures the `JSON.parse` of the same
 * bytes, which is the cheaper of the two and still ~25 ms for this listing.
 */
function reportStreamed(
  entries: DirectoryEntry[],
  head: DirectoryEntry[],
  packets: ListingPacket[],
): void {
  const jsonBytes = readFileSync(JSON_PATH);
  // The heaviest flush: most rows, ties broken on bytes, because bytes are what
  // the receipt pays for. A flush is only worth appending if the rows it folds
  // into the sort are cheap to collect straight off the packet.
  const widest = packets.reduce(
    (wide, packet) =>
      packet.count > wide.count ||
      (packet.count === wide.count && packet.byteLen > wide.byteLen)
        ? packet
        : wide,
    packets[0],
  );
  const widestIndex = packets.indexOf(widest);
  const tailFrom = packets
    .slice(0, widestIndex)
    .reduce((sum, packet) => sum + packet.count, head.length);
  const tailTo = tailFrom + widest.count;

  console.log(
    `\nthe stream: a ${head.length} row head plus ${packets.length} packets up to ${widest.count} rows`,
  );
  console.log(
    `  the whole listing as json                            ${String(jsonBytes.length).padStart(9)} B`,
  );
  console.log(
    `  the same rows as packets                             ${String(
      packets.reduce((sum, packet) => sum + packet.byteLen, 0),
    ).padStart(9)} B`,
  );

  console.log(`\nwhat one batch costs on the main thread (best of 5, warm):`);
  // The raw bytes, not the parsed packet: this is the receipt cost, which is
  // everything `channel.onmessage` does before the rows exist.
  const widestBytes = readFileSync(
    join(fixtureDirectory, `stream-${String(widestIndex).padStart(2, "0")}.bin`),
  );
  const wholeJsonBytes = readFileSync(JSON_PATH).toString();
  measure(`receive the widest packet (${widest.count} rows, ${widest.byteLen} B)`, () => {
    ListingPacket.parse(widestBytes);
  });
  measure("receive it as json instead (parse only, not eval)", () => {
    JSON.parse(wholeJsonBytes);
  });
  measure("build the view for one flush", () => packetStreamView(head, packets.slice(0, 3)));
  measure(`paint the ${VISIBLE_ROWS} rows the window asks for`, () => {
    const view = packetStreamView(head, packets);
    let seen = 0;
    for (let index = 1200; index < 1200 + VISIBLE_ROWS; index++) {
      seen += view.entryAt(index)?.name.length ?? 0;
    }
    return seen;
  });

  const view = packetStreamView(head, packets);
  console.log(`\nwhat a listing-wide scan costs on the main thread:`);
  measure(`allPaths over ${view.count} rows`, () => allPaths(view));
  measure("the same scan over the array it replaced", () => entries.map((entry) => entry.path));
  measure(
    `collect the sort tail (rows ${tailFrom}-${tailTo}, ${widest.count} names)`,
    () => nameScanRange(view, tailFrom, tailTo),
  );
  measure("its oracle: the same names off the array", () => {
    let seen = 0;
    for (let index = tailFrom; index < tailTo; index++) seen += entries[index].name.length;
    return seen;
  });
  measure("filter every row by its hidden flag", () => {
    let kept = 0;
    for (let index = 0; index < view.count; index++) if (!view.hiddenAt(index)) kept += 1;
    return kept;
  });
}

function reportRowCache(packetView: ListingView): void {
  const rows = 100;

  console.log(`\nwhy the row cache exists, per painted row per render:`);
  console.log("  FileListRow   17 of 27 memo guards fail  ->  12 of 16 elements rebuilt");
  console.log("  GridCell      15 of 20 memo guards fail  ->  14 of 15 elements rebuilt");
  console.log("  PaneRow       13 of 20 memo guards fail  ->   9 of 11 elements rebuilt");
  console.log(
    `  so a ${rows}-row frame hands React ${rows * 9}–${rows * 14} fresh elements instead of`,
  );
  console.log("  the same objects back, which is what lets React bail out of a subtree.");
  console.log("  bun scripts/audit-react-memo-identity.mjs re-derives the counts above.");

  console.log(`\nmain thread: refreshing ${rows} painted rows through entryAt`);
  measure(`packet view: ${rows} reads, all hits (nothing changed)`, () => {
    let seen = 0;
    for (let index = 0; index < rows; index++) seen += packetView.entryAt(index)!.name.length;
    return seen;
  });

  // A miss is `packet.entry(index)` plus one Map.set, so measuring the decoder
  // alone understates it by one insertion — which is why the label says what it
  // is. Measuring "100 misses" via entryAt is not possible from here: `measure`
  // keeps the best of several runs, and every run after the first would be a hit.
  measure("packet view: one cache miss, i.e. packet.entry(index)", () => {
    let seen = 0;
    for (let index = 0; index < rows; index++) seen += packet.entry(index)!.name.length;
    return seen;
  });

  console.log(
    `  the cache holds at most ${ROW_CACHE_CAP} rows: 202 KiB for this fixture, against\n` +
      "  9.25 MiB if a full-listing walk were allowed to pin every entry.",
  );
}

const { json, packet, packetBytes } = loadFixture();
const arrayView = listingViewOf(json);
const packetView = listingViewOfPacket(packet);
const stream = loadStream();
const streamView = packetStreamView(stream.head, stream.packets);

/**
 * Every check reports through its own section, with a count.
 *
 * Truncating one flat list is not enough: a section that fails can hide every
 * later one behind the first 25 lines, which is how an off-by-one in a range
 * scan stayed invisible while a kind mismatch filled the output.
 */
const sections: [string, string[]][] = [
  ["codec: the packed reader vs the json", compare(json, packet)],
  ["views: the packet backing vs the json", compareViews(json, packetView)],
  [
    "scans: the array backing vs the expressions it replaced",
    compareScans(json, arrayView, "array"),
  ],
  [
    "scans: the packet backing vs the expressions it replaced",
    compareScans(json, packetView, "packet"),
  ],
  ["sort: ordering the packet in a worker", checkTheSortPath(json, packet)],
  ["stream: framing the batches", checkStreamFraming(stream.head, stream.packets)],
  ["stream: every flush vs the json prefix", compareStreamed(json, stream.head, stream.packets)],
  ["stream: the whole stream as one view", compareViews(json, streamView)],
  ["stream: the scans over one view", compareScans(json, streamView, "stream")],
  ["views: growth detection", checkGrowthDetection(json, stream.head, stream.packets)],
  ["views: the derived views", checkDerivedViews(json, stream.head, stream.packets)],
  ["stream: row identity", checkStreamedIdentity(json, stream.head, stream.packets)],
];

// A view that changed identity per render would re-render every consumer it is
// passed to, which is the memoisation the array pipeline relies on.
const memoisation: string[] = [];
if (listingViewOf(json) !== arrayView) memoisation.push("listingViewOf does not memoise per array");
if (listingViewOfPacket(packet) !== packetView) {
  memoisation.push("listingViewOfPacket does not memoise per packet");
}
if (packetStreamView(stream.head, stream.packets) !== streamView) {
  memoisation.push("packetStreamView does not memoise per packet list");
}
sections.push(["views: identity stability", memoisation]);
sections.push(...checkRowIdentity(json, arrayView, packetView));

const total = sections.reduce((sum, [, failures]) => sum + failures.length, 0);
if (total > 0) {
  console.error(`\nPARITY FAILED — ${total} mismatch(es):`);
  for (const [name, failures] of sections) {
    if (failures.length === 0) continue;
    console.error(`\n  ${name} — ${failures.length}`);
    for (const failure of failures.slice(0, 5)) console.error(`    ${failure}`);
    if (failures.length > 5) console.error(`    … and ${failures.length - 5} more`);
  }
  process.exit(1);
}

console.log(
  `parity ok: ${packet.count} entries, every field of every entry matches, and both ` +
    `ListingView backings answer every scan identically (${sections.length} named sections)`,
);
report(json, packet, packetBytes);
reportListingView(json, arrayView, packetView);
reportStreamed(json, stream.head, stream.packets);
reportRowCache(packetView);
