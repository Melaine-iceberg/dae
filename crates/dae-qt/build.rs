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
            "src/overview_model.rs",
            "src/terminal.rs",
            "src/trash_model.rs",
            "src/updater.rs",
            "src/window.rs",
            "src/workspace.rs",
        ]);

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
    }
}
