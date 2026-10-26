# qt-list-spike — gate 1 of the Qt Quick decision

Standalone spike for **§5 gate 1** of
[`docs/decisions/engine-qt-quick.md`](../../docs/decisions/engine-qt-quick.md):

> Rust `QAbstractListModel` + QML `ListView.reuseItems` over 120k rows with
> incremental patches — scroll p95 ≤ 8 ms, PSS ≤ 250 MB.

It is not part of the app. It links `crates/dae-core` as a path dependency and
feeds the model from the **real** listing pipeline: `open_streamed_listing`,
the 512-entry head, doubling batches up to 8,192, and the `BatchTarget` seam.
Nothing about the row shape is faked. The QML window (header, 26 px delegate,
name/size/mtime columns) is the same shape as the explorer row.

## Result

**Both thresholds pass.** Windows 11 / Qt 6.12.0 / D3D11 / 120 Hz display,
120,000 entries, `reuseItems: true`, 24 visible rows patched every 250 ms and
a random scrollbar jump every second:

| run | work p95 | GUI p95 | render p95 | cadence p95 | PSS |
|---|---:|---:|---:|---:|---:|
| 12 s, #1 | **3.93 ms** | 2.84 ms | 1.09 ms | 9.77 ms | 61.2 MB |
| 12 s, #2 | **5.68 ms** | 4.71 ms | 0.98 ms | 12.23 ms | 58.2 MB |
| 12 s, #3 | **2.14 ms** | 1.15 ms | 0.99 ms | 9.03 ms | 57.8 MB |
| 12 s, vsync on | 2.47 ms | 1.41 ms | 1.06 ms | 9.30 ms | 57.4 MB |
| 30 s soak | 5.41 ms | 4.38 ms | 1.03 ms | 11.94 ms | 59.0 MB |
| 60 s, no jumps | 1.74 ms | 1.03 ms | 0.70 ms | 9.18 ms | 56.7 MB |
| 60 s, jumps | 5.93 ms | 4.90 ms | 1.03 ms | 12.22 ms | 58.6 MB |
| 12 s, all-CJK names | 2.93 ms | 1.36 ms | 1.57 ms | 9.54 ms | 115.6 MB (peak 123.7) |

* **work p95** = GUI event-loop busy time + render-thread sync+paint time. This
  is the number the ≤ 8 ms gate is compared against; the raw `frameSwapped`
  interval is reported separately as **cadence**.
* **PSS** is the Windows analogue of Linux PSS (see below). The all-CJK run is
  the worst case. The mixed fixture sits at ~57 MB; a random jump only pays the
  font step if it lands on a CJK name, and none of the committed mixed runs did
  (below).
* 236 delegate objects were created in every run, including the 60-second one.
  236 is the ~2,600 px `cacheBuffer` worth of rows — `reuseItems` is recycling,
  not recreating.

Raw JSON for every run is in [`results/`](results/). The CJK window grab is
[`results/win11-120k-cjk-window.png`](results/win11-120k-cjk-window.png).

### Icon A/B (2026-10-26)

The row now draws the OS icon through the product's `fileicon://` provider; the
spike links `crates/dae-qt` and registers the same provider. Same machine, same
fixtures, `--icons on|off` alternated in one session:

| run | icons | work p95 | GUI p95 | render p95 | cadence p95 | PSS |
|---|---:|---:|---:|---:|---:|---:|
| 12 s #1 | off | 1.82 ms | 0.85 ms | 0.97 ms | 10.48 ms | 59.1 MB |
| 12 s #1 | on | **1.68 ms** | 0.75 ms | 0.94 ms | 10.18 ms | 62.8 MB |
| 12 s #2 | off | 1.64 ms | 0.70 ms | 0.94 ms | 6.55 ms | 59.3 MB |
| 12 s #2 | on | **1.60 ms** | 0.69 ms | 0.92 ms | 6.53 ms | 63.3 MB |
| 12 s #3 | off | 1.65 ms | 0.74 ms | 0.91 ms | 10.91 ms | 58.9 MB |
| 12 s #3 | on | **1.66 ms** | 0.74 ms | 0.92 ms | 8.90 ms | 62.1 MB |
| 12 s all-CJK #1 | off | 2.02 ms | 0.72 ms | 1.30 ms | 6.64 ms | 106.3 MB |
| 12 s all-CJK | on | **1.99 ms** | 0.70 ms | 1.29 ms | 6.65 ms | 110.1 MB |
| 12 s all-CJK #2 | off | 1.98 ms | 0.70 ms | 1.28 ms | 6.66 ms | 109.0 MB |
| 30 s soak | on | **1.55 ms** | 0.66 ms | 0.89 ms | 9.98 ms | 111.3 MB |

**The icon frame costs nothing measurable; PSS +3–4 MB.** The three on/off
pairs give work p95 1.60–1.66 vs 1.64–1.82 ms — the on runs are marginally
lower, which is run-to-run variance, not a speedup. PSS is +3.7 / +4.0 / +3.2 MB:
14 extension textures plus the provider's bounded byte cache. The all-CJK pair is
flat too (1.99 vs 1.98–2.02 ms; 110.1 vs 106.3–109.0 MB).

The 30 s soak reaches 111.3 MB because a random jump landed on a CJK name and
paid the one-time font fallback at ~6 s (50.5 → 109.2 MB, then flat for 25 s) —
not because icons accumulate. Its work p95 is the lowest of the session, again
within noise.

The absolute numbers are lower than the October 25 table above on the same
workload (2.1–5.7 ms no longer reproduces; the machine was busier then). The gate
decision is unchanged, and the icon delta is the same-session A/B, not a
cross-day absolute.

**Scope:** the fixture is files-only, so every row uses a shared
`type/<ext>` URL — 14 provider calls and 14 textures for the whole run. A folder
listing asks by path (custom folder icons); that path is bounded by visible rows
and the 512-entry cache, but this A/B does not measure it. A directory-heavy
fixture is the next check if that code changes.

Raw JSON: `results/win11-120k-{icons,noicons}-off-*.json`,
`results/win11-120k-cjk-noicons-*.json`, `results/win11-120k-icons-cjk.json`,
`results/win11-120k-icons-off-30s.json`. Window grab:
[`results/win11-120k-icons-window.png`](results/win11-120k-icons-window.png).

### Why "work", not the frame interval

The display runs at 120 Hz, so an **empty** list also presents at ~8.33 ms. On
this machine `QSG_NO_VSYNC=1` does not lift D3D11's presentation throttle either;
the interval stays pinned to the display. That number can tell you a frame was
dropped, but it cannot tell you how much work the row model costs.

The spike therefore measures the two disjoint halves of a frame:

| half | hook |
|---|---|
| GUI thread | `QAbstractEventDispatcher::awake` → `aboutToBlock` busy time, accumulated between `frameSwapped` callbacks (delegate reuse, bindings, the scroll handler, timers). |
| render thread | `QQuickWindow::beforeSynchronizing` → `afterRendering`, split into sync and paint. |

The GUI thread blocks while the render thread syncs, so adding the two p95s is
a conservative per-frame cost. The unmodified `frameSwapped` interval is still
recorded; it is what the `cadence p95` column shows.

## Workload

* One `DirectoryListModel` (Rust `QAbstractListModel`, `#[base = QAbstractListModel]`)
  receives head + streamed batches through `BatchTarget`. The model stores the
  name, size, mtime, directory bit and hidden bit per row.
* `ListView.reuseItems: true`, `cacheBuffer: 2600`, 26 px fixed-height
  delegates with icon + name + size + mtime. The icon is the product's OS icon
  (the provider from `crates/dae-qt`); `--icons off` swaps it back to the drawn
  square for the A/B above.
* The scroll handler moves `contentY` by three rows on every `frameSwapped`,
  which requests the next frame — the loop is driven by presented frames, not a
  timer. A 1 s timer jumps to a random position, and a 250 ms timer applies one
  `dataChanged` over 24 visible rows.
* The app waits 1 s after the listing is complete before recording, then runs
  for `--seconds`.

### The CJK font step (not a leak)

Scrolling into the first CJK filename loads a font fallback: PSS gains about
52 MB once and then stays put. The all-CJK fixture pays it deterministically
during the settle after load (PSS 40.7 → 92.4 MB before measurement starts,
123.7 MB peak once scrolling warms the glyph cache). A mixed run only pays it if
a random jump lands on a CJK name; neither committed 60 s mixed run did, so both
stay flat at ~55–59 MB. The real shell should preload the CJK fallback font so
the first Chinese name in the explorer does not pay this synchronously.

## Fixture

The 120k-entry directories are generated by the existing `listing-pipeline`
harness; empty files are enough because the listing reads names and metadata,
not contents:

```bash
cd bench/listing-pipeline
cargo build --release
./target/release/listing-pipeline prepare K:/dae-listing-bench/huge     120000
./target/release/listing-pipeline prepare K:/dae-listing-bench/huge-cjk 120000 cjk
```

## Build and run

```bash
cd bench/qt-list-spike
export QMAKE="C:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
cargo build --release

# gate run: unthrottled frame-work measurement
PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH" \
  ./target/release/qt-list-spike.exe \
  --path K:/dae-listing-bench/huge \
  --seconds 12 \
  --out results/win11-120k-off-1.json

# display-cadence comparison
... --vsync on

# isolate the stress knobs (default both on)
... --jumps off --patches off
```

The app quits itself after the measuring window and writes one JSON document
(stdout and `--out`). `--grab <png>` optionally saves the window content.

Run the same command twice, once with `--vsync off` and once with `--vsync on`,
to get the frame-work and display-cadence pair.

## Method notes

* **PSS.** `QueryWorkingSetEx` charges every resident page once and a shared
  page `1 / share_count` of its size. The OS caps the count at 7, so pages
  mapped more often are slightly over-reported. The query buffers add a couple
  of MB to each reading. Working set and private commit are recorded alongside.
* **Absolute numbers are one machine at one moment.** The runs above had
  browser and IDE processes in the background; the three 12 s runs span
  2.14–5.68 ms for the same binary, and the CJK font fallback adds ~52 MB on
  its first use.
* **The model is the only data source.** The fixture is a real directory, the
  rows arrive as `ListingRow` values from `dae-core`, and `data()` formats the
  size and UTC timestamp on the model side.
