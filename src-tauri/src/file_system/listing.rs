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

use super::error::FileSystemError;
use super::local::{self, DirectoryListing, DirectoryListingCursor};
use super::types::{DirectoryEntry, DirectoryView};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::Manager;
use tauri_specta::Event;

/// Entries in the batch `read_directory` answers with. Small enough that a
/// large directory paints long before its walk has finished.
pub const FIRST_BATCH_SIZE: usize = 512;

/// Ceiling for the batches that follow. A directory with 50_000 entries then
/// costs a handful of messages instead of a hundred, while no single message
/// grows large enough to stall the IPC.
pub const MAX_BATCH_SIZE: usize = 8192;

/// Batches that may sit between the reader and the emitter. Bounded so the
/// reader cannot run ahead and hold a whole huge directory in memory.
const PIPELINE_DEPTH: usize = 2;

/// One batch of a streamed listing. The head (`read_directory`'s response) and
/// these batches together form the complete directory.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-entries")]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntriesBatch {
    /// Echoes the `stream_id` the caller passed to `read_directory`.
    pub stream_id: String,
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
    /// True on the final batch: the listing is complete.
    pub done: bool,
}

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

/// Stops an in-flight listing when the view navigates away. Ids the backend no
/// longer knows are ignored: the listing may already have finished.
#[tauri::command]
#[specta::specta]
pub fn cancel_directory_listing(stream_id: String, app: tauri::AppHandle) {
    app.state::<DirectoryListingState>().cancel(&stream_id);
}

/// Opens a streamed listing: reserves `stream_id`, reads the first batch, and
/// hands the unread remainder to a reader thread.
///
/// Blocking — it opens the directory — so callers run it on the blocking pool.
/// A directory that fits in one batch comes back complete, with its
/// reservation already released, and streams nothing.
pub fn open_streamed_listing(
    app: &tauri::AppHandle,
    path: PathBuf,
    stream_id: String,
) -> Result<DirectoryView, FileSystemError> {
    let state = app.state::<DirectoryListingState>();
    // Reserved before the directory is opened so that a cancellation arriving
    // while the head is still being read takes effect instead of being
    // ignored as an unknown id.
    let cancelled = state.register(&stream_id);

    let head = match local::open_directory_listing(path, FIRST_BATCH_SIZE) {
        Ok(head) => head,
        Err(error) => {
            state.forget(&stream_id);
            return Err(error);
        }
    };

    let DirectoryListing { mut view, cursor } = head;
    let Some(cursor) = cursor else {
        state.forget(&stream_id);
        return Ok(view);
    };

    view.stream_id = Some(stream_id.clone());
    spawn_listing_reader(app, stream_id, view.path.clone(), cursor, cancelled);
    Ok(view)
}

/// Streams the unread remainder of a listing as [`DirectoryEntriesBatch`]
/// events, then stops.
///
/// `cancelled` is the flag [`open_streamed_listing`] reserved: the reader
/// checks it before every batch, and the emitter drops queued batches as soon
/// as it is set.
fn spawn_listing_reader(
    app: &tauri::AppHandle,
    stream_id: String,
    path: String,
    cursor: DirectoryListingCursor,
    cancelled: Arc<AtomicBool>,
) {
    let (batch_tx, batch_rx) = sync_channel::<(Vec<DirectoryEntry>, bool)>(PIPELINE_DEPTH);

    // The serialization half: one thread that does nothing but turn batches
    // into events, so the reader never waits on the IPC while it could be
    // reading the next batch off the disk.
    spawn_batch_emitter(
        app.clone(),
        stream_id.clone(),
        path,
        Arc::clone(&cancelled),
        batch_rx,
    );

    // The I/O half: owns the directory iterator until it is exhausted.
    let app = app.clone();
    thread::spawn(move || {
        read_remaining_batches(cursor, batch_tx, &cancelled);
        // Releasing the sender ends the emitter once it drained the batches
        // that were already in flight.
        app.state::<DirectoryListingState>().forget(&stream_id);
    });
}

fn spawn_batch_emitter(
    app: tauri::AppHandle,
    stream_id: String,
    path: String,
    cancelled: Arc<AtomicBool>,
    batches: Receiver<(Vec<DirectoryEntry>, bool)>,
) {
    thread::spawn(move || {
        while let Ok((entries, done)) = batches.recv() {
            // A cancelled listing is one nobody is displaying any more: its
            // remaining batches are dropped instead of serialized.
            if cancelled.load(AtomicOrdering::Acquire) {
                return;
            }

            let _ = DirectoryEntriesBatch {
                stream_id: stream_id.clone(),
                path: path.clone(),
                entries,
                done,
            }
            .emit(&app);

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
    batches: SyncSender<(Vec<DirectoryEntry>, bool)>,
    cancelled: &AtomicBool,
) {
    let mut batch_size = FIRST_BATCH_SIZE;

    loop {
        if cancelled.load(AtomicOrdering::Acquire) {
            return;
        }

        let (entries, exhausted) = cursor.take(batch_size);

        // A failed send means the emitter is gone (the app is shutting down);
        // an exhausted directory means the listing is complete.
        if batches.send((entries, exhausted)).is_err() || exhausted {
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
    use std::fs;

    #[test]
    fn collects_the_unread_remainder_across_batches() {
        let directory =
            std::env::temp_dir().join(format!("dae-listing-split-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        for index in 0..7 {
            fs::write(directory.join(format!("entry-{index}.txt")), "content").expect("write file");
        }

        let listing =
            super::super::local::open_directory_listing(directory.clone(), 2).expect("read head");
        assert_eq!(listing.view.entries.len(), 2, "the head holds one batch");

        let mut cursor = listing.cursor.expect("a directory of 7 entries has a remainder");
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

        let listing =
            super::super::local::open_directory_listing(directory.clone(), 512).expect("read head");
        assert_eq!(listing.view.entries.len(), 1);
        assert!(
            listing.cursor.is_none(),
            "an exhausted directory streams nothing"
        );
        assert!(listing.view.stream_id.is_none(), "a complete view has no id");

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn grows_batches_up_to_the_ceiling() {
        assert_eq!(next_batch_size(FIRST_BATCH_SIZE), FIRST_BATCH_SIZE * 2);
        assert_eq!(next_batch_size(MAX_BATCH_SIZE), MAX_BATCH_SIZE);
        assert_eq!(next_batch_size(MAX_BATCH_SIZE * 4), MAX_BATCH_SIZE);
    }
}
