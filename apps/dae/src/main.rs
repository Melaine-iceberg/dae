//! The Qt Quick shell binary.
//!
//! `main` only creates the application and the QML engine; views live in
//! `qml/`, the bridge in `crates/dae-qt`, and the domain in `dae-core`.

// A GUI binary must not allocate a console window in release. Debug keeps it,
// because that is where the startup profile lines are printed.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    if std::env::var_os("QSG_RHI_BACKEND").is_none() {
        // No other thread exists yet; this is the same single-threaded setup
        // the spike did before creating QGuiApplication.
        unsafe {
            std::env::set_var("QSG_RHI_BACKEND", "d3d11");
        }
    }

    let mut application = QGuiApplication::new();

    // The alpha buffer is chosen before the engine creates the first window;
    // DwmEnableBlurBehindWindow then composites the backdrop behind it.
    dae_qt::init();

    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        engine
            .as_mut()
            .load(&QUrl::from("qrc:/qt/qml/dae/qml/Main.qml"));
    } else {
        eprintln!("dae: could not create QQmlApplicationEngine");
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}
