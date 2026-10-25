//! The delivery seam of a streamed listing.
//!
//! Reading a large directory is domain logic; how the batches reach a UI is
//! not. The Tauri shell has two transports — an event carrying
//! [`crate::types::DirectoryEntry`] objects, and a columnar packet over an IPC
//! channel — and a Qt shell would have a third (model inserts). This module
//! holds only the seam: a batch of rows and a target that consumes it.

use crate::fs::entry_codec::{self, ListingRow};
use crate::host::{Event as UiEvent, Host};
use std::sync::Arc;

/// A batch of already-read listing rows, ready for delivery.
pub struct Batch {
    pub stream_id: String,
    pub path: String,
    pub rows: Vec<ListingRow>,
    /// True on the final batch: the listing is complete.
    pub done: bool,
}

/// Where the batches of a streamed listing go.
///
/// Delivery is decided by the shell: Tauri picks an event or a packet channel,
/// a Qt shell would insert into its model. Returning `false` means the receiver
/// is gone and the emitter thread should stop instead of packing batches
/// nobody will read.
pub trait BatchTarget: Send + Sync + 'static {
    fn deliver(&self, batch: Batch) -> bool;
}

/// Delivers batches as `DirectoryEntries` events through the [`Host`] seam.
///
/// Every row is spelled into a full path before it is handed over, because the
/// event consumer has no packet decoder to do that for it. Delivery always
/// reports success: `Host::emit` has no failure channel by design (a `Result`
/// there would make every OS watcher invent a policy for "nobody is
/// listening"), and Tauri's `emit` did not report a missing listener either.
pub struct EventsTarget {
    host: Arc<dyn Host>,
}

impl EventsTarget {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
}

impl BatchTarget for EventsTarget {
    fn deliver(&self, batch: Batch) -> bool {
        let base = entry_codec::base_of(&batch.path);
        self.host.emit(UiEvent::DirectoryEntries {
            stream_id: batch.stream_id,
            path: batch.path,
            entries: batch
                .rows
                .into_iter()
                .map(|row| row.into_entry(&base))
                .collect(),
            done: batch.done,
        });
        true
    }
}
