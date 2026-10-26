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

    CxxQtBuilder::new_qml_module(QmlModule::new("dae"))
        .qt_module("Quick")
        .include_dir("cpp")
        .cpp_files([
            "cpp/image_provider.cpp",
            "cpp/image_provider.h",
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
            "src/window.rs",
            "src/workspace.rs",
        ])
        .build();
}
