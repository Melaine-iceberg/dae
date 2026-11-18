//! Whole-volume filename indexing through the NTFS USN journal — the Windows
//! half of `fs::file_index`, Everything-style: the MFT is read once at
//! startup, and every later change is answered from the journal instead of
//! a directory walk.
//!
//! Two entry points, both engine-free and both owned by the orchestrator in
//! `mod.rs`:
//!
//! * [`capable`] — a cheap probe that this process may read a fixed volume's
//!   journal at all. Journal-less, non-NTFS and non-elevated machines answer
//!   `false`, which is what routes them to the feeder crawl instead.
//! * [`start`] — sweeps every fixed NTFS volume, hands its incremental
//!   watcher to a background thread, and answers with the volume roots that
//!   got covered. A volume the sweep cannot finish is logged and omitted
//!   from that list, so the orchestrator can hand exactly those to the
//!   feeder; being on the list means the watcher thread is already running.
//!
//! The sweep speaks file reference numbers (FRN), not paths, so the tree in
//! memory — FRN → (parent FRN, name, is a directory) — is what turns records
//! into the canonical spellings the store expects: `C:\Users\...`, native
//! separators, no trailing separator except on volume roots. Those strings
//! are already canonical (they come from the file system itself, symlink
//! free), which is why nothing here calls `dunce`.
//!
//! Nothing propagates errors into a caller: every failure is a `log::warn!`
//! and, for a whole volume, an omission from [`start`]'s list. Journal
//! positions live in memory only — a restart re-sweeps from scratch, and
//! that re-sweep is what repairs anything a missed event could leave behind.

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::{mem::size_of, ptr, slice, thread};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_INSUFFICIENT_BUFFER, ERROR_JOURNAL_ENTRY_DELETED,
    GENERIC_READ, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetDriveTypeW, GetLogicalDrives, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V1,
    READ_USN_JOURNAL_DATA_V1, USN_JOURNAL_DATA_V1, USN_JOURNAL_DATA_V2, USN_REASON_FILE_CREATE,
    USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME, USN_RECORD_V2,
    USN_RECORD_V3,
};
use windows::core::{HRESULT, HSTRING, PCWSTR};

use crate::fs::file_index::store::{Entry, Store};

/// One `DeviceIoControl` output buffer: large enough to amortize a sweep
/// round trip over hundreds of records, small enough to stay in cache.
const IO_BUFFER_BYTES: usize = 64 * 1024;

/// Store rows per upsert transaction during a sweep — the feeder's batch
/// size, so both population paths cost the same per row on huge volumes.
const UPSERT_BATCH: usize = 5_000;

/// `GetDriveTypeW`'s answer for a volume mounted in the machine. Network
/// shares and card readers are not swept: the journal CTLs would only fail
/// against them, and removable media belongs to the feeder.
const DRIVE_FIXED: u32 = 3;

/// The MFT record number NTFS always assigns its root directory (5), with the
/// 16-bit sequence-number prefix masked off — the fallback for a volume whose
/// root record does not point at itself.
const MFT_INDEX_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const ROOT_RECORD_INDEX: u64 = 5;

/// The journal reasons that can move a path: creations, deletions and both
/// halves of a rename. Content writes and metadata churn cannot change a
/// name, so the journal is asked to never even report them.
const WATCHED_REASONS: u32 = USN_REASON_FILE_CREATE
    | USN_REASON_FILE_DELETE
    | USN_REASON_RENAME_OLD_NAME
    | USN_REASON_RENAME_NEW_NAME;

/// How long the watcher naps when a journal read wakes without a record. The
/// blocking form of the read should never do that; the nap is the cheap
/// insurance that a driver quirk cannot turn into a spin.
const IDLE_RECHECK: Duration = Duration::from_millis(50);

/// One MFT node as the sweep learned it. The FRN is the map key; `parent`,
/// `name` and `is_dir` are everything path resolution needs.
#[derive(Clone, Debug)]
struct Node {
    parent: u64,
    name: String,
    is_dir: bool,
}

/// FRN → node. One per volume: the sweep fills it, the watcher keeps it
/// current, and every record the watcher applies is answered against it.
type FrnTree = HashMap<u64, Node>;

/// One decoded `USN_RECORD_V2`/`V3`, in the same shape whether it came from
/// the sweep or the incremental journal read.
#[derive(Clone, Debug)]
struct Record {
    frn: u64,
    parent: u64,
    name: String,
    is_dir: bool,
    reason: u32,
}

/// Where a volume's journal stands: its identity and the ends of its
/// record range.
struct Journal {
    id: u64,
    next_usn: i64,
    max_usn: i64,
}

/// Everything a successful sweep produced that the watcher thread needs.
struct Covered {
    tree: FrnTree,
    root_frn: u64,
    journal_id: u64,
    start_usn: i64,
}

/// Whether any fixed volume's journal can be read in this process — the
/// orchestrator's gate between the USN path and the feeder crawl.
///
/// The store is not consulted: the probe is about the process's right to
/// read a volume device, which no amount of indexing can substitute for.
/// Opening a handle and asking for the journal state costs two round trips
/// per fixed volume and reads no records.
pub fn capable(_store: &Store) -> bool {
    for root in fixed_volume_roots() {
        if let Ok(handle) = VolumeHandle::open(&root) {
            if query_journal(handle.0).is_ok() {
                return true;
            }
        }
    }
    false
}

/// Sweeps every fixed NTFS volume and starts watching it, blocking until the
/// sweep part is done. Answers with the covered volume roots (`C:\`
/// spellings); a volume whose journal cannot be read — no journal, non-NTFS,
/// access denied — is logged and omitted so the orchestrator can hand it to
/// the feeder crawl instead.
///
/// The watcher threads outlive the call and each owns its volume's share of
/// the store. `NextUsn` is captured before the sweep, so the watcher replays
/// whatever changed while the MFT was being read; replayed upserts land on
/// rows the sweep already wrote, which is harmless.
pub fn start(store: Store) -> Vec<String> {
    let mut covered = Vec::new();
    for root in fixed_volume_roots() {
        // Failure logging happens inside `cover_volume`; here an omission
        // simply means the feeder picks the volume up.
        let Some(volume) = cover_volume(&store, &root) else {
            continue;
        };
        let watcher_store = store.clone();
        let watcher_root = root.clone();
        let spawn = thread::Builder::new()
            .name(format!("usn-journal-{}", root.trim_end_matches('\\')))
            .spawn(move || {
                watch_volume(
                    watcher_store,
                    watcher_root,
                    volume.root_frn,
                    volume.tree,
                    volume.journal_id,
                    volume.start_usn,
                )
            });
        if let Err(error) = spawn {
            // The sweep data still answers searches; only the live updates
            // are lost until the next start.
            log::warn!("Could not start the USN watcher for {root}: {error}");
        }
        covered.push(root);
    }
    covered
}

/// Fixed volumes (`C:\`; not network shares or card readers). The drive
/// bitmap read costs no device round trip, so an unreachable drive cannot
/// stall the enumeration.
fn fixed_volume_roots() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    let mut roots = Vec::new();

    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let root = format!("{}:\\", char::from(b'A' + index as u8));
        let mut wide: Vec<u16> = root.encode_utf16().collect();
        wide.push(0);
        // SAFETY: `wide` is NUL-terminated and `GetDriveTypeW` only reads it.
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } == DRIVE_FIXED {
            roots.push(root);
        }
    }
    roots
}

/// An open `\\.\C:` device handle, closed on drop: every path through a
/// volume ends here, and a leaked handle would pin a volume the user is
/// trying to eject.
struct VolumeHandle(HANDLE);

impl VolumeHandle {
    /// Opens the volume behind `root` for the journal CTLs. `GENERIC_READ` is
    /// what they demand; read/write sharing is what lets the system keep
    /// using the disk while it is indexed.
    fn open(root: &str) -> windows::core::Result<Self> {
        // `\\.\C:` addresses the volume device, not a directory on it.
        let device = HSTRING::from(format!(r"\\.\{}", root.trim_end_matches('\\')));
        let handle = unsafe {
            CreateFileW(
                &device,
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }?;
        Ok(Self(handle))
    }
}

impl Drop for VolumeHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateFileW` and is closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Sweeps one volume into the store: journal query, MFT sweep, full-path
/// resolution, batched upserts, `mark_root`. Each failure is logged with its
/// reason and answered with `None`, which is what routes the volume to the
/// feeder.
fn cover_volume(store: &Store, root: &str) -> Option<Covered> {
    let covered = (|| -> Result<Covered, String> {
        let handle = VolumeHandle::open(root).map_err(|error| error.to_string())?;
        let journal = query_journal(handle.0).map_err(|error| error.to_string())?;
        let tree = sweep(handle.0, journal.max_usn).map_err(|error| error.to_string())?;
        let root_frn = find_root_frn(&tree)
            .ok_or_else(|| "the MFT sweep found no root directory record".to_owned())?;
        upsert_tree(store, root, root_frn, &tree).map_err(|error| error.to_string())?;
        store
            .mark_root(root, true)
            .map_err(|error| error.to_string())?;
        Ok(Covered {
            tree,
            root_frn,
            journal_id: journal.id,
            // Before the sweep on purpose: the watcher replays the changes
            // that happened while the MFT was being read.
            start_usn: journal.next_usn,
        })
    })();

    match covered {
        Ok(volume) => Some(volume),
        Err(reason) => {
            log::warn!("USN index skipped {root}: {reason}; the crawl fallback covers it");
            None
        }
    }
}

/// `FSCTL_QUERY_USN_JOURNAL`: whether this volume keeps a journal at all,
/// and where its records end. Non-NTFS and journal-less volumes fail here,
/// which is what routes them to the feeder.
fn query_journal(handle: HANDLE) -> windows::core::Result<Journal> {
    let mut buffer = vec![0u8; size_of::<USN_JOURNAL_DATA_V2>()];
    let mut bytes = 0u32;
    // SAFETY: `buffer` is owned output storage; `USN_JOURNAL_DATA_V2` is the
    // largest shape the CTL answers with, and every field read below sits in
    // the version-independent prefix.
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes),
            None,
        )?;
    }
    if (bytes as usize) < size_of::<USN_JOURNAL_DATA_V1>() {
        return Err(windows::core::Error::from_hresult(HRESULT::from_win32(
            ERROR_INSUFFICIENT_BUFFER.0,
        )));
    }
    // SAFETY: the bounds check above covers the whole prefix; the read is
    // unaligned because the buffer is a byte vector.
    let state = unsafe { ptr::read_unaligned(buffer.as_ptr() as *const USN_JOURNAL_DATA_V1) };
    Ok(Journal {
        id: state.UsnJournalID,
        next_usn: state.NextUsn,
        max_usn: state.MaxUsn,
    })
}

/// Whole-MFT sweep via `FSCTL_ENUM_USN_DATA`: every record contributes one
/// FRN → (parent, name, kind) node, until the CTL reports
/// `ERROR_HANDLE_EOF` — which is how it says the walk ran off the end of the
/// file record table.
fn sweep(handle: HANDLE, max_usn: i64) -> windows::core::Result<FrnTree> {
    let mut input = MFT_ENUM_DATA_V1 {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: max_usn,
        MinMajorVersion: 2,
        MaxMajorVersion: 3,
    };
    let mut buffer = vec![0u8; IO_BUFFER_BYTES];
    let mut tree = FrnTree::new();

    loop {
        let mut bytes = 0u32;
        // SAFETY: `input` is an owned CTL input, `buffer` is output storage
        // sized for a batch of records, and `bytes` receives the count.
        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(ptr::from_ref(&input).cast()),
                size_of::<MFT_ENUM_DATA_V1>() as u32,
                Some(buffer.as_mut_ptr().cast()),
                buffer.len() as u32,
                Some(&mut bytes),
                None,
            )
        };
        match result {
            Err(error) if error.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0) => break,
            Err(error) => return Err(error),
            Ok(()) => {
                if (bytes as usize) < 8 {
                    break;
                }
                // The first 8 bytes name the record to continue from.
                input.StartFileReferenceNumber =
                    u64::from_le_bytes(buffer[..8].try_into().unwrap());
                for record in decode_records(&buffer[8..bytes as usize]) {
                    tree.insert(
                        record.frn,
                        Node {
                            parent: record.parent,
                            name: record.name,
                            is_dir: record.is_dir,
                        },
                    );
                }
            }
        }
    }
    Ok(tree)
}

/// The volume root's FRN. On NTFS the root directory record points at itself;
/// the MFT-record-5 convention is the fallback. Either answer is
/// deterministic (lowest FRN wins), so two sweeps of one volume agree.
fn find_root_frn(tree: &FrnTree) -> Option<u64> {
    let mut self_parented: Option<u64> = None;
    for (&frn, node) in tree.iter() {
        if node.parent == frn {
            self_parented = Some(match self_parented {
                Some(found) => found.min(frn),
                None => frn,
            });
        }
    }
    if let Some(frn) = self_parented {
        return Some(frn);
    }
    tree.iter().find_map(|(&frn, node)| {
        (node.is_dir && frn & MFT_INDEX_MASK == ROOT_RECORD_INDEX).then_some(frn)
    })
}

/// The full path of `frn`: its name, its parent's name, and so on up to
/// `root_frn`, joined onto `root_path` with native separators. `None` means
/// the chain never reaches the root — an orphan, a missing ancestor, or a
/// loop in a corrupt record.
fn resolve_path(root_path: &str, root_frn: u64, frn: u64, tree: &FrnTree) -> Option<String> {
    // Collected leaf-first, joined root-first.
    let mut components: Vec<&str> = Vec::new();
    let mut current = frn;
    while current != root_frn {
        let node = tree.get(&current)?;
        if components.len() > tree.len() {
            // A chain longer than the tree has repeated a record: corrupt
            // parent loop rather than a real ancestor.
            return None;
        }
        components.push(node.name.as_str());
        current = node.parent;
    }
    Some(joined(root_path, &components))
}

/// Every reachable path in one pass: each unresolved node's parents are
/// walked until one already has a path, then the walked chain resolves in a
/// single unwind. Every node is walked at most once, so a sweep of five
/// million records stays linear instead of re-walking each file's ancestry.
fn resolve_tree(root_path: &str, root_frn: u64, tree: &FrnTree) -> HashMap<u64, String> {
    let mut paths: HashMap<u64, String> = HashMap::with_capacity(tree.len());
    paths.insert(root_frn, root_path.to_owned());
    let mut dead: HashSet<u64> = HashSet::new();

    for (&start, _) in tree.iter() {
        if paths.contains_key(&start) || dead.contains(&start) {
            continue;
        }
        // Walk until a node with a known path, a node known unreachable, or
        // a step off the tree (a revisit means the chain loops).
        let mut chain: Vec<u64> = Vec::new();
        let mut current = start;
        while !paths.contains_key(&current)
            && !dead.contains(&current)
            && !chain.contains(&current)
            && tree.contains_key(&current)
        {
            chain.push(current);
            current = tree[&current].parent;
        }
        if let Some(base) = paths.get(&current).cloned() {
            let mut path = base;
            for &frn in chain.iter().rev() {
                let node = &tree[&frn];
                if !path.ends_with('\\') {
                    path.push('\\');
                }
                path.push_str(&node.name);
                paths.insert(frn, path.clone());
            }
        } else {
            // Cycles, missing parents and chains through other unreachable
            // nodes all land here together.
            dead.extend(chain);
        }
    }
    paths
}

/// `root_path` followed by `components` outermost-first, one `\` between
/// them. A volume root (`C:\`) already ends in its separator and never gets
/// a second one.
fn joined(root_path: &str, components: &[&str]) -> String {
    let names_len: usize = components.iter().map(|name| name.len()).sum();
    let mut path = String::with_capacity(root_path.len() + names_len + components.len());
    path.push_str(root_path);
    for name in components.iter().rev() {
        if !path.ends_with('\\') {
            path.push('\\');
        }
        path.push_str(name);
    }
    path
}

/// One name under a resolved directory path, native separator between.
fn join_component(base: &str, name: &str) -> String {
    let mut path = String::with_capacity(base.len() + name.len() + 1);
    path.push_str(base);
    if !path.ends_with('\\') {
        path.push('\\');
    }
    path.push_str(name);
    path
}

/// What one journal record means for the index. A delete outranks its
/// companion reasons; a rename splits into the old spelling (row removal)
/// and the new one (row insert, or a subtree move).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Plan {
    delete: bool,
    remove_old: bool,
    upsert: bool,
}

/// Classifies a record by its reason bits. A `USN_REASON_FILE_DELETE` wins
/// over anything else on the same record, so a file created and deleted
/// before its close still classifies as gone.
fn classify_record(reason: u32) -> Plan {
    if reason & USN_REASON_FILE_DELETE != 0 {
        return Plan {
            delete: true,
            ..Plan::default()
        };
    }
    Plan {
        delete: false,
        remove_old: reason & USN_REASON_RENAME_OLD_NAME != 0,
        upsert: reason & (USN_REASON_RENAME_NEW_NAME | USN_REASON_FILE_CREATE) != 0,
    }
}

/// Follows one volume's journal until the journal dies. Never returns while
/// the volume stays readable: `ReadUsnJournal` with `BytesToWaitFor: 1` parks
/// the thread until the system writes the next record, so an idle disk costs
/// nothing.
fn watch_volume(
    store: Store,
    root: String,
    mut root_frn: u64,
    mut tree: FrnTree,
    journal_id: u64,
    start_usn: i64,
) {
    let handle = match VolumeHandle::open(&root) {
        Ok(handle) => handle,
        Err(error) => {
            log::warn!("USN watcher for {root} could not reopen the volume: {error}");
            return;
        }
    };
    let mut read = READ_USN_JOURNAL_DATA_V1 {
        StartUsn: start_usn,
        ReasonMask: WATCHED_REASONS,
        ReturnOnlyOnClose: 0,
        // The blocking form: no records yet means wait, not poll.
        Timeout: 0,
        BytesToWaitFor: 1,
        UsnJournalID: journal_id,
        MinMajorVersion: 2,
        MaxMajorVersion: 3,
    };
    let mut buffer = vec![0u8; IO_BUFFER_BYTES];

    loop {
        let mut bytes = 0u32;
        // SAFETY: `read` is an owned CTL input, `buffer` is output storage
        // sized for a batch of records, and `bytes` receives the count.
        let result = unsafe {
            DeviceIoControl(
                handle.0,
                FSCTL_READ_USN_JOURNAL,
                Some(ptr::from_ref(&read).cast()),
                size_of::<READ_USN_JOURNAL_DATA_V1>() as u32,
                Some(buffer.as_mut_ptr().cast()),
                buffer.len() as u32,
                Some(&mut bytes),
                None,
            )
        };
        match result {
            Ok(()) => {
                if (bytes as usize) <= 8 {
                    thread::sleep(IDLE_RECHECK);
                    continue;
                }
                // The first 8 bytes are where the next read continues.
                read.StartUsn = i64::from_le_bytes(buffer[..8].try_into().unwrap());
                for record in decode_records(&buffer[8..bytes as usize]) {
                    apply_record(&store, &root, root_frn, &mut tree, record);
                }
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_JOURNAL_ENTRY_DELETED.0) => {
                // The journal wrapped past the read position: the replay
                // window is gone, so rebuild from a fresh sweep instead of
                // trusting a partial history.
                log::warn!("USN journal for {root} wrapped past the reader; re-sweeping");
                if !resweep(&store, &root, handle.0, &mut read, &mut root_frn, &mut tree) {
                    return;
                }
            }
            Err(error) => {
                // A deleted or disabled journal, or the volume went away:
                // this volume's index goes stale until the next start.
                log::warn!("USN watcher for {root} stopped: {error}");
                return;
            }
        }
    }
}

/// Rebuild after the journal wrapped: fresh sweep, stale rows dropped with
/// `forget_root`, journal position re-anchored. `false` means give up on
/// this volume; the caller then lets the index go stale until the next
/// start, which is the only honest answer once neither the replay window
/// nor the rebuild survived.
fn resweep(
    store: &Store,
    root: &str,
    handle: HANDLE,
    read: &mut READ_USN_JOURNAL_DATA_V1,
    root_frn: &mut u64,
    tree: &mut FrnTree,
) -> bool {
    let journal = match query_journal(handle) {
        Ok(journal) => journal,
        Err(error) => {
            log::warn!("USN journal for {root} is no longer readable: {error}");
            return false;
        }
    };
    let swept = match sweep(handle, journal.max_usn) {
        Ok(swept) => swept,
        Err(error) => {
            log::warn!("Re-sweep of {root} failed: {error}");
            return false;
        }
    };
    let Some(swept_root) = find_root_frn(&swept) else {
        log::warn!("Re-sweep of {root} found no root directory record");
        return false;
    };
    if let Err(error) = store.forget_root(root) {
        log::warn!("Could not drop stale rows for {root}: {error}");
        return false;
    }
    *tree = swept;
    *root_frn = swept_root;
    if let Err(error) = upsert_tree(store, root, swept_root, tree) {
        log::warn!("Could not re-populate {root}: {error}");
        return false;
    }
    if let Err(error) = store.mark_root(root, true) {
        log::warn!("Could not re-mark {root} covered: {error}");
    }
    // A recreated journal gets a new id; either way the read re-anchors.
    read.UsnJournalID = journal.id;
    read.StartUsn = journal.next_usn;
    true
}

/// Applies one journal record to the FRN tree and the store. Records arrive
/// in journal order, so the tree is always the pre-event state the record
/// describes — which is what makes an old-path resolution possible at all.
fn apply_record(store: &Store, root: &str, root_frn: u64, tree: &mut FrnTree, record: Record) {
    let plan = classify_record(record.reason);
    if plan.delete {
        remove_entry(store, root, root_frn, tree, &record);
        return;
    }
    if plan.remove_old {
        // The old spelling of a rename. A directory's row is left standing
        // for the new-name record to turn into one subtree move; a file has
        // no subtree and can go immediately.
        if !record.is_dir {
            if let Some(old_path) = path_from_record(root, root_frn, tree, &record) {
                if let Err(error) = store.remove(&[old_path]) {
                    log::warn!("Could not drop a renamed-away row for {root}: {error}");
                }
            }
        }
    }
    if plan.upsert {
        upsert_entry(store, root, root_frn, tree, record);
    }
}

/// The path a record spells itself: its own name under its parent, with the
/// rest of the chain from the tree. Used where the record is the only
/// witness of the spelling — a delete of something the tree never saw.
fn path_from_record(root: &str, root_frn: u64, tree: &FrnTree, record: &Record) -> Option<String> {
    let parent = resolve_path(root, root_frn, record.parent, tree)?;
    Some(join_component(&parent, &record.name))
}

/// Turns a creation or new-name record into a store row. When the FRN was
/// already known under a different spelling this is a move: the tree still
/// holds the old state, so the old path resolves, and a directory's whole
/// subtree is moved by one store `rename`.
fn upsert_entry(store: &Store, root: &str, root_frn: u64, tree: &mut FrnTree, record: Record) {
    let Some(parent_path) = resolve_path(root, root_frn, record.parent, tree) else {
        return;
    };
    let path = join_component(&parent_path, &record.name);

    let previous = tree.get(&record.frn);
    let is_dir = previous.map_or(record.is_dir, |node| node.is_dir);
    let old_path = previous
        .filter(|node| node.parent != record.parent || node.name != record.name)
        .and_then(|_| resolve_path(root, root_frn, record.frn, tree));
    if let Some(old_path) = old_path {
        if old_path != path {
            let result = if is_dir {
                store.rename(&old_path, &path)
            } else {
                store.remove(&[old_path.clone()])
            };
            if let Err(error) = result {
                log::warn!("Could not move {old_path} to {path} in the index: {error}");
            }
        }
    }

    tree.insert(
        record.frn,
        Node {
            parent: record.parent,
            name: record.name.clone(),
            is_dir: record.is_dir,
        },
    );
    let entry = Entry {
        path,
        name: record.name,
        parent: parent_path,
        is_dir: record.is_dir,
    };
    if let Err(error) = store.upsert(&[entry]) {
        log::warn!("Could not index a change on {root}: {error}");
    }
}

/// A deletion's rows: the path the tree knows (or, for something it never
/// saw, the record's own spelling), plus every descendant of a deleted
/// directory, read out of the tree before the nodes go. The journal reports
/// the directory alone, so the subtree is this module's job.
fn remove_entry(store: &Store, root: &str, root_frn: u64, tree: &mut FrnTree, record: &Record) {
    let is_dir = tree
        .get(&record.frn)
        .map_or(record.is_dir, |node| node.is_dir);
    let path = resolve_path(root, root_frn, record.frn, tree)
        .or_else(|| path_from_record(root, root_frn, tree, record));
    let mut subtree: Vec<u64> = if is_dir {
        descendants(tree, record.frn)
    } else {
        Vec::new()
    };
    if !subtree.contains(&record.frn) {
        subtree.push(record.frn);
    }

    let Some(path) = path else {
        // No spelling to remove by, but the nodes must still go so later
        // records do not resolve through a deleted parent.
        for frn in &subtree {
            tree.remove(frn);
        }
        return;
    };

    let mut paths = vec![path];
    for &frn in &subtree {
        if frn != record.frn {
            if let Some(under) = resolve_path(root, root_frn, frn, tree) {
                paths.push(under);
            }
        }
    }
    for frn in subtree {
        tree.remove(&frn);
    }
    if let Err(error) = store.remove(&paths) {
        log::warn!("Could not drop deleted rows for {root}: {error}");
    }
}

/// Every FRN inside `frn`'s subtree, `frn` included. Built by fixpoint over
/// the parent edges rather than a child index: the tree is dominated by
/// names, a second edge set would double it, and directory deletions are
/// rare enough to pay a scan.
fn descendants(tree: &FrnTree, frn: u64) -> Vec<u64> {
    let mut known = HashSet::from([frn]);
    let mut grew = true;
    while grew {
        grew = false;
        for (&child, node) in tree.iter() {
            if !known.contains(&child) && known.contains(&node.parent) {
                known.insert(child);
                grew = true;
            }
        }
    }
    known.into_iter().collect()
}

/// Resolves the whole FRN tree to paths and upserts it in batches, volume
/// root entry included (`C:\` itself, the one row whose parent is empty).
/// Orphans — `$MFT` and friends, whose parent never reaches the root — have
/// no path and are left out, which is also what keeps system metadata out
/// of the index.
fn upsert_tree(store: &Store, root: &str, root_frn: u64, tree: &FrnTree) -> rusqlite::Result<()> {
    let paths = resolve_tree(root, root_frn, tree);
    let mut batch: Vec<Entry> = Vec::with_capacity(UPSERT_BATCH);
    batch.push(Entry {
        path: root.to_owned(),
        name: root.trim_end_matches('\\').to_owned(),
        parent: String::new(),
        is_dir: true,
    });
    for (&frn, node) in tree.iter() {
        if frn == root_frn {
            continue;
        }
        // A resolved node's parent is resolved by construction; the guard
        // only covers a tree mutated under the sweep, which cannot happen.
        let (Some(path), Some(parent)) = (paths.get(&frn), paths.get(&node.parent)) else {
            continue;
        };
        batch.push(Entry {
            path: path.clone(),
            name: node.name.clone(),
            parent: parent.clone(),
            is_dir: node.is_dir,
        });
        if batch.len() >= UPSERT_BATCH {
            store.upsert(&batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        store.upsert(&batch)?;
    }
    Ok(())
}

/// Decodes the packed record run that follows a CTL's 8-byte position field.
/// Stops at the first record it cannot vouch for — a zero length, a truncated
/// tail, an unknown version — rather than guessing the stride.
fn decode_records(payload: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let mut offset = 0;
    while payload.len() - offset >= size_of::<USN_RECORD_V2>() {
        let length = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        if length == 0 || payload.len() - offset < length {
            break;
        }
        let major = u16::from_le_bytes(payload[offset + 4..offset + 6].try_into().unwrap());
        let decoded = match major {
            3 => decode_v3(payload, offset, length),
            2 => decode_v2(payload, offset, length),
            _ => None,
        };
        let Some(record) = decoded else {
            break;
        };
        records.push(record);
        offset += length;
    }
    records
}

/// One V2 record: 64-bit FRNs, the shape NTFS answers with.
fn decode_v2(payload: &[u8], offset: usize, length: usize) -> Option<Record> {
    if length < size_of::<USN_RECORD_V2>() {
        return None;
    }
    // SAFETY: `offset` was bounds-checked against the payload, and the
    // reads are unaligned because records are byte-packed.
    let base = unsafe { payload.as_ptr().add(offset) };
    let record = unsafe { ptr::read_unaligned(base as *const USN_RECORD_V2) };
    let name_offset = record.FileNameOffset as usize;
    let name_length = record.FileNameLength as usize;
    if name_offset + name_length > length {
        return None;
    }
    let name = decode_name(unsafe { base.add(name_offset) }, name_length);
    Some(Record {
        frn: record.FileReferenceNumber,
        parent: record.ParentFileReferenceNumber,
        name,
        is_dir: record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        reason: record.Reason,
    })
}

/// One V3 record: 128-bit FRNs of which NTFS fills the low 64 bits, so the
/// reference numbers read the same as V2's.
fn decode_v3(payload: &[u8], offset: usize, length: usize) -> Option<Record> {
    if length < size_of::<USN_RECORD_V3>() {
        return None;
    }
    // SAFETY: `offset` was bounds-checked against the payload; the read is
    // unaligned because records are byte-packed.
    let base = unsafe { payload.as_ptr().add(offset) };
    let record = unsafe { ptr::read_unaligned(base as *const USN_RECORD_V3) };
    let name_offset = record.FileNameOffset as usize;
    let name_length = record.FileNameLength as usize;
    if name_offset + name_length > length {
        return None;
    }
    let name = decode_name(unsafe { base.add(name_offset) }, name_length);
    Some(Record {
        frn: u64::from_le_bytes(
            record.FileReferenceNumber.Identifier[..8]
                .try_into()
                .unwrap(),
        ),
        parent: u64::from_le_bytes(
            record.ParentFileReferenceNumber.Identifier[..8]
                .try_into()
                .unwrap(),
        ),
        name,
        is_dir: record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        reason: record.Reason,
    })
}

/// The record's name as UTF-16, decoded lossily: a filename the volume
/// accepted is decoded the same way the shell would show it.
fn decode_name(base: *const u8, length: usize) -> String {
    // SAFETY: `length` bytes at `base` were bounds-checked by the caller.
    let bytes = unsafe { slice::from_raw_parts(base, length) };
    String::from_utf16_lossy(
        &bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<u16>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape a real NTFS root record has: MFT record 5 with its
    /// sequence-number prefix.
    const ROOT: u64 = 0x5000_0000_0000_0005;

    fn node(parent: u64, name: &str, is_dir: bool) -> Node {
        Node {
            parent,
            name: name.to_owned(),
            is_dir,
        }
    }

    fn tree_of(entries: &[(u64, u64, &str, bool)]) -> FrnTree {
        entries
            .iter()
            .map(|&(frn, parent, name, is_dir)| (frn, node(parent, name, is_dir)))
            .collect()
    }

    fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// A V2 record packed the way NTFS lays it out: fixed 60-byte head, name
    /// at the end, length padded to 8.
    fn pack_v2(frn: u64, parent: u64, attributes: u32, reason: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let length = (60 + units.len() * 2 + 7) & !7;
        let mut bytes = vec![0u8; length];
        put_u32(&mut bytes, 0, length as u32);
        put_u16(&mut bytes, 4, 2);
        put_u64(&mut bytes, 8, frn);
        put_u64(&mut bytes, 16, parent);
        put_u32(&mut bytes, 40, reason);
        put_u32(&mut bytes, 52, attributes);
        put_u16(&mut bytes, 56, (units.len() * 2) as u16);
        put_u16(&mut bytes, 58, 60);
        for (at, unit) in units.iter().enumerate() {
            put_u16(&mut bytes, 60 + at * 2, *unit);
        }
        bytes
    }

    /// A V3 record: same fields, but the 128-bit reference numbers push
    /// everything after them by 16 bytes.
    fn pack_v3(frn: u64, parent: u64, attributes: u32, reason: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let length = (76 + units.len() * 2 + 7) & !7;
        let mut bytes = vec![0u8; length];
        put_u32(&mut bytes, 0, length as u32);
        put_u16(&mut bytes, 4, 3);
        // The 128-bit reference numbers carry the 64-bit value in their low
        // half; the high half stays zero.
        bytes[8..16].copy_from_slice(&frn.to_le_bytes());
        bytes[24..32].copy_from_slice(&parent.to_le_bytes());
        put_u32(&mut bytes, 56, reason);
        put_u32(&mut bytes, 68, attributes);
        put_u16(&mut bytes, 72, (units.len() * 2) as u16);
        put_u16(&mut bytes, 74, 76);
        for (at, unit) in units.iter().enumerate() {
            put_u16(&mut bytes, 76 + at * 2, *unit);
        }
        bytes
    }

    #[test]
    fn resolves_a_nested_path_from_the_root_down() {
        let tree = tree_of(&[
            (ROOT, ROOT, "", true),
            (0x5000_0000_0000_0011, ROOT, "Users", true),
            (0x5000_0000_0000_0012, 0x5000_0000_0000_0011, "me", true),
            (
                0x5000_0000_0000_0013,
                0x5000_0000_0000_0012,
                "notes.txt",
                false,
            ),
        ]);
        let path = resolve_path("C:\\", ROOT, 0x5000_0000_0000_0013, &tree).unwrap();
        assert_eq!(path, "C:\\Users\\me\\notes.txt");
    }

    #[test]
    fn resolves_the_root_to_the_volume_path() {
        let tree = tree_of(&[(ROOT, ROOT, "", true)]);
        assert_eq!(resolve_path("C:\\", ROOT, ROOT, &tree).unwrap(), "C:\\");
    }

    #[test]
    fn an_orphan_never_reaches_the_root() {
        // `$MFT` and friends: parent 0, a chain that starts nowhere.
        let tree = tree_of(&[
            (ROOT, ROOT, "", true),
            (0x5000_0000_0000_0000, 0, "$MFT", false),
        ]);
        assert_eq!(
            resolve_path("C:\\", ROOT, 0x5000_0000_0000_0000, &tree),
            None
        );
    }

    #[test]
    fn a_missing_ancestor_has_no_path() {
        let tree = tree_of(&[(ROOT, ROOT, "", true)]);
        assert_eq!(
            resolve_path("C:\\", ROOT, 0x5000_0000_0000_0042, &tree),
            None
        );
    }

    #[test]
    fn a_corrupt_parent_loop_has_no_path() {
        let tree = tree_of(&[
            (ROOT, ROOT, "", true),
            (0x5000_0000_0000_0021, 0x5000_0000_0000_0022, "a", true),
            (0x5000_0000_0000_0022, 0x5000_0000_0000_0021, "b", true),
        ]);
        assert_eq!(
            resolve_path("C:\\", ROOT, 0x5000_0000_0000_0021, &tree),
            None
        );
        assert_eq!(
            resolve_path("C:\\", ROOT, 0x5000_0000_0000_0022, &tree),
            None
        );
    }

    #[test]
    fn resolve_tree_answers_every_reachable_node() {
        let orphan = 0x5000_0000_0000_0000;
        let tree = tree_of(&[
            (ROOT, ROOT, "", true),
            (0x5000_0000_0000_0011, ROOT, "Users", true),
            (0x5000_0000_0000_0012, 0x5000_0000_0000_0011, "me", true),
            (
                0x5000_0000_0000_0013,
                0x5000_0000_0000_0012,
                "notes.txt",
                false,
            ),
            (orphan, 0, "$MFT", false),
        ]);
        let paths = resolve_tree("C:\\", ROOT, &tree);
        assert_eq!(paths[&ROOT], "C:\\");
        assert_eq!(paths[&0x5000_0000_0000_0011], "C:\\Users");
        assert_eq!(paths[&0x5000_0000_0000_0012], "C:\\Users\\me");
        assert_eq!(paths[&0x5000_0000_0000_0013], "C:\\Users\\me\\notes.txt");
        assert!(!paths.contains_key(&orphan));
    }

    #[test]
    fn resolve_tree_poisons_only_a_loop_s_own_chain() {
        let healthy = 0x5000_0000_0000_0013;
        let tree = tree_of(&[
            (ROOT, ROOT, "", true),
            (0x5000_0000_0000_0021, 0x5000_0000_0000_0022, "a", true),
            (0x5000_0000_0000_0022, 0x5000_0000_0000_0021, "b", true),
            (healthy, ROOT, "notes.txt", false),
        ]);
        let paths = resolve_tree("C:\\", ROOT, &tree);
        assert_eq!(paths[&healthy], "C:\\notes.txt");
        assert!(!paths.contains_key(&0x5000_0000_0000_0021));
        assert!(!paths.contains_key(&0x5000_0000_0000_0022));
    }

    #[test]
    fn finds_the_record_that_points_at_itself() {
        let other_root = 0x6000_0000_0000_0005;
        let tree = tree_of(&[
            (other_root, other_root, "second", true),
            (ROOT, ROOT, "", true),
        ]);
        assert_eq!(find_root_frn(&tree), Some(ROOT));
    }

    #[test]
    fn falls_back_to_mft_record_five_without_a_self_parent() {
        // A volume whose root record carries some other parent still names
        // itself by its MFT record number.
        let record_five = 0x7000_0000_0000_0005;
        let tree = tree_of(&[
            (record_five, 0x7000_0000_0000_0009, "Volume", true),
            (0x5000_0000_0000_0042, 0, "$MFT", false),
        ]);
        assert_eq!(find_root_frn(&tree), Some(record_five));
    }

    #[test]
    fn gives_up_when_no_root_shape_exists() {
        let tree = tree_of(&[(0x5000_0000_0000_0042, 0, "$MFT", false)]);
        assert_eq!(find_root_frn(&tree), None);
    }

    #[test]
    fn a_delete_outranks_its_companion_reasons() {
        use windows::Win32::System::Ioctl::USN_REASON_CLOSE;

        // Created and deleted before its close: still gone.
        let reason = USN_REASON_FILE_CREATE | USN_REASON_FILE_DELETE | USN_REASON_CLOSE;
        assert_eq!(
            classify_record(reason),
            Plan {
                delete: true,
                ..Plan::default()
            }
        );
    }

    #[test]
    fn the_old_half_of_a_rename_only_removes() {
        assert_eq!(
            classify_record(USN_REASON_RENAME_OLD_NAME),
            Plan {
                remove_old: true,
                ..Plan::default()
            }
        );
    }

    #[test]
    fn the_new_half_and_a_creation_upsert() {
        assert_eq!(
            classify_record(USN_REASON_RENAME_NEW_NAME),
            Plan {
                upsert: true,
                ..Plan::default()
            }
        );
        assert_eq!(
            classify_record(USN_REASON_FILE_CREATE),
            Plan {
                upsert: true,
                ..Plan::default()
            }
        );
    }

    #[test]
    fn a_rename_that_reports_both_halves_at_once_does_each() {
        let plan = classify_record(USN_REASON_RENAME_OLD_NAME | USN_REASON_RENAME_NEW_NAME);
        assert_eq!(plan.remove_old, true);
        assert_eq!(plan.upsert, true);
        assert_eq!(plan.delete, false);
    }

    #[test]
    fn content_churn_is_ignored() {
        use windows::Win32::System::Ioctl::{
            USN_REASON_BASIC_INFO_CHANGE, USN_REASON_CLOSE, USN_REASON_DATA_OVERWRITE,
        };
        let reason = USN_REASON_DATA_OVERWRITE | USN_REASON_BASIC_INFO_CHANGE | USN_REASON_CLOSE;
        assert_eq!(classify_record(reason), Plan::default());
    }

    #[test]
    fn decodes_a_run_of_v2_records() {
        let first = pack_v2(
            0x5000_0000_0000_0011,
            ROOT,
            FILE_ATTRIBUTE_DIRECTORY.0,
            USN_REASON_FILE_CREATE,
            "Users",
        );
        let second = pack_v2(
            0x5000_0000_0000_0013,
            0x5000_0000_0000_0011,
            0,
            USN_REASON_FILE_CREATE,
            "notes.txt",
        );
        let mut payload = first;
        payload.extend_from_slice(&second);

        let records = decode_records(&payload);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].frn, 0x5000_0000_0000_0011);
        assert_eq!(records[0].parent, ROOT);
        assert_eq!(records[0].name, "Users");
        assert!(records[0].is_dir);
        assert_eq!(records[0].reason, USN_REASON_FILE_CREATE);
        assert_eq!(records[1].name, "notes.txt");
        assert!(!records[1].is_dir);
    }

    #[test]
    fn decodes_v3_records() {
        let payload = pack_v3(
            0x5000_0000_0000_0011,
            ROOT,
            FILE_ATTRIBUTE_DIRECTORY.0,
            USN_REASON_RENAME_NEW_NAME,
            "moved.txt",
        );
        let records = decode_records(&payload);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].frn, 0x5000_0000_0000_0011);
        assert_eq!(records[0].parent, ROOT);
        assert_eq!(records[0].name, "moved.txt");
        assert_eq!(records[0].reason, USN_REASON_RENAME_NEW_NAME);
    }

    #[test]
    fn stops_on_a_truncated_tail() {
        let mut payload = pack_v2(
            0x5000_0000_0000_0011,
            ROOT,
            0,
            USN_REASON_FILE_CREATE,
            "notes.txt",
        );
        payload.truncate(payload.len() - 1);
        assert!(decode_records(&payload).is_empty());
    }

    #[test]
    fn ignores_an_unknown_version() {
        let mut payload = pack_v2(
            0x5000_0000_0000_0011,
            ROOT,
            0,
            USN_REASON_FILE_CREATE,
            "notes.txt",
        );
        put_u16(&mut payload, 4, 9);
        assert!(decode_records(&payload).is_empty());
    }

    #[test]
    fn descendants_cover_the_subtree_and_nothing_else() {
        let tree = tree_of(&[
            (5, 5, "", true),
            (10, 5, "a", true),
            (11, 10, "one.txt", false),
            (12, 10, "two.txt", false),
            (20, 5, "b", true),
            (21, 20, "elsewhere.txt", false),
            (30, 0, "$MFT", false),
        ]);
        let subtree: HashSet<u64> = descendants(&tree, 10).into_iter().collect();
        assert_eq!(subtree.len(), 3);
        assert!(subtree.contains(&10));
        assert!(subtree.contains(&11));
        assert!(subtree.contains(&12));
    }

    #[test]
    fn joins_without_doubling_the_root_separator() {
        assert_eq!(join_component("C:\\", "Users"), "C:\\Users");
        assert_eq!(join_component("C:\\Users", "me"), "C:\\Users\\me");
    }
}
