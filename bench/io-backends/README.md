# io-backends

A standalone benchmark that answers one question with numbers: **would moving
`dae`'s file I/O from Tokio's blocking thread pool to a completion-based runtime
(IOCP on Windows, io_uring on Linux) make it faster?**

It is not part of the app. Nothing in `src-tauri/` or the frontend references
it, and `cargo build` in `src-tauri` never compiles it.

```bash
cd bench/io-backends
cargo build --release

# 1. datasets (~105 GiB with the default sizes: an 8 GiB cache-resident blob,
#    a 96 GiB cache-defeating one, and ~25 000 files; idempotent)
./target/release/io-backends prepare all --root K:/dae-io-bench

# 2. the full sweep into ./results/*.tsv
./run-all.sh K:/dae-io-bench

# 3. the small-file batching curve into ./results/smallfiles-batch-*.tsv
./run-batch-curve.sh K:/dae-io-bench
```

`--blob-mib` lowers the cache-resident blob so the harness can be dry-run
without writing 8 GiB first.

The conclusion and the measured tables are in `docs/decisions/completion-io.md`.
Raw medians per scenario are committed too: `results/summary.md` (Windows /
ReFS / NVMe) and `results-linux/summary.md` (WSL2 / ext4, io_uring).

## What it compares

One shared `Vec<Op>` describes the work; every backend runs the same list, on
the same paths, at the same concurrency. Only dispatch differs.

| backend | what it is |
|---|---|
| `tokio` | one `tokio::task::spawn_blocking` per operation — what a Tauri command does today |
| `tokio-chunked` | one `spawn_blocking` per batch, looping inside — the shape of `read_directory`, `stat_named_children`, the copy path |
| `rayon` | the dedicated `transfer_pool` with blocking calls |
| `compio` | completion-based: IOCP with `OVERLAPPED` file I/O on Windows, io_uring on Linux |

`compio` 0.19 is the completion runtime: its Windows driver creates a real
completion port (`CreateIoCompletionPort` + `GetQueuedCompletionStatusEx`) and
opens files with `FILE_FLAG_OVERLAPPED`, so the comparison is against genuine
IOCP — not against a thread pool wearing the name.

Its `fs` layer is only partly asynchronous, and the benchmark shows it: on
Windows `compio-fs` 0.12.1 implements `File::open`, `metadata`, `set_len` and
every directory operation by falling back to a blocking pool
(`spawn_blocking_with`). Only reads and writes go through the driver.

## Workloads

| workload | shape | mirrors |
|---|---|---|
| `dispatch` | 100 000 no-ops | the cost of dispatching one operation, with no syscall in the way |
| `perop` | 20 000 one-byte reads | the cost of one real operation through each mechanism |
| `randread` | 65 536 × 4 KiB reads at random offsets over 8 GiB | thumbnail / preview / checksum chunk reads (cache-resident) |
| `randread-big` | 32 768 × 4 KiB reads at random offsets over the 96 GiB blob | the same, reaching the device |
| `seqread` | 8 GiB in 4 MiB positional reads | a batch of previews, checksums, content search |
| `seqread-big` | a rotating 32 GiB window of the 96 GiB blob, in 4 MiB reads | the same, cold |
| `smallfiles` | 20 000 × 8 KiB files, opened and read whole | a folder of source files or screenshots |
| `enumerate` | 36 000 entries, `read_dir` + metadata | `read_directory`, the explorer's critical path |
| `copy` | 4 000 × 256 KiB (1 GiB) | paste a folder |
| `write` | 1 024 × 4 MiB in fresh files | save / extract / download |

## Method notes

**`threads` is outstanding requests, not threads.** Every backend keeps exactly
`threads` operations in flight: the two windowed backends start a new op as soon
as the oldest finishes, rayon's pool does the same by work stealing, and the
chunked backend splits the list into `threads` slices. So the number a
completion driver is supposed to be able to raise — the outstanding-request
count — is held equal while the mechanism varies.

**Handles are opened outside the clock.** Positional reads run against a handle
opened before the pass, so `CreateFileW` (which `compio-fs` does on a blocking
thread) does not dominate a 4 KiB read.

**"Cold" is arranged, not enforced.** Windows has no user-mode way to drop the
page cache, so the cold workloads read a 32 GiB window that rotates by a whole
window per pass: two other windows (64 GiB) are read before a window returns,
which is more than this machine's 39 GiB of RAM. Every pass starts with its
window evicted, and backends stay comparable.

**`--batch N` prices the dispatch, not the I/O.** It is the number of
operations one `tokio` blocking call carries: `1` is what a Tauri command does
today (one call per file), `16`–`32` is where the curve flattens, and
`tokio-chunked` is the same idea taken to one call per thread. Keep `batch *
threads <= len`, or some threads get nothing to do.

**CPU time is reported only where it means something.** `GetProcessTimes`
advances in whole 15.625 ms clock ticks and can charge a tick that began before
the measured window, so a pass shorter than 8 ticks per thread prints `-`.

**Warm passes are discarded.** One unmeasured pass precedes the measured ones
(except on the cold workloads, where a warm pass would defeat the point), and
the reported median is the *middle pass itself*, not a blend of fields from
different passes.

**Same allocator as the app.** `mimalloc` is installed globally, matching
`src-tauri/src/main.rs`: the crate's own note measures a 31% win on directory
listing from the allocator alone, so a benchmark under the system heap would
have been measuring something else.

## Caveats

- `copy` is the one workload where the completion backend is *not* doing the
  same syscall as the others: `std::fs::copy` is `CopyFileExW`, a single kernel
  call with no user-space buffer, and compio has no equivalent.
  `DAE_BENCH_COPY_MODE` selects what it does instead — `kernel` (the same
  `fs::copy`, on a blocking thread: what a pragmatic port would ship) or
  `readwrite` (read into user space and write back out, on the driver).
- io_uring needs a Linux box: `run-linux.sh` copies the crate to a native
  filesystem, builds it there and runs a reduced sweep, where compio binds to
  io_uring (it prints the driver it picked, so a `polling` fallback cannot pass
  for io_uring). The WSL2 numbers in `results-linux/` come with a warning: its
  cross-thread wake costs 65 us per handoff against 175 us per raw
  `pthread_create` (`probe-linux-dispatch.sh`), which makes the `tokio` baseline
  there far worse than on bare metal.
- Absolute numbers depend on the volume. Here it is ReFS on a ZHITAI TiPlus7100
  NVMe SSD, 8 cores / 16 threads, 39 GiB RAM.
