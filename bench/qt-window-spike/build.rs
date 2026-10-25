use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // The probe subclasses nothing: it observes and answers Windows messages
    // through QAbstractNativeEventFilter, and applies the backdrop with DWM.
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=dwmapi");
    }

    CxxQtBuilder::new_qml_module(QmlModule::new("dae.windowspike").qml_file("qml/main.qml"))
        .qt_module("Quick")
        .include_dir("cpp")
        .files(["src/bridge.rs"])
        .build();
}
