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
        .include_dir("cpp")
        .cpp_files([
            "cpp/image_provider.cpp",
            "cpp/image_provider.h",
            "cpp/qml_debug.cpp",
            "cpp/terminal_item.cpp",
            "cpp/terminal_item.h",
        ])
        .files([
            "src/config.rs",
            "src/ime.rs",
            "src/model.rs",
            "src/preview.rs",
            "src/search_model.rs",
            "src/shell.rs",
            "src/sidebar_model.rs",
            "src/terminal.rs",
            "src/trash_model.rs",
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
