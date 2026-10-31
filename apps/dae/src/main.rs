//! The Qt Quick shell binary.
//!
//! `main` only creates the application and the QML engine; views live in
//! `qml/`, the bridge in `crates/dae-qt`, and the domain in `dae-core`.

// A GUI binary must not allocate a console window in release. Debug keeps it,
// because that is where the startup profile lines are printed.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Ported from the Tauri shell's `main.rs`, and it belongs here for the same
// reason it lived in that bin target: `#[global_allocator]` is installed by the
// crate that links the final executable. `dae-qt` and `dae-core` arrive as
// rlibs, so declaring it there would make the shim a link-order dependency.
//
// Scope: this covers Rust-side allocations only. The QML engine, `QString`,
// `QQuickItem` and the scene graph allocate inside the prebuilt Qt DLLs, whose
// import tables point straight at `ucrtbase.dll` — a `malloc` symbol in this
// binary cannot intercept them. That is acceptable because the measured win was
// never on the Qt side: it is the directory enumeration in `dae-core`.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

// Defined in `cpp/qml_cache_init.cpp`. The call does nothing on its own: it is
// the reference the linker needs to keep the generated QML units (and the loader
// that registers them) from being dropped out of the C++ static archive. Without
// it the engine silently compiles the `.qml` sources at run time — see
// `docs/qml-aot.md`.
unsafe extern "C" {
    fn dae_qml_cache_init();
}

fn main() {
    // Windows pins D3D11, which is the backend gates 1–4 were measured on.
    // macOS and Linux keep Qt's own choice (Metal and OpenGL respectively);
    // the gated native paths there are Qt's platform integration, not a
    // renderer this shell chose.
    #[cfg(windows)]
    if std::env::var_os("QSG_RHI_BACKEND").is_none() {
        // No other thread exists yet; this is the same single-threaded setup
        // the spike did before creating QGuiApplication.
        unsafe {
            std::env::set_var("QSG_RHI_BACKEND", "d3d11");
        }
    }

    let mut application = QGuiApplication::new();
    // Before `claim`: a failed scheme registration or a refused instance lock
    // is exactly the kind of failure nobody can reproduce later, and starting
    // here means it lands in the file rather than in a stderr a release build
    // does not have.
    dae_qt::logging::init();

    // A later launch must not create a second shell. The owner receives the
    // command line over a local socket; this process exits before the engine or
    // any window exists. Claiming also registers `dae://` for this user.
    if !dae_qt::activation::claim() {
        return;
    }

    // The alpha buffer is chosen before the engine creates the first window;
    // DwmEnableBlurBehindWindow then composites the backdrop behind it.
    dae_qt::init();
    // Registers the precompiled QML units before anything can ask for them.
    // This is also the only reference to `cpp/qml_cache_init.cpp`, which is what
    // keeps the units linked at all.
    unsafe { dae_qml_cache_init() };

    let mut engine = QQmlApplicationEngine::new();
    let engine_ptr = engine.as_mut_ptr();
    // The persisted language has to be installed before `Main.qml` is loaded,
    // otherwise the first window paints its source strings once and only
    // retranslates on the next switch. The engine is attached so a change from
    // the settings panel can retranslate live.
    dae_qt::i18n::attach_engine(engine_ptr as usize);
    dae_qt::i18n::apply_stored();
    if let Some(mut engine) = engine.as_mut() {
        dae_qt::preview::register_image_providers(engine_ptr);
        engine
            .as_mut()
            .load(&QUrl::from("qrc:/qt/qml/dae/qml/Main.qml"));
    } else {
        log::error!(target: "dae", "could not create QQmlApplicationEngine");
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}
