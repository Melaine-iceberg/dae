//! Directory enumeration on Windows, through `FindFirstFileExW`.
//!
//! `std::fs::read_dir` on Windows already asks for `FindExInfoBasic`, and its
//! source says why it stops there (`library/std/src/sys/fs/windows.rs`):
//!
//! ```text
//! We can pass FIND_FIRST_EX_LARGE_FETCH to dwAdditionalFlags to speed up things
//! more, but as we don't know user's use profile of this function, lets be
//! conservative.
//! ```
//!
//! A file manager opening a 35,000-entry folder *is* the use profile that flag
//! exists for, and it is the only difference this module makes to the system
//! calls: the same `FindExInfoBasic`, the same `FindExSearchNameMatch`, the same
//! entries in the same order. The flag asks the kernel to read a much larger
//! block of the directory into its own buffer, so the per-entry `FindNextFileW`
//! calls are served from memory rather than each costing a trip to the
//! filesystem.
//!
//! Two things make this faster than `std::fs::read_dir` rather than merely
//! different, and `bench/listing-pipeline` measures them apart:
//!
//! - `FIND_FIRST_EX_LARGE_FETCH`, above. On a 35,803 entry directory the walk
//!   goes 12.8 ms -> 9.4 ms (~26%); on 120,000 entries, 35.6 ms -> 27.1 ms
//!   (~24%).
//! - No `DirEntry` per entry. `std` clones an `Arc<PathBuf>` and copies the
//!   592-byte `WIN32_FIND_DATAW` for every entry it yields, and the row then
//!   costs `file_name()`'s wide -> `OsString` -> `String` round trip. The row
//!   here is written straight from the buffer `FindNextFileW` filled.
//!
//! Together the full listing pipeline (walk + rows + ordering) goes
//! 18.8 ms -> 13.8 ms on 35,803 entries and 51.5 ms -> 38.5 ms on 120,000.
//! What the user waits for *first* — the 512-entry head — is all but unchanged:
//! at 250 entries the two walks measured the same 0.05 ms, which is why this is
//! about the listing completing rather than the pane opening.
//!
//! # The path this opens
//!
//! A verbatim (`\\?\`) spelling, which is what `GetFullPathNameW` +
//! `prefer_verbatim` produce in `std`'s own `get_long_path`, and which the
//! listing has already resolved once: `types::canonical_path` canonicalizes and
//! then drops the prefix only where dropping it cannot change which entry is
//! named. Opening the verbatim spelling directly is what keeps a directory whose
//! path exceeds `MAX_PATH` readable — the same reason that function keeps the
//! prefix for such a path — and it means the walk never has to rebuild a path or
//! ask the OS to normalize one.
//!
//! # Why the rows are built here
//!
//! [`WindowsDirectoryEntries::take_rows`] returns the same [`ListingRow`]s
//! `local::directory` builds from a `DirEntry`, field for field: the kind from
//! the same attribute bits `std`'s `FileType` reads (including the reparse-tag
//! rule that makes a symlink a symlink), the size only for files, the same two
//! DOS attribute flags, and the modification time from the same `FILETIME`
//! `Metadata::modified` reports. `the_two_walks_build_the_same_rows` in
//! `directory.rs` pins that down against the `std` walk on a real directory,
//! because nothing else would notice if the two drifted.

use crate::fs::entry_codec::ListingRow;
use crate::fs::error::FileSystemError;
use crate::fs::types::EntryKind;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, FILETIME, GetLastError, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

/// Ceiling for the rows one batch reserves up front, so a huge batch size cannot
/// ask the allocator for gigabytes before a single entry has been read. Mirrors
/// the constant the `std` walk reserves against in `local::directory`.
const MAX_BATCH_RESERVE: usize = 8192;

/// `IO_REPARSE_TAG_*` values with this bit set name what the point redirects to;
/// `std`'s `FileType::new` reports those as symlinks. A reparse point without it
/// (a deduplication stub, say) is an ordinary file or directory.
const IO_REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;

/// 100 ns intervals between 1601-01-01 (`FILETIME`'s epoch) and the Unix epoch,
/// which is what `SystemTime::duration_since(UNIX_EPOCH)` subtracts on Windows.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// A live directory iteration: the find handle plus the entry last handed out.
///
/// Owned by one thread at a time and moved to the listing's reader thread, which
/// is why the `Send` impl below is load-bearing rather than a formality — `std`
/// declares the same one for the same reason.
pub struct WindowsDirectoryEntries {
    handle: Option<HANDLE>,
    /// The first entry, which `FindFirstFileExW` returns rather than
    /// `FindNextFileW`.
    pending: Option<WIN32_FIND_DATAW>,
    /// The buffer `FindNextFileW` fills, reused for every call.
    current: WIN32_FIND_DATAW,
}

// SAFETY: a find handle is not thread-affine — `FindNextFileW` may be called
// from any thread — and the struct owns it exclusively. `std` makes the same
// claim for `FindNextFileHandle`.
unsafe impl Send for WindowsDirectoryEntries {}

impl WindowsDirectoryEntries {
    /// Opens `directory`, which must be canonical: the same requirement
    /// [`local::directory::open_canonical_listing`] already documents for its caller.
    pub fn open(directory: &Path) -> Result<Self, FileSystemError> {
        let pattern = search_pattern(directory);
        let mut first = WIN32_FIND_DATAW::default();

        // SAFETY: `pattern` is NUL-terminated and outlives the call; `first` is
        // a correctly-sized out-parameter; `FindExSearchNameMatch` documents
        // that no search filter is read.
        let opened = unsafe {
            FindFirstFileExW(
                PCWSTR(pattern.as_ptr()),
                FindExInfoBasic,
                (&raw mut first).cast(),
                FindExSearchNameMatch,
                None,
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };

        match opened {
            Ok(handle) => Ok(Self {
                handle: Some(handle),
                pending: Some(first),
                current: WIN32_FIND_DATAW::default(),
            }),
            Err(_) => {
                // SAFETY: reading the calling thread's last error is always sound.
                let code = unsafe { GetLastError() };
                if code == ERROR_FILE_NOT_FOUND {
                    // The pattern matched nothing, which for a directory that
                    // exists is an empty directory rather than a failure. `std`
                    // answers the same way: the walk reports exhaustion on its
                    // first call. A nonexistent path answers
                    // `ERROR_PATH_NOT_FOUND` instead and keeps its error.
                    Ok(Self {
                        handle: None,
                        pending: None,
                        current: WIN32_FIND_DATAW::default(),
                    })
                } else {
                    Err(io_error(code.0).into())
                }
            }
        }
    }

    /// Pulls up to `count` entries as rows, and whether the directory ran out.
    ///
    /// Same contract as `DirectoryListingCursor::take_rows`: at most `count`
    /// entries are consumed, an entry that cannot be read ends the listing
    /// rather than failing it, and the call after the last entry reports
    /// exhaustion.
    pub fn take_rows(&mut self, count: usize) -> (Vec<ListingRow>, bool) {
        let mut rows = Vec::with_capacity(count.min(MAX_BATCH_RESERVE));

        loop {
            if rows.len() >= count {
                return (rows, false);
            }

            match self.advance() {
                Ok(true) => rows.push(row_from_find_data(&self.current)),
                Ok(false) => return (rows, true),
                // A mid-listing failure: the entries already read are the
                // listing, and `advance` has closed the handle, so the next call
                // reports exhaustion. The `std` walk reports an error and then
                // exhaustion, which the listing treats the same way.
                Err(_) => return (rows, true),
            }
        }
    }

    /// Moves to the next entry, skipping the `.` and `..` entries the OS
    /// reports. `false` means the directory is exhausted.
    fn advance(&mut self) -> Result<bool, std::io::Error> {
        if let Some(first) = self.pending.take() {
            self.current = first;
            if !is_dot_entry(&self.current) {
                return Ok(true);
            }
        }

        let Some(handle) = self.handle else {
            return Ok(false);
        };

        loop {
            // SAFETY: `handle` came from `FindFirstFileExW` and is still open —
            // `close` takes it before the handle is dropped — and `current` is a
            // correctly-sized out-parameter.
            match unsafe { FindNextFileW(handle, &raw mut self.current) } {
                Ok(()) => {
                    if !is_dot_entry(&self.current) {
                        return Ok(true);
                    }
                }
                Err(_) => {
                    // SAFETY: reading the calling thread's last error is always sound.
                    let code = unsafe { GetLastError() };
                    self.close();
                    return if code == ERROR_NO_MORE_FILES {
                        Ok(false)
                    } else {
                        Err(io_error(code.0))
                    };
                }
            }
        }
    }

    fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: the handle is ours, and this is the only place it is
            // closed — `Drop` closes whatever is left.
            unsafe {
                let _ = FindClose(handle);
            }
        }
    }
}

impl Drop for WindowsDirectoryEntries {
    fn drop(&mut self) {
        self.close();
    }
}

/// The row for one `WIN32_FIND_DATAW`, spelled the way
/// `local::directory::row_from` spells it from the `std::fs::Metadata` that
/// holds the same struct.
fn row_from_find_data(data: &WIN32_FIND_DATAW) -> ListingRow {
    let attributes = data.dwFileAttributes;
    let is_reparse_point = attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    // `std` only trusts `dwReserved0` as a reparse tag when the attribute says
    // the entry is one; otherwise the field is reserved and reads as zero.
    let reparse_tag = if is_reparse_point {
        data.dwReserved0
    } else {
        0
    };
    let is_symlink = is_reparse_point && reparse_tag & IO_REPARSE_TAG_NAME_SURROGATE != 0;
    let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;

    // The same sequence `local::directory::entry_kind` runs over `FileType`:
    // `is_dir` and `is_file` are both false for a symlink, whatever it points at.
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
        .then_some(((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64);

    ListingRow {
        name: name_of(data),
        // A child of the directory being listed: the packet names the base once.
        path: None,
        kind,
        modified_at: modified_at_millis(&data.ftLastWriteTime),
        size,
        hidden: attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
        read_only: attributes & FILE_ATTRIBUTE_READONLY.0 != 0,
    }
}

/// The entry's name as `DirEntry::file_name().to_string_lossy()` would spell it:
/// the NUL-terminated wide buffer, decoded lossily.
fn name_of(data: &WIN32_FIND_DATAW) -> String {
    let end = data
        .cFileName
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(data.cFileName.len());

    String::from_utf16_lossy(&data.cFileName[..end])
}

/// `Metadata::modified()` as milliseconds since the Unix epoch — the same answer
/// `local::directory::modified_at_millis` computes from the same `FILETIME`,
/// including the `None` for a timestamp before the epoch.
fn modified_at_millis(written: &FILETIME) -> Option<u64> {
    let intervals = ((written.dwHighDateTime as u64) << 32) | written.dwLowDateTime as u64;

    intervals
        .checked_sub(FILETIME_UNIX_EPOCH)
        .map(|since_epoch| since_epoch / 10_000)
}

/// `std` skips the `.` and `..` entries `FindFirstFileExW` reports, by looking at
/// the first three UTF-16 units of the name.
fn is_dot_entry(data: &WIN32_FIND_DATAW) -> bool {
    matches!(&data.cFileName[0..3], [0x2E, 0, ..] | [0x2E, 0x2E, 0, ..])
}

/// The `directory\*` pattern to search, in a verbatim spelling.
///
/// The mapping is `std`'s own (`get_long_path`, the `prefer_verbatim` branch): a
/// drive path gets `\\?\`, a UNC path becomes `\\?\UNC\`, a device path
/// (`\\.\`) is dereferenced to `\\?\`, and something already verbatim is left
/// alone. `std` only takes that branch for long paths because it has to call
/// `GetFullPathNameW` first; here the path is already canonical and absolute, so
/// the prefix can be applied unconditionally — which is also what keeps a
/// directory past `MAX_PATH` readable through this walk.
fn search_pattern(directory: &Path) -> Vec<u16> {
    const SEPARATOR: u16 = b'\\' as u16;
    const ALT_SEPARATOR: u16 = b'/' as u16;
    const COLON: u16 = b':' as u16;
    const QUERY: u16 = b'?' as u16;
    const DOT: u16 = b'.' as u16;

    let units: Vec<u16> = directory.as_os_str().encode_wide().collect();
    let mut pattern = Vec::with_capacity(units.len() + 8);

    match units.as_slice() {
        // Already verbatim, or in the NT namespace.
        [SEPARATOR, SEPARATOR, QUERY, SEPARATOR, ..] | [SEPARATOR, QUERY, QUERY, SEPARATOR, ..] => {
            pattern.extend_from_slice(&units)
        }
        // `\\.\C:\...` names the volume directly, which `\\?\` also does.
        [SEPARATOR, SEPARATOR, DOT, SEPARATOR, rest @ ..] => {
            pattern.extend_from_slice(&[SEPARATOR, SEPARATOR, QUERY, SEPARATOR]);
            pattern.extend_from_slice(rest);
        }
        // `\\server\share` is `\\?\UNC\server\share`.
        [SEPARATOR, SEPARATOR, rest @ ..] => {
            pattern.extend(b"\\\\?\\UNC\\".iter().map(|&byte| u16::from(byte)));
            pattern.extend_from_slice(rest);
        }
        // `C:\...`: the drive-absolute spelling, which is the one a verbatim
        // prefix applies to.
        [drive, COLON, SEPARATOR, ..] if *drive != SEPARATOR && *drive != ALT_SEPARATOR => {
            pattern.extend_from_slice(&[SEPARATOR, SEPARATOR, QUERY, SEPARATOR]);
            pattern.extend_from_slice(&units);
        }
        // Anything else is relative and cannot be verbatim; it is passed through
        // as the OS expects it. The listing never reaches this arm — the path it
        // opens has been canonicalized.
        _ => pattern.extend_from_slice(&units),
    }

    if !matches!(pattern.last(), Some(&unit) if unit == SEPARATOR || unit == ALT_SEPARATOR) {
        pattern.push(SEPARATOR);
    }
    pattern.push(b'*' as u16);
    pattern.push(0);

    pattern
}

fn io_error(code: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(code as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(spelling: &str) -> Vec<u16> {
        let mut units: Vec<u16> = spelling.encode_utf16().collect();
        units.push(0);
        units
    }

    /// The mapping has to be `std`'s, or a listing would look in a different
    /// place than the rest of the app does.
    #[test]
    fn spells_the_search_pattern_the_way_std_does() {
        let pattern = |spelling: &str| search_pattern(Path::new(spelling));

        assert_eq!(pattern(r"C:\dir"), wide(r"\\?\C:\dir\*"));
        assert_eq!(pattern(r"C:\"), wide(r"\\?\C:\*"));
        assert_eq!(pattern(r"C:\dir\nested"), wide(r"\\?\C:\dir\nested\*"));

        // Already verbatim, and the NT namespace, are left alone.
        assert_eq!(pattern(r"\\?\C:\dir"), wide(r"\\?\C:\dir\*"));
        assert_eq!(pattern(r"\??\C:\dir"), wide(r"\??\C:\dir\*"));

        // A UNC share cannot carry a drive prefix.
        assert_eq!(pattern(r"\\server\share"), wide(r"\\?\UNC\server\share\*"));
        assert_eq!(
            pattern(r"\\server\share\dir"),
            wide(r"\\?\UNC\server\share\dir\*")
        );

        // A device path names the volume directly, which `\\?\` also does.
        assert_eq!(pattern(r"\\.\C:"), wide(r"\\?\C:\*"));

        // Relative, which cannot be verbatim, is passed through as the OS wants.
        assert_eq!(pattern(r"dir\sub"), wide(r"dir\sub\*"));
        assert_eq!(pattern("dir"), wide(r"dir\*"));
    }

    /// The kind rules are the ones with the most ways to be subtly wrong, and
    /// they are the ones `std`'s `FileType` decides from the same two fields.
    #[test]
    fn reads_the_kind_from_the_same_bits_std_reads() {
        let data = |attributes: u32, reparse_tag: u32| WIN32_FIND_DATAW {
            dwFileAttributes: attributes,
            dwReserved0: reparse_tag,
            ..Default::default()
        };

        let directory = FILE_ATTRIBUTE_DIRECTORY.0;
        let reparse = FILE_ATTRIBUTE_REPARSE_POINT.0;
        // `IO_REPARSE_TAG_SYMLINK`, which is a name surrogate;
        // `IO_REPARSE_TAG_DEDUP` is not.
        let symlink_tag = 0xA000_000C;
        let dedup_tag = 0x8000_0013;

        assert_eq!(
            row_from_find_data(&data(directory, 0)).kind,
            EntryKind::Directory
        );
        assert_eq!(row_from_find_data(&data(0, 0)).kind, EntryKind::File);
        // A reparse point that is not a name surrogate is an ordinary entry.
        assert_eq!(
            row_from_find_data(&data(directory, dedup_tag)).kind,
            EntryKind::Directory
        );
        assert_eq!(
            row_from_find_data(&data(0, dedup_tag)).kind,
            EntryKind::File
        );
        // A name surrogate is a symlink whatever it points at — `FileType::is_dir`
        // is false for one, so a directory symlink must not report as a directory.
        assert_eq!(
            row_from_find_data(&data(reparse | directory, symlink_tag)).kind,
            EntryKind::Symlink
        );
        assert_eq!(
            row_from_find_data(&data(reparse, symlink_tag)).kind,
            EntryKind::Symlink
        );
        // The reserved field is only a tag when the attribute says so.
        assert_eq!(
            row_from_find_data(&data(directory, symlink_tag)).kind,
            EntryKind::Directory
        );
    }

    /// Size is reported for files and for nothing else, and the two attribute
    /// flags come from the DOS bits — both as `row_from` does it.
    #[test]
    fn reads_size_and_flags_like_the_std_walk() {
        let data = WIN32_FIND_DATAW {
            dwFileAttributes: FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_READONLY.0,
            nFileSizeHigh: 1,
            nFileSizeLow: 2,
            ..Default::default()
        };

        let row = row_from_find_data(&data);
        assert_eq!(row.size, Some((1 << 32) | 2));
        assert!(row.hidden);
        assert!(row.read_only);
        assert_eq!(row.kind, EntryKind::File);

        let directory = WIN32_FIND_DATAW {
            dwFileAttributes: FILE_ATTRIBUTE_DIRECTORY.0,
            nFileSizeHigh: 1,
            nFileSizeLow: 2,
            ..Default::default()
        };
        assert_eq!(row_from_find_data(&directory).size, None);
    }

    #[test]
    fn skips_the_dot_entries_the_os_reports() {
        let named = |name: &str| {
            let mut data = WIN32_FIND_DATAW::default();
            for (slot, unit) in data.cFileName.iter_mut().zip(name.encode_utf16()) {
                *slot = unit;
            }
            data
        };

        assert!(is_dot_entry(&named(".")));
        assert!(is_dot_entry(&named("..")));
        assert!(!is_dot_entry(&named(".hidden")));
        assert!(!is_dot_entry(&named("name")));
    }

    /// A timestamp before the Unix epoch is `None`, not a wrapped number: `std`
    /// reports `duration_since(UNIX_EPOCH)` failing, and the row keeps that.
    #[test]
    fn reads_modification_times_like_the_std_walk() {
        let at = |intervals: u64| FILETIME {
            dwLowDateTime: intervals as u32,
            dwHighDateTime: (intervals >> 32) as u32,
        };

        assert_eq!(modified_at_millis(&at(FILETIME_UNIX_EPOCH)), Some(0));
        assert_eq!(
            modified_at_millis(&at(FILETIME_UNIX_EPOCH + 12_345)),
            Some(1)
        );
        assert_eq!(modified_at_millis(&at(FILETIME_UNIX_EPOCH - 1)), None);
        assert_eq!(modified_at_millis(&at(0)), None);
    }
}
