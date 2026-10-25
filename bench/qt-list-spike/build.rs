use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("dae.listspike").qml_file("qml/main.qml"))
        .qt_module("Quick")
        .include_dir("cpp")
        .files(["src/bridge.rs"])
        .build();
}
