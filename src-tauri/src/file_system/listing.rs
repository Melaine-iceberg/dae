//! Streamed directory listings.
//!
//! A large directory used to cost the explorer three serial steps before it
//! could paint a single row: the whole `read_dir` walk, a sort over every
//! entry, and one large IPC message serialized on the runtime thread.
//! `read_directory` now answers with a first batch right away and hands the
//! unread remainder to this module, which pumps it out as
//! [`DirectoryEntriesBatch`] events while the explorer is already usable.
//!
//! Reading and serialization deliberately live on separate threads. The reader
//! owns nothing but the OS directory iterator, so JSON serialization of batch
//! *n* overlaps with the disk walk for batch *n + 1* instead of delaying it.

use super::entry_codec::ListingRow;
use super::error::FileSystemError;
use super::local::{self, DirectoryListing, DirectoryListingCursor};
use super::types::{DirectoryView, canonical_path};
use super::watch;
use dae_core::fs::listing::{Batch, BatchTarget};
use dae_core::host::Host;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

pub use dae_core::fs::listing::{FIRST_BATCH_SIZE, MAX_BATCH_SIZE};

/// Batches that may sit between the reader and the emitter. Bounded so the
/// reader cannot run ahead and hold a whole huge directory in memory.
const PIPELINE_DEPTH: usize = 2;

/// Live listings keyed by stream id, so navigating away stops the read
/// instead of walking a directory nobody is looking at any more.
#[derive(Default)]
pub struct DirectoryListingState {
    listings: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl DirectoryListingState {
    fn register(&self, stream_id: &str) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.listings
            .lock()
            .expect("directory listing state lock poisoned")
            .insert(stream_id.to_owned(), Arc::clone(&cancelled));
        cancelled
    }

    fn forget(&self, stream_id: &str) {
        self.listings
            .lock()
            .expect("directory listing state lock poisoned")
            .remove(stream_id);
    }

    fn cancel(&self, stream_id: &str) {
        if let Some(cancelled) = self
            .listings
            .lock()
            .expect("directory listing state lock poisoned")
            .remove(stream_id)
        {
            cancelled.store(true, AtomicOrdering::Release);
        }
    }
}

/// See `commands::cancel_directory_listing` for the contract; this half is engine-free.
pub fn cancel_directory_listing(
    stream_id: String,
)  {
    state().cancel(&stream_id);
}

/// The process-wide registry of live listings.
///
/// One app per process, so this is the lifetime Tauri's managed state had. The
/// type stays constructible per test.
fn state() -> &'static DirectoryListingState {
    static STATE: OnceLock<DirectoryListingState> = OnceLock::new();
    STATE.get_or_init(DirectoryListingState::default)
}

/// Opens a streamed listing: reserves `stream_id`, reads the first batch, and
/// hands the unread remainder to a reader thread.
///
/// `watcher_id` names the view this listing belongs to; when present, the
/// directory watcher for `requested_path` is armed under it *before* the first
/// entry is read, so a change landing while the listing runs is either already
/// in the listing or reported as an event. Arming it afterwards — which is how
/// the explorer used to do it, from a separate command — left a window in
/// which a change was invisible to both, and the only way to close it was to
/// re-read the whole directory; that re-read also cancelled the stream, so a
/// large directory snapped back to its first batch until the re-read finished.
/// Reads that do not display a directory (sidebar tree, Miller columns) pass
/// `None` and stay unobserved.
///
/// Blocking — it opens the directory — so callers run it on the blocking pool.
/// A directory that fits in one batch comes back complete, with its
/// reservation already released, and streams nothing.
pub fn open_streamed_listing(
    host: Arc<dyn Host>,
    requested_path: PathBuf,
    stream_id: String,
    watcher_id: Option<String>,
    target: Arc<dyn BatchTarget>,
) -> Result<DirectoryView, FileSystemError> {
    // Reserved before the directory is opened so that a cancellation arriving
    // while the head is still being read takes effect instead of being
    // ignored as an unknown id.
    let cancelled = state().register(&stream_id);

    // Canonicalized once, so the watcher that may be armed below names the
    // same directory the view reports; the frontend matches events against the
    // displayed path.
    let path = match canonical_path(&requested_path) {
        Ok(path) => path,
        Err(error) => {
            state().forget(&stream_id);
            return Err(error.into());
        }
    };

    if let Some(watcher_id) = &watcher_id {
        watch::arm_local_watcher(Arc::clone(&host), path.clone(), watcher_id);
    }

    let head = match local::open_canonical_listing(path, FIRST_BATCH_SIZE) {
        Ok(head) => head,
        Err(error) => {
            state().forget(&stream_id);
            return Err(error);
        }
    };

    let DirectoryListing { mut view, cursor } = head;
    let Some(cursor) = cursor else {
        state().forget(&stream_id);
        return Ok(view);
    };

    view.stream_id = Some(stream_id.clone());
    spawn_listing_reader(stream_id, view.path.clone(), cursor, cancelled, target);
    Ok(view)
}

/// Streams the unread remainder of a listing to `target`, then stops.
///
/// `cancelled` is the flag [`open_streamed_listing`] reserved: the reader
/// checks it before every batch, and the emitter drops queued batches as soon
/// as it is set.
fn spawn_listing_reader(
    stream_id: String,
    path: String,
    cursor: DirectoryListingCursor,
    cancelled: Arc<AtomicBool>,
    target: Arc<dyn BatchTarget>,
) {
    let (batch_tx, batch_rx) = sync_channel::<(Vec<ListingRow>, bool)>(PIPELINE_DEPTH);

    // The serialization half: one thread that does nothing but turn batches
    // into deliveries, so the reader never waits on the IPC while it could be
    // reading the next batch off the disk.
    spawn_batch_emitter(
        stream_id.clone(),
        path,
        Arc::clone(&cancelled),
        target,
        batch_rx,
    );

    // The I/O half: owns the directory iterator until it is exhausted.
    thread::spawn(move || {
        read_remaining_batches(cursor, batch_tx, &cancelled);
        // Releasing the sender ends the emitter once it drained the batches
        // that were already in flight.
        state().forget(&stream_id);
    });
}

fn spawn_batch_emitter(
    stream_id: String,
    path: String,
    cancelled: Arc<AtomicBool>,
    target: Arc<dyn BatchTarget>,
    batches: Receiver<(Vec<ListingRow>, bool)>,
) {
    thread::spawn(move || {
        while let Ok((rows, done)) = batches.recv() {
            // A cancelled listing is one nobody is displaying any more: its
            // remaining batches are dropped instead of serialized.
            if cancelled.load(AtomicOrdering::Acquire) {
                return;
            }

            // A delivery that fails ends the stream: the reader thread would
            // otherwise keep packing batches into a channel nobody holds.
            if !target.deliver(Batch {
                stream_id: stream_id.clone(),
                path: path.clone(),
                rows,
                done,
            }) {
                return;
            }

            if done {
                return;
            }
        }
    });
}

/// Walks the rest of the directory in growing batches. The final batch is
/// always sent, even when empty, so the frontend learns the listing ended.
fn read_remaining_batches(
    mut cursor: DirectoryListingCursor,
    batches: SyncSender<(Vec<ListingRow>, bool)>,
    cancelled: &AtomicBool,
) {
    let mut batch_size = FIRST_BATCH_SIZE;

    loop {
        if cancelled.load(AtomicOrdering::Acquire) {
            return;
        }

        let (rows, exhausted) = cursor.take_rows(batch_size);

        // A failed send means the emitter is gone (the app is shutting down);
        // an exhausted directory means the listing is complete.
        if batches.send((rows, exhausted)).is_err() || exhausted {
            return;
        }

        batch_size = next_batch_size(batch_size);
    }
}

/// Size of the batch after one of `current` entries. Growing the batches keeps
/// the total message count small for huge directories, while the early small
/// batches keep the frontend's per-batch work (sorting, rendering) cheap.
fn next_batch_size(current: usize) -> usize {
    (current * 2).min(MAX_BATCH_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::types::EntryKind;
    use crate::host::testing::RecordingHost;
    use std::fs;

    /// The events target feeds the sidebar and the Miller columns. It has to
    /// spell every row against the directory before handing it over: only the
    /// packet target is allowed to leave that to the webview, because only it
    /// has a decoder on the other end.
    #[test]
    fn an_events_target_spells_the_rows_and_hands_the_batch_to_the_seam() {
        use dae_core::fs::listing::EventsTarget;

        let recorder = Arc::new(RecordingHost::default());
        let host: Arc<dyn Host> = recorder.clone();
        let row = ListingRow {
            name: "a.txt".to_owned(),
            path: None,
            kind: EntryKind::File,
            modified_at: Some(1),
            size: Some(2),
            hidden: false,
            read_only: false,
        };

        let target = EventsTarget::new(host);
        assert!(target.deliver(Batch {
            stream_id: "s-1".to_owned(),
            path: "remote:/dir/".to_owned(),
            rows: vec![row],
            done: true,
        }));

        let events = recorder.take();
        let [
            dae_core::host::Event::DirectoryEntries {
                stream_id,
                path,
                entries,
                done,
            },
        ] = events.as_slice()
        else {
            panic!("expected one listing event, got {events:?}");
        };
        assert_eq!(stream_id, "s-1");
        assert_eq!(path, "remote:/dir/");
        assert!(done);
        assert_eq!(entries[0].path, "remote:/dir/a.txt");
        assert_eq!(entries[0].name, "a.txt");
    }

    #[test]
    fn collects_the_unread_remainder_across_batches() {
        let directory =
            std::env::temp_dir().join(format!("dae-listing-split-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        for index in 0..7 {
            fs::write(directory.join(format!("entry-{index}.txt")), "content").expect("write file");
        }

        let listing = super::super::local::open_canonical_listing(
            canonical_path(&directory).expect("canonicalize the test directory"),
            2,
        )
        .expect("read head");
        assert_eq!(listing.view.entries.len(), 2, "the head holds one batch");

        let mut cursor = listing
            .cursor
            .expect("a directory of 7 entries has a remainder");
        let mut names: Vec<String> = listing
            .view
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();

        // Drain the way the reader thread does: `take` reports exhaustion only
        // on the call after the last entry, so the final pass is empty.
        let mut exhausted = false;
        while !exhausted {
            let (entries, is_last) = cursor.take(3);
            names.extend(entries.into_iter().map(|entry| entry.name));
            exhausted = is_last;
        }

        names.sort();
        assert_eq!(
            names,
            (0..7)
                .map(|index| format!("entry-{index}.txt"))
                .collect::<Vec<_>>()
        );

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn treats_a_directory_that_fits_in_one_batch_as_complete() {
        let directory =
            std::env::temp_dir().join(format!("dae-listing-head-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(directory.join("only.txt"), "content").expect("write file");

        let listing = super::super::local::open_canonical_listing(
            canonical_path(&directory).expect("canonicalize the test directory"),
            512,
        )
        .expect("read head");
        assert_eq!(listing.view.entries.len(), 1);
        assert!(
            listing.cursor.is_none(),
            "an exhausted directory streams nothing"
        );
        assert!(
            listing.view.stream_id.is_none(),
            "a complete view has no id"
        );

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn grows_batches_up_to_the_ceiling() {
        assert_eq!(next_batch_size(FIRST_BATCH_SIZE), FIRST_BATCH_SIZE * 2);
        assert_eq!(next_batch_size(MAX_BATCH_SIZE), MAX_BATCH_SIZE);
        assert_eq!(next_batch_size(MAX_BATCH_SIZE * 4), MAX_BATCH_SIZE);
    }
}
