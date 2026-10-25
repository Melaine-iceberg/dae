# qt-window-spike — gate 3 of the Qt Quick decision

Standalone spike for **§5 gate 3** of
[`docs/decisions/engine-qt-quick.md`](../../docs/decisions/engine-qt-quick.md):

> Windows 11 frameless window + Mica + custom title bar + drag / resize /
> Snap Layouts — all native, and a DWM tint switch that does not flash.

It is not part of the app. The backdrop **decision** is a port of
[`src-tauri/src/window_material/windows.rs`](../../src-tauri/src/window_material/windows.rs)
(the same `CurrentBuild` / `EnableTransparency` registry reads and the same
22000 / 22523 thresholds), because the Qt shell must not disagree with the
Tauri shell about which machine has a backdrop. The **application** is raw
Win32 (`DwmEnableBlurBehindWindow` + `DWMWA_SYSTEMBACKDROP_TYPE`), since Qt has
no `window-vibrancy` equivalent. The title bar is drawn in QML; the native
half answers `WM_NCHITTEST`, keeps the move/size loops native, and drives real
`SendInput` gestures for the scripted run.

## Result

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
| `qml/main.qml` | custom chrome, material switch, the scripted run and its checks |
| `src/material.rs` | ported backdrop decision (registry reads, build thresholds, tests) |
| `src/bridge.rs` | QML-facing `WindowProbe`; result/log file writes |
| `cpp/window_probe.h` | DWM backdrop, native hit test, `WM_NCCALCSIZE`, `SendInput`, captures |
| `src/main.rs` | `QGuiApplication`, alpha buffer, QML engine |
| `run.sh` | build + the gate run |
| `results/win11-gate3.json` | all 63 checks and the raw sample/stats record |

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
