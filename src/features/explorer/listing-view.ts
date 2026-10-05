import { ListingPacket, packEntries } from "./entry-codec";
import type { DirectoryEntry, EntryKind } from "./types";

/**
 * The read interface the explorer's views consume, instead of an array.
 *
 * Two implementations back it today. `ArrayListingView` wraps the array the
 * JSON path already built and costs what indexing it costs; `PacketStreamView`
 * wraps the columnar packets the streaming path receives and decodes a field at
 * a time. The point of the indirection is not the indirection — it is that
 * every consumer is written against accessors, so the same view, filter and
 * sort run over either. A 35,803 entry directory arrives as packed bytes — 39 %
 * of what the same listing costs as JSON, and each of those bytes names its
 * directory once rather than once per row — and `entryAt` materialises only the
 * ~40 rows a frame paints.
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
   * The directory these rows hang off, spelled exactly as the backend spelled it
   * when it packed them. A packet stores its paths relative to it and `pathAt`
   * prepends it back, so a patch has to pack its rows against the same one for
   * them to read back like the rows they replace.
   *
   * `""` for a listing that was never packed — a head array on its own, or search
   * results — where every row carries its whole path. Reading it is always safe:
   * an empty base is the neutral element, so nothing gets stripped or prepended.
   */
  readonly base: string;
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
  /** Every row of an array listing spells its own whole path. */
  readonly base = "";

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
  /**
   * The directory the stream was read from, which every packet of it names in
   * its own header. One value for the whole listing: the packets are batches of
   * one directory, so their bases agree, and the first is as good as any. Empty
   * for a listing that has received nothing but its head.
   */
  readonly base: string;
  /** Where each packet starts in the assembled index space. Ascending. */
  readonly starts: Int32Array;
  private readonly total: number;
  private readonly rows = new RowCache();

  constructor(head: readonly DirectoryEntry[], packets: readonly ListingPacket[]) {
    this.head = head;
    this.packets = packets;
    this.base = packets.length === 0 ? "" : packets[0].base;
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
 *
 * Those two shapes age differently, and `sharedRowCount` is where it matters:
 * an ascending map survives its source growing, because the added rows land
 * past every index it already holds. That is what makes a filtered listing
 * recognisable as the same listing one batch longer. A permutation has no such
 * property.
 */
class MappedListingView implements ListingView {
  /**
   * Readable by the rest of the module rather than private: `sharedRowCount`
   * and `packetBacked` have to reach through a derived view to the one it
   * wraps, and this class is not exported, so nothing outside the module can
   * name these anyway.
   */
  readonly source: ListingView;
  readonly map: Int32Array;

  constructor(source: ListingView, map: Int32Array) {
    this.source = source;
    this.map = map;
  }

  get count(): number {
    return this.map.length;
  }

  /** The listing it wraps: reordering or filtering rows does not move them. */
  get base(): string {
    return this.source.base;
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
 * Every branch either matches the whole of `previous` or answers `null`. A
 * partial answer would be worse than useless to the callers, which read this as
 * "the new snapshot is the old one grown" and take `previous.count` as the
 * length of the shared run.
 *
 * A streamed listing arrives as a sequence of growing snapshots, and the sort
 * worker's job is to fold each new tail into the order it already has rather
 * than ordering the whole thing again. That is only valid when the new snapshot
 * really is the old one plus a tail — which this answers by comparing the
 * storage the two views were built from, by identity, rather than comparing
 * values: the pipeline rebuilds the backing array or packet list and reuses the
 * rows, so identity is exactly the "same listing, grown" relation.
 *
 * A filtered view answers through the view it wraps. `filteredListingView`
 * scans its source in ascending order and keeps indices, so as the listing
 * streams the map is appended to and every existing entry keeps pointing at the
 * same row — the same relation, one level up. That is what lets a filtered
 * listing fold batch by batch instead of re-sorting, and it is why the ordering
 * hook is allowed to defer for one.
 *
 * The map comparison is what separates that from a filter *change*, which
 * rebuilds the map rather than extending it, but it is not airtight: a new
 * filter that keeps the old filter's rows first, and adds none before the last
 * of them, reads as growth too. The cost of that coincidence is one batch of
 * rows painted from the previous filter while the worker catches up, and the
 * order it settles on is still right — the primitives accumulated for the
 * shared prefix describe the same rows either way. `sortedListingView` composes
 * a permutation rather than an ascending map, so it matches only while the
 * order happens to be the identity.
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

  if (previous instanceof MappedListingView && next instanceof MappedListingView) {
    // The source decides first: a filter change over a listing that did not
    // grow, a re-sort, and a directory switch all have to answer `null` here
    // whatever the maps happen to look like.
    const source = sharedRowCount(previous.source, next.source);
    if (source === null) return null;

    const before = previous.map;
    const after = next.map;
    if (before.length > after.length) return null;

    for (let index = 0; index < before.length; index += 1) {
      // `>= source` is a row the two sources do not share, so a map entry that
      // reaches into the grown tail ends the run even when both maps name it.
      if (before[index] !== after[index] || before[index] >= source) return null;
    }

    return before.length;
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
 * One row a watcher patch resolved, against the listing it patches.
 *
 * `index` is an index into the *displayed* listing rather than into the packets
 * behind it, so a patch composes onto a listing that was patched before: the
 * previous patch's map is rewritten, not decoded. `entry` is the row's current
 * content, or `null` when the name no longer resolves to anything.
 */
export interface ListingPatchChange {
  index: number;
  entry: DirectoryEntry | null;
}

/** What a patched view was built from, so the next patch can grow it again. */
interface PatchProvenance {
  /** The head array of the listing the patch rows are appended to. Kept by
   *  identity, which is what `sharedRowCount` compares first. */
  head: readonly DirectoryEntry[];
  packets: readonly ListingPacket[];
  /** Rows in that listing — where the next batch of patch rows starts. */
  sourceCount: number;
  /** The displayed rows as source indices. */
  map: Int32Array | null;
}

/**
 * A patched view by its provenance.
 *
 * The mapping cannot be recovered from the published view's shape: a patch that
 * removes a row and a filter that hides one are both a `MappedListingView` with
 * holes, and patching through a filter would rewrite hidden rows while keeping
 * the rows the user filtered out. Keying on the view this module handed back is
 * exact — and the navigator's own listing is the only thing ever patched.
 */
const patchProvenance = new WeakMap<ListingView, PatchProvenance>();

/**
 * Returns `view` with `changes` applied in place, or `null` when `view` is not a
 * listing this module can patch.
 *
 * This is what a directory change costs when the explorer already holds the
 * listing: one `stat` per named child over IPC, one scan of the listing's names
 * to locate the rows (see `indicesOfNames`), and a map of the rows it already
 * has. The alternative — re-read the directory — is a full enumeration, the
 * packed bytes for every row in the folder, a full `isSameListing` scan, and a
 * sort the worker has to start over because a fresh head array is not the one it
 * ordered. `ExplorerNavigator.patch` holds the measured comparison.
 *
 * The three outcomes are not equally cheap, and that is inherent rather than
 * incidental:
 *
 * - Rows *added*: the listing grows by one packet and keeps its head and the
 *   packets before it, so `sharedRowCount` reads it as the same listing grown and
 *   the worker folds the new rows into the order it holds. Nothing is
 *   re-ordered — and that survives a listing that was patched before, because
 *   the previous patch's map is a prefix of the new one.
 * - Rows *rewritten*: the row's own sort primitives may have changed, so the
 *   order cannot be trusted and the worker starts over. The map keeps the
 *   listing's length and replaces the slot.
 * - Rows *removed*: same as a rewrite — a hole ends the run `sharedRowCount`
 *   recognises. The map drops the slot.
 *
 * A rewrite or a removal also has to be visible to the worker as a different
 * listing, and it is: `sharedRowCount` compares storage by identity, so a map
 * that points a row somewhere else does not match the map it replaced.
 */
export function patchedListingView(
  view: ListingView,
  changes: readonly ListingPatchChange[],
): ListingView | null {
  let held: PatchProvenance;
  if (view instanceof PacketStreamView) {
    held = { head: view.head, map: null, packets: view.packets, sourceCount: view.count };
  } else {
    const provenance = patchProvenance.get(view);
    if (!provenance) return null;
    held = provenance;
  }

  /** The rows the patch appends to the listing's index space, in pack order. */
  const fresh: DirectoryEntry[] = [];
  /** Positions in `fresh` of rows the listing did not already hold. */
  const created: number[] = [];
  /**
   * What happens to a displayed row: the position in `fresh` that replaces it,
   * or `null` when it goes away. One map rather than a set of each, so a row
   * cannot be counted as both replaced and removed.
   */
  const slots = new Map<number, number | null>();

  for (const { entry, index } of changes) {
    if (index < 0) {
      if (entry !== null) {
        created.push(fresh.length);
        fresh.push(entry);
      }
      continue;
    }

    // A caller scans the listing it patches, so an out-of-range index means the
    // listing changed underneath it. Refuse rather than rewrite a row that is
    // now somebody else's.
    if (index >= view.count) return null;

    if (entry === null) {
      slots.set(index, null);
      continue;
    }

    slots.set(index, fresh.length);
    fresh.push(entry);
  }

  // Nothing to add and nothing to remove: the rows on screen already say what
  // the filesystem does. Returning the same view costs the caller no re-render,
  // which is the point of asking for the named children rather than the folder.
  if (fresh.length === 0 && slots.size === 0) return view;

  // A patch is packed against the listing's own base, so a new row gives up the
  // directory the rows around it already gave up and reads back spelled like
  // them. The listing this view wraps is the only source of that spelling — a
  // caller could not rebuild it without re-reading the directory.
  const packets =
    fresh.length === 0 ? held.packets : [...held.packets, packEntries(view.base, fresh)];
  const source = packetStreamView(held.head, packets);
  /** Where the patch rows start in the grown listing's index space. */
  const tail = held.sourceCount;

  // Rows added to a listing that was never patched: the grown listing *is* the
  // answer, with no map in front of it, and that is what keeps it foldable.
  if (held.map === null && slots.size === 0) return source;

  let dropped = 0;
  for (const slot of slots.values()) {
    if (slot === null) dropped += 1;
  }

  const map = new Int32Array(view.count - dropped + created.length);
  let cursor = 0;
  for (let index = 0; index < view.count; index += 1) {
    const slot = slots.get(index);
    if (slot !== undefined) {
      if (slot !== null) map[cursor++] = tail + slot;
      continue;
    }
    map[cursor++] = held.map === null ? index : held.map[index];
  }
  for (const position of created) {
    map[cursor++] = tail + position;
  }

  const patched = new MappedListingView(source, map);
  patchProvenance.set(patched, {
    head: held.head,
    map,
    packets,
    sourceCount: tail + fresh.length,
  });
  return patched;
}

/**
 * The display index of each name in `names`, for a patch to address its rows.
 *
 * One scan for the whole set: a patch's names are a handful and a listing is
 * routinely tens of thousands of rows, so a pass per name would be a pass per
 * name. The scan reads `nameAt`, which decodes each row's name from the packet
 * blob — measured alongside `isSameListing`, a whole 35k scan of that shape is
 * a couple of milliseconds against the ~123 ms re-sort it replaces.
 */
export function indicesOfNames(
  view: ListingView,
  names: ReadonlySet<string>,
): Map<string, number> {
  const found = new Map<string, number>();
  if (names.size === 0) return found;

  for (let index = 0; index < view.count; index += 1) {
    const name = view.nameAt(index);
    if (name === undefined || !names.has(name) || found.has(name)) continue;

    found.set(name, index);
    if (found.size === names.size) break;
  }

  return found;
}

/**
 * Whether the row at `index` already says exactly what `entry` says.
 *
 * The patch asks this before it rewrites a row, because a watcher fires for
 * changes a listing does not show (a rename onto itself, a write that left the
 * size and time alone) and rewriting such a row would reorder the whole
 * directory to paint what the user can already see. It is the cheap stand-in for
 * `isSameListing`, which can only answer after a full re-read.
 */
export function listingRowMatches(
  view: ListingView,
  index: number,
  entry: DirectoryEntry,
): boolean {
  return (
    view.nameAt(index) === entry.name &&
    view.pathAt(index) === entry.path &&
    view.kindAt(index) === entry.kind &&
    view.sizeAt(index) === (entry.size ?? undefined) &&
    view.modifiedAt(index) === (entry.modifiedAt ?? undefined) &&
    view.hiddenAt(index) === entry.hidden &&
    view.readOnlyAt(index) === entry.readOnly
  );
}

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
 * Whether this view's rows come from the channel's packets rather than an array,
 * counting the views filtered off one.
 *
 * It matters to the ordering hook, which paints the leading rows of a snapshot
 * the worker has not answered for yet. That is only safe where the worker's
 * reply is certain to be adopted later, and a reply is adopted only when
 * `sharedRowCount` recognises the new snapshot as the old one grown. For a
 * byte-backed view that always holds — same `head`, one more packet — and a
 * filter preserves exactly that relation while narrowing the rows, so a chain
 * of them still sits on a listing the worker can fold a batch into. An
 * array-backed view carries no such guarantee at any depth: search results
 * arrive as a fresh array of fresh objects per response (`setResponse` in
 * `directory-search`), so the relation is `null` and a prefix painted for one
 * would stay on screen for good. The hook therefore defers only for these.
 *
 * The hook hands this the view it orders — a filter chain over a listing, never
 * an ordering. A permutation composed by `sortedListingView` is not an ascending
 * map, so `sharedRowCount` would not recognise its source as grown and a prefix
 * painted for one would never be replaced; nothing in the ordering path passes
 * one here, and this deliberately makes no attempt to detect that case.
 */
export function packetBacked(view: ListingView): boolean {
  if (view instanceof PacketStreamView) return true;

  return view instanceof MappedListingView && packetBacked(view.source);
}

/**
 * The first `count` rows of `view`, or `view` itself when it holds fewer.
 *
 * The ordering hook's fallthrough paints a snapshot it cannot afford to order
 * in a frame as its leading rows; this is what bounds that. Reads delegate to
 * `view`, so a painted row keeps its object identity and nothing is copied.
 */
export function prefixListingView(view: ListingView, count: number): ListingView {
  if (count >= view.count) {
    return view;
  }

  const indices = new Int32Array(count);
  for (let index = 0; index < count; index += 1) {
    indices[index] = index;
  }

  return new MappedListingView(view, indices);
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
export function entriesWhere(view: ListingView, keep: (path: string) => boolean): DirectoryEntry[] {
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
