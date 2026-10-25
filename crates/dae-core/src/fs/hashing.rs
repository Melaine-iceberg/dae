//! Streaming file checksum calculation for the properties dialog.
//!
//! Verifying a download means hashing the whole file, which can take a while
//! on large files, so the backend reads the file once in chunks, updates all
//! three digests (MD5, SHA-1, SHA-256) in a single pass, and streams progress
//! events until the final digests arrive — mirroring the folder-size scan.

use super::error::FileSystemError;
use super::types::path_to_string;
use super::vfs;
use sha2::Digest;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::host::{Event as UiEvent, Host};
use std::sync::OnceLock;

/// How often partial byte counts are pushed to the UI while the read runs.
const HASH_PROGRESS_INTERVAL_MS: u64 = 200;
/// One mebibyte per read keeps the digest loop fast without over-buffering.
const HASH_CHUNK_SIZE: usize = 1024 * 1024;

pub use crate::hashing::FileHashDigests;

/// Live hash runs keyed by operation id, so closing the dialog stops a run.
#[derive(Default)]
pub struct FileHashState {
    operations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl FileHashState {
    fn register(&self, operation_id: &str) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.operations
            .lock()
            .expect("file hash state lock poisoned")
            .insert(operation_id.to_owned(), Arc::clone(&cancelled));
        cancelled
    }

    fn forget(&self, operation_id: &str) {
        self.operations
            .lock()
            .expect("file hash state lock poisoned")
            .remove(operation_id);
    }

    fn cancel(&self, operation_id: &str) {
        if let Some(cancelled) = self
            .operations
            .lock()
            .expect("file hash state lock poisoned")
            .remove(operation_id)
        {
            cancelled.store(true, AtomicOrdering::Release);
        }
    }
}

/// See `commands::start_file_hash_calculation` for the contract; this half is engine-free.
pub fn start_file_hash_calculation(
    operation_id: String,
    path: String,
    host: std::sync::Arc<dyn crate::host::Host>,
) -> Result<(), FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::Unsupported(path));
    }

    let file_path = PathBuf::from(&path);
    if !file_path.is_file() {
        return Err(FileSystemError::InvalidInput(path));
    }

    let cancelled = state().register(&operation_id);
    let done_operation_id = operation_id.clone();

    std::thread::spawn(move || {
        hash_and_report(host.as_ref(), &operation_id, &file_path, &cancelled);
        state().forget(&done_operation_id);
    });

    Ok(())
}

/// See `commands::cancel_file_hash_calculation` for the contract; this half is engine-free.
pub fn cancel_file_hash_calculation(
    operation_id: String,
)  {
    state().cancel(&operation_id);
}

/// The process-wide registry of hash runs. One app per process, so this is the
/// lifetime the managed state had; tests keep constructing their own.
fn state() -> &'static FileHashState {
    static STATE: OnceLock<FileHashState> = OnceLock::new();
    STATE.get_or_init(FileHashState::default)
}

/// Runs the digest pass and emits progress plus the final report. Cancelled
/// runs end quietly — the requester is already gone.
fn hash_and_report(
    host: &dyn Host,
    operation_id: &str,
    path: &Path,
    cancelled: &AtomicBool,
) {
    let mut last_emit_ms = 0u64;
    let mut emit = |bytes_read: u64,
                    total_bytes: u64,
                    completed: bool,
                    digests: Option<FileHashDigests>,
                    error: Option<String>,
                    force: bool| {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);

        if !force && now_ms < last_emit_ms + HASH_PROGRESS_INTERVAL_MS {
            return;
        }
        last_emit_ms = now_ms;

        host.emit(UiEvent::FileHashProgress {
            operation_id: operation_id.to_owned(),
            path: path_to_string(path),
            bytes_read,
            total_bytes,
            completed,
            digests,
            error,
        });
    };

    match hash_file(path, cancelled, &mut |bytes_read, total_bytes| {
        emit(bytes_read, total_bytes, false, None, None, false);
    }) {
        Ok(Some(digests)) => {
            let total = digests_len_hint(path);
            emit(total, total, true, Some(digests), None, true);
        }
        Ok(None) => {} // Cancelled: no final report.
        Err(error) => {
            let total = digests_len_hint(path);
            emit(total, total, true, None, Some(error.to_string()), true);
        }
    }
}

/// Best-effort file length for the final report after a failed read.
fn digests_len_hint(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Streams `path` through all three digesters in one pass, reporting the
/// running byte count to `progress` after every chunk. Returns `Ok(None)`
/// when `cancelled` trips mid-read, so callers can skip the final report.
pub(super) fn hash_file(
    path: &Path,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> io::Result<Option<FileHashDigests>> {
    let mut file = File::open(path)?;
    let total_bytes = file.metadata()?.len();

    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK_SIZE];
    let mut bytes_read = 0u64;

    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Ok(None);
        }

        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        md5.update(&buffer[..count]);
        sha1.update(&buffer[..count]);
        sha256.update(&buffer[..count]);
        bytes_read += count as u64;
        progress(bytes_read, total_bytes);
    }

    Ok(Some(FileHashDigests {
        md5: lowercase_hex(&md5.finalize()),
        sha1: lowercase_hex(&sha1.finalize()),
        sha256: lowercase_hex(&sha256.finalize()),
    }))
}

/// Lowercase hex without any crate dependency. digest 0.11 removed the
/// `LowerHex` impl that `format!("{:x}", …)` used to rely on.
fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The well-known digests of the three-byte input "abc" — a quick
    /// regression guard for the single-pass multi-digest wiring.
    #[test]
    fn hashes_file_content_with_all_three_algorithms() {
        let path = std::env::temp_dir().join(format!("dae-hash-test-{}.bin", std::process::id()));
        fs::write(&path, "abc").expect("write test file");

        let cancelled = AtomicBool::new(false);
        let mut last = (0u64, 0u64);
        let digests = hash_file(&path, &cancelled, &mut |read, total| last = (read, total))
            .expect("hash test file")
            .expect("completed run");

        assert_eq!(digests.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(digests.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            digests.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(last, (3, 3));

        fs::remove_file(path).expect("remove test file");
    }

    /// Hashing runs over multiple chunks on files larger than one chunk, so
    /// the multi-chunk path must produce the same digest as a one-shot hash.
    #[test]
    fn hashes_multi_chunk_files_like_a_single_pass() {
        let path = std::env::temp_dir().join(format!("dae-hash-chunks-{}.bin", std::process::id()));
        // Two and a half chunks of deterministic pseudo-random bytes.
        let content: Vec<u8> = (0..HASH_CHUNK_SIZE * 2 + HASH_CHUNK_SIZE / 2)
            .map(|index| (index * 31 + 7) as u8)
            .collect();
        fs::write(&path, &content).expect("write test file");

        let expected_sha256 = lowercase_hex(&sha2::Sha256::digest(&content));

        let cancelled = AtomicBool::new(false);
        let mut reports = 0u32;
        let digests = hash_file(&path, &cancelled, &mut |_, _| reports += 1)
            .expect("hash test file")
            .expect("completed run");

        assert_eq!(digests.sha256, expected_sha256);
        assert!(reports >= 3, "expected one progress report per chunk");

        fs::remove_file(path).expect("remove test file");
    }

    #[test]
    fn cancels_hash_runs_before_they_start() {
        let path = std::env::temp_dir().join(format!("dae-hash-cancel-{}.bin", std::process::id()));
        fs::write(&path, "content").expect("write test file");

        let cancelled = AtomicBool::new(true);
        let mut reports = 0u32;
        let result = hash_file(&path, &cancelled, &mut |_, _| reports += 1);

        assert!(matches!(result, Ok(None)));
        assert_eq!(reports, 0);

        fs::remove_file(path).expect("remove test file");
    }
}
