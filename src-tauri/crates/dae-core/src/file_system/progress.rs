//! The progress contract the transfer engine reports through. The shell owns
//! the event type and the emitter; the core only knows this trait.

use std::path::Path;

pub trait FileOperationProgressReporterTrait: Send + Sync {
    fn start(&self, total: u64);
    fn begin_entry(&self, path: &Path);
    fn advance(&self, path: &Path) {
        self.advance_by(1, path);
    }
    fn advance_by(&self, units: u64, path: &Path);
    fn finish(&self);
}
