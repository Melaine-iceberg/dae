//! A verbatim mirror of the app's local listing pipeline, plus candidate
//! orderings to measure against it.
//!
//! Every function under "the mirror" is copied from the app with the imports
//! adjusted and nothing else changed:
//!
//! | here | app |
//! |---|---|
//! | [`EntryKind`] | `file_system::types::EntryKind` |
//! | [`ListingRow`] | `file_system::entry_codec::ListingRow` (the `path` field is kept so the struct keeps its size) |
//! | [`entry_kind_rank`] | `file_system::types::entry_kind_rank` |
//! | [`name_sort_key`] | `file_system::types::name_sort_key` |
//! | [`entry_kind`] | `file_system::local::directory::entry_kind` |
//! | [`modified_at_millis`] | `file_system::local::directory::modified_at_millis` |
//! | [`entry_state_flags`] | `file_system::local::directory::entry_state_flags` |
//! | [`row_from`] | `file_system::local::directory::row_from` |
//! | [`listing_row`] | `file_system::local::directory::listing_row` |
//! | [`collect_rows`] | `file_system::local::directory::collect_rows` |
//! | [`next_batch_size`] | `file_system::listing::next_batch_size` |
//!
//! The one deliberate divergence is the thread split in the app's `collect_rows`:
//! on Unix a batch of 128 or more has its metadata reads spread over rayon's
//! global pool, and this crate has no rayon dependency, so [`collect_rows`] is
//! always the serial arm. That arm is the one Windows takes for every batch
//! (`PARALLEL_ENTRY_THRESHOLD` is `None` there), and the ordering work measured
//! here runs on one thread on every platform — the parallel arm splits
//! `listing_row`, never the sort. A Unix number from this harness would therefore
//! understate the metadata half and overstate the sort's share.
//!
//! Nothing reads the rows back: it is the ordering and the row construction under
//! measurement, not the packet that carries them.

use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
use crate::windows_find::{FindData, Win32Find};

/// `FILE_ATTRIBUTE_HIDDEN` / `FILE_ATTRIBUTE_READONLY`, spelled out so the
/// mirror does not need the `windows` crate: `std::os::windows::fs::MetadataExt`
/// exposes the same two bits through `Metadata::file_attributes`.
#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
#[cfg(windows)]
const FILE_ATTRIBUTE_READONLY: u32 = 0x1;

// ---------------------------------------------------------------- the mirror

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

/// Two of these fields are read — `kind` and `name`, the ones the ordering
/// compares. The rest exist because the app's struct has them: a mirror whose row
/// was smaller would measure a different row.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ListingRow {
    pub name: String,
    /// `None` for the ordinary child of the directory being listed: the packet
    /// names the base once. The app's `ListingRow` carries a `String` here, so
    /// this field exists to keep the two structs the same size.
    pub path: Option<String>,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
    pub hidden: bool,
    pub read_only: bool,
}

pub fn entry_kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::Symlink => 1,
        EntryKind::File => 2,
        EntryKind::Other => 3,
    }
}

/// The display order of one entry: kind first, then the name case-insensitively,
/// then the name itself so two names that fold to the same key still have one
/// order regardless of the order the directory was read in.
pub fn name_sort_key(kind: EntryKind, name: &str) -> (u8, String, String) {
    (entry_kind_rank(&kind), name.to_lowercase(), name.to_owned())
}

pub fn entry_kind(file_type: fs::FileType) -> EntryKind {
    if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

pub fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

#[cfg(windows)]
pub fn entry_state_flags(metadata: &fs::Metadata, _name: &str) -> (bool, bool) {
    let attributes = metadata.file_attributes();
    (
        attributes & FILE_ATTRIBUTE_HIDDEN != 0,
        attributes & FILE_ATTRIBUTE_READONLY != 0,
    )
}

#[cfg(unix)]
pub fn entry_state_flags(metadata: &fs::Metadata, name: &str) -> (bool, bool) {
    use std::os::unix::fs::PermissionsExt;

    (
        name.starts_with('.'),
        metadata.permissions().mode() & 0o200 == 0,
    )
}

fn row_from(name: String, metadata: fs::Metadata) -> ListingRow {
    let kind = entry_kind(metadata.file_type());
    let size = matches!(kind, EntryKind::File).then_some(metadata.len());
    let (hidden, read_only) = entry_state_flags(&metadata, &name);

    ListingRow {
        name,
        path: None,
        kind,
        modified_at: modified_at_millis(&metadata),
        size,
        hidden,
        read_only,
    }
}

fn listing_row(entry: fs::DirEntry) -> Option<ListingRow> {
    let metadata = entry.metadata().ok()?;
    Some(row_from(entry.file_name().to_string_lossy().into_owned(), metadata))
}

/// Size of the batch after one of `current` entries.
pub fn next_batch_size(current: usize) -> usize {
    (current * 2).min(MAX_BATCH_SIZE)
}

/// Ceiling for the batches that follow the head, from `file_system::listing`.
pub const MAX_BATCH_SIZE: usize = 8192;

/// Entries in the batch `read_directory` answers with, from `file_system::listing`.
pub const FIRST_BATCH_SIZE: usize = 512;

// ------------------------------------------------------------ the orderings

/// Case-insensitive comparison matching `str::to_lowercase()` exactly, without
/// building either lowercase string.
///
/// The fast path is exact rather than approximate: for an ASCII input,
/// `str::to_lowercase()` is `to_ascii_lowercase()` byte for byte, so a bytewise
/// compare of the two lowers is the same comparison the app's cached key makes.
/// Anything non-ASCII falls through to the `char::to_lowercase()` streams, whose
/// concatenation is what `str::to_lowercase()` returns; comparing those streams
/// code point by code point is the same order as comparing the two `String`s
/// (UTF-8 preserves code point order).
fn cmp_lowercase(left: &str, right: &str) -> Ordering {
    if left.is_ascii() && right.is_ascii() {
        let left = left.as_bytes();
        let right = right.as_bytes();
        let shared = left.len().min(right.len());

        for index in 0..shared {
            let ordering = left[index].to_ascii_lowercase().cmp(&right[index].to_ascii_lowercase());
            if ordering != Ordering::Equal {
                return ordering;
            }
        }

        return left.len().cmp(&right.len());
    }

    let mut left = left.chars().flat_map(char::to_lowercase);
    let mut right = right.chars().flat_map(char::to_lowercase);

    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => {
                let ordering = left.cmp(&right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

/// The same ordering, expressed as a comparator instead of a cached key.
///
/// For distinct names `(rank, lowercase, name)` is a total order, so the
/// unstably-sorted result is the same sequence the app's stable cached-key sort
/// produces — the sort has no ties to break by arrival order.
pub fn cmp_rows(left: &ListingRow, right: &ListingRow) -> Ordering {
    entry_kind_rank(&left.kind)
        .cmp(&entry_kind_rank(&right.kind))
        .then_with(|| cmp_lowercase(&left.name, &right.name))
        .then_with(|| left.name.cmp(&right.name))
}

/// What a walk hands over, and how it becomes a row.
///
/// The two enumerations differ in more than the flag: `std` yields a
/// `fs::DirEntry` (an `Arc<PathBuf>` clone plus a 592-byte `WIN32_FIND_DATAW`
/// copy per entry) whose `metadata()` is that same struct converted; the Win32
/// walk yields the struct itself. Both rows are therefore built from the same
/// bytes — what changes is how many times those bytes are copied on the way.
pub trait RawEntry {
    fn into_row(self) -> Option<ListingRow>;
}

impl RawEntry for std::fs::DirEntry {
    fn into_row(self) -> Option<ListingRow> {
        listing_row(self)
    }
}

#[cfg(windows)]
impl RawEntry for FindData {
    fn into_row(self) -> Option<ListingRow> {
        Some(row_from_find_data(&self))
    }
}

/// The row for one `WIN32_FIND_DATAW`, built the way `row_from` builds it from
/// `std::fs::Metadata` — same fields, same source, because `std`'s Windows
/// `Metadata` *is* the `WIN32_FIND_DATAW` (`DirEntry::metadata` is
/// `Ok(self.data.into())`, see `library/std/src/sys/fs/windows.rs`).
#[cfg(windows)]
fn row_from_find_data(data: &FindData) -> ListingRow {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    /// Set when `dwReserved0` is a reparse tag naming what the point redirects
    /// to; `std` calls this a "name surrogate" and reports those as symlinks.
    const IO_REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;
    /// 100 ns intervals between 1601-01-01 and 1970-01-01.
    const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

    // `std`'s `FileType::new(attributes, reparse_tag)`: a reparse point whose tag
    // is a name surrogate is a symlink, and a symlink is never also a directory.
    let is_reparse_point = data.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let reparse_tag = if is_reparse_point { data.reserved0 } else { 0 };
    let is_symlink = is_reparse_point && reparse_tag & IO_REPARSE_TAG_NAME_SURROGATE != 0;
    let is_directory = data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;

    let kind = if !is_symlink && is_directory {
        EntryKind::Directory
    } else if !is_symlink && !is_directory {
        EntryKind::File
    } else if is_symlink {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    };

    let size = matches!(kind, EntryKind::File)
        .then_some(((data.size_high as u64) << 32) | (data.size_low as u64));
    let modified_at = (((data.last_write_time.high as u64) << 32)
        | data.last_write_time.low as u64)
        .checked_sub(FILETIME_UNIX_EPOCH)
        .map(|intervals| intervals / 10_000);

    ListingRow {
        name: Win32Find::name_of(data),
        path: None,
        kind,
        modified_at,
        size,
        hidden: data.attributes & FILE_ATTRIBUTE_HIDDEN != 0,
        read_only: data.attributes & FILE_ATTRIBUTE_READONLY != 0,
    }
}

/// A source of raw directory entries: whichever enumeration is being measured.
pub trait Source {
    type Raw: RawEntry;
    fn next_entry(&mut self) -> Option<std::io::Result<Self::Raw>>;
}

pub struct StdSource(std::fs::ReadDir);

impl StdSource {
    pub fn open(directory: &Path) -> std::io::Result<Self> {
        Ok(Self(fs::read_dir(directory)?))
    }
}

impl Source for StdSource {
    type Raw = std::fs::DirEntry;

    fn next_entry(&mut self) -> Option<std::io::Result<std::fs::DirEntry>> {
        self.0.next()
    }
}

#[cfg(windows)]
pub struct Win32Source(Win32Find);

#[cfg(windows)]
impl Win32Source {
    pub fn open(directory: &Path, large_fetch: bool) -> std::io::Result<Self> {
        Ok(Self(Win32Find::open(directory, large_fetch)?))
    }
}

#[cfg(windows)]
impl Source for Win32Source {
    type Raw = FindData;

    fn next_entry(&mut self) -> Option<std::io::Result<FindData>> {
        match self.0.next_data() {
            Ok(Some(data)) => Some(Ok(*data)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

// ------------------------------------------------------------ the pipeline

/// Which enumeration a measured configuration runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// `std::fs::read_dir` — what the app does today, on every platform.
    Std,
    /// `FindFirstFileExW` + `FindNextFileW`, the app's own Windows path.
    Win32,
}

/// Whether the ordering is skipped, and if not, which key it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Stop after the rows are built: the ordering's cost is the difference.
    Skip,
    /// The app's cached `(rank, lowercase, name)` key.
    AppKey,
    /// The allocation-free comparator.
    Comparator,
}

/// One thing to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub walk: Walk,
    /// Only meaningful for [`Walk::Win32`]; the `std` walk passes no additional
    /// flags at all, which is exactly what `std` does.
    pub large_fetch: bool,
    /// Walk only, with no row construction, so the enumeration is visible alone.
    pub rows: bool,
    pub order: SortKey,
}

impl Config {
    /// `std` | `std+rows` | `std+key` | `std+cmp` | `win32` | `win32-nofetch` |
    /// `win32+rows` | `win32+key` | `win32-cmp`
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.split('+');
        let head = parts.next().unwrap_or_default();

        let (walk, large_fetch) = match head {
            "std" => (Walk::Std, false),
            "win32" => (Walk::Win32, true),
            "win32-nofetch" => (Walk::Win32, false),
            other => {
                return Err(format!(
                    "unknown walk `{other}` (expected std, win32 or win32-nofetch)"
                ));
            }
        };

        let mut rows = false;
        let mut order = SortKey::Skip;
        for part in parts {
            match part {
                "rows" => rows = true,
                "key" => {
                    rows = true;
                    order = SortKey::AppKey;
                }
                "cmp" => {
                    rows = true;
                    order = SortKey::Comparator;
                }
                other => return Err(format!("unknown phase `{other}`")),
            }
        }

        Ok(Self {
            walk,
            large_fetch,
            rows,
            order,
        })
    }

    pub fn label(&self) -> String {
        let walk = match (self.walk, self.large_fetch) {
            (Walk::Std, _) => "std walk",
            (Walk::Win32, true) => "win32 walk (large fetch)",
            (Walk::Win32, false) => "win32 walk (no large fetch)",
        };

        match (self.rows, self.order) {
            (false, _) => walk.to_owned(),
            (true, SortKey::Skip) => format!("{walk} + rows"),
            (true, SortKey::AppKey) => format!("{walk} + rows + app sort key"),
            (true, SortKey::Comparator) => format!("{walk} + rows + comparator"),
        }
    }

    /// Whether this configuration is a complete listing pipeline, which is what
    /// the headline comparison is between.
    pub fn is_pipeline(&self) -> bool {
        self.order != SortKey::Skip
    }
}

/// Where one pipeline run's time went, in nanoseconds.
#[derive(Debug, Default, Clone, Copy)]
pub struct Timings {
    /// `next_entry` calls only — the enumeration and its system calls.
    pub walk_ns: u128,
    /// Turning raw entries into rows: metadata, the name `String`, the row.
    pub rows_ns: u128,
    /// The ordering.
    pub sort_ns: u128,
    pub entries: u64,
}

impl Timings {
    pub fn total_ns(&self) -> u128 {
        self.walk_ns + self.rows_ns + self.sort_ns
    }
}

/// Runs one listing the way `open_streamed_listing` + `read_remaining_batches`
/// do: a 512-entry head, then batches doubling up to 8192, each batch walked,
/// turned into rows, and ordered.
///
/// The phases are timed separately per batch and summed. The app interleaves
/// them on one thread, so the sum is not the wall time of a real read — but the
/// phases are the ones being compared, and none of them overlaps another inside
/// a batch.
pub fn measure_into<S: Source>(
    mut source: S,
    config: Config,
    timings: &mut Timings,
) -> std::io::Result<()> {
    let mut batch_size = FIRST_BATCH_SIZE;

    loop {
        // The walk: what enumerating costs, with no per-entry work of ours.
        let started = std::time::Instant::now();
        let mut raw = Vec::with_capacity(batch_size.min(MAX_BATCH_SIZE));
        let mut exhausted = false;
        let mut seen = 0;

        while seen < batch_size {
            match source.next_entry() {
                Some(Ok(entry)) => {
                    raw.push(entry);
                    seen += 1;
                }
                // A failing entry is skipped, exactly as the app skips it.
                Some(Err(_)) => continue,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        timings.walk_ns += started.elapsed().as_nanos();
        timings.entries += raw.len() as u64;

        if !config.rows {
            drop(raw);
            if exhausted {
                return Ok(());
            }
            batch_size = next_batch_size(batch_size);
            continue;
        }

        let started = std::time::Instant::now();
        let mut rows: Vec<ListingRow> = raw.into_iter().filter_map(RawEntry::into_row).collect();
        timings.rows_ns += started.elapsed().as_nanos();

        if config.order != SortKey::Skip {
            let started = std::time::Instant::now();
            match config.order {
                SortKey::AppKey => {
                    rows.sort_by_cached_key(|row| name_sort_key(row.kind, &row.name));
                }
                SortKey::Comparator => rows.sort_unstable_by(cmp_rows),
                SortKey::Skip => unreachable!(),
            }
            timings.sort_ns += started.elapsed().as_nanos();
            std::hint::black_box(&rows);
        }

        if exhausted {
            return Ok(());
        }
        batch_size = next_batch_size(batch_size);
    }
}

/// Opens whichever walk a configuration names and measures one listing through
/// it. Kept here rather than in `main.rs` so the two sources have one call site.
pub fn measure_config(path: &Path, config: Config, timings: &mut Timings) -> std::io::Result<()> {
    match config.walk {
        Walk::Std => measure_into(StdSource::open(path)?, config, timings),
        #[cfg(windows)]
        Walk::Win32 => measure_into(Win32Source::open(path, config.large_fetch)?, config, timings),
        #[cfg(not(windows))]
        Walk::Win32 => Err(std::io::Error::other(
            "the Win32 walk is only available on Windows",
        )),
    }
}

/// The names of a full listing under a configuration — the parity check between
/// two orderings.
pub fn names_of(path: &Path, config: Config) -> std::io::Result<Vec<String>> {
    match config.walk {
        Walk::Std => {
            let mut source = StdSource::open(path)?;
            Ok(run_names(&mut source, config))
        }
        #[cfg(windows)]
        Walk::Win32 => {
            let mut source = Win32Source::open(path, config.large_fetch)?;
            Ok(run_names(&mut source, config))
        }
        #[cfg(not(windows))]
        Walk::Win32 => Err(std::io::Error::other(
            "the Win32 walk is only available on Windows",
        )),
    }
}

/// The name-collecting body of [`names_of`], generic over the walk.
fn run_names<S: Source>(source: &mut S, config: Config) -> Vec<String> {
    let mut names = Vec::new();
    let mut batch_size = FIRST_BATCH_SIZE;

    loop {
        let mut raw = Vec::with_capacity(batch_size.min(MAX_BATCH_SIZE));
        let mut exhausted = false;
        let mut seen = 0;

        while seen < batch_size {
            match source.next_entry() {
                Some(Ok(entry)) => {
                    raw.push(entry);
                    seen += 1;
                }
                Some(Err(_)) => continue,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }

        let mut rows: Vec<ListingRow> = raw.into_iter().filter_map(RawEntry::into_row).collect();
        match config.order {
            SortKey::Comparator => rows.sort_unstable_by(cmp_rows),
            _ => rows.sort_by_cached_key(|row| name_sort_key(row.kind, &row.name)),
        }
        names.extend(rows.into_iter().map(|row| row.name));

        if exhausted {
            return names;
        }
        batch_size = next_batch_size(batch_size);
    }
}

/// Does this host spend its listing run in the serial arm the app uses on
/// Windows? Printed with the results so a Unix run cannot be mistaken for one.
pub const SERIAL_ARM: bool = cfg!(not(unix));
