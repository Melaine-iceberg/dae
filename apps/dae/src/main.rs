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
    // `DAE_STARTUP_PERF=1` prints process-relative marks on stderr. Started
    // before anything else so the first mark lines up with the spawn a harness
    // measures.
    dae_qt::window::startup_begin();
    dae_qt::window::startup_mark("main");

    // Armed before QGuiApplication and before `logging::init`: the harness
    // starts its grace window at process spawn, so the timer has to count from
    // the same instant. No-op unless `perf-train` is compiled in.
    perf_train::arm();

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
    dae_qt::window::startup_mark("qgui-application");

    // The first font-database query builds the whole Windows database
    // (200–300 ms); every query API pays it, and `apply_system_font` no longer
    // queries. Qt documents `QFontDatabase` as thread-safe, so pay it here on a
    // worker while the main thread does logging, claiming and engine load.
    // Measured (10 interleaved runs, release + AOT): first-frame median
    // 1200 → 1133 ms. The first text that needs the database waits on it.
    #[cfg(windows)]
    std::thread::spawn(dae_qt::window::warm_font_database);

    // Before `claim`: a failed scheme registration or a refused instance lock
    // is exactly the kind of failure nobody can reproduce later, and starting
    // here means it lands in the file rather than in a stderr a release build
    // does not have.
    dae_qt::logging::init();
    dae_qt::window::startup_mark("logging");

    // A later launch must not create a second shell. The owner receives the
    // command line over a local socket; this process exits before the engine or
    // any window exists. Claiming also registers `dae://` for this user.
    if !dae_qt::activation::claim() {
        return;
    }
    dae_qt::window::startup_mark("claim");

    // The alpha buffer is chosen before the engine creates the first window;
    // DwmEnableBlurBehindWindow then composites the backdrop behind it.
    dae_qt::init();
    dae_qt::window::startup_mark("qt-init");

    // Registers the precompiled QML units before anything can ask for them.
    // This is also the only reference to `cpp/qml_cache_init.cpp`, which is what
    // keeps the units linked at all.
    unsafe { dae_qml_cache_init() };

    let mut engine = QQmlApplicationEngine::new();
    let engine_ptr = engine.as_mut_ptr();
    dae_qt::window::startup_mark("qml-engine");
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
        dae_qt::window::startup_mark("qml-loaded");
        dae_qt::window::watch_first_frame(engine_ptr as usize);
    } else {
        log::error!(target: "dae", "could not create QQmlApplicationEngine");
    }

    if let Some(application) = application.as_mut() {
        application.exec();
    }
}

/// Quits on a timer, for the profiled builds only.
///
/// Ported from the Tauri shell (`src-tauri/src/lib.rs::spawn_training_quit`)
/// because the reason is the same: LLVM's PGO counters are written from
/// `atexit` / `.fini_array` hooks, so the process has to leave through the C
/// runtime's exit path for a profile to exist, and an unattended training run
/// (`scripts/opt/pgo.mjs train`) has nobody to close it. `DAE_PERF_TRAIN_QUIT_MS`
/// is unset in every normal launch.
///
/// `std::process::exit` reaches `ExitProcess` on Windows and skips those hooks,
/// and killing the process from outside skips them by definition. The Qt shell
/// goes one step further than the Tauri one: it writes the counters with
/// `__llvm_profile_write_file` and then leaves through `_exit` (Unix) or
/// `TerminateProcess` (Windows), because Qt's teardown measured 20s on Windows
/// and can hang under `xvfb` — long enough to fall outside the harness's grace
/// window.
#[cfg(feature = "perf-train")]
mod perf_train {
    pub fn arm() {
        let Some(milliseconds) = std::env::var("DAE_PERF_TRAIN_QUIT_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
        else {
            return;
        };

        // stderr is captured into `.perf/training.log`; the two lines are how a
        // failed run tells "the hook never fired" from "it fired and the exit
        // hung". The absolute timestamp correlates the log with the CI job.
        eprintln!(
            "[perf] training quit armed at {} (+{milliseconds}ms)",
            epoch_ms()
        );

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(milliseconds));
            flush();
            finish();
        });
    }

    fn epoch_ms() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0)
    }

    /// compiler-rt's own "save now" entry point, the function its exit handler
    /// calls. It covers counters from the Rust crates and, when the bridge was
    /// compiled by clang, the cxx-qt C++ units too: both register with the one
    /// profiling runtime `-Cprofile-generate` links.
    fn flush() {
        // SAFETY: the symbol is part of that runtime, and `perf-train` is only
        // ever set for the instrumented build; any other build fails to link.
        unsafe extern "C" {
            fn __llvm_profile_write_file() -> i32;
        }
        // SAFETY: compiler-rt documents this as callable from any thread.
        let status = unsafe { __llvm_profile_write_file() };
        eprintln!(
            "[perf] profile counters flushed at {} (status {status})",
            epoch_ms()
        );
    }

    #[cfg(not(windows))]
    fn finish() -> ! {
        // SAFETY: `_exit` skips every exit handler. The counters were already
        // written by `flush`, and the handlers Qt and the platform plugins
        // register can take seconds or hang under xvfb.
        unsafe extern "C" {
            fn _exit(status: i32) -> !;
        }
        unsafe { _exit(0) }
    }

    #[cfg(windows)]
    fn finish() -> ! {
        // `_exit` is not enough here: it reaches `ExitProcess`, which still
        // runs every loaded DLL's detach routine, and Qt's D3D11 backend spends
        // ~20s there (`QDxgiVSyncService not destroyed in time`) — longer than
        // the harness's grace window at 45s. `TerminateProcess` skips detach
        // entirely, which is safe because the counters are already on disk.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
            fn TerminateProcess(process: *mut core::ffi::c_void, exit_code: u32) -> i32;
        }
        unsafe {
            TerminateProcess(GetCurrentProcess(), 0);
        }
        // `TerminateProcess` only returns on failure; failing to end the
        // process is still better reported by the harness's kill than by
        // falling through into Qt's teardown.
        std::process::abort()
    }
}

/// An ordinary build has no profiling runtime, so there is nothing to arm.
#[cfg(not(feature = "perf-train"))]
mod perf_train {
    pub fn arm() {}
}
