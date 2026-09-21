//! Columnar packing for streamed directory listings.
//!
//! A listing reaches the frontend as one JSON object per entry, and the largest
//! local directory measured here (35,803 entries) then costs ~5 MB the webview
//! must parse before a single row paints. Almost all of that is redundancy: two
//! strings per entry whose bulk is a repeated parent path, and small integers
//! written as decimal text.
//!
//! This module packs the same listing into one flat buffer instead — fixed-width
//! columns for the numeric and boolean fields, the strings as UTF-8 blobs behind
//! a shared offset table. Nothing is reinterpreted: the same values come out.
//! See `src/features/explorer/entry-codec.ts` for the reader, and
//! `scripts/check-entry-codec-parity.ts` for the cross-language check that
//! keeps the two honest.
//!
//! Packing and parsing share [`Layout`], so they cannot drift on where a section
//! begins; the accessors all derive from the same table. Truncation and corrupt
//! offsets are rejected in [`Packet::parse`] rather than left to panic in an
//! accessor, because the buffer crosses a process boundary.
//!
//! Deliberate non-goals: this is not a general serialization format. It carries
//! exactly `DirectoryEntry`, has no schema evolution beyond [`VERSION`], and is
//! not a stable on-disk format — a version bump is expected to be a coordinated
//! change on both sides.

// Only the writer half of this module runs in the shipped binary: Rust packs a
// listing with `pack_with_header_flags`, and the frontend decodes it, so the
// reader below has no non-test caller by design rather than by oversight. It
// still earns its place — the round-trip and rejection tests in `mod tests`
// pin the format down, and this file stays the authoritative definition that
// `entry-codec.ts` mirrors.
//
// Scoped to `not(test)` deliberately: a test build keeps reporting genuinely
// unused items, so real dead code in the writer half cannot hide behind this.
#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;

use super::types::{DirectoryEntry, EntryKind};

/// Marks a buffer as a packed listing. Reads as "dae, layout 1".
pub const MAGIC: [u8; 4] = *b"DAE1";

/// Bumped whenever a section is added, removed, or reordered. The reader
/// rejects anything it does not know rather than mis-reading it.
pub const VERSION: u16 = 1;

/// Fixed header: magic, version, header flags, then the three lengths.
pub const HEADER_LEN: usize = 32;

/// Where the header flags sit: a `u16` after the magic and the version. It was
/// reserved-and-zero until the batched stream needed to mark its last packet.
pub const HEADER_FLAGS_OFFSET: usize = 6;

/// The packet closes the stream it belongs to. Without it a reader cannot tell
/// a final empty batch from a batch that has not arrived yet.
pub const HEADER_FLAG_FINAL: u16 = 1 << 0;

/// Per-entry bytes outside the string blobs: kind, flags, modified, size, and
/// one offset each for the name and the path.
pub const FIXED_BYTES_PER_ENTRY: usize = 1 + 1 + 8 + 8 + 4 + 4;

/// Both offset tables carry one entry past the last one — that trailing offset
/// is what makes a string `i` the slice `[offsets[i], offsets[i + 1])`. So a
/// packet costs `HEADER_LEN + TRAILING_OFFSETS_LEN + FIXED_BYTES_PER_ENTRY *
/// count` plus the two blobs.
pub const TRAILING_OFFSETS_LEN: usize = 4 + 4;

pub const FLAG_HIDDEN: u8 = 1 << 0;
pub const FLAG_READ_ONLY: u8 = 1 << 1;
pub const FLAG_HAS_MODIFIED_AT: u8 = 1 << 2;
pub const FLAG_HAS_SIZE: u8 = 1 << 3;

const KIND_DIRECTORY: u8 = 0;
const KIND_FILE: u8 = 1;
const KIND_SYMLINK: u8 = 2;
const KIND_OTHER: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// The buffer is shorter than the layout its own header describes.
    Truncated { expected: usize, actual: usize },
    /// The buffer does not begin with [`MAGIC`].
    NotAPacket,
    /// The header declares a [`VERSION`] this build does not read.
    UnsupportedVersion(u16),
    /// The declared lengths overflow the address space, so no buffer could hold
    /// the layout they describe. Rejected before any arithmetic wraps.
    TooLarge,
    /// An offset is not non-decreasing, or it leaves the blob it indexes.
    CorruptOffsets,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected, actual } => write!(
                formatter,
                "the listing packet is truncated: {actual} bytes, {expected} expected"
            ),
            Self::NotAPacket => write!(formatter, "the buffer is not a listing packet"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "listing packet version {version} is not supported"
                )
            }
            Self::TooLarge => write!(formatter, "the listing packet lengths overflow"),
            Self::CorruptOffsets => write!(formatter, "the listing packet has corrupt offsets"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Where every section of a packet starts, derived from the three lengths in
/// the header. Shared by [`pack`] and [`Packet`] so the writer and the reader
/// cannot disagree about the layout.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub kinds: usize,
    pub flags: usize,
    pub modified: usize,
    pub sizes: usize,
    pub name_offsets: usize,
    pub path_offsets: usize,
    pub names: usize,
    pub paths: usize,
    /// Total bytes of a complete packet, including the header.
    pub byte_len: usize,
}

impl Layout {
    /// `None` when the three lengths describe a layout that cannot fit in
    /// memory, which is what a corrupt or hostile header looks like.
    pub fn new(count: usize, names_len: usize, paths_len: usize) -> Option<Self> {
        let eight_per_entry = count.checked_mul(8)?;
        let four_per_entry = count.checked_add(1)?.checked_mul(4)?;

        let kinds = HEADER_LEN;
        let flags = kinds.checked_add(count)?;
        let modified = flags.checked_add(count)?;
        let sizes = modified.checked_add(eight_per_entry)?;
        let name_offsets = sizes.checked_add(eight_per_entry)?;
        let path_offsets = name_offsets.checked_add(four_per_entry)?;
        let names = path_offsets.checked_add(four_per_entry)?;
        let paths = names.checked_add(names_len)?;

        Some(Self {
            kinds,
            flags,
            modified,
            sizes,
            name_offsets,
            path_offsets,
            names,
            paths,
            byte_len: paths.checked_add(paths_len)?,
        })
    }
}

/// Packs a listing into one buffer with no header flags set. The batch shape is
/// the caller's business: the streamed listings pack one batch at a time, so
/// this is called per batch.
pub fn pack(entries: &[DirectoryEntry]) -> Vec<u8> {
    pack_with_header_flags(entries, 0)
}

/// Packs a listing into one buffer, carrying `header_flags` in the header.
///
/// The flags describe the packet rather than its contents, which is why they
/// live in the header alongside the lengths: a reader streaming packets out of
/// one connection has to know where the stream ends, and the entries themselves
/// cannot say.
pub fn pack_with_header_flags(entries: &[DirectoryEntry], header_flags: u16) -> Vec<u8> {
    let mut names = Vec::new();
    let mut paths = Vec::new();
    for entry in entries {
        names.extend_from_slice(entry.name.as_bytes());
        paths.extend_from_slice(entry.path.as_bytes());
    }

    let count = entries.len();
    let layout = Layout::new(count, names.len(), paths.len())
        .expect("a listing that fits in memory has a layout");

    let mut buffer = vec![0u8; layout.byte_len];
    buffer[0..4].copy_from_slice(&MAGIC);
    buffer[4..6].copy_from_slice(&VERSION.to_le_bytes());
    buffer[HEADER_FLAGS_OFFSET..HEADER_FLAGS_OFFSET + 2]
        .copy_from_slice(&header_flags.to_le_bytes());
    // 20..32 is reserved padding: left zeroed so a future field can be added
    // without moving the lengths.
    buffer[8..12].copy_from_slice(&(count as u32).to_le_bytes());
    buffer[12..16].copy_from_slice(&(names.len() as u32).to_le_bytes());
    buffer[16..20].copy_from_slice(&(paths.len() as u32).to_le_bytes());

    let mut name_offset = 0u32;
    let mut path_offset = 0u32;

    for (index, entry) in entries.iter().enumerate() {
        buffer[layout.kinds + index] = kind_code(entry.kind);

        let mut flags = 0;
        if entry.hidden {
            flags |= FLAG_HIDDEN;
        }
        if entry.read_only {
            flags |= FLAG_READ_ONLY;
        }
        if entry.modified_at.is_some() {
            flags |= FLAG_HAS_MODIFIED_AT;
        }
        if entry.size.is_some() {
            flags |= FLAG_HAS_SIZE;
        }
        buffer[layout.flags + index] = flags;

        // Absent values are written as zero and read back as `None` through the
        // flags, so a `None` never becomes a visible timestamp of 0.
        write_u64(
            &mut buffer,
            layout.modified + index * 8,
            entry.modified_at.unwrap_or(0),
        );
        write_u64(
            &mut buffer,
            layout.sizes + index * 8,
            entry.size.unwrap_or(0),
        );

        write_u32(&mut buffer, layout.name_offsets + index * 4, name_offset);
        write_u32(&mut buffer, layout.path_offsets + index * 4, path_offset);

        name_offset += entry.name.len() as u32;
        path_offset += entry.path.len() as u32;
    }

    // The trailing offsets point one past the last entry, which is what makes
    // slice `i` = `[offsets[i], offsets[i + 1])` work without a special case.
    write_u32(&mut buffer, layout.name_offsets + count * 4, name_offset);
    write_u32(&mut buffer, layout.path_offsets + count * 4, path_offset);

    buffer[layout.names..layout.names + names.len()].copy_from_slice(&names);
    buffer[layout.paths..layout.paths + paths.len()].copy_from_slice(&paths);

    buffer
}

fn kind_code(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => KIND_DIRECTORY,
        EntryKind::File => KIND_FILE,
        EntryKind::Symlink => KIND_SYMLINK,
        EntryKind::Other => KIND_OTHER,
    }
}

fn kind_from_code(code: u8) -> Option<EntryKind> {
    match code {
        KIND_DIRECTORY => Some(EntryKind::Directory),
        KIND_FILE => Some(EntryKind::File),
        KIND_SYMLINK => Some(EntryKind::Symlink),
        KIND_OTHER => Some(EntryKind::Other),
        _ => None,
    }
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(raw)
}

fn write_u32(buffer: &mut [u8], at: usize, value: u32) {
    buffer[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(buffer: &mut [u8], at: usize, value: u64) {
    buffer[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// A borrowed reader over a packed listing.
///
/// Every accessor is bounds-checked against the index and the validated layout,
/// so a malformed buffer can produce a wrong value only if [`Layout`] itself is
/// wrong — which is why parse validates the offset tables up front.
#[derive(Debug, Clone, Copy)]
pub struct Packet<'a> {
    bytes: &'a [u8],
    layout: Layout,
    count: usize,
}

impl<'a> Packet<'a> {
    /// Validates the header, the declared lengths, and both offset tables.
    /// Everything the accessors rely on is checked here.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, CodecError> {
        if bytes.len() < HEADER_LEN {
            return Err(CodecError::Truncated {
                expected: HEADER_LEN,
                actual: bytes.len(),
            });
        }

        if bytes[0..4] != MAGIC {
            return Err(CodecError::NotAPacket);
        }

        let version = read_u16(bytes, 4);
        if version != VERSION {
            return Err(CodecError::UnsupportedVersion(version));
        }

        let count = read_u32(bytes, 8) as usize;
        let names_len = read_u32(bytes, 12) as usize;
        let paths_len = read_u32(bytes, 16) as usize;
        let layout = Layout::new(count, names_len, paths_len).ok_or(CodecError::TooLarge)?;

        if bytes.len() < layout.byte_len {
            return Err(CodecError::Truncated {
                expected: layout.byte_len,
                actual: bytes.len(),
            });
        }

        let packet = Self {
            bytes,
            layout,
            count,
        };
        packet.validate_offsets(packet.name_offsets(), names_len)?;
        packet.validate_offsets(packet.path_offsets(), paths_len)?;

        Ok(packet)
    }

    /// Bytes a complete packet occupies, so a caller reading a concatenated
    /// stream knows where the next one starts.
    pub fn byte_len(&self) -> usize {
        self.layout.byte_len
    }

    /// The flags [`pack_with_header_flags`] was called with. Not validated: an
    /// unknown bit is a future feature, not a corrupt packet.
    pub fn header_flags(&self) -> u16 {
        read_u16(self.bytes, HEADER_FLAGS_OFFSET)
    }

    /// Whether this packet closes its stream. Meaningless for a packet that
    /// travels on its own — `pack` leaves the flags clear, and a lone packet is
    /// complete by construction.
    pub fn is_final(&self) -> bool {
        self.header_flags() & HEADER_FLAG_FINAL != 0
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn name(&self, index: usize) -> Option<&'a str> {
        self.string_at(index, true)
    }

    pub fn path(&self, index: usize) -> Option<&'a str> {
        self.string_at(index, false)
    }

    pub fn kind(&self, index: usize) -> Option<EntryKind> {
        (index < self.count).then(|| kind_from_code(self.bytes[self.layout.kinds + index]))?
    }

    fn flags(&self, index: usize) -> Option<u8> {
        (index < self.count).then(|| self.bytes[self.layout.flags + index])
    }

    /// Absent timestamps and sizes stay absent: the flags say whether the zeros
    /// in the column mean anything.
    pub fn modified_at(&self, index: usize) -> Option<u64> {
        let flags = self.flags(index)?;
        (flags & FLAG_HAS_MODIFIED_AT != 0)
            .then(|| read_u64(self.bytes, self.layout.modified + index * 8))
    }

    pub fn size(&self, index: usize) -> Option<u64> {
        let flags = self.flags(index)?;
        (flags & FLAG_HAS_SIZE != 0).then(|| read_u64(self.bytes, self.layout.sizes + index * 8))
    }

    pub fn is_hidden(&self, index: usize) -> bool {
        self.flags(index)
            .is_some_and(|flags| flags & FLAG_HIDDEN != 0)
    }

    pub fn is_read_only(&self, index: usize) -> bool {
        self.flags(index)
            .is_some_and(|flags| flags & FLAG_READ_ONLY != 0)
    }

    /// Materialises one entry. This is the expensive accessor — the whole point
    /// of the format is that the frontend calls it for the visible rows only.
    pub fn entry(&self, index: usize) -> Option<DirectoryEntry> {
        if index >= self.count {
            return None;
        }

        Some(DirectoryEntry {
            name: self.name(index)?.to_owned(),
            path: self.path(index)?.to_owned(),
            kind: self.kind(index)?,
            modified_at: self.modified_at(index),
            size: self.size(index),
            hidden: self.is_hidden(index),
            read_only: self.is_read_only(index),
        })
    }

    /// Borrows every entry. For parity checks and for the cases that genuinely
    /// need the whole list on one side — not for the render path.
    pub fn entries(&self) -> Vec<DirectoryEntry> {
        (0..self.count)
            .filter_map(|index| self.entry(index))
            .collect()
    }

    /// The name offset table alone — `[name_offsets, path_offsets)` — not
    /// everything up to the name blob, which would also swallow the path table
    /// and make every offset check read the wrong word.
    fn name_offsets(&self) -> &'a [u8] {
        &self.bytes[self.layout.name_offsets..self.layout.path_offsets]
    }

    fn path_offsets(&self) -> &'a [u8] {
        &self.bytes[self.layout.path_offsets..self.layout.names]
    }

    fn string_at(&self, index: usize, is_name: bool) -> Option<&'a str> {
        if index >= self.count {
            return None;
        }

        let (offsets, blob_start, blob_len) = if is_name {
            (
                self.name_offsets(),
                self.layout.names,
                self.layout.paths - self.layout.names,
            )
        } else {
            (
                self.path_offsets(),
                self.layout.paths,
                self.layout.byte_len - self.layout.paths,
            )
        };

        let start = read_u32(offsets, index * 4) as usize;
        let end = read_u32(offsets, (index + 1) * 4) as usize;
        if start > end || end > blob_len {
            return None;
        }

        // Names come from Rust `String`s, so this is a UTF-8 guarantee rather
        // than a check; returning `None` beats panicking if a buffer is hand-made.
        std::str::from_utf8(&self.bytes[blob_start + start..blob_start + end]).ok()
    }

    /// Non-decreasing, starting at zero, and never leaving the blob. Anything
    /// else would turn a slice in [`Self::string_at`] into a panic.
    fn validate_offsets(&self, offsets: &[u8], blob_len: usize) -> Result<(), CodecError> {
        debug_assert_eq!(offsets.len(), (self.count + 1) * 4);

        if self.count > 0 && read_u32(offsets, 0) != 0 {
            return Err(CodecError::CorruptOffsets);
        }

        let mut previous = 0;
        for index in 0..=self.count {
            let offset = read_u32(offsets, index * 4) as usize;
            if offset < previous || offset > blob_len {
                return Err(CodecError::CorruptOffsets);
            }
            previous = offset;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> DirectoryEntry {
        DirectoryEntry {
            name: name.to_owned(),
            path: format!("C:/code/dae/{name}"),
            kind,
            modified_at: Some(1_700_000_000_000),
            size: Some(4_096),
            hidden: false,
            read_only: false,
        }
    }

    /// Field-by-field rather than `PartialEq` on `DirectoryEntry`: the type is
    /// shared with the bindings, and a derive there would be there for the sake
    /// of a test.
    fn assert_same(actual: &DirectoryEntry, expected: &DirectoryEntry) {
        assert_eq!(actual.name, expected.name, "name");
        assert_eq!(actual.path, expected.path, "path");
        assert_eq!(actual.kind, expected.kind, "kind");
        assert_eq!(actual.modified_at, expected.modified_at, "modified_at");
        assert_eq!(actual.size, expected.size, "size");
        assert_eq!(actual.hidden, expected.hidden, "hidden");
        assert_eq!(actual.read_only, expected.read_only, "read_only");
    }

    /// A listing that uses every branch of the format: all four kinds, both
    /// optional fields present and absent, both flags, and names that are
    /// prefixes of one another so a wrong offset table shows up as the wrong
    /// string rather than as an out-of-range read.
    fn mixed_listing() -> Vec<DirectoryEntry> {
        vec![
            entry("src", EntryKind::Directory),
            entry("srcRust", EntryKind::Directory),
            DirectoryEntry {
                name: "项目文档".to_owned(),
                path: "C:/code/dae/项目文档".to_owned(),
                kind: EntryKind::File,
                modified_at: None,
                size: None,
                hidden: true,
                read_only: false,
            },
            DirectoryEntry {
                name: "🦀-emoji".to_owned(),
                path: "C:/code/dae/🦀-emoji".to_owned(),
                kind: EntryKind::Symlink,
                modified_at: Some(0),
                size: Some(0),
                hidden: false,
                read_only: true,
            },
            DirectoryEntry {
                name: "e\u{301}combining".to_owned(),
                path: "C:/code/dae/e\u{301}combining".to_owned(),
                kind: EntryKind::Other,
                modified_at: None,
                size: Some(u64::MAX),
                hidden: true,
                read_only: true,
            },
            entry("", EntryKind::File),
            entry("a\tb\nc", EntryKind::File),
        ]
    }

    #[test]
    fn the_layout_matches_the_published_constants() {
        let empty = Layout::new(0, 0, 0).expect("layout");
        assert_eq!(
            empty.byte_len,
            HEADER_LEN + TRAILING_OFFSETS_LEN,
            "a listing with no entries is the header plus the two trailing offsets"
        );

        let three = Layout::new(3, 0, 0).expect("layout");
        assert_eq!(
            three.byte_len,
            HEADER_LEN + TRAILING_OFFSETS_LEN + FIXED_BYTES_PER_ENTRY * 3,
            "an empty-string listing costs exactly the per-entry constant"
        );
    }

    #[test]
    fn round_trips_every_field() {
        let entries = mixed_listing();
        let buffer = pack(&entries);
        let packet = Packet::parse(&buffer).expect("parse");

        assert_eq!(packet.count(), entries.len());
        assert_eq!(packet.byte_len(), buffer.len());

        for (index, expected) in entries.iter().enumerate() {
            let actual = packet.entry(index).expect("entry");
            assert_same(&actual, expected);
        }

        assert!(packet.entry(entries.len()).is_none(), "index past the end");
    }

    #[test]
    fn absent_fields_stay_absent() {
        let entry = DirectoryEntry {
            name: "no-metadata".to_owned(),
            path: "C:/code/dae/no-metadata".to_owned(),
            kind: EntryKind::File,
            modified_at: None,
            size: None,
            hidden: false,
            read_only: false,
        };

        let buffer = pack(std::slice::from_ref(&entry));
        let packet = Packet::parse(&buffer).expect("parse");
        let read = packet.entry(0).expect("entry");

        assert_eq!(
            read.modified_at, None,
            "a missing timestamp must not read as 0"
        );
        assert_eq!(read.size, None, "a missing size must not read as 0");
    }

    #[test]
    fn zeros_survive_as_values_not_absences() {
        let entry = DirectoryEntry {
            name: "zero".to_owned(),
            path: "zero".to_owned(),
            kind: EntryKind::File,
            modified_at: Some(0),
            size: Some(0),
            hidden: false,
            read_only: false,
        };

        let buffer = pack(std::slice::from_ref(&entry));
        let packet = Packet::parse(&buffer).expect("parse");
        let read = packet.entry(0).expect("entry");

        assert_eq!(read.modified_at, Some(0));
        assert_eq!(read.size, Some(0));
    }

    #[test]
    fn empty_listing_round_trips() {
        let buffer = pack(&[]);
        assert_eq!(buffer.len(), HEADER_LEN + TRAILING_OFFSETS_LEN);

        let packet = Packet::parse(&buffer).expect("parse");
        assert_eq!(packet.count(), 0);
        assert!(packet.entry(0).is_none());
        assert!(packet.entries().is_empty());
    }

    #[test]
    fn a_listing_of_identical_names_keeps_them_distinct() {
        let entries = vec![entry("dup", EntryKind::File); 512];
        let buffer = pack(&entries);
        let packet = Packet::parse(&buffer).expect("parse");

        for index in 0..512 {
            assert_eq!(packet.name(index), Some("dup"), "entry {index}");
        }
    }

    #[test]
    fn truncation_is_rejected_at_every_length() {
        let buffer = pack(&mixed_listing());

        for length in 0..buffer.len() {
            let result = Packet::parse(&buffer[..length]);
            assert!(
                result.is_err(),
                "a {length}-byte prefix of a {}-byte packet must not parse",
                buffer.len()
            );
        }

        assert!(Packet::parse(&buffer).is_ok(), "the whole packet parses");
    }

    #[test]
    fn a_longer_buffer_reports_its_own_length() {
        let entries = mixed_listing();
        let mut padded = pack(&entries);
        let packet_len = padded.len();
        padded.extend_from_slice(&[0xff; 8]);

        let packet = Packet::parse(&padded).expect("parse");
        assert_eq!(
            packet.byte_len(),
            packet_len,
            "trailing bytes are not the packet"
        );
        assert_eq!(packet.entry(0).expect("entry").name, "src");
    }

    #[test]
    fn the_header_flags_survive_a_round_trip() {
        let entries = mixed_listing();
        let plain = pack(&entries);
        let marked = pack_with_header_flags(&entries, HEADER_FLAG_FINAL);

        let unmarked = Packet::parse(&plain).expect("parse");
        assert_eq!(unmarked.header_flags(), 0, "`pack` sets nothing");
        assert!(
            !unmarked.is_final(),
            "a packet packed on its own is not marked final: `pack` is used for \
             one-shot transfers as well as for streams"
        );

        let packet = Packet::parse(&marked).expect("parse");
        assert_eq!(packet.header_flags(), HEADER_FLAG_FINAL);
        assert!(packet.is_final());

        // The flags live in reserved space, so setting them must not move a
        // single other byte — that is the whole reason they went there instead
        // of into the layout.
        assert_eq!(plain.len(), marked.len());
        assert_eq!(plain[..HEADER_FLAGS_OFFSET], marked[..HEADER_FLAGS_OFFSET]);
        assert_eq!(
            plain[HEADER_FLAGS_OFFSET + 2..],
            marked[HEADER_FLAGS_OFFSET + 2..]
        );
        assert_eq!(packet.count(), entries.len());
        assert_eq!(
            packet.name(0),
            unmarked.name(0),
            "the reader sees the same first entry either way"
        );
    }

    #[test]
    fn an_unknown_header_flag_is_not_an_error() {
        // Forward compatibility: a flag this build does not know is a feature
        // it has not been taught, not a corrupt packet. Rejecting it would make
        // every future flag a breaking change.
        let entries = mixed_listing();
        let buffer = pack_with_header_flags(&entries, 0x8000);
        let packet = Packet::parse(&buffer).expect("parse");
        assert_eq!(packet.count(), entries.len());
        assert!(!packet.is_final());
    }

    #[test]
    fn concatenated_packets_can_be_walked_one_at_a_time() {
        // What a stream reader does: parse a packet, advance by `byte_len`,
        // repeat. The listing batches arrive back to back, so `byte_len` has to
        // be exactly the packet's own length rather than the buffer's.
        let first = mixed_listing();
        let second = vec![
            entry("tail", EntryKind::File),
            entry("end", EntryKind::Other),
        ];

        let mut stream = pack(&first);
        stream.extend_from_slice(&pack(&second));

        let mut offset = 0;
        let mut seen: Vec<String> = Vec::new();
        while offset < stream.len() {
            let packet = Packet::parse(&stream[offset..]).expect("parse");
            for index in 0..packet.count() {
                seen.push(packet.entry(index).expect("entry").name);
            }
            assert!(packet.byte_len() > 0, "a walk that does not advance loops");
            offset += packet.byte_len();
        }

        let expected: Vec<String> = first
            .iter()
            .chain(second.iter())
            .map(|entry| entry.name.clone())
            .collect();
        assert_eq!(seen, expected);
        assert_eq!(offset, stream.len(), "the walk lands exactly on the end");
    }

    #[test]
    fn wrong_magic_is_rejected() {
        let mut buffer = pack(&mixed_listing());
        buffer[0] = b'X';

        assert_eq!(Packet::parse(&buffer).unwrap_err(), CodecError::NotAPacket);
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let mut buffer = pack(&mixed_listing());
        buffer[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());

        assert_eq!(
            Packet::parse(&buffer).unwrap_err(),
            CodecError::UnsupportedVersion(VERSION + 1),
            "a version bump must be a coordinated change, not a silent misread"
        );
    }

    #[test]
    fn a_count_larger_than_the_buffer_is_rejected() {
        let mut buffer = pack(&mixed_listing());
        buffer[8..12].copy_from_slice(&1_000_000u32.to_le_bytes());

        assert!(
            matches!(Packet::parse(&buffer), Err(CodecError::Truncated { .. })),
            "a header claiming a million entries cannot describe this buffer"
        );
    }

    #[test]
    fn a_full_width_header_is_rejected_rather_than_read() {
        let mut buffer = pack(&mixed_listing());
        buffer[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        buffer[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        buffer[16..20].copy_from_slice(&u32::MAX.to_le_bytes());

        // Which of the two errors comes out depends on the target's pointer
        // width: on 64-bit the declared layout is representable and the buffer
        // is simply not that long, on 32-bit the arithmetic itself overflows.
        // Either way it is refused, and that is the invariant being tested.
        assert!(
            matches!(
                Packet::parse(&buffer),
                Err(CodecError::Truncated { .. } | CodecError::TooLarge)
            ),
            "a header of 0xFFFFFFFF lengths must be refused, not read"
        );
    }

    #[test]
    fn a_layout_that_cannot_fit_is_rejected() {
        assert!(
            Layout::new(usize::MAX / 2, 0, 0).is_none(),
            "a count whose column offsets overflow has no layout"
        );
        assert!(
            Layout::new(0, usize::MAX, 0).is_none(),
            "a name blob larger than the address space has no layout"
        );
        assert!(
            Layout::new(0, 0, usize::MAX).is_none(),
            "a path blob larger than the address space has no layout"
        );
    }

    #[test]
    fn a_backwards_offset_is_rejected() {
        let entries = mixed_listing();
        let buffer = pack(&entries);
        let layout = Layout::new(entries.len(), 0, 0).expect("layout");

        // "src" is three bytes, so offset 1 is 3 and offset 2 has to be at least
        // that. Anything below it means the table is not what the writer built.
        let mut corrupt = buffer.clone();
        write_u32(&mut corrupt, layout.name_offsets + 4, 3);
        write_u32(&mut corrupt, layout.name_offsets + 8, 1);

        assert_eq!(
            Packet::parse(&corrupt).unwrap_err(),
            CodecError::CorruptOffsets
        );
    }

    #[test]
    fn an_offset_past_the_blob_is_rejected() {
        let entries = mixed_listing();
        let buffer = pack(&entries);
        let layout = Layout::new(entries.len(), 0, 0).expect("layout");

        let mut corrupt = buffer.clone();
        write_u32(
            &mut corrupt,
            layout.name_offsets + entries.len() * 4,
            u32::MAX,
        );

        assert_eq!(
            Packet::parse(&corrupt).unwrap_err(),
            CodecError::CorruptOffsets
        );
    }

    #[test]
    fn a_nonzero_starting_offset_is_rejected() {
        let entries = mixed_listing();
        let buffer = pack(&entries);
        let layout = Layout::new(entries.len(), 0, 0).expect("layout");

        let mut corrupt = buffer.clone();
        write_u32(&mut corrupt, layout.path_offsets, 1);

        assert_eq!(
            Packet::parse(&corrupt).unwrap_err(),
            CodecError::CorruptOffsets,
            "every offset table has to start at zero, or an empty first string \
             would read the wrong bytes"
        );
    }

    #[test]
    fn the_packet_is_never_larger_than_the_json_it_replaces() {
        // The claim this format rests on. For a realistic listing the packed
        // packet has to be strictly smaller than the JSON, or the whole change
        // buys nothing.
        let entries: Vec<DirectoryEntry> = (0..2_000)
            .map(|index| entry(&format!("some-module-{index}.tsx"), EntryKind::File))
            .collect();

        let json = serde_json::to_vec(&entries).expect("json");
        let packed = pack(&entries);

        assert!(
            packed.len() < json.len(),
            "packed {} bytes vs json {} bytes",
            packed.len(),
            json.len()
        );
    }

    /// The fixture the TypeScript reader is checked against. Writes
    /// `entries.json` (the existing representation) and `entries.bin` (the
    /// packed one) next to it, so the two readers can be compared field by
    /// field without either language being the source of truth about the other.
    ///
    /// `#[ignore]` because it writes into the workspace:
    /// `cargo test --lib -- --ignored emits_the_parity_fixture`.
    #[test]
    #[ignore = "writes the parity fixture into .workbuddy/scratch"]
    fn emits_the_parity_fixture() {
        let entries = fixture_listing();
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.workbuddy/scratch/entry-codec-fixture");

        std::fs::create_dir_all(&directory).expect("create the fixture directory");

        let json = serde_json::to_vec(&entries).expect("serialise the fixture");
        let packed = pack(&entries);

        std::fs::write(directory.join("entries.json"), &json).expect("write the json fixture");
        std::fs::write(directory.join("entries.bin"), &packed).expect("write the packed fixture");

        // The streamed shape, which is what the app actually receives: the head
        // as the JSON `DirectoryView.entries`, then packets in the sizes
        // `listing.rs` grows them by, the last one marked final. Written here
        // rather than reproduced in TypeScript so the frontend reader is
        // checked against packets this packer produced, framing included.
        const FIRST_BATCH: usize = 512;
        let head = &entries[..FIRST_BATCH.min(entries.len())];
        std::fs::write(
            directory.join("stream-head.json"),
            serde_json::to_vec(head).expect("serialise the stream head"),
        )
        .expect("write the stream head");

        let mut written = FIRST_BATCH.min(entries.len());
        let mut batch_size = FIRST_BATCH;
        let mut batch = 0;
        while written < entries.len() {
            let end = (written + batch_size).min(entries.len());
            let done = end == entries.len();
            let flags = if done { HEADER_FLAG_FINAL } else { 0 };
            std::fs::write(
                directory.join(format!("stream-{batch:02}.bin")),
                pack_with_header_flags(&entries[written..end], flags),
            )
            .expect("write a stream batch");

            written = end;
            batch_size = (batch_size * 2).min(8192);
            batch += 1;
        }
        println!(
            "  stream: head {} entries + {} packets, growing 512 -> 8192",
            head.len(),
            batch
        );

        println!(
            "\n{} entries: json {} bytes, packed {} bytes ({:.1}% of json)",
            entries.len(),
            json.len(),
            packed.len(),
            packed.len() as f64 / json.len() as f64 * 100.0
        );
        println!(
            "  strings: names {} bytes, paths {} bytes",
            entries.iter().map(|entry| entry.name.len()).sum::<usize>(),
            entries.iter().map(|entry| entry.path.len()).sum::<usize>()
        );
        println!("  fixture: {}", directory.display());
    }

    /// A deterministic listing of `count` entries: no RNG, so the fixture file
    /// is reproducible byte for byte and a diff in it means a real change.
    ///
    /// The mix is meant to look like a real source tree — mostly short ASCII
    /// names with numeric suffixes, a slice of CJK ones, all four kinds, and the
    /// odd entry missing its metadata.
    fn sample_entries(count: usize) -> Vec<DirectoryEntry> {
        const STEMS: [&str; 12] = [
            "src", "lib", "assets", "target", "build", "tests", "docs", "README", "Cargo",
            "package", "index", "mod",
        ];
        const EXTENSIONS: [&str; 8] = ["rs", "ts", "tsx", "json", "md", "toml", "png", "svg"];
        const CJK: [&str; 5] = ["项目文档", "测试", "配置", "截图", "设计稿"];

        (0..count)
            .map(|index| {
                let stem = STEMS[index % STEMS.len()];
                let extension = EXTENSIONS[index % EXTENSIONS.len()];
                let name = if index % 17 == 0 {
                    format!("{}-{}.md", CJK[index % CJK.len()], index % 977)
                } else {
                    format!("{stem}-{}-{}.{extension}", index % 9973, index)
                };
                let path = format!("C:/code/dae/src/{name}");
                let kind = match index % 11 {
                    0 => EntryKind::Directory,
                    7 => EntryKind::Symlink,
                    10 => EntryKind::Other,
                    _ => EntryKind::File,
                };

                DirectoryEntry {
                    name,
                    path,
                    kind,
                    modified_at: (index % 13 != 0).then_some(1_700_000_000_000 + index as u64 * 37),
                    size: (index % 11 != 3).then_some(index as u64 * 37),
                    hidden: index % 23 == 0,
                    read_only: index % 31 == 0,
                }
            })
            .collect()
    }

    /// Deterministic listing for the parity fixture: the frontend reader is
    /// checked against the same awkward names the Rust round-trip tests use.
    ///
    /// Sized at the largest directory this work was measured against, so the
    /// parity check and the size comparison both run at the scale the format
    /// exists for rather than at a size that flatters it.
    fn fixture_listing() -> Vec<DirectoryEntry> {
        let mut entries = sample_entries(35_803);

        entries.extend([
            DirectoryEntry {
                name: "项目文档".to_owned(),
                path: "C:/code/dae/项目文档".to_owned(),
                kind: EntryKind::File,
                modified_at: None,
                size: None,
                hidden: true,
                read_only: false,
            },
            DirectoryEntry {
                name: "🦀".to_owned(),
                path: "C:/code/dae/🦀".to_owned(),
                kind: EntryKind::Symlink,
                modified_at: Some(0),
                size: Some(0),
                hidden: false,
                read_only: true,
            },
            DirectoryEntry {
                name: "e\u{301}".to_owned(),
                path: "C:/code/dae/e\u{301}".to_owned(),
                kind: EntryKind::Other,
                modified_at: None,
                size: Some(u64::MAX),
                hidden: true,
                read_only: true,
            },
            DirectoryEntry {
                name: String::new(),
                path: "C:/code/dae".to_owned(),
                kind: EntryKind::Directory,
                modified_at: None,
                size: None,
                hidden: false,
                read_only: false,
            },
        ]);

        entries
    }

    /// Prints what a 35,803 entry listing costs to pack and to read back, so a
    /// change to the layout can be judged against the JSON path it replaces.
    ///
    /// `cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_large_listing`
    #[test]
    #[ignore = "prints measurements for a 35,803 entry listing"]
    fn reports_the_cost_of_a_large_listing() {
        use std::time::Instant;

        let entries = sample_entries(35_803);

        let started = Instant::now();
        let packed = pack(&entries);
        let packing = started.elapsed();

        let started = Instant::now();
        let json = serde_json::to_vec(&entries).expect("json");
        let encoding_json = started.elapsed();

        let packet = Packet::parse(&packed).expect("parse");

        let started = Instant::now();
        let mut name_bytes = 0usize;
        for index in 0..packet.count() {
            name_bytes += packet.name(index).expect("name").len();
        }
        let names_only = started.elapsed();

        let started = Instant::now();
        let materialised = packet.entries();
        let all_entries = started.elapsed();

        println!("\n35,803 entries:");
        println!("  pack            {packing:?}  ({} bytes)", packed.len());
        println!(
            "  serde_json      {encoding_json:?}  ({} bytes)",
            json.len()
        );
        println!(
            "  packed/json     {:.1}%",
            packed.len() as f64 / json.len() as f64 * 100.0
        );
        println!("  names only      {names_only:?}  ({name_bytes} name bytes)");
        println!(
            "  all entries     {all_entries:?}  ({} materialised)",
            materialised.len()
        );
    }
}
