# qt-toolchain: does cxx-qt build and link on this machine?

Not a benchmark — a toolchain smoke test for `docs/decisions/engine-qt-quick.md`
§6 ("双工具链" was the largest open risk). It is not part of any build and no
workspace references it.

It answers one question end to end: **does a Rust `#[qobject]` become a QML type,
load into a `QQmlApplicationEngine`, and call back into Rust?** The QML calls
`incrementNumber()` from `Component.onCompleted`, and the Rust side prints.

```bash
# Windows, Qt installed by aqt (see the decision doc for the exact aqt version)
export QMAKE="C:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
cargo build
PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH" ./target/debug/qt-smoke.exe
```

Expected, and what this machine printed:

```
rust: starting
rust: number -> 2
```

The second line is the proof: it can only be printed by the QML delegate having
resolved `dae.smoke` (the module `cxx-qt-build` generated), instantiated the
`SmokeObject`, and invoked a `#[qinvokable]` on it.
