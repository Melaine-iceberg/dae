use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // The probe talks to the Win32 IME directly; cxx-qt only links Qt.
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=imm32");
    }

    CxxQtBuilder::new_qml_module(QmlModule::new("dae.imespike").qml_file("qml/main.qml"))
        .qt_module("Quick")
        .include_dir("cpp")
        .files(["src/bridge.rs"])
        .build();
}
