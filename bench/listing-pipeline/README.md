# listing-pipeline

A standalone benchmark that answers one question with numbers: **where does the
time of a large local directory listing go, and what is a cheaper enumeration and
a cheaper ordering key worth?**

It is not part of the app. Nothing in `src-tauri/` or the frontend references it,
and `cargo build` in `src-tauri` never compiles it.

```bash
cd bench/listing-pipeline
cargo build --release

# a dataset to measure (idempotent; ~35,803 files in one directory)
./target/release/listing-pipeline prepare K:/dae-listing-bench/big 35803
# 250 entries is the common case, `cjk` is all non-ASCII names
./target/release/listing-pipeline prepare K:/dae-listing-bench/small 250
./target/release/listing-pipeline prepare K:/dae-listing-bench/cjk 35803 cjk

# the sweep into ./results/*.tsv
./target/release/listing-pipeline measure K:/dae-listing-bench/big \
  --passes 9 --warmup 3 > results/big.tsv
# how many allocations a listing makes under each ordering
./target/release/listing-pipeline allocs K:/dae-listing-bench/big
```

The conclusion and the measured tables are in
[`docs/decisions/windows-directory-enumeration.md`](../../docs/decisions/windows-directory-enumeration.md).
Raw medians per pass are committed under `results/`.

## What it compares

Every configuration runs the same listing over the same directory — a
512-entry head, then batches doubling up to 8,192, exactly as
`open_streamed_listing` + `read_remaining_batches` do — and only the enumeration
and the ordering vary:

| walk | what it is |
|---|---|
| `std` | `std::fs::read_dir` + `DirEntry::metadata()` — what the app does today |
| `win32` | `FindFirstFileExW` with `FIND_FIRST_EX_LARGE_FETCH`, rows built from the find data |
| `win32-nofetch` | the same call without the flag, which isolates it |

| ordering | what it is |
|---|---|
| *(none)* | stop after the rows are built, so the walk's own cost is visible |
| `+key` | `name_sort_key` — the app's cached `(rank, lowercase, name)` key |
| `+cmp` | `name_order` — the same total order, compared instead of built |

So `std+key` is the pipeline as it shipped, and `win32+cmp` is the pipeline with
both changes.

## What it mirrors

`src/pipeline.rs` is a verbatim copy of the app's listing pipeline with the
imports adjusted:

| here | app |
|---|---|
| `ListingRow` | `file_system::entry_codec::ListingRow` |
| `name_sort_key` | `file_system::types::name_sort_key` |
| `cmp_rows` / `cmp_lowercase` | `file_system::types::name_order` |
| `entry_kind`, `modified_at_millis`, `entry_state_flags` | `file_system::local::directory` |
| `row_from`, `listing_row`, `collect_rows` | `file_system::local::directory` |
| `next_batch_size`, `FIRST_BATCH_SIZE`, `MAX_BATCH_SIZE` | `file_system::listing` |

`src/windows_find.rs` is the Windows enumeration, with the bindings declared by
hand (five functions, one struct, laid out as `std` lays them out) so the crate
keeps a single dependency. `src/pipeline.rs::row_from_find_data` is
`local::directory::row_from` reading a `WIN32_FIND_DATAW` instead of the
`std::fs::Metadata` that *is* that struct on Windows.

**The one deliberate divergence** is the thread split: on Unix the app spreads a
batch of 128 or more over rayon's global pool, and this crate has no rayon
dependency, so `collect_rows` here is always the serial arm. That arm is what
Windows takes for every batch, and the ordering is serial on every platform, so a
Unix number from this harness would understate the metadata half and overstate
the ordering's share. There is a second, smaller gap in the same direction: the
Win32 walk here collects a batch of `WIN32_FIND_DATAW` values and then builds
rows from them, where the app builds each row as it advances — so the numbers
below *understate* the `win32` walk slightly.

**Same allocator as the app.** `mimalloc` is installed as the global allocator,
matching `src-tauri/src/main.rs`. Listing allocates per entry and the app's own
notes measure a 31% win on directory listing from the allocator alone, so a
benchmark on the system heap would be measuring something else. That is worth
knowing when comparing these numbers with the ones from the app's own
`reports_the_cost_of_a_large_listing` test: a `cargo test --lib` binary does not
include `main.rs`, so it runs on the system heap, where the same listing is
~30% slower for the same pipeline.

## Method notes

**Phases are timed apart.** The walk, the row construction and the ordering each
get their own `Instant` window per batch, summed over the batches. The app
interleaves them on one thread, so the sum is not the wall time of a real read —
but the phases are the ones being compared and none of them overlaps another
inside a batch. `+rows` is therefore what the `+key` and `+cmp` columns are
measured against, rather than each other.

**Order parity is checked before anything is timed.** `measure` re-reads the
directory under every pipeline configuration and compares the full name sequence
against `std+key`, and prints `identical` or the index of the first difference. A
faster order that is a *different* order is not a result.

**The flag is A/B'd against itself, in both orders.** `win32` and
`win32-nofetch` differ by exactly one flag, and a run of one against the other
was repeated with the variant order reversed to rule out a cache-warming order
effect; the gap held at 12.7–13.4 ms vs 9.3–9.8 ms across six runs.

**Warm passes are discarded.** Each configuration gets `--warmup` unmeasured
passes, then the reported median is the middle of the measured passes.

**Absolute numbers depend on the volume and on how warm the directory index is.**
`std` walking the 35,803-entry fixture measured 10.3 ms immediately after the
fixture was written and 14.2 ms a few minutes later with the index evicted. The
*ratios* reproduce across runs; a single table's absolute numbers should be read
as one machine's state at one moment. The harness is also single-threaded, so a
busy machine slows every row together — check the load before quoting a number.

**Caveat: `copy`-style file reads are not in scope.** This measures enumeration
and ordering only; the app's file I/O mechanism was measured separately and
declined in [`docs/decisions/completion-io.md`](../../docs/decisions/completion-io.md).

## Datasets

`prepare` writes empty files, because the listing cost is the walk plus the
metadata plus the ordering and an empty file has all three: `WIN32_FIND_DATAW`
carries the size without reading a byte of data. Names come from a 40-prefix ×
14-suffix × 3-joiner × 5-casing generator, so comparisons have to walk past
shared prefixes the way real directories make them; one name in 37 is non-ASCII,
and `prepare <dir> <count> cjk` makes every name non-ASCII, which is the case the
Unicode arm of the comparator has to be measured on rather than assumed about.

## Results

Windows / ReFS on a ZHITAI TiPlus7100 NVMe, Ryzen 9 7940H (8C/16T), mimalloc,
quiet machine. Pass-by-pass output is committed under `results/`.

**The walk on its own** (enumeration with no row construction), median of 25
passes for the two small fixtures and 9 for the large ones:

| entries | `std` | `win32-nofetch` | `win32` |
|---|---|---|---|
| 250 | 0.081 ms | 0.070 ms | 0.070 ms |
| 512 | 0.135 ms | 0.119 ms | 0.106 ms |
| 35,803 | 14.15 ms\* | 13.03 ms | 9.76 ms\* |
| 120,000 | 55.78 ms\* | 53.76 ms | 40.32 ms\* |

\* the 35,803 and 120,000 runs measure four configurations and no walk-only
`std` row, so those two figures are the *walk phase* of the `std+key` and
`win32+key` rows of the same run — same clock, same state, same pass.

**The whole pipeline**, `results/big.tsv`, `results/huge.tsv` and
`results/cjk.tsv`:

| entries | `std+key` (shipped) | `win32+key` | `win32+cmp` |
|---|---|---|---|
| 35,803 (mixed names) | 17.99 ms | 13.02 ms (-27.6%) | 12.64 ms (-29.8%) |
| 120,000 | 69.65 ms | 52.17 ms (-25.1%) | 50.71 ms (-27.2%) |
| 35,803 (all non-ASCII) | 17.86 ms | 12.93 ms (-27.6%) | 12.53 ms (-29.8%) |

Allocations per entry, 35,803 entries (`results/allocs.tsv`). The ordering is two
`String`s per row and nothing else changes between the two walks:

| configuration | allocations | per entry |
|---|---|---|
| `std+key` | 144,209 | 4.03 |
| `std+cmp` | 72,595 | 2.03 |
| `win32+key` | 144,121 | 4.03 |
| `win32+cmp` | 72,507 | 2.03 |

The one thing the flag does **not** help is the part the user waits for first: the
512-entry head gains 29 µs (0.135 ms -> 0.106 ms), which is inside a frame. What
it buys is the listing finishing — 4.97 ms off 35,803 entries and 17.5 ms off
120,000 before the ordering is touched at all.
