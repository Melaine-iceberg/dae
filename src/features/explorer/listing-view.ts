import { ListingPacket } from "./entry-codec";
import type { DirectoryEntry, EntryKind } from "./types";

/**
 * The read interface the explorer's views consume, instead of an array.
 *
 * Two implementations back it today. `ArrayListingView` wraps the array the
 * JSON path already built and costs what indexing it costs; `PacketStreamView`
 * wraps the columnar packets the streaming path receives and decodes a field at
 * a time. The point of the indirection is not the indirection — it is that
 * every consumer is written against accessors, so the same view, filter and
 * sort run over either. A 35,803 entry directory arrives as ~1.2 MB of packed
 * bytes, and `entryAt` materialises only the ~40 rows a frame paints.
 *
 * That is also why this module owns the listing-wide scans the views used to
 * express as `entries.filter(...)` / `.map(...)`. They are scans over the
 * scalar accessors — `pathAt`, `kindAt`, `nameAt`, `hiddenAt`, `sizeAt`,
 * `modifiedAt` — never over `entryAt`. Reaching for an object here would
 * reintroduce the full materialisation the format exists to avoid, and it would
 * evict the painted rows from the row cache while it did it.
 *
 * Entry objects must be treated as read-only by callers. Their identity is
 * stable for an index that stays in the backing view's row cache, and that is
 * load bearing rather than incidental: React Compiler keys most of a row's memo
 * guards on the `entry` object itself, because they protect closures that
 * capture it. Measured on the real compiler output, a fresh object with equal
 * values fails 17 of `FileListRow`'s 27 guards and rebuilds 12 of its 16
 * elements on every render — see `scripts/audit-react-memo-identity.mjs`.
 *
 * Identity is *not* stable across a view being rebuilt, though, so render keys
 * still come from `pathAt`, and the derived views (`sortedListingView`,
 * `filteredListingView`) deliberately delegate their row reads to the view they
 * wrap rather than caching their own copies: a re-sort or a filter change then
 * leaves every painted row's object exactly where it was.
 */

export interface ListingView {
  /** Rows in the listing. Every index below this is readable. */
  readonly count: number;
  /**
   * The whole entry at `index`, or `undefined` past the end. This is the
   * expensive accessor for a byte-backed view — it decodes strings and builds an
   * object — so the render path calls it for visible rows only, and list-wide
   * work goes through the scalar accessors below.
   *
   * Repeated calls for an index that stays in the row cache return the *same*
   * object. See the identity note at the top of this module for why that matters.
   */
  entryAt(index: number): DirectoryEntry | undefined;
  kindAt(index: number): EntryKind | undefined;
  pathAt(index: number): string | undefined;
  /**
   * The remaining fields, one accessor each, so that a listing-wide scan never
   * has to build an object to read a single field of it. Absent optionals read
   * as `undefined` here and as `null` on the materialised entry, which is how
   * the generated `DirectoryEntry` spells them.
   */
  nameAt(index: number): string | undefined;
  hiddenAt(index: number): boolean;
  readOnlyAt(index: number): boolean;
  sizeAt(index: number): number | undefined;
  modifiedAt(index: number): number | undefined;
}

class ArrayListingView implements ListingView {
  readonly entries: readonly DirectoryEntry[];

  constructor(entries: readonly DirectoryEntry[]) {
    this.entries = entries;
  }

  get count(): number {
    return this.entries.length;
  }

  entryAt(index: number): DirectoryEntry | undefined {
    return this.entries[index];
  }

  kindAt(index: number): EntryKind | undefined {
    return this.entries[index]?.kind;
  }

  pathAt(index: number): string | undefined {
    return this.entries[index]?.path;
  }

  nameAt(index: number): string | undefined {
    return this.entries[index]?.name;
  }

  hiddenAt(index: number): boolean {
    return this.entries[index]?.hidden ?? false;
  }

  readOnlyAt(index: number): boolean {
    return this.entries[index]?.readOnly ?? false;
  }

  sizeAt(index: number): number | undefined {
    return this.entries[index]?.size ?? undefined;
  }

  modifiedAt(index: number): number | undefined {
    return this.entries[index]?.modifiedAt ?? undefined;
  }
}

/**
 * Rows a byte-backed view keeps materialised. Above every painted window the
 * app can produce (300 rows for three column panes at 4K) with room for a
 * burst of scrolling, and small enough that a pathological full-listing scan
 * through `entryAt` stays bounded.
 */
export const ROW_CACHE_CAP = 1024;

/**
 * Materialised rows of a byte-backed view, by index.
 *
 * Only `entryAt` fills this, so the listing-wide scans never touch it and never
 * pin the listing. The cap is what keeps that true for a caller that *does*
 * walk every index through `entryAt`: uncapped, one full materialisation
 * retains 9.25 MiB for a 35,807 entry listing (264.6 B/entry, measured). At
 * 1024 rows it is 202 KiB, while the largest plausible painted window — three
 * column panes at 4K, 300 rows — is 92 KiB. Eviction is FIFO over a Map's
 * insertion order, and costs one delete per miss, never per hit.
 *
 * Hit/miss costs, measured: 5.3 ns against 284 ns. Refreshing 100 painted rows
 * is 0.0011 ms when nothing changed and 0.0273 ms when every row is new — 0.16 %
 * of a 16.7 ms frame for the case where the listing genuinely changed and React
 * re-renders regardless.
 */
class RowCache {
  private readonly rows = new Map<number, DirectoryEntry>();

  get(index: number): DirectoryEntry | undefined {
    return this.rows.get(index);
  }

  set(index: number, entry: DirectoryEntry): void {
    if (this.rows.size >= ROW_CACHE_CAP) {
      const oldest = this.rows.keys().next().value;
      if (oldest !== undefined) this.rows.delete(oldest);
    }
    this.rows.set(index, entry);
  }
}

/**
 * A streamed listing: the batches that have arrived so far, in order.
 *
 * The head is the `readDirectory` response — 512 entries the backend sends as
 * JSON because the error path and the surrounding `DirectoryView` are already
 * JSON — and every packet after it is a batch that arrived over the IPC
 * channel. Index 0..head.length-1 reads the head array; from there the packets
 * are laid end to end.
 *
 * Rebuilt per flush rather than mutated, so that a view handed to a component
 * keeps its identity until the listing actually changes. That costs one pass
 * over the painted rows per flush, which is a frame in which React re-renders
 * the list anyway.
 */
class PacketStreamView implements ListingView {
  readonly head: readonly DirectoryEntry[];
  readonly packets: readonly ListingPacket[];
  /** Where each packet starts in the assembled index space. Ascending. */
  readonly starts: Int32Array;
  private readonly total: number;
  private readonly rows = new RowCache();

  constructor(head: readonly DirectoryEntry[], packets: readonly ListingPacket[]) {
    this.head = head;
    this.packets = packets;
    this.starts = new Int32Array(packets.length);

    let cursor = head.length;
    for (let ordinal = 0; ordinal < packets.length; ordinal++) {
      this.starts[ordinal] = cursor;
      cursor += packets[ordinal].count;
    }
    this.total = cursor;
  }

  get count(): number {
    return this.total;
  }

  entryAt(index: number): DirectoryEntry | undefined {
    const cached = this.rows.get(index);
    if (cached !== undefined) return cached;

    const entry = this.buildAt(index);
    if (entry === undefined) return undefined;

    this.rows.set(index, entry);
    return entry;
  }

  kindAt(index: number): EntryKind | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index].kind;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].kind(index - this.starts[ordinal]);
  }

  pathAt(index: number): string | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index].path;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].path(index - this.starts[ordinal]);
  }

  nameAt(index: number): string | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index].name;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].name(index - this.starts[ordinal]);
  }

  hiddenAt(index: number): boolean {
    if (index < 0 || index >= this.total) return false;
    if (index < this.head.length) return this.head[index].hidden;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].isHidden(index - this.starts[ordinal]);
  }

  readOnlyAt(index: number): boolean {
    if (index < 0 || index >= this.total) return false;
    if (index < this.head.length) return this.head[index].readOnly;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].isReadOnly(index - this.starts[ordinal]);
  }

  sizeAt(index: number): number | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index].size ?? undefined;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].size(index - this.starts[ordinal]);
  }

  modifiedAt(index: number): number | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index].modifiedAt ?? undefined;

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].modifiedAt(index - this.starts[ordinal]);
  }

  private buildAt(index: number): DirectoryEntry | undefined {
    if (index < 0 || index >= this.total) return undefined;
    if (index < this.head.length) return this.head[index];

    const ordinal = this.ordinalAt(index);
    return this.packets[ordinal].entry(index - this.starts[ordinal]);
  }

  /**
   * Which packet holds `index`. Called by every accessor, so it stays a plain
   * loop: the batches double in size as a listing streams, so even a 50,000
   * entry directory arrives as a handful of packets, and scanning back from the
   * newest one finds the row being painted first.
   */
  private ordinalAt(index: number): number {
    for (let ordinal = this.packets.length - 1; ordinal >= 0; ordinal--) {
      if (index >= this.starts[ordinal]) return ordinal;
    }
    return 0;
  }
}

/**
 * A view over another view's indices: display index `i` reads source index
 * `map[i]`.
 *
 * Both derived views are this one. Ordering is a permutation of the source
 * indices; filtering is the kept indices in ascending order. Reads delegate to
 * the source rather than materialising a second copy, which is what keeps a
 * painted row's object identity intact across a re-sort or a filter change —
 * the two events that re-render a large list without changing any entry.
 */
class MappedListingView implements ListingView {
  private readonly source: ListingView;
  private readonly map: Int32Array;

  constructor(source: ListingView, map: Int32Array) {
    this.source = source;
    this.map = map;
  }

  get count(): number {
    return this.map.length;
  }

  entryAt(index: number): DirectoryEntry | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.entryAt(source);
  }

  kindAt(index: number): EntryKind | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.kindAt(source);
  }

  pathAt(index: number): string | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.pathAt(source);
  }

  nameAt(index: number): string | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.nameAt(source);
  }

  hiddenAt(index: number): boolean {
    const source = this.sourceAt(index);
    return source < 0 ? false : this.source.hiddenAt(source);
  }

  readOnlyAt(index: number): boolean {
    const source = this.sourceAt(index);
    return source < 0 ? false : this.source.readOnlyAt(source);
  }

  sizeAt(index: number): number | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.sizeAt(source);
  }

  modifiedAt(index: number): number | undefined {
    const source = this.sourceAt(index);
    return source < 0 ? undefined : this.source.modifiedAt(source);
  }

  private sourceAt(index: number): number {
    return index >= 0 && index < this.map.length ? this.map[index] : -1;
  }
}

/**
 * How many leading rows `previous` and `next` share, or `null` when they are
 * not two snapshots of the same listing.
 *
 * A streamed listing arrives as a sequence of growing snapshots, and the sort
 * worker's job is to fold each new tail into the order it already has rather
 * than ordering the whole thing again. That is only valid when the new snapshot
 * really is the old one plus a tail — which this answers by comparing the
 * storage the two views were built from, by identity, rather than comparing
 * values: the pipeline rebuilds the backing array or packet list and reuses the
 * rows, so identity is exactly the "same listing, grown" relation.
 *
 * Derived views (`sortedListingView`, `filteredListingView`) have no storage of
 * their own and answer `null`: a filter change is a different row set, and a
 * re-sort has nothing to append to.
 */
export function sharedRowCount(previous: ListingView, next: ListingView): number | null {
  if (previous instanceof ArrayListingView && next instanceof ArrayListingView) {
    const before = previous.entries;
    const after = next.entries;
    if (before.length > after.length) return null;

    for (let index = 0; index < before.length; index += 1) {
      if (before[index] !== after[index]) return null;
    }
    return before.length;
  }

  if (previous instanceof PacketStreamView && next instanceof PacketStreamView) {
    // The head is a per-listing array, so a different one is a different
    // listing even if the packets happen to match.
    if (previous.head !== next.head) return null;
    if (previous.packets.length > next.packets.length) return null;

    for (let ordinal = 0; ordinal < previous.packets.length; ordinal += 1) {
      if (previous.packets[ordinal] !== next.packets[ordinal]) return null;
    }
    return previous.count;
  }

  return null;
}

/**
 * Views are memoised per backing object so that a component receiving one as a
 * prop keeps a stable identity across renders — the JSON path publishes a fresh
 * array per streamed batch, so this is per batch, not per render.
 */
const arrayViews = new WeakMap<readonly DirectoryEntry[], ListingView>();
const streamViews = new WeakMap<readonly ListingPacket[], ListingView>();

/** Wraps a listing that is already ordered. The caller's array supplies the order. */
export function listingViewOf(entries: readonly DirectoryEntry[]): ListingView {
  const existing = arrayViews.get(entries);
  if (existing) return existing;

  const view = new ArrayListingView(entries);
  arrayViews.set(entries, view);
  return view;
}

/**
 * Wraps the packets a streamed listing has received so far, behind the head the
 * backend answered with. The `packets` array is the memo key: the stream
 * appends by publishing a new array, exactly as the JSON path publishes a new
 * entry array.
 */
export function packetStreamView(
  head: readonly DirectoryEntry[],
  packets: readonly ListingPacket[],
): ListingView {
  const existing = streamViews.get(packets);
  if (existing) return existing;

  const view = new PacketStreamView(head, packets);
  streamViews.set(packets, view);
  return view;
}

/**
 * Wraps a single packed listing with no head — what a listing that arrived
 * entirely as one packet looks like, and what the parity check compares the
 * other backings against.
 *
 * Valid for directory listings only. `DirectoryEntry` is the generated type
 * plus an optional `relativePath` that the *search* path adds; a packet carries
 * the generated fields and cannot encode it, so a packet-backed view would drop
 * it silently. That field drives the relative-location label (`file-list.tsx`),
 * the command bar's hint, and the Git badge's rule that multi-level search hits
 * get no badge.
 */
export function listingViewOfPacket(packet: ListingPacket): ListingView {
  const existing = singlePacketViews.get(packet);
  if (existing) return existing;

  const view = packetStreamView([], [packet]);
  singlePacketViews.set(packet, view);
  return view;
}

/**
 * `packetStreamView` memoises on the packets *array*, and this entry point
 * builds a fresh one-element array per call — so it needs its own map, or every
 * call would hand back a new view and lose the identity a component's props
 * depend on.
 */
const singlePacketViews = new WeakMap<ListingPacket, ListingView>();

/**
 * Orders a view by `order`, an `Int32Array` where `order[displayIndex]` is the
 * source index. This is what the sort worker returns, and wrapping rather than
 * rebuilding the list is the whole reason it returns indices.
 */
export function sortedListingView(view: ListingView, order: Int32Array): ListingView {
  const existing = orderedViews.get(order);
  if (existing?.source === view) return existing.view;

  const ordered = new MappedListingView(view, order);
  orderedViews.set(order, { source: view, view: ordered });
  return ordered;
}

/**
 * Ordered views by permutation. The source is checked as well as the
 * permutation because nothing stops a caller from reusing one `order` against a
 * different listing; that case loses the memo (a fresh, still-correct view per
 * call) rather than returning a view of the wrong listing.
 */
const orderedViews = new WeakMap<Int32Array, { source: ListingView; view: ListingView }>();

/**
 * The rows of `view` that `keep` accepts. Returns `view` itself when nothing is
 * dropped, which is what lets the pipeline keep one identity when the user has
 * no filter on — the common case, since hidden files are shown by default.
 *
 * `keep` receives the *index*, not an entry: the predicate reads whatever
 * scalar accessors it needs off the view, so a full scan builds no objects.
 */
export function filteredListingView(
  view: ListingView,
  keep: (index: number) => boolean,
): ListingView {
  const kept: number[] = [];
  let dropped = 0;

  for (let index = 0; index < view.count; index += 1) {
    if (keep(index)) {
      kept.push(index);
    } else {
      dropped += 1;
    }
  }

  return dropped === 0 ? view : new MappedListingView(view, Int32Array.from(kept));
}

/**
 * Every path in the listing, in order — the select-all shortcut, and the
 * available-selection set the explorer rebuilds on every listing change.
 *
 * Deliberately not `[...].map(...)`: an array here is what a byte-backed view
 * cannot produce without materialising the whole listing.
 */
export function allPaths(view: ListingView): string[] {
  const paths: string[] = [];
  for (let index = 0; index < view.count; index += 1) {
    const path = view.pathAt(index);
    if (path !== undefined) paths.push(path);
  }
  return paths;
}

/** Every name in the listing, in order — the duplicate-name check on create. */
export function allNames(view: ListingView): string[] {
  const names: string[] = [];
  for (let index = 0; index < view.count; index += 1) {
    const name = view.nameAt(index);
    if (name !== undefined) names.push(name);
  }
  return names;
}

/**
 * The entries whose path `keep` accepts, materialised.
 *
 * Selection is the caller: it is never the whole listing, and a scan that
 * matched every row would mean the user selected everything. The scan itself
 * runs over `pathAt`, so the rows that do not match are never built and never
 * enter the row cache.
 */
export function entriesWhere(
  view: ListingView,
  keep: (path: string) => boolean,
): DirectoryEntry[] {
  const found: DirectoryEntry[] = [];
  for (let index = 0; index < view.count; index += 1) {
    const path = view.pathAt(index);
    if (path === undefined || !keep(path)) continue;

    const entry = view.entryAt(index);
    if (entry !== undefined) found.push(entry);
  }
  return found;
}

/**
 * Paths for `[first, lastExclusive)` — half-open like `Array.prototype.slice`,
 * which is what every caller used to write. Shift-select passes `index + 1`,
 * the marquee band passes `lastRow + 1`.
 *
 * Out-of-range bounds are clamped rather than treated as `slice` treats them.
 * `slice(-3, 2)` counts back from the end, which would select the wrong rows
 * silently; there is no sensible reason for a caller to pass a negative index
 * here, so the clamp is the failure mode that stays visible.
 */
export function pathsInRange(view: ListingView, first: number, lastExclusive: number): string[] {
  const start = Math.max(0, first);
  const end = Math.min(view.count, lastExclusive);

  const paths: string[] = [];
  for (let index = start; index < end; index += 1) {
    const path = view.pathAt(index);
    if (path !== undefined) paths.push(path);
  }
  return paths;
}

/**
 * The entries for `[first, lastExclusive)`, materialised.
 *
 * One grid row is the only caller, and its last row is short whenever the
 * listing does not divide evenly into columns — hence the `undefined` break
 * rather than padding.
 */
export function entriesInRange(
  view: ListingView,
  first: number,
  lastExclusive: number,
): DirectoryEntry[] {
  const end = Math.min(view.count, lastExclusive);

  const entries: DirectoryEntry[] = [];
  for (let index = Math.max(0, first); index < end; index += 1) {
    const entry = view.entryAt(index);
    if (entry === undefined) break;
    entries.push(entry);
  }
  return entries;
}

const directoryPathCache = new WeakMap<ListingView, Set<string>>();

/**
 * The paths of every directory in the listing.
 *
 * Cached on the view because the drag path asks for it on every pointer move
 * and nothing in it can change while the view lives — the pipeline rebuilds the
 * array instead of mutating it. Uncached this is a full scan per move event,
 * which at 35k entries is the sort of cost that shows up as drag lag.
 */
export function directoryPathSet(view: ListingView): ReadonlySet<string> {
  const cached = directoryPathCache.get(view);
  if (cached) return cached;

  const directories = new Set<string>();
  for (let index = 0; index < view.count; index += 1) {
    if (view.kindAt(index) !== "directory") continue;

    const path = view.pathAt(index);
    if (path !== undefined) directories.add(path);
  }

  directoryPathCache.set(view, directories);
  return directories;
}

/**
 * The subset of `sourcePaths` that exists in this listing *and* is a directory.
 * An entry dragged from a Miller column child pane is not in the root listing
 * at all, so membership is what decides, not the entry's own kind.
 */
export function draggableDirectoryPaths(
  view: ListingView,
  sourcePaths: readonly string[],
): string[] {
  const directories = directoryPathSet(view);
  return sourcePaths.filter((path) => directories.has(path));
}
