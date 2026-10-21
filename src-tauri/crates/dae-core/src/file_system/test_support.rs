//! Shared helpers for the file-system unit tests.

use super::progress::FileOperationProgressReporterTrait;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// Counts progress units and asserts on `finish` that the walk reported
/// exactly the `total` it started with.
pub struct TestProgress {
    pub completed: AtomicU64,
    pub total: AtomicU64,
}

impl TestProgress {
    pub fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }
}

impl FileOperationProgressReporterTrait for TestProgress {
    fn start(&self, total: u64) {
        self.total.store(total, AtomicOrdering::Relaxed);
    }

    fn begin_entry(&self, _path: &Path) {}

    fn advance_by(&self, units: u64, _path: &Path) {
        self.completed.fetch_add(units, AtomicOrdering::Relaxed);
    }

    fn finish(&self) {
        assert_eq!(
            self.completed.load(AtomicOrdering::Relaxed),
            self.total.load(AtomicOrdering::Relaxed)
        );
    }
}
