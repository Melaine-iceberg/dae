//! Links the vendored libghostty-vt static library.
//!
//! The prebuilt artifacts live in `ghostty-vt/lib/<target-triple>/`. Rebuild
//! them with `scripts/build-ghostty-vt.ps1` (Windows) or
//! `scripts/build-ghostty-vt.sh` (Linux/macOS); both need the Zig version
//! pinned in `ghostty-vt/VERSION`.

use std::env;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").expect("TARGET set by cargo");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let lib_dir = manifest.join("ghostty-vt").join("lib").join(&target);

    let (lib_name, file_name) = if target.contains("windows") {
        ("ghostty-vt-static", "ghostty-vt-static.lib")
    } else {
        ("ghostty-vt", "libghostty-vt.a")
    };

    let archive = lib_dir.join(file_name);
    println!("cargo:rerun-if-changed={}", archive.display());
    if !archive.is_file() {
        panic!(
            "libghostty-vt is not vendored for {target}: expected {}. \
             Run scripts/build-ghostty-vt.ps1 (Windows) or scripts/build-ghostty-vt.sh \
             (Linux/macOS) to build it.",
            archive.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static={lib_name}");
    if target.contains("windows") {
        // Zig's Windows static library calls the NT API directly.
        println!("cargo:rustc-link-lib=ntdll");
    }
}
