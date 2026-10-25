# qt-window-spike — gates 3 and 4 of the Qt Quick decision

Standalone spike for **§5 gates 3 and 4** of
[`docs/decisions/engine-qt-quick.md`](../../docs/decisions/engine-qt-quick.md):

> Gate 3: Windows 11 frameless window + Mica + custom title bar + drag / resize /
> Snap Layouts — all native, and a DWM tint switch that does not flash.
>
> Gate 4: tear a tab out into a second window — ≤ 20 MB PSS and ≤ 150 ms — and
> prove that the gate-3 path is per window, not per process.

It is not part of the app. The backdrop **decision** is a port of
[`src-tauri/src/window_material/windows.rs`](../../src-tauri/src/window_material/windows.rs)
(the same `CurrentBuild` / `EnableTransparency` registry reads and the same
22000 / 22523 thresholds), because the Qt shell must not disagree with the
Tauri shell about which machine has a backdrop. The **application** is raw
Win32 (`DwmEnableBlurBehindWindow` + `DWMWA_SYSTEMBACKDROP_TYPE`), since Qt has
no `window-vibrancy` equivalent. The title bar is drawn in QML; the native
half answers `WM_NCHITTEST`, keeps the move/size loops native, and drives real
`SendInput` gestures for the scripted run.

## Gate 3 result

**Pass: 63 checks, 0 failures.** Windows 11 Version 26H2 / build 26340 /
Qt 6.12.0 / D3D11 / 3840×2160 at 150% (work area 3840×2088).

| area | measured |
|---|---|
| Mica applied | `DWMWA_SYSTEMBACKDROP_TYPE` read back as **2** (Mica) in both dark and light; Acrylic read back as **3**, `None` as **1** |
| transparent path | window is `WS_EX_LAYERED`; `DwmEnableBlurBehindWindow` (empty blur region) succeeded; DWM composition on |
| hit test map | caption `HTCAPTION`, maximize `HTMAXBUTTON`, min/close `HTCLIENT`, all eight resize codes, content `HTCLIENT` |
| drag | real mouse moved the window **+80, +55 px**; `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` both observed; `WM_NCLBUTTONDOWN` with `HTCAPTION` |
| resize | left edge **+120 px** (width +120, left −120); bottom-right corner **+60 × +40 px**; native size loop entered/exited |
| maximize | custom button → client rect exactly the **work area (0,0,3840,2088)**; while zoomed the edges answer `HTCLIENT` (no resize border); restore returns the exact prior rect |
| Snap Layouts | hovering the maximize button opened the flyout (25,291 changed pixels in a 1,000×650 probe); clicking its left zone snapped to **0,0,1920,2088** — exactly the left half of the work area |
| DWM switch | 34 samples at ~20 ms; stable luminance **25.09 / 155.99**; DWM cross-fades over **18 samples**; 0 black frames, 0 white frames, 0 out-of-envelope frames, 0 `WM_ERASEBKGND`; **1** Qt frame (the status-line update before sampling) |

The gate forces Mica, but the Tauri decision on this build is **Acrylic**
(build ≥ 22523). Both were applied and read back; `none` was exercised as the
fallback. The machine-specific decision is in `material_decision` in the
result JSON.

Raw record and evidence:

* [`results/win11-gate3.json`](results/win11-gate3.json) — all checks, every
  sample, initial and final window state.
* [`results/mica-dark.png`](results/mica-dark.png) /
  [`mica-light.png`](results/mica-light.png) — the forced Mica states.
* [`results/snap-flyout.png`](results/snap-flyout.png) — the flyout open over
  the custom chrome; [`snap-left-half.png`](results/snap-left-half.png) — after
  clicking the left zone.
* [`results/maximized.png`](results/maximized.png) — maximized content stops at
  the work area, taskbar still visible.
* [`results/dwm-before.png`](results/dwm-before.png),
  [`dwm-light-settled.png`](results/dwm-light-settled.png),
  [`dwm-dark-settled.png`](results/dwm-dark-settled.png) — the two settled
  tints and the state the switch started from.
* [`results/acrylic-dark.png`](results/acrylic-dark.png),
  [`none-dark.png`](results/none-dark.png) — the other two materials.

## Gate 4 result

**Pass: 167 checks, 0 failures.** Same machine / Qt / DPI as gate 3. Six
open/close cycles; each cycle opens a 900×620 frameless Mica window bound to the
**same** 120,000-row `QAbstractListModel` as the source window, waits for its
first frame, drags it, closes it, and samples PSS after the close settles.

| metric | six cycles | threshold |
|---|---:|---:|
| **tear-off latency** (request → first `frameSwapped`) | 67.21 / 59.08 / 60.56 / 76.82 / 58.52 / 59.40 ms, max **76.82 ms** | ≤ 150 ms |
| **PSS increment** (window open − baseline) | **+13.74 MB** cold, then +6.24 / +5.48 / +5.79 / +5.12 / +5.27 MB | ≤ 20 MB |
| PSS after close (vs same-cycle baseline) | +9.49 MB after the first, then +0.61 / +0.47 / +1.56 / −0.54 / −0.66 MB | — |
| closed PSS across the six cycles | 105.92 → 109.70 MB (**+3.78 MB**, shared-page noise) | — |
| closed private commit across the six cycles | 178.64 → 179.20 MB (**+0.56 MB**) | — |

Read that table as **one cold window plus five warm ones**:

* The first tear-off pays **+13.74 MB PSS** and leaves **~9.5 MB PSS / ~25 MB
  private commit** behind after close. That is one-time render-loop / QML / font
  machinery, not a per-window cost: cycles 2–6 open for **5.1–6.2 MB** and close
  back to within **+1.6 MB** of their own baseline. Private commit after close is
  flat (178–180 MB across all six), so the small PSS creep is shared-page noise.
* **No warm pool is needed.** The old WebView pool bought a ~65 ms adoption
  median for +200 MB; this spike tears off a real window in 59–77 ms while
  keeping the tab's 120k-row model shared, not copied.
* The latency splits into QML instantiation **0.8–1.6 ms**, native window
  creation + style + Mica + show **10–18 ms**, and the first presented frame
  (pipeline + swapchain + texture upload) **~47–66 ms**. The largest term is the
  first render, not window construction. The first cycle has the slowest window
  setup (`shown` 18.1 ms); the slowest first frame is a later cycle (76.8 ms),
  which is the expected run-to-run spread of the first D3D present.

The multi-window half is checked on the second HWND, not inherited from gate 3:

| check | second window |
|---|---|
| native chrome | `WS_THICKFRAME \| WS_MAXIMIZEBOX` present (style `0x16cf0000`), client covers the whole window |
| Mica | `DWMWA_SYSTEMBACKDROP_TYPE` reads back **2**; immersive-dark **true** |
| transparency path | `WS_EX_LAYERED` + `DwmEnableBlurBehindWindow` succeeded |
| hit-test map | caption `2`, close `1`, maximize `9`, content `1`, left border `10` |
| drag | real `SendInput` moved it exactly **+60, +40 px**; `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` both observed on this HWND |
| frames | counted per window (`frameSwapped`), not through the source window |
| close | `WM_CLOSE` → QML `closing`, window hides, then QML destroys it; source window still alive |

Raw record and per-cycle screenshots:
[`results/win11-gate4.json`](results/win11-gate4.json) and
[`results/gate4-window-{1..6}.png`](results/gate4-window-1.png).

## How the window is put together

1. **Transparent surface.** `QQuickWindow::setDefaultAlphaBuffer(true)` runs
   before the engine creates the window; QML sets `color: "transparent"` and
   `Qt.FramelessWindowHint`. The raw call that makes DWM composite behind it
   is `DwmEnableBlurBehindWindow` with an empty region — the same thing
   winit's `with_transparent(true)` does for Tauri — not
   `DwmExtendFrameIntoClientArea`.
2. **Mica/Acrylic.** `DwmSetWindowAttribute(38, DWMSBT_MAINWINDOW |
   DWMSBT_TRANSIENTWINDOW)`; the immersive-dark attribute (20) is what tints
   both. `material.rs` decides whether to ask at all.
3. **Native chrome.** Qt creates the window as `WS_POPUP`; the probe adds
   `WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU`
   after creation. Qt's frameless `WM_NCCALCSIZE` returns 0, so the client
   covers the whole window. The QML title bar is 40 dp, the three buttons are
   46 dp columns at its right, and `hit_test` maps those to the native codes.
4. **Native loops.** `HTCAPTION` and the resize codes are returned from
   `WM_NCHITTEST`; the probe does **not** call `DefWindowProc` itself. Qt's
   WndProc does that for an unhandled non-client mouse event, which starts the
   native modal move/size loop (and Aero Snap, double-click maximize, etc.).
5. **Snap Layouts.** Maximizing is the one button that must stay
   `HTMAXBUTTON` while merely hovered — that is the entire contract Windows 11
   uses to open the flyout. Minimize and close are ordinary `HTCLIENT` areas
   and are drawn/clicked in QML.
6. **Maximize.** Pressing the native maximize zone starts DefWindowProc's
   non-client button tracking, which **owns the matching
   `WM_NCLBUTTONUP`** — the native filter never sees it (`nc_up=0` in the
   result). The spike polls `GetAsyncKeyState(VK_LBUTTON)` until release, then
   calls `QWindow::showMaximized()`/`showNormal()` 80 ms later, after the
   tracking loop has unwound. Calling `ShowWindow`/`showMaximized` inside the
   tracking loop crashed Qt in the first build, and so did calling
   `DefWindowProc` from inside a `QAbstractNativeEventFilter`.

Two more Windows details are measured rather than assumed:

* **`WM_MOVING` / `WM_SIZING` never reach `QAbstractNativeEventFilter`** —
  they are in Qt's `QWindowsContext::isInputMessage`, so the filter is skipped.
  The proof that the native loop ran is `WM_ENTERSIZEMOVE` /
  `WM_EXITSIZEMOVE`, which do reach it.
* **Maximized client area.** With `WS_THICKFRAME` and Qt's frameless
  `WM_NCCALCSIZE` returning 0, a maximized window's client is the window rect,
  which Windows sizes to the work area **plus the invisible resize border** —
  22 px larger in both dimensions at 150%. The spike intercepts
  `WM_NCCALCSIZE` while zoomed and hands DWM the work area; without it the QML
  content ran 11 px under the taskbar on every edge.

## The DWM switch: what "no flicker" was measured against

The script leaves the QML theme alone and toggles only the DWM immersive-dark
attribute, so any frame difference is the backdrop switch, not a QML repaint.
At ~20 ms intervals (34 samples) the title-bar strip moves between stable
luminance 25.09 and 155.99 through 18 in-between samples: **DWM cross-fades
the tint over roughly 150–200 ms**. That is the compositor's animation, not a
flash. The checks reject a black frame (< 8), a white frame (> 247), or any
sample outside the two stable levels ± 6. None occurred. Qt presented one
frame during the sequence (the status line updated just before sampling began)
and `WM_ERASEBKGND` stayed at zero.

The first version of this measurement moved the sample steps through the QML
plan engine; each plan step's property writes made Qt present a frame per
sample, which hid whether DWM caused a repaint. The samples now run from one
repeating timer, with no QML property writes inside the window. The
`--idle-probe` diagnostic (idle → trivial invokable → screen capture) is kept
to show that neither an invokable nor the `BitBlt` capture causes a frame.

## Snap Layouts detection by pixels

The Windows 11 flyout is composited by DWM/Shell and **does not appear in
`EnumWindows`** — the run that produced this spike saw no new top-level
window. Visibility is therefore measured from the screen: capture the region
under the maximize button before the hover, then again during it, and count
changed pixels. The bounding box is both the evidence and the anchor for the
scripted click. The flyout's first tile is the two-column layout; its left
zone is 218 px left and 86 px down from the maximize button at 150%. Clicking
there produces exactly `[0, 0, 1920, 2088]`.

## Gate 4: how the second window is measured

1. **One model, two views.** The source window loads the 120k fixture through
   `dae_core::fs::listing::open_streamed_listing` (the gate-1 path). The torn-off
   window binds the same `DirectoryListModel` object. Tearing a tab out moves the
   view; it does not build a second row store.
2. **The view component is preloaded.** `main.qml` holds a `Component` for
   `TearOffWindow.qml`, so the measured interval is window creation and first
   frame, not QML file I/O — the shipping shell has its QML loaded already.
3. **Native style before show.** QML creates the window with `visible: false`
   and a unique title, calls `attachSecondary(title)` (the C++ looks it up in
   `QGuiApplication::topLevelWindows()`, forces the HWND with `winId()`, applies
   the gate-3 `WS_*` style), applies Mica, installs a per-HWND frame counter,
   and only then sets `visible = true`. A visible flash of the system frame
   would therefore fail the run rather than go unnoticed.
4. **Clock marks.** Rust records `request`, `created`, `shown`, `frame` and
   `settled`. `frame` is the first `frameSwapped` of the new window, which is the
   number compared with 150 ms. The phases are reported separately.
5. **PSS per cycle.** `QueryWorkingSetEx` (the gate-1 metric) is sampled after
   the loading settles (`baseline`), after the new window's first frame plus
   800 ms (`open`), and 800 ms after `destroy()` (`closed`). The query buffers
   are allocated by a warmup sample before the baseline, so they do not appear
   as window cost. The first cycle's one-time caches are reported separately from
   the warm cycles.
6. **Multi-window path.** The secondary HWND answers `WM_NCHITTEST` itself,
   carries its own DPR (title bar / button sizes), receives its own
   `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE`, and the script drags it with real
   `SendInput`. The source window stays alive and keeps its own counters.

The run is `./run-gate4.sh` (or the command below). It takes about 20 seconds
for six cycles, takes the foreground, and moves the torn-off window with real
input, exactly like gate 3.

## Build and run

```bash
cd bench/qt-window-spike
export QMAKE="C:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
export PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH"
cargo build --release

./target/release/qt-window-spike.exe \
  --out results/win11-gate3.json --shots results
```

or `./run.sh`. The run takes about 25 seconds and takes the foreground, moving
and resizing the window with real `SendInput` events. It exits 0 only when
every check passes.

Gate 4 reuses the same binary and component, but with a listing model and a
second window:

```bash
./run-gate4.sh              # six cycles, 120k fixture
CYCLES=3 DATASET=K:/dae-listing-bench/small ./run-gate4.sh
```

The script builds, passes `--gate4 --cycles 6 --path …`, writes
`results/win11-gate4.json`, and exits 0 only when every check passes.

Manual mode leaves the window open without the script; the sidebar switches
material and theme by hand:

```bash
./target/release/qt-window-spike.exe --manual
```

What to look for by hand:

1. Hover the maximize button: the Snap Layouts flyout appears; click a zone
   and the window snaps.
2. Drag an empty part of the title bar; drag an edge or corner to resize.
3. Click the max button: it maximizes; click again to restore. No system
   title bar ever appears, and the content stops at the taskbar.
4. Switch light/dark: the Mica tint cross-fades, with no white or black frame.

Diagnostic modes: `--probe-only` (one window grab), `--flyout-probe` (hover
and dump windows), `--snap-probe` (hover, click the first zone), `--max-probe`
(maximize through Qt's API without a click), `--idle-probe` (frame counter
with and without invokables/captures).

## Files

| path | role |
|---|---|
| `qml/main.qml` | custom chrome, material switch, the gate-3 script, and the gate-4 tear-off script |
| `qml/TearOffWindow.qml` | the torn-off window: same chrome, list view over the shared model |
| `src/material.rs` | ported backdrop decision (registry reads, build thresholds, tests) |
| `src/bridge.rs` | QML-facing `WindowProbe`; gate-3 result/log file writes |
| `src/model.rs` | `DirectoryListModel` over `dae-core`'s streamed listing batches |
| `src/tearoff.rs` | `TearOffProbe`: phase clock, PSS samples, secondary-window invokables |
| `src/memory.rs` | PSS / working-set / private-commit sampler |
| `src/format.rs` | delegate text (size, UTC mtime) |
| `cpp/window_probe.h` | DWM backdrop, per-HWND style/hit test/`WM_NCCALCSIZE`, `SendInput`, captures |
| `src/main.rs` | `QGuiApplication`, alpha buffer, QML engine |
| `run.sh` / `run-gate4.sh` | build + gate 3 / gate 4 |
| `results/win11-gate3.json` | all 63 gate-3 checks and the raw sample/stats record |
| `results/win11-gate4.json` | all 167 gate-4 checks, six cycles, per-cycle timings and PSS |

## Method notes

* **One machine.** 150% scaling, 3840×2160, Windows 11 build 26340. The
  invisible-border width (11 px), the flyout offsets and the cross-fade sample
  count are scale/version specific; the checks state them as measured, not as
  universal constants.
* **Real input, guarded.** Mouse gestures are `SendInput` at absolute virtual
  desktop coordinates; the window is brought forward first and the drag
  starts only when it is foreground. The run leaves the cursor where the last
  gesture ended.
* **Screen, not window DC.** Captures use `BitBlt(..., CAPTUREBLT)` from the
  screen, so DWM-composited Mica and the layered flyout are included. A
  window-DC capture would show neither.
* **Two decisions, one source.** `material_decision` in the result is what the
  Tauri module would choose on this machine (Acrylic); the gate separately
  forces Mica and checks the read-back, so neither the decision nor the
  application path is inferred from the other.
* **Maximize hover is still native.** Because the maximize button is always
  `HTMAXBUTTON`, QML gets no hover events for it; the spike draws no hover
  state on that button. A shipping shell should forward `WM_NCMOUSEMOVE` with
  `TME_NONCLIENT` to QML (about the same size as the IME focus guard). The
  minimize and close buttons are client areas and do have QML hover.
* **Gate 4 is one cold window plus warm ones.** The first cycle commits the
  render loop's one-time caches (~9.5 MB PSS / ~25 MB private after close);
  cycles 2–6 add 5.1–6.2 MB PSS and return to their baseline. Reporting the
  average across all six would hide that split, so the table shows both the
  cold cycle and the warm range.
* **PSS vs private commit on close.** PSS counts resident shared pages, so the
  closed number drifts by a page-cache-sized amount (+3.78 MB over five extra
  cycles). Private commit is flat (+0.56 MB over the same span), which is the
  reason the no-leak check uses commit and the PSS numbers are reported raw.
* **Tear-off, not tab-drag.** The measured interval starts when the script asks
  for the window. The drag gesture that would trigger it in the shell is not
  part of the 150 ms; gate 3 already measured the native drag loop.
* **The source view stays open.** The increment is the gross cost of the new
  view while the source window keeps rendering. A real tear-off that closes the
  source tab can only be cheaper; the number here is the conservative one.
