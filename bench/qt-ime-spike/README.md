# qt-ime-spike — gate 2 of the Qt Quick decision

Standalone spike for **§5 gate 2** of
[`docs/decisions/engine-qt-quick.md`](../../docs/decisions/engine-qt-quick.md):

> Chinese IME in a custom-styled QML `TextInput` — no lost preedit characters,
> the candidate window stays on the caret, and composition can be cancelled.

It is not part of the app. The window has three custom-styled `TextInput`s: a
path-bar field that mirrors `text` into a label on every change, a command-bar
field at the bottom-right edge of the screen, and a field inside a scrolled
`Flickable`. A Rust/cxx-qt probe and a header-only native half inject keys with
`SendInput`, read the IMM32 composition directly, compare it against the QML
properties, and capture the desktop while the Microsoft Pinyin candidate window
is up.

## Result

**Pass: 7 scenarios, 214 checks, 0 failures.**
Windows 11 Insider / Qt 6.12.0 / 3840×2160 at 150% (DPR 1.5) / Microsoft Pinyin
(`HKL 0x08040804`, `TextInputHost.exe`).

| scenario | typed | preedit (QML = IMM32) | result |
|---|---|---|---|
| A normal commit | `nihao` | `ni'hao` | commits `你好` |
| A backspace | `nihao` → 2×Backspace → `ao` | `ni'hao` → `nih` → `ni'hao` | commits `你好` |
| A Escape cancel | `zhongwen` | `zhong'wen` | preedit cleared, text stays `keep`, no commit |
| A rapid | `zhongwenshuru` at 35 ms/key | `zhong'wen'shu'ru` | commits `中文输入` |
| B screen bottom-right | `nihao` | `ni'hao` | candidate flips **above** the caret |
| B click away | `shijie`, then click A | `shi'jie` | preedit cleared, B stays `keep`, no raw pinyin |
| C scrolled Flickable (`contentY=140`) | `wenjian` | `wen'jian` | commits `文件`, candidate follows the offset |

The apostrophes are Microsoft Pinyin's own syllable separators, present in
**both** channels. The checks compare the letter sequence, so they are not test
artifacts.

### Numbers

* **Preedit integrity.** At every checkpoint (the rapid scenario compares the
  whole burst), QML `TextInput.preeditText` equalled IMM32 `GCS_COMPSTR` letter
  for letter; all 68 `SendInput` deliveries (letters, Space, Esc, Backspace) were
  accepted. The committed `text` delta equalled `GCS_RESULTSTR` exactly
  (`你好`, `中文输入`, `文件`).
* **Candidate anchor.** The `CANDIDATEFORM` Qt hands to the IME
  (`CFS_EXCLUDE`, client pixels) vs
  `QInputMethod::cursorRectangle() × devicePixelRatio`: maximum error over all
  seven composition snapshots is **0.43 px** on x and **0 px** on y. At the
  screen's bottom-right edge the IME flips the list above the caret.
* **Cancellation.** Escape clears the preedit, leaves `text` unchanged, emits no
  commit, and produces `WM_IME_ENDCOMPOSITION`.
* **Input pipeline.** 68 `SendInput` attempts, 68 delivered, 0 aborted because
  of a lost foreground. The probe refuses to type unless its own window is both
  foreground and focused.
* **Scaling.** The app window is 1024×880 logical; Qt reports DPR 1.5 and a
  3840×2160 physical screen — the setting where candidate-window placement bugs
  usually surface.

### The one non-obvious finding: focus loss commits raw pinyin

Clicking from a composing field into another field does **not** cancel the
composition in Qt 6.12. `QWindowsInputContext::reset()` commits the stored
composition string to the *old* focus object:

```cpp
// qtbase/src/plugins/platforms/windows/qwindowsinputcontext.cpp
void QWindowsInputContext::reset()
{
    ...
    if (m_compositionContext.isComposing && !m_compositionContext.focusObject.isNull()) {
        QInputMethodEvent event;
        if (!m_compositionContext.composition.isEmpty())
            event.setCommitString(m_compositionContext.composition);
        QCoreApplication::sendEvent(m_compositionContext.focusObject, &event);
        ...
```

The string is the raw composition (`shi'jie`), not the converted candidate. This
is control-agnostic: a `QTextEdit` would receive the same `QInputMethodEvent`, so
switching the text layer would not fix it. The spike carries a ~50-line native
guard (`FocusCancelFilter` in [`cpp/ime_probe.h`](cpp/ime_probe.h)): on a mouse
press outside the focused item, or on Tab, it calls
`ImmNotifyIME(NI_COMPOSITIONSTR, CPS_CANCEL)` *before* Qt changes the focus
object. The platform context then has nothing to commit, the preedit clears, and
the old field keeps `keep`. The real shell needs the same guard.

### Candidate-window evidence

The TextInputHost IMM bridge returns nothing from `ImmGetCandidateList`, so the
candidate strings and the final window rectangle are verified from the capture
rather than from IMM. BitBlt with `CAPTUREBLT` includes the layered popup.
Checked-in crops (app content only; the full desktop grabs are not checked in):

* [`results/win11-ime-a.png`](results/win11-ime-a.png) — normal field, list below
  the composition.
* [`results/win11-ime-b.png`](results/win11-ime-b.png) — bottom-right field, list
  flipped above the caret.
* [`results/win11-ime-c.png`](results/win11-ime-c.png) — scrolled field, list
  follows the content offset.

Raw record (all 214 checks, snapshots, native `WM_IME_*` log):
[`results/win11-ime.json`](results/win11-ime.json).

## Build and run

```bash
cd bench/qt-ime-spike
export QMAKE="C:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
export PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH"
cargo build --release

QT_ASSUME_STDERR_HAS_CONSOLE=1 ./target/release/qt-ime-spike.exe \
  --out results/win11-ime.json --shots results/shots
```

The window takes the foreground, waits for focus, and runs the script for about
40 seconds. It writes one JSON document plus a `.log` next to it, and quits with
0 only when every check passes.

Manual mode skips the script and leaves the window open; type with the real IME
and press <kbd>F12</kbd> to capture the desktop:

```bash
./target/release/qt-ime-spike.exe --manual --keep-open
```

What to look for:

1. Type `nihao` in A: the preedit is underlined inline and the candidate list is
   attached to it.
2. Press <kbd>Esc</kbd>: the preedit disappears and A's text is unchanged.
3. Type `nihao` in B at the bottom-right: the candidate list flips above the
   caret instead of running off the screen.
4. Type `wenjian` in C inside the scrolled container: the list follows the
   content offset, not the viewport origin.
5. Start a composition in B, then click A: the composition is cancelled and B
   keeps its text (the focus-cancel guard).

## Method notes

* **Two channels, not one.** The native side reads the HIMC on every
  `WM_IME_COMPOSITION` (`GCS_COMPSTR` / `GCS_RESULTSTR`) and again in each
  snapshot; QML contributes `preeditText`, `displayText` and
  `inputMethodComposing`. A lost character in QML's property plumbing shows up
  as QML ≠ native even when the IME itself is fine.
* **Anchor formula.** `qwindowsinputcontext.cpp` does
  `cursorRectangle.topLeft() * QHighDpiScaling::factor(window)` and passes that
  to `CANDIDATEFORM` in client coordinates (no window origin added). The probe
  therefore compares the form's point with
  `QInputMethod::cursorRectangle() × window.devicePixelRatio()`, and separately
  maps both to screen for the screenshot crop.
* **Synthetic keys.** `SendInput` is guarded by
  `GetForegroundWindow() == hwnd` and `GetGUIThreadInfo().hwndFocus == hwnd`; if
  either check fails it aborts rather than typing into another application. A
  real click is sent into the field before each scenario, so QML item focus and
  the OS focus path are both exercised.
* **Focus-cancel guard scope.** A press inside the focused item (including its
  cursor delegate) does not cancel; only presses that would move focus away do.
* **One machine, one IME.** The run is Windows 11 + Microsoft Pinyin at 150%.
  The absolute numbers (and the IME's segmentation, e.g. `ni'hao` vs `n'ni'hao`)
  are specific to this dictionary and configuration; the checks are relational
  (QML vs native), not dictionary-based.

## Files

| path | role |
|---|---|
| `qml/main.qml` | three styled inputs, live mirror, scripted scenarios, checks |
| `src/bridge.rs` | QML-facing `ImeProbe` object and JSON/log file writes |
| `cpp/ime_probe.h` | IMM32 reads, SendInput, desktop capture, focus-cancel guard |
| `results/win11-ime.json` | checks, per-step timeline, snapshots, native event log |
| `results/win11-ime-{a,b,c}.png` | app-only candidate-window crops |
