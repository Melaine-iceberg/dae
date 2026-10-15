//! The I/O operations this benchmark measures, and the two ways of running
//! them: blocking calls on a thread pool, and completion-based async calls.
//!
//! One [`Op`] is one unit of work a file manager performs on one path — read a
//! slice, read a file whole, copy a file, list a directory. Keeping the
//! operations backend-agnostic is what makes the comparison honest: every
//! backend runs the *same* op list, on the same paths, in the same order and
//! with the same concurrency. Only the dispatch mechanism differs.
//!
//! Positional reads carry a handle index rather than a path, so a read can be
//! issued against a handle opened once, outside the measured region. That is
//! how the app reads (a listing keeps its directory handle; previews and the
//! properties dialog keep their file open) and it keeps `CreateFileW` out of
//! the numbers, which would otherwise dominate a 4 KiB read.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// Buffer size for the streaming paths (copy, whole-file write). 1 MiB is the
/// same order as the app's `STREAM_CHUNK_BYTES`.
pub const CHUNK: usize = 1 << 20;

#[derive(Clone, Debug)]
pub enum Op {
    /// Do nothing. Used by the `dispatch` scenario to price one operation's
    /// dispatch — the Tokio blocking pool's handoff is a measurable part of
    /// what `spawn_blocking`-per-command costs, and it is invisible in any
    /// workload where the work itself dominates.
    Nop,
    /// Read `len` bytes at `offset` from the handle at `handles[file]`.
    ReadAt {
        file: usize,
        offset: u64,
        len: usize,
    },
    /// Open `path` and read it to EOF — the preview / thumbnail / checksum
    /// shape.
    ReadWhole { path: Arc<PathBuf> },
    /// Create `path` and write `len` bytes into it.
    WriteWhole { path: Arc<PathBuf>, len: u64 },
    /// One `read_dir` walk of `path`, calling `metadata` on every entry, which
    /// is what an explorer listing costs on the filesystem side.
    ListDir { path: Arc<PathBuf> },
    /// Copy one file. The blocking backends reach for `std::fs::copy` (which is
    /// `CopyFileExW` on Windows); the completion backend has no equivalent and
    /// reads and writes instead — see `copy_mode`.
    Copy {
        from: Arc<PathBuf>,
        to: Arc<PathBuf>,
    },
}

/// Pre-opened handles for [`Op::ReadAt`].
pub struct SyncPool {
    files: Vec<File>,
}

impl Default for SyncPool {
    fn default() -> Self {
        Self { files: Vec::new() }
    }
}

impl SyncPool {
    pub fn open(paths: &[PathBuf]) -> io::Result<Self> {
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            files.push(File::open(path)?);
        }
        Ok(Self { files })
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    // `seek_read` is `ReadFile` with an explicit `OVERLAPPED.Offset`: it neither
    // moves the handle's file pointer nor serializes against other threads
    // reading the same handle, so N threads reading one handle really are N
    // outstanding reads — which is the number the completion backend is judged
    // against.
    file.seek_read(buf, offset)
}

/// Runs one op with blocking calls. Returns the op's result: bytes read or
/// written, or entries listed.
pub fn run_sync(pool: &SyncPool, op: &Op) -> io::Result<u64> {
    match op {
        Op::Nop => Ok(0),
        Op::ReadAt { file, offset, len } => {
            let mut buf = vec![0_u8; *len];
            // A short read at EOF is a valid result, not an error: the op lists
            // are built from the real file sizes, so this is only exercised when
            // a file is truncated underneath the run.
            let read = read_at(&pool.files[*file], &mut buf, *offset)?;
            Ok(read as u64)
        }
        Op::ReadWhole { path } => {
            let mut file = File::open(&**path)?;
            let mut buf = Vec::with_capacity(CHUNK);
            file.read_to_end(&mut buf)?;
            Ok(buf.len() as u64)
        }
        Op::WriteWhole { path, len } => {
            let mut file = File::create(&**path)?;
            let block = vec![0x5A_u8; CHUNK];
            let mut written = 0_u64;
            while written < *len {
                let want = (*len - written).min(CHUNK as u64) as usize;
                file.write_all(&block[..want])?;
                written += want as u64;
            }
            file.flush()?;
            Ok(written)
        }
        Op::ListDir { path } => {
            let mut entries = 0_u64;
            for entry in std::fs::read_dir(&**path)? {
                let entry = entry?;
                // The app asks for the metadata of every row it renders. On
                // Windows this reads `WIN32_FIND_DATA` the iterator already
                // holds, so it is free there and one extra syscall elsewhere.
                let _ = entry.metadata()?;
                entries += 1;
            }
            Ok(entries)
        }
        Op::Copy { from, to } => {
            let copied = std::fs::copy(&**from, &**to)?;
            Ok(copied)
        }
    }
}

/// Completion-based counterpart of [`run_sync`].
///
/// `compio` on Windows is a real IOCP driver: files are opened with
/// `FILE_FLAG_OVERLAPPED` and reads are `ReadFile` with an `OVERLAPPED`, whose
/// completion is queued to the port. On Linux the same code path is `io_uring`.
/// Nothing in this file is `cfg`-switched, so the two platforms run the same
/// benchmark.
pub struct CompioPool {
    files: Vec<compio::fs::File>,
}

impl CompioPool {
    pub async fn open(paths: &[PathBuf]) -> io::Result<Self> {
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            files.push(compio::fs::File::open(path).await?);
        }
        Ok(Self { files })
    }
}

use compio::BufResult;
use compio::io::{AsyncReadAt, AsyncReadAtExt, AsyncWriteAtExt};

pub async fn run_compio(pool: Arc<CompioPool>, op: Op) -> io::Result<u64> {
    match op {
        Op::Nop => Ok(0),
        Op::ReadAt { file, offset, len } => {
            let buf = vec![0_u8; len];
            let BufResult(result, _buf) = pool.files[file].read_at(buf, offset).await;
            Ok(result? as u64)
        }
        Op::ReadWhole { path } => {
            let file = compio::fs::File::open(&*path).await?;
            let BufResult(read, buf) = file.read_to_end_at(Vec::with_capacity(CHUNK), 0).await;
            let read = read?;
            debug_assert_eq!(read, buf.len());
            Ok(read as u64)
        }
        Op::WriteWhole { path, len } => {
            let mut file = compio::fs::File::create(&*path).await?;
            let mut block = vec![0x5A_u8; CHUNK];
            let mut written = 0_u64;
            while written < len {
                let want = (len - written).min(CHUNK as u64) as usize;
                // `truncate`/`resize` only ever move the vector's length inside
                // its capacity, so the buffer is reused rather than reallocated
                // per chunk. Only the final chunk takes the shrink branch.
                block.truncate(want);
                let BufResult(result, returned) = file.write_all_at(block, written).await;
                result?;
                written += want as u64;
                block = returned;
                block.resize(CHUNK, 0x5A);
            }
            Ok(written)
        }
        Op::ListDir { path } => {
            // There is no async directory iteration to be had: `compio-fs`
            // 0.12.1 exposes no `read_dir` at all, and Windows has no
            // completion-based directory enumeration to bind to in the first
            // place (`FindFirstFileW`/`FindNextFileW` are synchronous APIs).
            // A real port therefore has to fall back to a blocking pool here,
            // which is what this does — and what makes the enumerate numbers
            // identical across backends.
            let handle = compio::runtime::spawn_blocking(move || {
                let pool = SyncPool::default();
                run_sync(&pool, &Op::ListDir { path })
            });
            Ok(handle
                .await
                .map_err(|error| io::Error::other(error.to_string()))??)
        }
        Op::Copy { from, to } => {
            if copy_mode() == CopyMode::Kernel {
                // What a pragmatic port does: keep `CopyFileExW` and pay for a
                // blocking pool thread. `run_sync` above is the same call.
                let handle = compio::runtime::spawn_blocking(move || {
                    let pool = SyncPool::default();
                    run_sync(&pool, &Op::Copy { from, to })
                });
                return Ok(handle
                    .await
                    .map_err(|error| io::Error::other(error.to_string()))??);
            }

            let source = compio::fs::File::open(&*from).await?;
            let size = source.metadata().await?.len();
            let mut dest = compio::fs::File::create(&*to).await?;

            let mut block = vec![0_u8; CHUNK];
            let mut position = 0_u64;
            while position < size {
                // `read_at`, not `read_exact_at`: the latter sizes its loop from
                // the buffer's *capacity*, so a recycled 1 MiB buffer whose
                // length has been trimmed to the last short chunk makes it wait
                // for bytes that are not there. `read_at` respects the length,
                // and the short-read branch below is what a copy wants anyway.
                let BufResult(result, returned) = source.read_at(block, position).await;
                let read = result?;
                if read == 0 {
                    break;
                }
                block = returned;
                block.truncate(read);
                let BufResult(result, returned) = dest.write_all_at(block, position).await;
                result?;
                position += read as u64;
                block = returned;
                block.resize(CHUNK, 0);
            }
            Ok(position)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// `CopyFileExW` on the blocking pool.
    Kernel,
    /// Read into user space and write back out, on the completion driver.
    ReadWrite,
}

/// Which shape the completion backend uses for [`Op::Copy`]. Both are measured;
/// see the report.
pub fn copy_mode() -> CopyMode {
    static MODE: std::sync::OnceLock<CopyMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("DAE_BENCH_COPY_MODE").as_deref() {
        Ok("readwrite") => CopyMode::ReadWrite,
        _ => CopyMode::Kernel,
    })
}
