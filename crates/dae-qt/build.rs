use cxx_qt_build::{CxxQtBuilder, QmlModule};

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
}
