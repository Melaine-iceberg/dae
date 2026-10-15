//! The workloads, and the datasets they run on.
//!
//! Every shape here is one the app actually performs, with the sizes chosen so
//! that the work is large enough to measure and small enough to run often:
//!
//! | workload | app path it mirrors |
//! |---|---|
//! | `randread` | 4 KiB reads scattered over a file — thumbnail / preview / hashing chunk reads |
//! | `randread-big` | the same, on a file larger than the page cache, so the reads reach the device |
//! | `seqread` | whole-file sequential reads at depth — a batch of previews, checksums, content search |
//! | `seqread-big` | the same, cold |
//! | `smallfiles` | opening and reading many small files — a folder of source files or screenshots |
//! | `enumerate` | `read_directory`, the explorer's critical path: 36 000 entries with metadata |
//! | `copy` | paste a 1 GiB folder, the `transfer_pool` / `fs::copy` path |
//! | `write` | a batch of writes that each create their file — save / extract / download |
//!
//! Datasets are shared between workloads that can share them (`randread` and
//! `seqread` read the same 8 GiB blob) so the disk cost of a full sweep stays
//! reasonable.

use crate::ops::Op;
use crate::rng::Rng;
use rayon::prelude::*;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Blob read by `randread` / `seqread`. Fits in this machine's page cache, so
/// it measures the dispatch mechanism and the copy out of cache.
pub const BLOB_MIB: u64 = 8 * 1024;

/// Blob read by the two cold workloads. Deliberately larger than the 39 GiB of
/// RAM: a cache that cannot hold the file is the only way to see what the
/// device does with a deep queue, which is the one place a completion driver is
/// supposed to win.
pub const BIG_BLOB_MIB_DEFAULT: u64 = 96 * 1024;

/// How much of the big blob one cold pass reads.
///
/// The page cache cannot be dropped from user mode on Windows, so "cold" is
/// arranged rather than enforced: a pass reads a 32 GiB window, and the window
/// rotates by a whole window per pass. Two other windows (64 GiB) are read
/// before a window comes round again, which is more than the 39 GiB of RAM —
/// so every pass starts with that window evicted, and backends stay comparable
/// instead of the second one measuring the first one's cache.
pub const COLD_WINDOW_MIB: u64 = 32 * 1024;

pub const RANDREAD_READS: usize = 65_536;

/// Handles the `*-multi` variants spread their reads over.
///
/// `randread` and `randread-big` issue every positional read against one shared
/// handle — what "parallel chunk reads of one file" looks like. That turned out
/// to be a variable in its own right: the blocking backends get *slower* with
/// more threads on one handle, so the multi variants hold everything else equal
/// and give each in-flight read its own handle.
pub const MULTI_HANDLES: usize = 64;
pub const RANDREAD_BLOCK: usize = 4 * 1024;
pub const BIG_RANDREAD_READS: usize = 32_768;
/// 4 MiB chunks: large enough that per-op overhead is not the whole story,
/// small enough to give the deep-queue test something to queue.
pub const SEQ_BLOCK: usize = 4 * 1024 * 1024;

pub const SMALL_FILES: usize = 20_000;
pub const SMALL_FILE_KIB: u64 = 8;

/// Mirrors the 35 821-entry directory the crate's mimalloc note cites.
pub const ENUM_DIRS: usize = 8;
pub const ENUM_PER_DIR: usize = 4_500;

pub const COPY_FILES: usize = 4_000;
pub const COPY_FILE_KIB: u64 = 256;
pub const COPY_DIRS: usize = 16;

pub const WRITE_CHUNKS: usize = 1_024;
pub const WRITE_CHUNK_MIB: u64 = 4;

/// No-ops for the `dispatch` scenario: enough that the per-operation cost of a
/// dispatch mechanism is not lost in the noise floor.
pub const DISPATCH_OPS: usize = 100_000;

/// One-byte reads for the `perop` scenario. Lower than `DISPATCH_OPS` because
/// each op is a real syscall plus, on the completion backends, a real
/// completion round trip.
pub const PEROP_OPS: usize = 20_000;

pub const NAMES: &[&str] = &[
    "dispatch",
    "perop",
    "randread",
    "randread-multi",
    "randread-big",
    "randread-big-multi",
    "seqread",
    "seqread-big",
    "smallfiles",
    "enumerate",
    "copy",
    "write",
];

/// One pass of work.
pub struct Workload {
    pub items: Vec<Op>,
    /// Handles to open before the clock starts, for [`Op::ReadAt`].
    pub handles: Vec<PathBuf>,
    /// Bytes this pass is expected to move, for the throughput column. Zero for
    /// a workload measured in entries.
    pub nominal_bytes: u64,
    /// Removed before each pass so a copy or write always starts fresh.
    pub cleanup: Option<Cleanup>,
}

/// What a scenario's headline rate counts. Printing MiB/s for a scenario that
/// moves no bytes would read as "infinitely fast".
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hero {
    Bytes,
    Ops,
    Entries,
}

pub fn hero(name: &str) -> Hero {
    match name {
        "dispatch" | "perop" => Hero::Ops,
        "enumerate" => Hero::Entries,
        _ => Hero::Bytes,
    }
}

/// What a mutated workload has to undo between passes.
///
/// `remove` is the parent of every pass's output rather than the pass's own
/// directory: passes are named after their index, so removing only the current
/// one would leave ~8 GiB behind per pass and fill the volume over a sweep.
pub struct Cleanup {
    pub remove: PathBuf,
    pub recreate: Vec<PathBuf>,
}

pub fn unit_label(name: &str) -> &'static str {
    match hero(name) {
        Hero::Bytes => "MiB/s",
        Hero::Ops => "kops/s",
        Hero::Entries => "entries/s",
    }
}

fn blob_dir(root: &Path) -> PathBuf {
    root.join("blobs")
}

fn blob_path(root: &Path, mib: u64) -> PathBuf {
    blob_dir(root).join(format!("blob-{mib}mib.bin"))
}

fn big_blob_path(root: &Path, big_mib: u64) -> PathBuf {
    blob_path(root, big_mib)
}

/// Creates every dataset the named workloads need. Idempotent: a file that is
/// already the right size is left alone.
pub fn prepare(name: &str, root: &Path, blob_mib: u64, big_mib: u64) -> Result<(), String> {
    let mut needed: Vec<&str> = match name {
        "all" => NAMES.to_vec(),
        other => vec![other],
    };
    if needed.contains(&"randread") {
        needed.push("seqread");
    }
    if needed.contains(&"randread-big") {
        needed.push("seqread-big");
    }

    for workload in needed {
        match workload {
            "randread" | "randread-multi" | "seqread" => {
                write_blob(&blob_path(root, blob_mib), blob_mib)?
            }
            "randread-big" | "randread-big-multi" | "seqread-big" => {
                write_blob(&big_blob_path(root, big_mib), big_mib)?
            }
            "smallfiles" => prepare_smallfiles(root)?,
            "enumerate" => prepare_enumerate(root)?,
            "copy" => prepare_copy(root)?,
            "write" => {
                fs_create_dir(&root.join("write"))?;
            }
            "dispatch" => {}
            // `perop` reads one byte of the small blob, so the dataset has to
            // exist for the measurement to be about the mechanism rather than
            // about a missing file.
            "perop" => write_blob(&blob_path(root, blob_mib), blob_mib)?,
            other => return Err(format!("unknown workload `{other}`")),
        }
    }

    let _ = fs_create_dir;
    Ok(())
}

fn fs_create_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("create {}: {error}", path.display()))
}

/// Writes `mib` MiB into `path`, reusing the file when it already has that
/// size. Content is a cheap pattern — nothing in the benchmark reads values, so
/// generating them randomly would only make preparation slower.
fn write_blob(path: &Path, mib: u64) -> Result<(), String> {
    fs_create_dir(path.parent().expect("blob path has a parent"))?;
    let target = mib * 1024 * 1024;

    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() == target
    {
        println!("# {} already prepared ({} MiB)", path.display(), mib);
        return Ok(());
    }

    println!("# writing {} ({} MiB)", path.display(), mib);
    let mut file =
        File::create(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    let block = vec![0xA5_u8; 4 * 1024 * 1024];
    let started = std::time::Instant::now();
    let mut written = 0_u64;
    let mut next_report = 2 * 1024 * 1024 * 1024_u64;
    while written < target {
        let want = (target - written).min(block.len() as u64) as usize;
        file.write_all(&block[..want])
            .map_err(|error| format!("write {}: {error}", path.display()))?;
        written += want as u64;
        if written >= next_report {
            let seconds = started.elapsed().as_secs_f64().max(0.001);
            println!(
                "#   {} MiB / {} MiB ({:.0} MiB/s)",
                written / (1024 * 1024),
                mib,
                (written as f64 / (1024.0 * 1024.0)) / seconds
            );
            next_report += 2 * 1024 * 1024 * 1024;
        }
    }
    file.flush().map_err(|error| format!("flush: {error}"))?;
    println!("#   done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

fn prepare_smallfiles(root: &Path) -> Result<(), String> {
    let dir = root.join("smallfiles");
    fs_create_dir(&dir)?;
    let payload = vec![0x33_u8; (SMALL_FILE_KIB * 1024) as usize];
    (0..SMALL_FILES).into_par_iter().try_for_each(|index| {
        let path = dir.join(format!("f-{index:05}.dat"));
        if path.exists() {
            return Ok(());
        }
        let mut file =
            File::create(&path).map_err(|error| format!("create {}: {error}", path.display()))?;
        file.write_all(&payload)
            .map_err(|error| format!("write {}: {error}", path.display()))
    })?;
    println!(
        "# {} small files ready ({SMALL_FILES} × {SMALL_FILE_KIB} KiB)",
        dir.display()
    );
    Ok(())
}

fn enumerate_dirs(root: &Path) -> Vec<PathBuf> {
    (0..ENUM_DIRS)
        .map(|index| root.join("enumerate").join(format!("d{index:02}")))
        .chain(std::iter::once(root.join("enumerate")))
        .collect()
}

fn prepare_enumerate(root: &Path) -> Result<(), String> {
    for dir in enumerate_dirs(root) {
        fs_create_dir(&dir)?;
    }
    (0..ENUM_DIRS).into_par_iter().try_for_each(|dir_index| {
        let dir = root.join("enumerate").join(format!("d{dir_index:02}"));
        (0..ENUM_PER_DIR).into_par_iter().try_for_each(|index| {
            let path = dir.join(format!("e-{dir_index:02}-{index:04}.tmp"));
            if path.exists() {
                return Ok(());
            }
            // Empty on purpose: the listing cost is the directory walk plus the
            // metadata, and `WIN32_FIND_DATA` carries both without a data read.
            File::create(&path)
                .map(|_| ())
                .map_err(|error| format!("create {}: {error}", path.display()))
        })
    })?;
    println!(
        "# {} enumerate entries ready ({} dirs × {ENUM_PER_DIR})",
        root.join("enumerate").display(),
        ENUM_DIRS
    );
    Ok(())
}

fn copy_source_dir(root: &Path) -> PathBuf {
    root.join("copy").join("src")
}

fn prepare_copy(root: &Path) -> Result<(), String> {
    let base = copy_source_dir(root);
    let payload = vec![0x77_u8; (COPY_FILE_KIB * 1024) as usize];
    let per_dir = COPY_FILES / COPY_DIRS;
    (0..COPY_DIRS).into_par_iter().try_for_each(|dir_index| {
        let dir = base.join(format!("d{dir_index:02}"));
        fs_create_dir(&dir)?;
        (0..per_dir).into_par_iter().try_for_each(|index| {
            let path = dir.join(format!("f-{index:04}.bin"));
            if path.exists() {
                return Ok(());
            }
            let mut file = File::create(&path)
                .map_err(|error| format!("create {}: {error}", path.display()))?;
            file.write_all(&payload)
                .map_err(|error| format!("write {}: {error}", path.display()))
        })
    })?;
    println!(
        "# copy source ready ({} files × {COPY_FILE_KIB} KiB)",
        COPY_FILES
    );
    Ok(())
}

/// Builds one pass. `run` distinguishes passes so that a pass writing files
/// never sees the previous pass's output.
pub fn build(
    name: &str,
    root: &Path,
    blob_mib: u64,
    big_mib: u64,
    run: u64,
) -> Result<Workload, String> {
    match name {
        "randread" => random_reads(&blob_path(root, blob_mib), RANDREAD_READS, run, 1),
        "randread-multi" => random_reads(
            &blob_path(root, blob_mib),
            RANDREAD_READS,
            run,
            MULTI_HANDLES,
        ),
        "randread-big" => random_reads(&big_blob_path(root, big_mib), BIG_RANDREAD_READS, run, 1),
        "randread-big-multi" => random_reads(
            &big_blob_path(root, big_mib),
            BIG_RANDREAD_READS,
            run,
            MULTI_HANDLES,
        ),
        "seqread" => sequential_reads(&blob_path(root, blob_mib), run, false),
        "seqread-big" => sequential_reads(&big_blob_path(root, big_mib), run, true),
        "smallfiles" => small_files(root),
        "enumerate" => enumerate(root),
        "copy" => copy(root, run),
        "write" => Ok(write(root, run)),
        "dispatch" => Ok(dispatch()),
        "perop" => perop(&blob_path(root, blob_mib)),
        other => Err(format!("unknown workload `{other}`")),
    }
}

/// Every workload that reads a prepared dataset checks it up front: failing
/// with "run `prepare` first" beats failing with the OS error of whichever
/// operation happened to touch a missing path first.
fn require_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    Err(format!(
        "{} is missing — run `prepare` first",
        path.display()
    ))
}

fn require_file(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| {
            format!(
                "{} is missing ({error}) — run `prepare` first",
                path.display()
            )
        })
}

fn random_reads(path: &Path, reads: usize, run: u64, handles: usize) -> Result<Workload, String> {
    // Seeded per pass so every backend in a pass sees the same offsets, and a
    // different pass does not simply repeat the same cached pages.
    let mut rng = Rng::new(0xD1E5_0000 + run);
    let size = require_file(path)?;
    let blocks = (size / RANDREAD_BLOCK as u64).max(1);
    let items = (0..reads)
        .map(|index| Op::ReadAt {
            file: index % handles,
            offset: rng.below(blocks) * RANDREAD_BLOCK as u64,
            len: RANDREAD_BLOCK,
        })
        .collect();

    Ok(Workload {
        items,
        handles: vec![path.to_path_buf(); handles],
        nominal_bytes: reads as u64 * RANDREAD_BLOCK as u64,
        cleanup: None,
    })
}

fn sequential_reads(path: &Path, run: u64, cold: bool) -> Result<Workload, String> {
    let size = require_file(path)?;
    let (start, length) = if cold {
        let window = COLD_WINDOW_MIB * 1024 * 1024;
        let windows = (size / window).max(1);
        // The pass index is the rotation: consecutive passes never read the same
        // window, which is what keeps them cold (see `COLD_WINDOW_MIB`).
        ((run % windows) * window, window.min(size))
    } else {
        (0, size)
    };

    let blocks = length.div_ceil(SEQ_BLOCK as u64);
    let items = (0..blocks)
        .map(|index| {
            let offset = index * SEQ_BLOCK as u64;
            Op::ReadAt {
                file: 0,
                offset: start + offset,
                // The last block is short when the window is not a multiple of
                // the block size, so a read never runs past the window's end.
                len: (length - offset).min(SEQ_BLOCK as u64) as usize,
            }
        })
        .collect();

    Ok(Workload {
        items,
        handles: vec![path.to_path_buf()],
        nominal_bytes: length,
        cleanup: None,
    })
}

fn small_files(root: &Path) -> Result<Workload, String> {
    let dir = root.join("smallfiles");
    require_dir(&dir)?;
    let items = (0..SMALL_FILES)
        .map(|index| Op::ReadWhole {
            path: Arc::new(dir.join(format!("f-{index:05}.dat"))),
        })
        .collect();

    Ok(Workload {
        items,
        handles: Vec::new(),
        nominal_bytes: SMALL_FILES as u64 * SMALL_FILE_KIB * 1024,
        cleanup: None,
    })
}

fn enumerate(root: &Path) -> Result<Workload, String> {
    require_dir(&root.join("enumerate"))?;
    let items = enumerate_dirs(root)
        .into_iter()
        .map(|path| Op::ListDir {
            path: Arc::new(path),
        })
        .collect();

    Ok(Workload {
        items,
        handles: Vec::new(),
        nominal_bytes: 0,
        cleanup: None,
    })
}

fn copy(root: &Path, run: u64) -> Result<Workload, String> {
    let base = copy_source_dir(root);
    require_dir(&base.join("d00"))?;
    let per_dir = COPY_FILES / COPY_DIRS;
    let dest_base = root.join("copy-dst");
    let dest_root = dest_base.join(format!("dst-{run}"));

    let mut items = Vec::with_capacity(COPY_FILES);
    let mut recreate = Vec::with_capacity(COPY_DIRS);
    for dir_index in 0..COPY_DIRS {
        let src_dir = base.join(format!("d{dir_index:02}"));
        let dst_dir = dest_root.join(format!("d{dir_index:02}"));
        recreate.push(dst_dir.clone());
        for index in 0..per_dir {
            items.push(Op::Copy {
                from: Arc::new(src_dir.join(format!("f-{index:04}.bin"))),
                // The op does not create its destination's directory: 4 000
                // extra `CreateDirectoryW` calls would show up in the numbers.
                // `reset` builds the tree before the clock starts.
                to: Arc::new(dst_dir.join(format!("f-{index:04}.bin"))),
            });
        }
    }

    Ok(Workload {
        items,
        handles: Vec::new(),
        nominal_bytes: COPY_FILES as u64 * COPY_FILE_KIB * 1024,
        cleanup: Some(Cleanup {
            remove: dest_base,
            recreate,
        }),
    })
}

fn write(root: &Path, run: u64) -> Workload {
    let base = root.join("write");
    let dir = base.join(format!("run-{run}"));
    let len = WRITE_CHUNK_MIB * 1024 * 1024;

    let items = (0..WRITE_CHUNKS)
        .map(|index| Op::WriteWhole {
            path: Arc::new(dir.join(format!("out-{index:04}.bin"))),
            len,
        })
        .collect();

    Workload {
        items,
        handles: Vec::new(),
        nominal_bytes: WRITE_CHUNKS as u64 * len,
        cleanup: Some(Cleanup {
            remove: base,
            recreate: vec![dir],
        }),
    }
}

fn dispatch() -> Workload {
    Workload {
        items: vec![Op::Nop; DISPATCH_OPS],
        handles: Vec::new(),
        nominal_bytes: 0,
        cleanup: None,
    }
}

/// One real, minimal I/O operation per op: a one-byte positional read of a
/// handle opened outside the clock. `ops/s` here is the cost of *one operation*
/// through each mechanism — the same `ReadFile` syscall for all of them, plus
/// the completion round trip where there is one.
fn perop(path: &Path) -> Result<Workload, String> {
    require_file(path)?;
    Ok(Workload {
        items: vec![
            Op::ReadAt {
                file: 0,
                offset: 0,
                len: 1,
            };
            PEROP_OPS
        ],
        handles: vec![path.to_path_buf()],
        nominal_bytes: PEROP_OPS as u64,
        cleanup: None,
    })
}

/// Removes a pass's leftovers. Called outside the measured region; for the
/// write and copy workloads it is what keeps a second pass from measuring an
/// overwrite instead of a create.
pub fn reset(workload: &Workload) {
    if let Some(cleanup) = &workload.cleanup {
        let _ = std::fs::remove_dir_all(&cleanup.remove);
        for dir in &cleanup.recreate {
            let _ = std::fs::create_dir_all(dir);
        }
    }
}
