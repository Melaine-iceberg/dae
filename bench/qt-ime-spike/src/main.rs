//! Gate 2 of `docs/decisions/engine-qt-quick.md` §5.
//!
//! A QML window with three custom-styled `TextInput`s. `qml/main.qml` drives a
//! scripted session through `ImeProbe`; the native half (cpp/ime_probe.h)
//! injects keystrokes, reads the IMM32 composition state and captures the
//! desktop while the Microsoft Pinyin candidate window is up.
//!
//! ```text
//! cargo build --release
//! PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH" \
//!   ./target/release/qt-ime-spike.exe --out results/win11-ime.json
//! ```

mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    let mut application = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    match engine.as_mut() {
        Some(engine) => {
            engine.load(&QUrl::from("qrc:/qt/qml/dae/imespike/qml/main.qml"));
        }
        None => eprintln!("could not create QQmlApplicationEngine"),
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}
