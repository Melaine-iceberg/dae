//! `FindFirstFileExW` directory iteration, with and without
//! `FIND_FIRST_EX_LARGE_FETCH`.
//!
//! Rust's `std::fs::read_dir` on Windows already asks for `FindExInfoBasic` — its
//! comment in `library/std/src/sys/fs/windows.rs` says so, and says why it stops
//! there:
//!
//! ```text
//! We can pass FIND_FIRST_EX_LARGE_FETCH to dwAdditionalFlags to speed up things
//! more, but as we don't know user's use profile of this function, lets be
//! conservative.
//! ```
//!
//! A file manager opening a 35,000-entry folder *is* that profile. The flag asks
//! the kernel to read a much larger block of the directory into its own buffer,
//! so the per-entry `FindNextFileW` calls are served from memory instead of each
//! costing a trip through the filesystem.
//!
//! The bindings are declared here rather than pulled in from a crate because the
//! harness is otherwise dependency-free, and because they are short: five
//! functions and one struct, laid out exactly as `std` declares them.
//!
//! The `Send` impl is not decoration — the app's `DirectoryListingCursor` owns a
//! live iterator across streamed batches and hands it to a reader thread.

#![cfg(windows)]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

type Handle = *mut core::ffi::c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_NO_MORE_FILES: u32 = 18;

/// Docs: "This flag can improve performance by reducing the number of calls to
/// the file system."
const FIND_FIRST_EX_LARGE_FETCH: u32 = 0x0000_0002;

/// `FINDEX_INFO_LEVELS::FindExInfoBasic` — skips filling `cAlternateFileName`,
/// the 8.3 name, which the app never reads.
const FIND_EX_INFO_BASIC: i32 = 1;
/// `FINDEX_SEARCH_OPS::FindExSearchNameMatch`.
const FIND_EX_SEARCH_NAME_MATCH: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

/// `WIN32_FIND_DATAW`, field for field as `std` declares it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FindData {
    pub attributes: u32,
    pub creation_time: FileTime,
    pub last_access_time: FileTime,
    pub last_write_time: FileTime,
    pub size_high: u32,
    pub size_low: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub file_name: [u16; 260],
    pub alternate_file_name: [u16; 14],
}

impl Default for FindData {
    fn default() -> Self {
        // SAFETY: every field is a plain integer or array of them.
        unsafe { std::mem::zeroed() }
    }
}

unsafe extern "system" {
    fn FindFirstFileExW(
        file_name: *const u16,
        info_level: i32,
        find_data: *mut FindData,
        search_op: i32,
        search_filter: *const core::ffi::c_void,
        additional_flags: u32,
    ) -> Handle;
    fn FindNextFileW(handle: Handle, find_data: *mut FindData) -> i32;
    fn FindClose(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

/// A live directory iteration: the find handle plus the entry last handed out.
pub struct Win32Find {
    handle: Option<Handle>,
    pending: Option<FindData>,
    current: FindData,
}

// SAFETY: the handle is a kernel object used by exactly one thread at a time —
// the reader thread that owns the cursor — and `FindData` is plain data.
unsafe impl Send for Win32Find {}

impl Win32Find {
    /// Opens `directory`. `large_fetch` is the only thing that varies between the
    /// two configurations this harness compares.
    pub fn open(directory: &Path, large_fetch: bool) -> io::Result<Self> {
        let mut pattern: Vec<u16> = directory.as_os_str().encode_wide().collect();
        let ends_with_separator = matches!(pattern.last(), Some(&unit) if unit == u16::from(b'\\') || unit == u16::from(b'/'));
        if !ends_with_separator {
            pattern.push(u16::from(b'\\'));
        }
        pattern.push(u16::from(b'*'));
        pattern.push(0);

        let mut first = FindData::default();
        let flags = if large_fetch { FIND_FIRST_EX_LARGE_FETCH } else { 0 };

        // SAFETY: `pattern` is NUL-terminated, `first` is a valid out-pointer,
        // and the search filter is documented as unused for name matching.
        let handle = unsafe {
            FindFirstFileExW(
                pattern.as_ptr(),
                FIND_EX_INFO_BASIC,
                &mut first,
                FIND_EX_SEARCH_NAME_MATCH,
                core::ptr::null(),
                flags,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: no preconditions beyond being called on a failed Win32 call.
            let error = unsafe { GetLastError() };
            // `ERROR_FILE_NOT_FOUND` means "the directory matched nothing", which
            // for an existing directory is simply empty. `std` checks the same
            // way rather than trusting the pattern to be a directory.
            if error == ERROR_FILE_NOT_FOUND && directory.is_dir() {
                return Ok(Self {
                    handle: None,
                    pending: None,
                    current: FindData::default(),
                });
            }
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        Ok(Self {
            handle: Some(handle),
            pending: Some(first),
            current: FindData::default(),
        })
    }

    /// The next entry, or `None` at the end of the directory. `.` and `..` are
    /// filtered here exactly as `std` filters them.
    pub fn next_data(&mut self) -> io::Result<Option<&FindData>> {
        let Some(handle) = self.handle else {
            return Ok(None);
        };

        if let Some(first) = self.pending.take() {
            if !is_dot_entry(&first) {
                self.current = first;
                return Ok(Some(&self.current));
            }
        }

        loop {
            // SAFETY: `handle` came from `FindFirstFileExW` and is still open;
            // `current` is a valid, correctly-sized out-buffer.
            let found = unsafe { FindNextFileW(handle, &mut self.current) };
            if found == 0 {
                self.handle = None;
                // SAFETY: no preconditions beyond being called on a failed Win32 call.
                let error = unsafe { GetLastError() };
                if error == ERROR_NO_MORE_FILES {
                    return Ok(None);
                }
                return Err(io::Error::from_raw_os_error(error as i32));
            }

            if !is_dot_entry(&self.current) {
                return Ok(Some(&self.current));
            }
        }
    }

    /// The entry's name as `DirEntry::file_name().to_string_lossy()` would spell
    /// it: a NUL-terminated UTF-16 buffer, decoded lossily.
    pub fn name_of(data: &FindData) -> String {
        let end = data
            .file_name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(data.file_name.len());
        String::from_utf16_lossy(&data.file_name[..end])
    }
}

impl Drop for Win32Find {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: the handle is ours and is closed exactly once.
            unsafe {
                FindClose(handle);
            }
        }
    }
}

/// `std` skips the `.` and `..` entries `FindFirstFileExW` reports: the first
/// three UTF-16 units are `.`, NUL, … or `.`, `.`, NUL, ….
fn is_dot_entry(data: &FindData) -> bool {
    matches!(
        &data.file_name[0..3],
        [0x2E, 0, ..] | [0x2E, 0x2E, 0, ..]
    )
}
