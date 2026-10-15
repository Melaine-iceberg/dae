# frontend-scans

A standalone Bun script that answers one question with numbers: **what does a
full listing-wide scan cost on a byte-backed `ListingView`, and what do those
scans add up to while a listing streams?**

It is not part of the app. Nothing in `src/` or `src-tauri/` references it, and
no build step runs it. It imports the *real* codec — `entry-codec.ts`, the same
module the running app decodes packets with — so the numbers are the shipped
decode path's, not a re-implementation.

```bash
# from the repository root
bun bench/frontend-scans/scan-cost.ts
bun bench/frontend-scans/scan-cost.ts --entries 120000
```

## What it measures

One packed listing is built in memory (`packEntries`), then:

| measurement | what it mirrors |
|---|---|
| `allNames full scan` | `allNames(entries)` — the type-ahead name table rebuilt for `FileList` |
| `allPaths full scan` | `allPaths(view)` — the path array the selection prune used to build |
| `cumulative …` | the same scan repeated over every published snapshot of one streamed listing (512 head, then batches doubling to 8,192 — `read_remaining_batches`) |
| `selection prune` | `presentPaths(view, selected)` from the shipped `listing-view.ts`, against the baseline it replaced (`new Set(allPaths(view))`) and by where the selected row sits |
| `type-ahead table` | `allNames` cold (a view it has never seen) against warm (the same view again) — the table is cached per view now |

Every scan keeps its array, so nothing here can be optimised away by the JIT;
each row is a median (seven passes after two warmups, unless the point is the
first call). The selection and type-ahead sections call the **shipped**
functions, so they measure the code that runs rather than a copy of it — the
frontend has no test runner, which is why this harness is also the behavioural
check for those two paths.

## Reading the numbers

- The per-row cost is what matters, not the total: ~70–100 ns/row at 35,803 rows.
- The cumulative figure is the one to quote for what a navigation used to pay,
  spread over ~8 animation frames while the listing streams.
- A selection prune's cost is set by *where the selected row is*, not by the
  listing's size: first row is free, the last row (or a deleted one) reads
  everything. The baseline line above the four cases is what the whole thing cost
  before.
- Compare this harness against itself, before and after a change. Absolute
  milliseconds are per-machine, and a busy machine moves every row at once.

The findings and the numbers taken from here live in
[`docs/performance-opportunities.md`](../../docs/performance-opportunities.md).
