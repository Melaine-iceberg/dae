//! Links the vendored libghostty-vt.
//!
//! Windows uses the official prebuilt `ghostty-vt.dll` through its import
//! library; every other target links the static archive built from the same
//! release. The artifacts live in `ghostty-vt/lib/<target-triple>/`; refresh
//! them from the release named in `ghostty-vt/VERSION` with
//! `scripts/build-ghostty-vt.ps1` (Windows) or `scripts/build-ghostty-vt.sh`
//! (Linux/macOS).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").expect("TARGET set by cargo");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let lib_dir = manifest.join("ghostty-vt").join("lib").join(&target);
    let windows = target.contains("windows");

    let (lib_name, file_name) = if windows {
        // The import library. The DLL beside it is what the loader needs.
        ("ghostty-vt", "ghostty-vt.lib")
    } else {
        ("ghostty-vt", "libghostty-vt.a")
    };

    let library = lib_dir.join(file_name);
    println!("cargo:rerun-if-changed={}", library.display());
    if !library.is_file() {
        panic!(
            "libghostty-vt is not vendored for {target}: expected {}. \
             Run scripts/build-ghostty-vt.ps1 (Windows) or scripts/build-ghostty-vt.sh \
             (Linux/macOS) to vendor it.",
            library.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    if !windows {
        println!("cargo:rustc-link-lib=static={lib_name}");
        return;
    }

    println!("cargo:rustc-link-lib=dylib={lib_name}");

    // Windows finds a DLL next to the executable, so put a copy where cargo
    // writes binaries for this profile: the profile directory itself for
    // `dae.exe`, and `deps/` for test and example binaries. OUT_DIR is
    // `<target>/<profile>/build/<pkg>-<hash>/out`.
    let dll = lib_dir.join("ghostty-vt.dll");
    println!("cargo:rerun-if-changed={}", dll.display());
    if !dll.is_file() {
        panic!(
            "libghostty-vt is not vendored for {target}: expected {}. \
             Run scripts/build-ghostty-vt.ps1 to vendor it.",
            dll.display()
        );
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("cargo profile directory")
        .to_path_buf();
    for dir in [profile_dir.clone(), profile_dir.join("deps")] {
        fs::create_dir_all(&dir).expect("create cargo output directory");
        let destination = dir.join("ghostty-vt.dll");
        fs::copy(&dll, &destination).unwrap_or_else(|error| {
            panic!("failed to place {}: {error}", destination.display())
        });
    }
}
