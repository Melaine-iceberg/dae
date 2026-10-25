//! Columnar packet delivery for streamed listings.
//!
//! This is the half of the old `BatchSink` that only a Tauri webview needs: the
//! explorer's own pane takes batches as raw bytes over an IPC channel, which is
//! the only transport that avoids evaluating a large payload as JavaScript
//! source (a 5 MB listing cost the webview 25 ms of parsing on the paint thread,
//! measured). A Qt shell replaces this with model inserts and deletes the file.
//!
//! `done` rides in the packet header rather than in a field, because the rows
//! cannot express "there are no more of me"; see
//! [`entry_codec::HEADER_FLAG_FINAL`].

use super::entry_codec::{self, HEADER_FLAG_FINAL};
use dae_core::fs::listing::{Batch, BatchTarget};
use tauri::ipc::{Channel, InvokeResponseBody};

/// Delivers batches to a webview channel as packed `DAE1` buffers.
pub struct ChannelTarget(Channel<InvokeResponseBody>);

impl ChannelTarget {
    pub fn new(channel: Channel<InvokeResponseBody>) -> Self {
        Self(channel)
    }
}

impl BatchTarget for ChannelTarget {
    fn deliver(&self, batch: Batch) -> bool {
        let flags = if batch.done { HEADER_FLAG_FINAL } else { 0 };
        self.0
            .send(InvokeResponseBody::Raw(entry_codec::pack_with_header_flags(
                &batch.rows,
                &entry_codec::base_of(&batch.path),
                flags,
            )))
            .is_ok()
    }
}
