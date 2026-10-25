//! Gate 3 of `docs/decisions/engine-qt-quick.md` §5.
//!
//! A frameless QQuickWindow with a custom title bar. The backdrop decision is
//! Rust's (`src/material.rs`, ported from the Tauri module); the DWM calls,
//! hit tests and input injection are native (`cpp/window_probe.h`). This file
//! draws the chrome and, in the default mode, drives the scripted run and
//! writes one JSON result.
//!
//! ```text
//! cargo build --release
//! PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH" \
//!   ./target/release/qt-window-spike.exe --out results/win11-gate3.json
//! ```

mod bridge;
mod material;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    if std::env::var_os("QSG_RHI_BACKEND").is_none() {
        std::env::set_var("QSG_RHI_BACKEND", "d3d11");
    }

    let mut application = QGuiApplication::new();
    // Translucent QQuickWindows need the alpha buffer chosen before the first
    // window exists. `DwmEnableBlurBehindWindow` then makes DWM composite the
    // backdrop behind it instead of the window's own black.
    bridge::enable_alpha();

    let mut engine = QQmlApplicationEngine::new();
    match engine.as_mut() {
        Some(engine) => {
            engine.load(&QUrl::from("qrc:/qt/qml/dae/windowspike/qml/main.qml"));
        }
        None => eprintln!("could not create QQmlApplicationEngine"),
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}
