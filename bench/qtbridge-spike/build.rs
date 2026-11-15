// Builds the one C++ translation unit the spike needs.
//
// The real port has to replace `cxx-qt-build` here: Qt Bridge's generated glue
// is built inside `qtbridge-runtime`, so a dependent crate owns its own
// `build.rs` and points `cc` at the Qt installation `qmake` reports. This is
// the minimum version of that script — moc for the `QQuickItem` subclass, the
// public include directories, and the four Qt libraries the shell's C++ half
// already links.

use std::path::PathBuf;
use std::process::Command;

fn qmake() -> String {
    std::env::var("QMAKE").unwrap_or_else(|_| "qmake".to_owned())
}

fn query(key: &str) -> PathBuf {
    let output = Command::new(qmake())
        .args(["-query", key])
        .output()
        .unwrap_or_else(|error| panic!("qmake -query {key} failed: {error}"));
    assert!(output.status.success(), "qmake -query {key} failed");
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

fn main() {
    let headers = query("QT_INSTALL_HEADERS");
    let libs = query("QT_INSTALL_LIBS");
    let bins = query("QT_INSTALL_BINS");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // moc runs on the header that declares `Q_OBJECT`; the generated source is
    // compiled with the hand-written one.
    let moc = bins.join(if cfg!(windows) { "moc.exe" } else { "moc" });
    let moc_out = out.join("moc_spike.cpp");
    let status = Command::new(&moc)
        .arg("cpp/spike.h")
        .arg("-o")
        .arg(&moc_out)
        .status()
        .expect("failed to run moc");
    assert!(status.success(), "moc failed");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .flag_if_supported("/Zc:__cplusplus")
        .flag_if_supported("/permissive-")
        .file("cpp/spike.cpp")
        .file(&moc_out);

    for module in ["", "QtCore", "QtGui", "QtQml", "QtQuick"] {
        build.include(headers.join(module));
    }
    build.compile("qtbridge_spike");

    println!("cargo:rustc-link-search=native={}", libs.display());
    for lib in ["Qt6Core", "Qt6Gui", "Qt6Qml", "Qt6Quick"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }

    println!("cargo:rerun-if-env-changed=QMAKE");
    println!("cargo:rerun-if-changed=cpp/spike.cpp");
    println!("cargo:rerun-if-changed=cpp/spike.h");
}
