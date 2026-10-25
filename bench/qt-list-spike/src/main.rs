//! Gate 1 of `docs/decisions/engine-qt-quick.md` §5.
//!
//! A 120k-row `ListView` fed by the real `dae_core::fs::listing` pipeline.
//! QML drives the scroll itself from `Window::frameSwapped`, so every frame
//! moves the content offset and asks for new delegate data; the app measures
//! the intervals between those signals and writes one JSON result.
//!
//! Default is vsync off (`QSG_NO_VSYNC=1`): the gate is about how long a frame
//! *takes*, not how long the presenter makes it wait. `--vsync on` measures the
//! same workload at the display cadence (≈16.7 ms), where a long interval means
//! a dropped frame instead.
//!
//! ```text
//! cargo build --release
//! PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH" \
//!   ./target/release/qt-list-spike.exe --path K:/dae-listing-bench/huge
//! ```

mod bridge;
mod format;
mod memory;

use std::env;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    let arguments: Vec<String> = env::args().collect();

    let vsync = argument(&arguments, "--vsync")
        .map(|value| value != "off")
        .unwrap_or(false);
    if vsync {
        env::remove_var("QSG_NO_VSYNC");
    } else {
        env::set_var("QSG_NO_VSYNC", "1");
    }

    if arguments.iter().any(|value| value == "--selftest") {
        eprintln!("spike: memory sample = {:?}", memory::sample());
        eprintln!("spike: cpu time = {:?}", memory::process_cpu_time());
        return;
    }

    if argument(&arguments, "--backend").is_none() && env::var_os("QSG_RHI_BACKEND").is_none() {
        env::set_var("QSG_RHI_BACKEND", "d3d11");
    }

    let mut application = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    match engine.as_mut() {
        Some(engine) => {
            engine.load(&QUrl::from("qrc:/qt/qml/dae/listspike/qml/main.qml"));
        }
        None => eprintln!("could not create QQmlApplicationEngine"),
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}

fn argument(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|value| value == name)?;
    arguments.get(position + 1).cloned()
}
