//! Streamed directory listings on top of `dae-core`.
//!
//! The head of a listing is returned synchronously, exactly like the shell's
//! `read_directory`; the remainder is produced on a reader thread and delivered
//! either as `explorer-directory-entries` events or as packed packets on a
//! channel. Both transports describe the same rows: the event path is plain
//! JSON, the packet path is `dae_core::file_system::entry_codec`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use dae_core::file_system::entry_codec;
use dae_core::file_system::local;
use dae_core::file_system::types::{DirectoryEntry, DirectoryView, canonical_path};
use dae_core::file_system::vfs;
use serde::Serialize;

use crate::{emit_channel, emit_event};

/// The head is deliberately small: the first screenful paints from it while the
/// rest of the directory is still being enumerated.
const FIRST_BATCH_SIZE: usize = 512;
const MAX_BATCH_SIZE: usize = 8192;
const EVENT_NAME: &str = "explorer-directory-entries";

#[derive(Clone, Copy)]
pub enum Sink {
    Events,
    Channel(u32),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntriesBatch {
    stream_id: String,
    path: String,
    entries: Vec<DirectoryEntry>,
    done: bool,
}

fn listings() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static LISTINGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    LISTINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register(stream_id: &str) -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut listings = listings().lock().unwrap();
    // A reused stream id is a new read; the old reader is told to stop by being
    // replaced, and its next cancellation check will observe `true` only if we
    // flip its own flag. Reusing ids is not expected, but replacing the entry
    // keeps a stale reader from deleting the new one on completion.
    if let Some(previous) = listings.insert(stream_id.to_owned(), cancelled.clone()) {
        previous.store(true, Ordering::Relaxed);
    }
    cancelled
}

fn forget(stream_id: &str, cancelled: &Arc<AtomicBool>) {
    let mut listings = listings().lock().unwrap();
    if listings
        .get(stream_id)
        .is_some_and(|current| Arc::ptr_eq(current, cancelled))
    {
        listings.remove(stream_id);
    }
}

pub fn cancel(stream_id: &str) {
    if let Some(cancelled) = listings().lock().unwrap().get(stream_id) {
        cancelled.store(true, Ordering::Relaxed);
    }
}

/// Opens a listing and schedules the remaining batches. `requested_path` is the
/// spelling the frontend navigated to; the returned view carries dae-core's
/// canonical spelling, as the shell does.
pub fn open(
    requested_path: String,
    stream_id: String,
    sink: Sink,
) -> Result<DirectoryView, String> {
    if !vfs::is_local_path(&requested_path) {
        // Remote backends are resolved through vfs and are never streamed: the
        // renderer only reaches local places in this slice of the migration.
        let backend = vfs::resolve(&requested_path).map_err(|error| error.to_string())?;
        return backend
            .read_dir(&requested_path)
            .map_err(|error| error.to_string());
    }

    let cancelled = register(&stream_id);
    let canonical = match canonical_path(&PathBuf::from(&requested_path)) {
        Ok(path) => path,
        Err(error) => {
            forget(&stream_id, &cancelled);
            return Err(error.to_string());
        }
    };
    let mut listing = match local::open_canonical_listing(canonical, FIRST_BATCH_SIZE) {
        Ok(listing) => listing,
        Err(error) => {
            forget(&stream_id, &cancelled);
            return Err(error.to_string());
        }
    };

    let Some(cursor) = listing.cursor.take() else {
        forget(&stream_id, &cancelled);
        return Ok(listing.view);
    };

    listing.view.stream_id = Some(stream_id.clone());
    let path = listing.view.path.clone();
    let stream = stream_id.clone();
    thread::Builder::new()
        .name("dae-directory-stream".into())
        .spawn(move || {
            stream_batches(stream, path, cursor, sink, cancelled.clone());
            forget(&stream_id, &cancelled);
        })
        .map_err(|error| error.to_string())?;

    Ok(listing.view)
}

fn stream_batches(
    stream_id: String,
    path: String,
    mut cursor: local::DirectoryListingCursor,
    sink: Sink,
    cancelled: Arc<AtomicBool>,
) {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }

        let (rows, exhausted) = cursor.take_rows(MAX_BATCH_SIZE);
        let done = exhausted;
        match sink {
            Sink::Events => {
                let base = entry_codec::base_of(&path);
                let entries = rows.into_iter().map(|row| row.into_entry(&base)).collect();
                let batch = EntriesBatch {
                    stream_id: stream_id.clone(),
                    path: path.clone(),
                    entries,
                    done,
                };
                if let Ok(payload) = serde_json::to_string(&batch) {
                    emit_event(EVENT_NAME, payload);
                }
            }
            Sink::Channel(channel) => {
                let base = entry_codec::base_of(&path);
                let flags = if done {
                    entry_codec::HEADER_FLAG_FINAL
                } else {
                    0
                };
                emit_channel(
                    channel,
                    None,
                    Some(entry_codec::pack_with_header_flags(&rows, &base, flags)),
                );
            }
        }

        if done {
            return;
        }
    }
}

/// Re-stat the named children after a watcher notification. The renderer sends
/// names rather than paths so the backend never has to trust a path prefix.
pub fn changes(directory: String, names: Vec<String>) -> Result<Vec<Option<DirectoryEntry>>, String> {
    local::stat_named_children(std::path::Path::new(&directory), names)
        .map_err(|error| error.to_string())
}
