use crate::file_system::entry_codec::{self, ListingRow};
use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, canonical_path, entry_sort_key,
    name_order, path_to_string,
};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY};

/// The beginning of a directory listing: the entries that were read, plus a
/// cursor onto the entries that are still unread.
///
/// The split is what makes large directories streamable: the head renders
/// immediately while [`DirectoryListingCursor`] keeps the OS directory
/// iterator alive on the thread that streams the rest (see
/// [`super::super::listing`]).
pub struct DirectoryListing {
    pub view: DirectoryView,
    /// `None` once the directory turned out to fit in the batch, which makes
    /// `view` complete.
    pub cursor: Option<DirectoryListingCursor>,
}

/// The unread remainder of a directory listing: the live OS iterator.
///
/// Windows reads through [`windows_directory::WindowsDirectoryEntries`] rather
/// than `std::fs::ReadDir`, which is the same enumeration with
/// `FIND_FIRST_EX_LARGE_FETCH` set and the row built from the find data instead
/// of through a `DirEntry` — see that module for the measurements. Both are
/// `Send`, because the reader thread that streams the remaining batches owns the
/// iterator. Entries whose metadata cannot be read are skipped rather than
/// failing the whole listing — WSL's `/proc` and `/run` over the `\\wsl$` 9P
/// share behave that way.
pub struct DirectoryListingCursor {
    #[cfg(windows)]
    iterator: super::windows_directory::WindowsDirectoryEntries,
    #[cfg(not(windows))]
    iterator: fs::ReadDir,
    /// What a child path is made of: the directory's display spelling plus the
    /// separator `PathBuf::push` would have added. The iterator builds the same
    /// string per entry out of the path this cursor was opened with, so a row
    /// can carry its name alone and let the reader of it — a packet, or the
    /// `into_entry` call below — put the two back together.
    base: String,
}

impl DirectoryListingCursor {
    /// Pulls up to `count` entries, sorted for display. The boolean reports
    /// that the directory is exhausted, so the caller can close the stream.
    pub fn take(&mut self, count: usize) -> (Vec<DirectoryEntry>, bool) {
        let base = self.base.clone();
        let (rows, exhausted) = self.take_rows(count);
        (
            rows.into_iter().map(|row| row.into_entry(&base)).collect(),
            exhausted,
        )
    }

    /// The same listing as [`Self::take`], without a path per entry: what a
    /// packed batch is made of. A 35,803 entry directory pays for one `String`
    /// per row here rather than a `PathBuf` and a second `String`, which
    /// measured ~4 ms of a serial batch.
    pub fn take_rows(&mut self, count: usize) -> (Vec<ListingRow>, bool) {
        let (mut rows, exhausted) = self.take_rows_unsorted(count);
        sort_rows(&mut rows);
        (rows, exhausted)
    }

    /// [`Self::take_rows`] without the ordering, which is where the two platforms
    /// differ: the `std` walk hands back `DirEntry`s that `collect_rows` turns
    /// into rows, while the Windows walk already has the rows and skips that step
    /// entirely.
    #[cfg(windows)]
    fn take_rows_unsorted(&mut self, count: usize) -> (Vec<ListingRow>, bool) {
        self.iterator.take_rows(count)
    }

    #[cfg(not(windows))]
    fn take_rows_unsorted(&mut self, count: usize) -> (Vec<ListingRow>, bool) {
        // `count` may be `usize::MAX` when a caller wants everything at once;
        // never reserve that much up front.
        let mut raw_entries = Vec::with_capacity(count.min(MAX_BATCH_RESERVE));
        let mut exhausted = false;

        for _ in 0..count {
            match self.iterator.next() {
                Some(Ok(entry)) => raw_entries.push(entry),
                // A failing entry is skipped; iteration continues (or ends)
                // on the next call.
                Some(Err(_)) => continue,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }

        (collect_rows(raw_entries), exhausted)
    }
}

/// Ceiling for the pre-allocation above, so a huge `count` cannot ask the
/// allocator for gigabytes before a single entry has been read.
#[cfg(not(windows))]
const MAX_BATCH_RESERVE: usize = 8192;

/// Batches at least this large get their metadata reads split across threads.
/// Reading one entry's metadata is a `lstat`/`statx` on Unix, and that single
/// system call per entry *is* what a large listing spends its time on, so
/// concurrency pays off from a fairly low entry count. Below the threshold the
/// work-stealing dance would cost more than the reads it replaces, and small
/// directories — the common case when navigating — stay serial.
#[cfg(unix)]
const PARALLEL_ENTRY_THRESHOLD: Option<usize> = Some(128);

/// Windows serves `DirEntry::metadata()` out of the `WIN32_FIND_DATA` the
/// enumeration already returned, so a batch there costs no system call per
/// entry: it is allocation-bound, and splitting it across threads would only
/// add scheduling overhead.
#[cfg(not(unix))]
const PARALLEL_ENTRY_THRESHOLD: Option<usize> = None;

/// Whether a batch of `len` raw entries is worth splitting across threads.
fn batch_is_worth_splitting(len: usize) -> bool {
    PARALLEL_ENTRY_THRESHOLD.is_some_and(|threshold| len >= threshold)
}

/// Orders rows for display. Both walks funnel through here, so a listing comes
/// back in the same order whichever one produced it.
fn sort_rows(rows: &mut [ListingRow]) {
    rows.sort_unstable_by(|left, right| name_order(left.kind, &left.name, right.kind, &right.name));
}

/// Turns raw directory entries into display rows.
///
/// The parallel arm runs on rayon's *global* pool — the one shared with the
/// rest of the UI's work, deliberately kept free of bulk transfers (see the
/// dedicated transfer pool in [`super::super::local::operations`]), because a
/// listing is on the critical path to painting the explorer.
///
/// The order entries arrive in does not affect the result: the caller sorts them
/// by a total order (see [`sort_rows`]).
///
/// On Windows this is the definition of the rows rather than the producer of
/// them — `windows_directory` builds the same rows from the find data, and only
/// `the_two_walks_build_the_same_rows` below reaches this one — which is exactly
/// why it is kept here rather than compiled away.
#[cfg_attr(windows, allow(dead_code))]
fn collect_rows(raw_entries: Vec<fs::DirEntry>) -> Vec<ListingRow> {
    if batch_is_worth_splitting(raw_entries.len()) {
        raw_entries
            .into_par_iter()
            .filter_map(listing_row)
            .collect()
    } else {
        raw_entries.into_iter().filter_map(listing_row).collect()
    }
}

/// Reads the first `first_batch` entries of an already-canonical directory.
///
/// The read path canonicalizes once and shares the result with the directory
/// watcher it arms before calling this (see
/// [`super::super::listing::open_streamed_listing`]), so the watcher and the
/// listing cannot disagree about which directory they describe.
pub fn open_canonical_listing(
    path: PathBuf,
    first_batch: usize,
) -> Result<DirectoryListing, FileSystemError> {
    let metadata = fs::metadata(&path)?;
    let display = path_to_string(&path);

    if !metadata.is_dir() {
        return Err(FileSystemError::NotDirectory(display));
    }

    let mut cursor = DirectoryListingCursor {
        #[cfg(windows)]
        iterator: crate::file_system::local::windows_directory::WindowsDirectoryEntries::open(
            &path,
        )?,
        #[cfg(not(windows))]
        iterator: fs::read_dir(&path)?,
        base: entry_codec::base_of(&display),
    };
    let (entries, exhausted) = cursor.take(first_batch);

    Ok(DirectoryListing {
        view: DirectoryView {
            path: display,
            breadcrumbs: build_breadcrumbs(&path),
            entries,
            stream_id: None,
        },
        cursor: (!exhausted).then_some(cursor),
    })
}

/// Reads a directory as one complete, sorted snapshot.
pub fn read_directory_sync(requested_path: PathBuf) -> Result<DirectoryView, FileSystemError> {
    let DirectoryListing { mut view, cursor } =
        open_canonical_listing(canonical_path(&requested_path)?, usize::MAX)?;

    if let Some(mut cursor) = cursor {
        // Unreachable with a `usize::MAX` batch size, but draining keeps the
        // two entry points equivalent if that ever changes.
        let (rest, _) = cursor.take(usize::MAX);
        view.entries.extend(rest);
        view.entries.sort_by_cached_key(entry_sort_key);
    }

    Ok(view)
}

/// Turns one raw directory entry into a listing row, or `None` when its metadata
/// cannot be read.
///
/// The kind is derived from the metadata that was read anyway rather than from
/// `DirEntry::file_type()`, so one entry costs exactly one metadata read: on
/// Unix `file_type()` is only free while the filesystem reports a usable
/// `d_type`, and falls back to the very same `lstat` otherwise.
///
/// No path: the row is a child of the directory being listed, so the base says
/// where it lives and `DirEntry::path()` never has to be called.
///
/// On Windows this is reached only by the parity test: the walk there builds
/// rows from the find data itself, without materialising a `DirEntry`.
#[cfg_attr(windows, allow(dead_code))]
fn listing_row(entry: fs::DirEntry) -> Option<ListingRow> {
    let metadata = entry.metadata().ok()?;
    Some(row_from(entry.file_name().to_string_lossy().into_owned(), metadata))
}

/// The row for a child whose name and metadata are in hand.
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

/// The display entry for a child whose metadata is already in hand.
///
/// Both read paths build entries through here — the listing from a `DirEntry`, a
/// patch from the name a watcher reported — so a patched row is spelled exactly
/// like the row it replaces. That is what lets the frontend match the two up by
/// name instead of re-reading the directory to find out.
fn entry_from(name: String, path: PathBuf, metadata: fs::Metadata) -> DirectoryEntry {
    // The row carries its whole spelling rather than hanging off a base: this is
    // one child whose path the caller already built, and a patch has no packet
    // to be spelled against until the frontend packs it.
    let mut row = row_from(name, metadata);
    row.path = Some(path_to_string(&path));
    row.into_entry("")
}

/// Stats exactly the named children of an already-canonical directory, one
/// answer per requested name, in the order requested.
///
/// A watcher reports a handful of names out of a listing of tens of thousands,
/// and re-reading the whole directory for them is the cost this exists to avoid.
/// `Ok(None)` is a name that no longer exists, which is the one answer that tells
/// the caller to drop its row. Any other failure fails the whole call: a listing
/// patched from a report the backend is unsure of would be wrong in a way nobody
/// would ever notice, so the caller re-reads instead.
pub fn stat_named_children(
    directory: &Path,
    names: Vec<String>,
) -> Result<Vec<Option<DirectoryEntry>>, FileSystemError> {
    let stat_one = |name: &str| -> Result<Option<DirectoryEntry>, FileSystemError> {
        let path = directory.join(name);

        // `symlink_metadata`, not `metadata`: the listing this patches was read
        // through `DirEntry::metadata()`, which does not follow the link either,
        // so a symlink has to keep reporting as one.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(FileSystemError::Io(error.to_string())),
        };

        Ok(Some(entry_from(name.to_owned(), path, metadata)))
    };

    // The same split the listing uses: a burst that touches thousands of names
    // is exactly the batch worth spreading over threads.
    let results: Vec<Result<Option<DirectoryEntry>, FileSystemError>> =
        if batch_is_worth_splitting(names.len()) {
            names
                .into_par_iter()
                .map(|name| stat_one(&name))
                .collect()
        } else {
            names.iter().map(|name| stat_one(name)).collect()
        };

    results.into_iter().collect()
}

pub fn modified_at_millis(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
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

/// `(hidden, read_only)` derived from metadata already fetched during
/// listing — no extra system calls. Windows reads the DOS attribute bits;
/// Unix approximates with the dot prefix and the owner write bit.
#[cfg(windows)]
pub fn entry_state_flags(metadata: &fs::Metadata, _name: &str) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;

    let attributes = metadata.file_attributes();
    (
        attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
        attributes & FILE_ATTRIBUTE_READONLY.0 != 0,
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

#[cfg(not(any(windows, unix)))]
pub fn entry_state_flags(_metadata: &fs::Metadata, _name: &str) -> (bool, bool) {
    (false, false)
}

pub fn build_breadcrumbs(path: &Path) -> Vec<Breadcrumb> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();

    ancestors
        .into_iter()
        .map(|ancestor| Breadcrumb {
            name: ancestor
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_to_string(ancestor)),
            path: path_to_string(ancestor),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    use super::super::properties::update_properties;
    #[cfg(windows)]
    use crate::file_system::types::PropertyChanges;

    #[test]
    fn builds_clickable_breadcrumbs_from_a_path() {
        let path = std::env::temp_dir().join("dae").join("nested");
        let breadcrumbs = build_breadcrumbs(&path);

        assert_eq!(
            breadcrumbs.last().map(|item| item.path.as_str()),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            breadcrumbs.last().map(|item| item.name.as_str()),
            Some("nested")
        );
    }

    #[test]
    fn reads_entries_from_a_directory() {
        let directory =
            std::env::temp_dir().join(format!("dae-file-system-test-{}", std::process::id()));
        let nested_directory = directory.join("folder");
        let file = directory.join("file.txt");

        fs::create_dir_all(&nested_directory).expect("create test directory");
        fs::write(&file, "test").expect("create test file");

        let view = read_directory_sync(directory.clone()).expect("read test directory");
        let names = view
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["folder", "file.txt"]);

        let folder_entry = &view.entries[0];
        let file_entry = &view.entries[1];
        assert!(folder_entry.modified_at.is_some());
        assert_eq!(folder_entry.size, None);
        assert!(file_entry.modified_at.is_some());
        assert_eq!(file_entry.size, Some(4));

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // temp fixture, deleted right after
    fn reports_read_only_and_hidden_entry_flags() {
        let directory =
            std::env::temp_dir().join(format!("dae-entry-flags-test-{}", std::process::id()));
        let plain_file = directory.join("plain.txt");
        let read_only_file = directory.join("locked.txt");
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(&plain_file, "plain").expect("create plain file");
        fs::write(&read_only_file, "locked").expect("create read-only file");

        let mut permissions = fs::metadata(&read_only_file)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&read_only_file, permissions).expect("mark file read-only");

        #[cfg(unix)]
        let hidden_file = {
            let hidden_file = directory.join(".hidden.txt");
            fs::write(&hidden_file, "hidden").expect("create hidden file");
            hidden_file
        };
        #[cfg(windows)]
        let hidden_file = {
            let hidden_file = directory.join("hidden.txt");
            fs::write(&hidden_file, "hidden").expect("create hidden file");
            update_properties(
                &hidden_file,
                &PropertyChanges {
                    hidden: Some(true),
                    ..Default::default()
                },
            )
            .expect("mark file hidden");
            hidden_file
        };

        let view = read_directory_sync(directory.clone()).expect("read test directory");
        let by_name = |name: &str| {
            view.entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap_or_else(|| panic!("entry {name} is missing from the listing"))
        };

        assert!(!by_name("plain.txt").hidden);
        assert!(!by_name("plain.txt").read_only);
        assert!(by_name("locked.txt").read_only);
        let hidden_name = hidden_file
            .file_name()
            .expect("hidden file has a name")
            .to_string_lossy()
            .into_owned();
        assert!(by_name(&hidden_name).hidden);

        // Restore writability so cleanup succeeds on every platform.
        let mut permissions = fs::metadata(&read_only_file)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&read_only_file, permissions).expect("clear read-only");

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn lists_a_batch_wide_enough_for_the_parallel_metadata_read() {
        let directory =
            std::env::temp_dir().join(format!("dae-wide-listing-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");

        // One past the parallel threshold where there is one; a wide batch
        // either way on Windows, whose threshold is deliberately absent.
        let count = PARALLEL_ENTRY_THRESHOLD.map_or(256, |threshold| threshold + 1);
        for index in 0..count {
            fs::write(directory.join(format!("entry-{index:04}.txt")), "content")
                .expect("write file");
        }

        let view = read_directory_sync(directory.clone()).expect("read wide directory");

        assert_eq!(view.entries.len(), count);
        let names = view
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "a wide batch is still sorted for display");
        assert!(
            view.entries
                .iter()
                .all(|entry| entry.size == Some(7) && entry.modified_at.is_some()),
            "every entry carries the metadata its batch read"
        );

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    /// A patched row replaces the row it matches, so the two must be spelled
    /// identically — the paths are what the frontend compares, and a difference
    /// of prefix or separator would show the same file twice.
    #[test]
    fn a_patched_entry_is_spelled_exactly_like_the_listed_one() {
        let directory =
            std::env::temp_dir().join(format!("dae-entry-patch-test-{}", std::process::id()));
        let nested = directory.join("folder");
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(directory.join("plain.txt"), "plain").expect("create file");
        fs::write(directory.join("改名的.txt"), "unicode name").expect("create unicode file");

        let canonical = canonical_path(&directory).expect("canonicalize the test directory");
        let listed = read_directory_sync(canonical.clone())
            .expect("read the directory")
            .entries;

        let names: Vec<String> = listed.iter().map(|entry| entry.name.clone()).collect();
        let patched = stat_named_children(&canonical, names.clone()).expect("stat the children");

        assert_eq!(patched.len(), names.len(), "one answer per name asked for");

        for (position, name) in names.iter().enumerate() {
            let entry = patched[position].as_ref().unwrap_or_else(|| {
                panic!("{name} exists in the listing, so it must exist in the patch")
            });
            let original = listed
                .iter()
                .find(|entry| &entry.name == name)
                .expect("listed entry");

            assert_eq!(&entry.path, &original.path, "path spelling is the match key");
            assert_eq!(&entry.kind, &original.kind, "a directory stays a directory");
            assert_eq!(&entry.name, &original.name);
            assert_eq!(entry.size, original.size);
            assert_eq!(entry.modified_at, original.modified_at);
            assert_eq!(entry.hidden, original.hidden);
            assert_eq!(entry.read_only, original.read_only);
        }

        // The name that is gone answers `None`, which is the one answer that
        // tells the caller to drop its row rather than to wait for a re-read.
        let mut with_gone = names;
        with_gone.push("no-such-file.txt".to_owned());
        let patched = stat_named_children(&canonical, with_gone).expect("stat with a missing name");
        assert!(
            patched.last().expect("an answer for the missing name").is_none(),
            "a name that is not on disk is reported as gone"
        );

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    /// Prints what a large local listing costs, and what the enumeration under it
    /// costs on its own.
    ///
    /// ```bash
    /// cd src-tauri/crates/dae-core
    /// DAE_LISTING_BENCH=K:/dae-listing-bench/big \
    ///   cargo test --lib --release -- --ignored --nocapture reports_where_a_large_listing_spends_its_time
    /// ```
    ///
    /// The dataset is whatever directory the variable names;
    /// `bench/listing-pipeline prepare` writes one (35,803 entries and 120,000 are
    /// the sizes the notes here and in
    /// `docs/decisions/windows-directory-enumeration.md` are written against).
    ///
    /// Four rows. `std walk` and `std rows` are the reference the metadata
    /// question is answered with: `fs::read_dir` with nothing built, then with each
    /// entry's row built from it (its metadata, its name, the row). On Windows a
    /// metadata read there is **not** a system call per entry —
    /// `DirEntry::metadata()` is served from the `WIN32_FIND_DATA` the enumeration
    /// already returned — which is why that row is a small share of the walk and
    /// why [`PARALLEL_ENTRY_THRESHOLD`] is `None` on Windows (`Some(128)` on Unix,
    /// where the same row is where a per-entry `lstat` lands, and where the app
    /// spreads such a batch over rayon — so this serial `rows` row overstates that
    /// arm). `streamed` is the pipeline the explorer actually calls on Windows:
    /// the `windows_directory` walk, one `FIRST_BATCH_SIZE` head and then batches
    /// doubling to `MAX_BATCH_SIZE` (`open_streamed_listing` plus
    /// `read_remaining_batches`). `listing` is `read_directory_sync`, the same walk
    /// read in one shot — it answers with `DirectoryEntry`s rather than packed
    /// rows, so it also carries a path per row and is the heavier of the two by
    /// design.
    ///
    /// Numbers from here and numbers from `bench/listing-pipeline` are not
    /// directly comparable: a `cargo test --lib` binary has no `main.rs`, so it
    /// runs on the system heap, while the harness (and the shipped binary)
    /// installs mimalloc, which this crate's notes measure at ~31% on a large
    /// listing. That inflates every row that allocates — `std rows` and
    /// `listing`'s paths — so the mirror harness is where the walk and ordering
    /// shapes are compared, and the `std walk`/`std rows` split is what this test
    /// is for. Compare each row against itself, before and after a change; the
    /// absolute milliseconds follow the volume's state.
    #[test]
    #[ignore = "prints measurements for the directory in DAE_LISTING_BENCH"]
    fn reports_where_a_large_listing_spends_its_time() {
        use std::time::Instant;

        const FIRST_BATCH_SIZE: usize = 512;
        const MAX_BATCH_SIZE: usize = 8192;

        let Some(directory) = std::env::var_os("DAE_LISTING_BENCH") else {
            panic!("set DAE_LISTING_BENCH to a directory to measure");
        };
        let path = canonical_path(Path::new(&directory)).expect("canonicalize the dataset");
        let entries = fs::read_dir(&path).expect("read the dataset").count() as u64;

        let walk = || {
            for entry in fs::read_dir(&path).expect("read the dataset") {
                std::hint::black_box(entry.expect("entry"));
            }
        };
        let rows = || {
            let mut built: Vec<ListingRow> = Vec::new();
            for entry in fs::read_dir(&path).expect("read the dataset") {
                if let Some(row) = listing_row(entry.expect("entry")) {
                    built.push(row);
                }
            }
            std::hint::black_box(built);
        };
        let listing = || {
            std::hint::black_box(read_directory_sync(path.clone()).expect("list the dataset"));
        };
        // One listing, walked the way `open_streamed_listing` plus
        // `read_remaining_batches` walk it.
        let drain = || -> usize {
            let listing =
                open_canonical_listing(path.clone(), FIRST_BATCH_SIZE).expect("open the listing");
            let mut rows = listing.view.entries.len();

            let Some(mut cursor) = listing.cursor else {
                return rows;
            };
            let mut batch_size = FIRST_BATCH_SIZE;
            loop {
                let (batch, exhausted) = cursor.take_rows(batch_size);
                rows += batch.len();
                if exhausted {
                    return rows;
                }
                batch_size = (batch_size * 2).min(MAX_BATCH_SIZE);
            }
        };

        // Two warm passes, and every timed pass checked: a listing that dropped
        // rows would time faster for the wrong reason.
        for _ in 0..2 {
            assert_eq!(drain(), entries as usize);
        }

        let median_ns = |body: &dyn Fn()| -> u128 {
            let mut samples: Vec<u128> = Vec::new();
            for _ in 0..7 {
                let started = Instant::now();
                body();
                samples.push(started.elapsed().as_nanos());
            }
            samples.sort_unstable();
            samples[samples.len() / 2]
        };

        let walk_ns = median_ns(&walk);
        let rows_ns = median_ns(&rows);
        let listing_ns = median_ns(&listing);
        let streamed_ns = median_ns(&|| {
            assert_eq!(drain(), entries as usize);
        });

        // Signed, because an ordering that costs nothing (the comparator, which
        // allocates nothing) can leave the one-shot listing under the rows it was
        // built from — the two runs are separate medians.
        let ms = |nanos: u128| nanos as f64 / 1e6;
        let walk_ms = ms(walk_ns);
        let rows_ms = ms(rows_ns);
        let listing_ms = ms(listing_ns);
        let streamed_ms = ms(streamed_ns);

        println!();
        println!("{entries} entries in {}", path.display());
        println!("  std walk        {walk_ms:>8.2} ms (fs::read_dir only: no rows built)");
        println!(
            "  std rows        {rows_ms:>8.2} ms (+{:.2} ms: metadata + name + row through std, {:.3} us/entry)",
            rows_ms - walk_ms,
            (rows_ms - walk_ms) * 1_000.0 / entries as f64
        );
        println!(
            "  streamed        {streamed_ms:>8.2} ms ({:.0} entries/s: the shipped shape, {FIRST_BATCH_SIZE} head then doubling batches)",
            entries as f64 / (streamed_ms / 1e3)
        );
        println!(
            "  listing         {listing_ms:>8.2} ms (read_directory_sync: the same walk one-shot, a path per row)"
        );
    }

    /// An empty directory is the one case where `FindFirstFileExW` fails and the
    /// listing is still correct: the pattern matches nothing, so the walk opens
    /// with no handle and reports exhaustion on its first call. Both entry points
    /// are checked, because the streaming head asks for 512 entries and the
    /// one-shot read asks for everything, and only the second would be reached by
    /// `read_directory_sync` alone.
    #[test]
    fn lists_an_empty_directory_without_a_stream() {
        let directory =
            std::env::temp_dir().join(format!("dae-empty-listing-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create an empty directory");

        let view = read_directory_sync(directory.clone()).expect("list an empty directory");
        assert!(view.entries.is_empty());
        assert!(view.stream_id.is_none());

        let listing = open_canonical_listing(
            canonical_path(&directory).expect("canonicalize the empty directory"),
            // The shell's first batch size; empty directories exercise the same path.
            512,
        )
        .expect("open an empty listing");
        assert!(listing.view.entries.is_empty());
        assert!(
            listing.cursor.is_none(),
            "an empty directory has nothing left to stream"
        );

        fs::remove_dir_all(directory).expect("remove the empty directory");
    }

    #[test]
    fn returns_not_directory_for_a_file_path() {
        let file =
            std::env::temp_dir().join(format!("dae-file-system-test-{}.txt", std::process::id()));
        fs::write(&file, "test").expect("create test file");

        let error = read_directory_sync(file.clone()).expect_err("a file is not a directory");

        fs::remove_file(file).expect("remove test file");

        assert!(matches!(error, FileSystemError::NotDirectory(_)));
    }

    /// The Windows walk builds its rows from the find data instead of from a
    /// `DirEntry`, so this is the only thing that would notice the two spellings
    /// drifting apart: same kind (symlinks included, which is the part with the
    /// most rules), same size, same timestamp, same two attribute flags, same
    /// name.
    #[cfg(windows)]
    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // temp fixture, deleted right after
    fn the_two_walks_build_the_same_rows() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let directory =
            std::env::temp_dir().join(format!("dae-walk-parity-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("folder")).expect("create a nested folder");
        fs::write(directory.join("plain.txt"), "content").expect("write a file");
        fs::write(directory.join("报告 2024.txt"), "内容").expect("write a non-ASCII file");
        fs::write(directory.join("empty"), "").expect("write an empty file");
        symlink_file(directory.join("plain.txt"), directory.join("link-to-file"))
            .expect("link to a file");
        symlink_dir(directory.join("folder"), directory.join("link-to-folder"))
            .expect("link to a folder");

        let read_only = directory.join("locked.txt");
        fs::write(&read_only, "locked").expect("write a read-only file");
        let mut permissions = fs::metadata(&read_only)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&read_only, permissions).expect("mark the file read-only");

        // Bulk, so the comparison covers ordering as well as fields: mixed case,
        // digits and a repeated prefix, which is what makes the two orderings
        // comparable over more than one pair.
        for index in 0..300 {
            let name = match index % 4 {
                0 => format!("entry-{index:03}.txt"),
                1 => format!("Entry-{index:03}.TXT"),
                2 => format!("ENTRY_{index:03}.log"),
                _ => format!("entry.{index:03}.json"),
            };
            fs::write(directory.join(name), "").expect("write a bulk file");
        }

        let canonical = canonical_path(&directory).expect("canonicalize the fixture");

        let mut std_rows = collect_rows(
            fs::read_dir(&canonical)
                .expect("read the fixture")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect the fixture entries"),
        );
        sort_rows(&mut std_rows);

        let mut windows_rows =
            super::super::windows_directory::WindowsDirectoryEntries::open(&canonical)
                .expect("open the fixture")
                .take_rows(usize::MAX)
                .0;
        sort_rows(&mut windows_rows);

        let describe = |rows: &[ListingRow]| {
            rows.iter()
                .map(|row| {
                    (
                        row.name.clone(),
                        row.kind,
                        row.size,
                        row.modified_at,
                        row.hidden,
                        row.read_only,
                        row.path.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };

        let std_rows = describe(&std_rows);
        let windows_rows = describe(&windows_rows);
        assert_eq!(std_rows, windows_rows);
        // A fixture where both walks agree because both found nothing would pass
        // the comparison above for the wrong reason.
        assert_eq!(std_rows.len(), 307, "the fixture holds 307 entries");
        assert!(
            std_rows.iter().any(|row| row.1 == EntryKind::Symlink),
            "the fixture keeps a symlink for the kind rule to disagree about"
        );
        assert!(std_rows.iter().any(|row| row.5), "and a read-only flag");

        // Restore writability so cleanup succeeds.
        let mut permissions = fs::metadata(&read_only)
            .expect("read file metadata")
            .permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&read_only, permissions).expect("restore the file's permissions");
        fs::remove_dir_all(&directory).expect("remove the fixture");
    }

    /// The window the verbatim spelling in `windows_directory::search_pattern`
    /// exists for: a path long enough that `std` converts it to `\\?\...` before
    /// calling `FindFirstFileExW`, but not long enough that
    /// `types::canonical_path` keeps the prefix — so the walk is handed a legacy
    /// spelling the OS refuses past `MAX_PATH`.
    #[cfg(windows)]
    #[test]
    fn lists_a_directory_whose_path_is_at_the_max_path_boundary() {
        use std::os::windows::ffi::OsStrExt;

        let root = std::env::temp_dir().join(format!("dae-max-path-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // Deep enough that the path lands in `248..=260` UTF-16 units, which is
        // the window where `dunce::simplified` still strips the verbatim prefix
        // and `std::fs::read_dir` would put it back on its own. The length is
        // built rather than hardcoded because the temp root and the process id
        // are not fixed.
        let mut deep = root.clone();
        while deep.as_os_str().encode_wide().count() < 230 {
            let step = deep.iter().count();
            deep.push(format!("segment-{step:04}"));
        }
        let have = deep.as_os_str().encode_wide().count();
        let last = deep
            .file_name()
            .expect("the loop pushed a segment")
            .to_string_lossy()
            .into_owned();
        let parent = deep.parent().expect("a nested path has a parent");
        deep = parent.join(format!("{last}{}", "p".repeat(250 - have)));

        let verbatim = PathBuf::from(format!(r"\\?\{}", deep.display()));
        // Creating it needs the verbatim spelling for the same reason reading it
        // does; the assertion below is about the reading.
        fs::create_dir_all(&verbatim).expect("create the deep directory");
        fs::write(verbatim.join("found.txt"), "content").expect("write a file in it");

        let canonical = canonical_path(&verbatim).expect("canonicalize the deep directory");
        let units: Vec<u16> = canonical.as_os_str().encode_wide().collect();
        assert!(
            !canonical.to_string_lossy().starts_with(r"\\?\"),
            "the fixture no longer exercises the boundary: {} units, still verbatim",
            units.len()
        );
        assert!(
            (248..=260).contains(&units.len()),
            "the fixture is {} units long, outside the boundary this test is about",
            units.len()
        );

        let view = read_directory_sync(canonical).expect("list the deep directory");
        assert_eq!(
            view.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["found.txt"]
        );

        fs::remove_dir_all(&verbatim).expect("remove the deep directory");
        let _ = fs::remove_dir_all(&root);
    }
}
