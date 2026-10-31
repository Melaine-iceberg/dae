use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// The QPA header under `include/QtGui/<version>/QtGui`. Qt ships it in every
/// install but does not have a `Qt6GuiPrivate` link target in aqt builds, so
/// the path is resolved rather than requested as a Qt module.
#[cfg(target_os = "linux")]
fn qt_private_gui_include() -> Option<std::path::PathBuf> {
    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "qmake".to_owned());
    let output = std::process::Command::new(qmake)
        .args(["-query", "QT_INSTALL_HEADERS"])
        .output()
        .ok()?;
    let headers = std::path::PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let gui = headers.join("QtGui");
    if gui.join("qpa/qplatformnativeinterface.h").exists() {
        return Some(gui);
    }
    for entry in std::fs::read_dir(&gui).ok()? {
        let candidate = entry.ok()?.path().join("QtGui");
        if candidate.join("qpa/qplatformnativeinterface.h").exists() {
            return Some(candidate);
        }
    }
    None
}

/// Adds the LLVM profile flags `scripts/opt/pgo.mjs` asks for to the C++ bridge
/// and the cxx-qt generated glue. This is the only part of the Qt shell that is
/// ours to optimise besides Rust: Qt itself arrives prebuilt.
///
/// Only clang can contribute to the `.profdata` rustc consumes: MSVC and GCC
/// have their own profilers whose output `llvm-profdata` cannot merge. The
/// compiler's LLVM major also has to match rustc's, because the profraw format
/// is versioned. Anything else is warned about and skipped — the Rust half still
/// gets its profile, which is where the measured startup cost is.
fn apply_cxx_profile(cc: &mut cc::Build) {
    for name in ["DAE_PERF_CXX_PROFILE_GENERATE", "DAE_PERF_CXX_PROFILE_USE"] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let generate =
        std::env::var_os("DAE_PERF_CXX_PROFILE_GENERATE").filter(|value| !value.is_empty());
    let use_profile =
        std::env::var_os("DAE_PERF_CXX_PROFILE_USE").filter(|value| !value.is_empty());
    if generate.is_none() && use_profile.is_none() {
        return;
    }

    let compiler = cc.get_compiler();
    if !compiler.is_like_clang() {
        println!(
            "cargo:warning=C++ bridge is not profiled: {:?} is not clang, and MSVC/GCC profiles cannot be merged with rustc's",
            compiler.path()
        );
        return;
    }

    let rustc_major = rustc_llvm_major();
    let compiler_major = compiler_llvm_major(&compiler);
    if let (Some(rustc), Some(clang)) = (rustc_major, compiler_major) {
        if rustc != clang {
            println!(
                "cargo:warning=C++ bridge is not profiled: clang is LLVM {clang}, rustc is LLVM {rustc}; profraw formats are versioned"
            );
            return;
        }
    }

    if let Some(directory) = generate {
        // clang writes this file name verbatim (rustc appends its own default
        // name), and `%m`/`%p` keep concurrent runs apart.
        let file = std::path::PathBuf::from(directory).join("cxx-%m-%p.profraw");
        cc.flag(format!(
            "-fprofile-instr-generate={}",
            file.to_string_lossy().replace('\\', "/")
        ));
    }
    if let Some(profile) = use_profile {
        cc.flag(format!(
            "-fprofile-instr-use={}",
            std::path::PathBuf::from(profile)
                .to_string_lossy()
                .replace('\\', "/")
        ));
        // The profile is shared with the Rust half; untrained functions are
        // expected and must not turn a release build into a warning flood.
        cc.flag("-Wno-profile-instr-unprofiled");
    }
}

fn compiler_llvm_major(compiler: &cc::Tool) -> Option<u32> {
    let output = std::process::Command::new(compiler.path())
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let rest = text.split("clang version ").nth(1)?;
    rest.split('.').next()?.parse().ok()
}

fn rustc_llvm_major() -> Option<u32> {
    let rustc = std::env::var_os("RUSTC")?;
    let output = std::process::Command::new(rustc).arg("-vV").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| line.starts_with("LLVM version:"))?;
    line.split_whitespace()
        .nth(2)?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn main() {
    // The native half talks to DWM, IMM32 and the window metrics API directly.
    // cxx-qt only links Qt.
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=dwmapi");
        println!("cargo:rustc-link-lib=imm32");
    }

    // Wayland blur binds a protocol on Qt's own `wl_display`; the generated
    // protocol code and the client library are only needed on Linux.
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=wayland-client");

    let mut builder = CxxQtBuilder::new_qml_module(QmlModule::new("dae"))
        .qt_module("Quick")
        .qt_module("Network")
        .include_dir("cpp")
        .cpp_files([
            "cpp/activation.cpp",
            "cpp/activation.h",
            "cpp/clipboard.cpp",
            "cpp/clipboard.h",
            "cpp/drag.cpp",
            "cpp/drag.h",
            "cpp/image_provider.cpp",
            "cpp/image_provider.h",
            "cpp/platform_watch.cpp",
            "cpp/platform_watch.h",
            "cpp/qml_debug.cpp",
            "cpp/terminal_item.cpp",
            "cpp/terminal_item.h",
            "cpp/translator.cpp",
            "cpp/translator.h",
        ])
        .files([
            "src/activation.rs",
            "src/appearance.rs",
            "src/config.rs",
            "src/icons.rs",
            "src/ime.rs",
            "src/model.rs",
            "src/open_with_model.rs",
            "src/preferences.rs",
            "src/preview.rs",
            "src/properties.rs",
            "src/search_model.rs",
            "src/shell.rs",
            "src/shell_command_model.rs",
            "src/sidebar_model.rs",
            "src/sidebar_tree_model.rs",
            "src/spaces_model.rs",
            "src/startup.rs",
            "src/overview_model.rs",
            "src/terminal.rs",
            "src/trash_model.rs",
            "src/updater.rs",
            "src/window.rs",
            "src/workspace.rs",
        ]);

    // One shared `cc::Build`, so this reaches the bridge sources, the cxx-qt
    // generated glue and the moc products alike.
    builder = unsafe { builder.cc_builder(|cc| apply_cxx_profile(cc)) };

    #[cfg(target_os = "linux")]
    {
        let private_include = qt_private_gui_include()
            .expect("Qt private QPA headers are required for Wayland background blur");
        // SAFETY: only an include path is added.
        builder = unsafe {
            builder.cc_builder(|cc| {
                cc.include(&private_include);
            })
        };
    }

    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        // SAFETY: the only change is one compile definition. QT_QML_DEBUG
        // opens a debug service that can load arbitrary QML, so it stays out
        // of release builds.
        unsafe {
            builder = builder.cc_builder(|cc| {
                cc.define("QT_QML_DEBUG", None);
            });
        }
    }

    builder.build();

    // The Wayland protocol is generated C, and its `wl_interface` tables are
    // what `window.h` binds to inside `extern "C"`. cxx-qt-build compiles every
    // `cpp_files` entry as C++, where a namespace-scope `const` has internal
    // linkage: the tables are emitted as private symbols and the C names stay
    // undefined, which the linker reports as `undefined symbol:
    // ext_background_effect_manager_v1_interface`. Compiled on its own as C,
    // after the bridge, so the archive lands after the objects that reference
    // it — the same ordering reason as the macOS backdrop below.
    #[cfg(target_os = "linux")]
    cc::Build::new()
        .file("cpp/wayland/ext-background-effect-v1-protocol.c")
        .compile("dae_wayland_protocol");

    // The macOS backdrop is Objective-C++ and is compiled separately so ARC
    // applies to that one file without leaking the flag into the C++ bridge.
    // It is built after the bridge so the archive appears after the objects
    // that reference `dae_qt_macos_apply_blur` on the linker line.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=AppKit");
        cc::Build::new()
            .file("cpp/macos_backdrop.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_backdrop");
        // The native menu is AppKit-only and has no C++ equivalent. It is a
        // separate archive for the same reason as the backdrop: ARC applies to
        // this file alone and the archive lands after the objects that
        // reference `dae_qt_install_macos_menu`.
        cc::Build::new()
            .file("cpp/macos_menu.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_menu");
        // The accent reader is the only AppKit-backed value Rust asks for
        // directly; it stays a C function so the Rust side needs no objc2
        // dependency just for one colour.
        cc::Build::new()
            .file("cpp/macos_platform.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_platform");
    }
}
