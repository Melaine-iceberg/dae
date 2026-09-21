import type { EntryKind } from "./types";
import type { DirectoryEntry } from "./types";

/**
 * Reader for the columnar listing packets `src-tauri/src/file_system/entry_codec.rs`
 * packs.
 *
 * The point of the format is that the frontend never has to materialise the
 * whole listing. A 35,803 entry directory arrives as one flat buffer, and the
 * virtualised list asks for the ~40 rows it is about to paint — everything else
 * stays as bytes. That is where the win is: measured in the harness next to
 * this file, turning 40 rows into objects costs ~0.01 ms, while parsing the same
 * listing as JSON cost ~25 ms of main-thread time whatever the renderer did with
 * it afterwards.
 *
 * The layout is duplicated from the Rust side on purpose. Both are validated
 * against the same fixture file by `scripts/check-entry-codec-parity.ts`, which
 * is what keeps the duplication from drifting — a mismatch there fails loudly
 * instead of rendering wrong names.
 */

/** Marks a buffer as a packed listing. Reads as "dae, layout 1". */
export const MAGIC = [0x44, 0x41, 0x45, 0x31] as const;

/** Rejected rather than misread, so a layout change is a coordinated one. */
export const VERSION = 1;

/** magic, version, reserved, count, names length, paths length, padding. */
export const HEADER_LEN = 32;

/** kind, flags, modified, size, name offset, path offset. */
export const FIXED_BYTES_PER_ENTRY = 26;

/** The two offset tables carry one entry past the last one. */
export const TRAILING_OFFSETS_LEN = 8;

/** Where the header flags sit: a `u16` after the magic and the version. */
export const HEADER_FLAGS_OFFSET = 6;

/**
 * The packet closes the stream it belongs to.
 *
 * The flag lives in the header rather than in a message of its own because a
 * Tauri channel delivers small payloads through `eval` and large ones through
 * `fetch` — a JSON marker would arrive as an object in one case and as bytes in
 * the other, leaving the reader to guess which it got. Raw packets always
 * arrive as an `ArrayBuffer`, so the end of the stream has to be something a
 * packet can say.
 */
export const HEADER_FLAG_FINAL = 1 << 0;

const FLAG_HIDDEN = 1 << 0;
const FLAG_READ_ONLY = 1 << 1;
const FLAG_HAS_MODIFIED_AT = 1 << 2;
const FLAG_HAS_SIZE = 1 << 3;

/**
 * The `u8` the Rust side writes, indexed by value. The order is the contract:
 * `KIND_*` in `entry_codec.rs` is the authority, and the parity check fails
 * loudly if a kind is ever inserted rather than appended.
 */
const KINDS: readonly EntryKind[] = ["directory", "file", "symlink", "other"];

/** Thrown for a buffer the Rust packer could not have produced. */
export class ListingPacketError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ListingPacketError";
  }
}

/**
 * One decoder for the whole packet: `TextDecoder` holds the ICU converter, and
 * building one per row showed up in the harness.
 */
const decoder = new TextDecoder();

/** Where every section of a packet starts, mirroring the Rust `Layout`. */
interface Layout {
  kinds: number;
  flags: number;
  modified: number;
  sizes: number;
  nameOffsets: number;
  pathOffsets: number;
  names: number;
  paths: number;
  byteLen: number;
}

function layoutFor(count: number, namesLen: number, pathsLen: number): Layout | null {
  const perEntry = FIXED_BYTES_PER_ENTRY;
  if (!Number.isSafeInteger(count) || count < 0) return null;
  if (!Number.isSafeInteger(namesLen) || namesLen < 0) return null;
  if (!Number.isSafeInteger(pathsLen) || pathsLen < 0) return null;

  const kinds = HEADER_LEN;
  const flags = kinds + count;
  const modified = flags + count;
  const sizes = modified + count * 8;
  const nameOffsets = sizes + count * 8;
  const pathOffsets = nameOffsets + (count + 1) * 4;
  const names = pathOffsets + (count + 1) * 4;
  const paths = names + namesLen;
  const byteLen = paths + pathsLen;

  if (byteLen > Number.MAX_SAFE_INTEGER) return null;

  // Cross-check against the published per-entry constant: if the arithmetic
  // above and that constant ever disagree, the constant is lying.
  const expected = HEADER_LEN + TRAILING_OFFSETS_LEN + perEntry * count + namesLen + pathsLen;
  if (byteLen !== expected) return null;

  return { byteLen, flags, kinds, modified, nameOffsets, names, pathOffsets, paths, sizes };
}

/**
 * A packed listing, read on demand.
 *
 * Reads are O(1) and allocation-light: `name`/`path` decode just that entry's
 * slice, `entry` builds one object. No accessor throws for an index past the
 * end — they return `undefined`, which is what the render path wants.
 */
export class ListingPacket {
  readonly count: number;
  /** Bytes a complete packet occupies, so a stream reader knows where the next starts. */
  readonly byteLen: number;

  private readonly view: DataView;
  private readonly bytes: Uint8Array;
  private readonly layout: Layout;

  private constructor(bytes: Uint8Array, layout: Layout, count: number) {
    this.bytes = bytes;
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    this.layout = layout;
    this.count = count;
    this.byteLen = layout.byteLen;
  }

  /**
   * Validates the header, the declared lengths, and both offset tables — the
   * same invariants the Rust reader enforces. Anything the accessors rely on is
   * checked here, so they can stay branch-free.
   */
  static parse(source: ArrayBuffer | ArrayBufferView): ListingPacket {
    const bytes =
      source instanceof ArrayBuffer
        ? new Uint8Array(source)
        : new Uint8Array(source.buffer, source.byteOffset, source.byteLength);

    if (bytes.byteLength < HEADER_LEN) {
      throw new ListingPacketError(
        `the listing packet is truncated: ${bytes.byteLength} bytes, ${HEADER_LEN} expected`,
      );
    }

    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (
      bytes[0] !== MAGIC[0] ||
      bytes[1] !== MAGIC[1] ||
      bytes[2] !== MAGIC[2] ||
      bytes[3] !== MAGIC[3]
    ) {
      throw new ListingPacketError("the buffer is not a listing packet");
    }

    const version = view.getUint16(4, true);
    if (version !== VERSION) {
      throw new ListingPacketError(`listing packet version ${version} is not supported`);
    }

    const count = view.getUint32(8, true);
    const namesLen = view.getUint32(12, true);
    const pathsLen = view.getUint32(16, true);
    const layout = layoutFor(count, namesLen, pathsLen);
    if (!layout) {
      throw new ListingPacketError("the listing packet lengths overflow");
    }

    if (bytes.byteLength < layout.byteLen) {
      throw new ListingPacketError(
        `the listing packet is truncated: ${bytes.byteLength} bytes, ${layout.byteLen} expected`,
      );
    }

    const packet = new ListingPacket(bytes, layout, count);
    packet.validateOffsets(packet.nameOffsets(), namesLen);
    packet.validateOffsets(packet.pathOffsets(), pathsLen);

    return packet;
  }

  /** Decodes just this entry's name. */
  name(index: number): string | undefined {
    return this.stringAt(index, true);
  }

  /**
   * The header flags the packer set. Not validated: an unknown bit is a feature
   * this build has not been taught, not a corrupt packet.
   */
  headerFlags(): number {
    return this.view.getUint16(HEADER_FLAGS_OFFSET, true);
  }

  /**
   * Whether this packet closes its stream. Always false for a packet that was
   * packed on its own — a lone packet is complete by construction, which is why
   * `pack` leaves the flags clear.
   */
  isFinal(): boolean {
    return (this.headerFlags() & HEADER_FLAG_FINAL) !== 0;
  }

  /** Decodes just this entry's path. */
  path(index: number): string | undefined {
    return this.stringAt(index, false);
  }

  kind(index: number): EntryKind | undefined {
    if (!this.inRange(index)) return undefined;
    // A code past the end cannot come from the Rust packer; `undefined` keeps a
    // hand-made buffer from claiming a kind that does not exist.
    const code = this.bytes[this.layout.kinds + index];
    return code < KINDS.length ? KINDS[code] : undefined;
  }

  /** `undefined` means absent, not `0` — the flags carry that distinction. */
  modifiedAt(index: number): number | undefined {
    const flags = this.flagsAt(index);
    if (flags === undefined || (flags & FLAG_HAS_MODIFIED_AT) === 0) return undefined;
    return Number(this.view.getBigUint64(this.layout.modified + index * 8, true));
  }

  /** `undefined` means absent, not `0`. */
  size(index: number): number | undefined {
    const flags = this.flagsAt(index);
    if (flags === undefined || (flags & FLAG_HAS_SIZE) === 0) return undefined;
    return Number(this.view.getBigUint64(this.layout.sizes + index * 8, true));
  }

  isHidden(index: number): boolean {
    const flags = this.flagsAt(index);
    return flags !== undefined && (flags & FLAG_HIDDEN) !== 0;
  }

  isReadOnly(index: number): boolean {
    const flags = this.flagsAt(index);
    return flags !== undefined && (flags & FLAG_READ_ONLY) !== 0;
  }

  /**
   * Materialises one entry. This is the expensive accessor, and the whole point
   * of the format is that the render path calls it for visible rows only.
   *
   * Absent optionals come out as `null` rather than `undefined`, because the
   * generated `DirectoryEntry` types them that way — the object has to be
   * indistinguishable from a JSON-decoded one for this to be a drop-in
   * replacement for the current path.
   */
  entry(index: number): DirectoryEntry | undefined {
    if (!this.inRange(index)) return undefined;

    const name = this.stringAt(index, true);
    const path = this.stringAt(index, false);
    const kind = this.kind(index);
    if (name === undefined || path === undefined || kind === undefined) return undefined;

    return {
      hidden: this.isHidden(index),
      kind,
      modifiedAt: this.modifiedAt(index) ?? null,
      name,
      path,
      readOnly: this.isReadOnly(index),
      size: this.size(index) ?? null,
    };
  }

  /** Borrows the name bytes without decoding them, for a sort that stays in bytes. */
  nameBytes(index: number): Uint8Array | undefined {
    const { start, end } = this.slice(index, true) ?? {};
    if (start === undefined || end === undefined) return undefined;
    return this.bytes.subarray(this.layout.names + start, this.layout.names + end);
  }

  private inRange(index: number): boolean {
    return Number.isInteger(index) && index >= 0 && index < this.count;
  }

  private stringAt(index: number, isName: boolean): string | undefined {
    const { start, end } = this.slice(index, isName) ?? {};
    if (start === undefined || end === undefined) return undefined;

    const blobStart = isName ? this.layout.names : this.layout.paths;
    return decoder.decode(this.bytes.subarray(blobStart + start, blobStart + end));
  }

  private flagsAt(index: number): number | undefined {
    return this.inRange(index) ? this.bytes[this.layout.flags + index] : undefined;
  }

  private slice(index: number, isName: boolean): { start: number; end: number } | undefined {
    if (!this.inRange(index)) return undefined;

    const offsets = isName ? this.layout.nameOffsets : this.layout.pathOffsets;
    const blobLen = (isName ? this.layout.paths : this.layout.byteLen) -
      (isName ? this.layout.names : this.layout.paths);
    const start = this.view.getUint32(offsets + index * 4, true);
    const end = this.view.getUint32(offsets + (index + 1) * 4, true);
    if (start > end || end > blobLen) return undefined;

    return { end, start };
  }

  private nameOffsets(): Uint8Array {
    return this.bytes.subarray(this.layout.nameOffsets, this.layout.names);
  }

  private pathOffsets(): Uint8Array {
    return this.bytes.subarray(this.layout.pathOffsets, this.layout.paths);
  }

  private validateOffsets(offsets: Uint8Array, blobLen: number): void {
    const view = new DataView(offsets.buffer, offsets.byteOffset, offsets.byteLength);

    if (this.count > 0 && view.getUint32(0, true) !== 0) {
      throw new ListingPacketError("the listing packet has corrupt offsets");
    }

    let previous = 0;
    for (let index = 0; index <= this.count; index++) {
      const offset = view.getUint32(index * 4, true);
      if (offset < previous || offset > blobLen) {
        throw new ListingPacketError("the listing packet has corrupt offsets");
      }
      previous = offset;
    }
  }
}
