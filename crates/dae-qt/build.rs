use std::path::{Path, PathBuf};
use std::process::Command;

fn qmake() -> PathBuf {
    if let Some(path) = std::env::var_os("QMAKE") {
        return PathBuf::from(path);
    }
    which("qmake6")
        .or_else(|| which("qmake"))
        .unwrap_or_else(|| PathBuf::from("qmake"))
}

fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
}

fn query(qmake_bin: &Path, key: &str) -> Option<PathBuf> {
    let output = Command::new(qmake_bin).args(["-query", key]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn find_moc(qmake_bin: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "moc.exe" } else { "moc" };
    for key in [
        "QT_HOST_LIBEXECS",
        "QT_INSTALL_LIBEXECS",
        "QT_HOST_BINS",
        "QT_INSTALL_BINS",
    ] {
        if let Some(dir) = query(qmake_bin, key) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    for key in ["QT_ROOT_DIR", "QTDIR"] {
        if let Some(root) = std::env::var_os(key) {
            for sub in ["libexec", "bin"] {
                let candidate = Path::new(&root).join(sub).join(exe);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    which("moc").expect("Qt moc was not found (set QMAKE or add Qt bin/libexec to PATH)")
}

/// The QPA header under `include/QtGui/<version>/QtGui`. Qt ships it in every
/// install but does not have a `Qt6GuiPrivate` link target in aqt builds, so
/// the path is resolved rather than requested as a Qt module.
#[cfg(target_os = "linux")]
fn qt_private_gui_include(headers: &Path) -> Option<PathBuf> {
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

/// Adds the LLVM profile flags `scripts/opt/pgo.mjs` asks for to the C++ bridge
/// and moc products. This is the only part of the Qt shell that is ours to
/// optimise besides Rust: Qt itself arrives prebuilt.
fn apply_cxx_profile(cc: &mut cc::Build) {
    for name in ["DAE_PERF_CXX_PROFILE_GENERATE", "DAE_PERF_CXX_PROFILE_USE"] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let generate =
        std::env::var_os("DAE_PERF_CXX_PROFILE_GENERATE").filter(|value| !value.is_empty());
    let use_profile =
        std::env::var_os("DAE_PERF_CXX_PROFILE_USE").filter(|value| !value.is_empty());
    if generate.is_none() && use_profile.is_none() {
        return;
    }

    let compiler = cc.get_compiler();
    if !compiler.is_like_clang() {
        println!(
            "cargo:warning=C++ bridge is not profiled: {:?} is not clang, and MSVC/GCC profiles cannot be merged with rustc's",
            compiler.path()
        );
        return;
    }

    let rustc_major = rustc_llvm_major();
    let compiler_major = compiler_llvm_major(&compiler);
    if let (Some(rustc), Some(clang)) = (rustc_major, compiler_major) {
        if rustc != clang {
            println!(
                "cargo:warning=C++ bridge is not profiled: clang is LLVM {clang}, rustc is LLVM {rustc}; profraw formats are versioned"
            );
            return;
        }
    }

    if let Some(directory) = generate {
        let file = PathBuf::from(directory).join("cxx-%m-%p.profraw");
        cc.flag(format!(
            "-fprofile-instr-generate={}",
            file.to_string_lossy().replace('\\', "/")
        ));
    }
    if let Some(profile) = use_profile {
        cc.flag(format!(
            "-fprofile-instr-use={}",
            PathBuf::from(profile)
                .to_string_lossy()
                .replace('\\', "/")
        ));
        cc.flag("-Wno-profile-instr-unprofiled");
    }
}

fn compiler_llvm_major(compiler: &cc::Tool) -> Option<u32> {
    let output = Command::new(compiler.path()).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let rest = text.split("clang version ").nth(1)?;
    rest.split('.').next()?.parse().ok()
}

fn rustc_llvm_major() -> Option<u32> {
    let rustc = std::env::var_os("RUSTC")?;
    let output = Command::new(rustc).arg("-vV").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| line.starts_with("LLVM version:"))?;
    line.split_whitespace()
        .nth(2)?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn main() {
    for var in ["QMAKE", "QT_ROOT_DIR", "QTDIR", "DAE_QML_PROFILE"] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    #[cfg(windows)]
    {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=dwmapi");
        println!("cargo:rustc-link-lib=imm32");
    }

    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=wayland-client");

    let qmake_bin = qmake();
    let headers = query(&qmake_bin, "QT_INSTALL_HEADERS")
        .expect("qmake -query QT_INSTALL_HEADERS failed (set QMAKE or add Qt to PATH)");
    let libs = query(&qmake_bin, "QT_INSTALL_LIBS")
        .expect("qmake -query QT_INSTALL_LIBS failed");
    let moc = find_moc(&qmake_bin);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let moc_headers = ["cpp/activation.h", "cpp/m3_items.h", "cpp/terminal_item.h"];
    let mut moc_outputs = Vec::with_capacity(moc_headers.len());
    for header in moc_headers {
        let stem = Path::new(header)
            .file_stem()
            .expect("header stem")
            .to_string_lossy();
        let moc_out = out_dir.join(format!("moc_{stem}.cpp"));
        let status = Command::new(&moc)
            .arg("-I")
            .arg("cpp")
            .arg(header)
            .arg("-o")
            .arg(&moc_out)
            .status()
            .unwrap_or_else(|error| panic!("failed to run moc on {header}: {error}"));
        assert!(status.success(), "moc failed on {header}");
        moc_outputs.push(moc_out);
    }

    let bridges = [
        "src/activation.rs",
        "src/appearance.rs",
        "src/config.rs",
        "src/icons.rs",
        "src/ime.rs",
        "src/preview.rs",
        "src/shell.rs",
        "src/terminal.rs",
        "src/window.rs",
        "src/workspace.rs",
    ];
    let cpp_files = [
        "cpp/activation.cpp",
        "cpp/clipboard.cpp",
        "cpp/drag.cpp",
        "cpp/image_provider.cpp",
        "cpp/m3_items.cpp",
        "cpp/material_marks.cpp",
        "cpp/platform_watch.cpp",
        "cpp/qml_debug.cpp",
        "cpp/terminal_item.cpp",
        "cpp/translator.cpp",
    ];
    let header_files = [
        "cpp/activation.h",
        "cpp/clipboard.h",
        "cpp/drag.h",
        "cpp/image_provider.h",
        "cpp/ime.h",
        "cpp/lookup.h",
        "cpp/m3_items.h",
        "cpp/macos_backdrop.h",
        "cpp/macos_menu.h",
        "cpp/macos_platform.h",
        "cpp/material_marks.h",
        "cpp/platform_watch.h",
        "cpp/terminal_item.h",
        "cpp/translator.h",
        "cpp/window.h",
    ];

    for path in bridges.iter().chain(cpp_files.iter()).chain(header_files.iter()) {
        println!("cargo:rerun-if-changed={path}");
    }

    let mut build = cxx_build::bridges(bridges);
    build
        .std("c++17")
        .flag_if_supported("/Zc:__cplusplus")
        .flag_if_supported("/permissive-")
        .include("cpp");
    if cfg!(target_env = "msvc") {
        build.flag("-utf-8");
    }

    for module in [
        "",
        "QtCore",
        "QtGui",
        "QtQml",
        "QtQuick",
        "QtQuickControls2",
        "QtNetwork",
    ] {
        let dir = if module.is_empty() {
            headers.clone()
        } else {
            headers.join(module)
        };
        if dir.exists() {
            build.include(dir);
        }
    }

    #[cfg(target_os = "macos")]
    build.flag("-F").flag(libs.to_string_lossy().as_ref());

    #[cfg(target_os = "linux")]
    if let Some(private_include) = qt_private_gui_include(&headers) {
        build.include(private_include);
    } else {
        panic!("Qt private QPA headers are required for Wayland background blur");
    }

    for file in cpp_files {
        build.file(file);
    }
    for moc_out in &moc_outputs {
        build.file(moc_out);
    }

    apply_cxx_profile(&mut build);

    let qml_profile = std::env::var_os("DAE_QML_PROFILE").is_some_and(|value| !value.is_empty());
    if std::env::var("PROFILE").as_deref() == Ok("debug") || qml_profile {
        build.define("QT_QML_DEBUG", None);
    }

    build.compile("dae_qt_cpp");

    println!("cargo:rustc-link-search=native={}", libs.display());
    for lib in [
        "Qt6Core",
        "Qt6Gui",
        "Qt6Qml",
        "Qt6Quick",
        "Qt6QuickControls2",
        "Qt6Network",
    ] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }

    // Vendored tree-sitter grammars (assets/grammars). Their published Rust
    // bindings either link an older tree-sitter runtime or (LaTeX) omit the
    // external scanner the parser references, so the generated C is compiled
    // here against the single runtime the `tree-sitter` crate provides.
    for grammar in ["dockerfile", "gitignore", "groovy", "json5", "kotlin", "latex", "less", "scss"] {
        let dir = Path::new("assets/grammars").join(grammar);
        let mut ts_build = cc::Build::new();
        ts_build
            .std("c11")
            .include(&dir)
            .file(dir.join("parser.c"))
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-Wno-unused-but-set-variable");
        if cfg!(target_env = "msvc") {
            ts_build.flag("-utf-8");
        }
        let scanner = dir.join("scanner.c");
        if scanner.exists() {
            ts_build.file(&scanner);
            println!("cargo:rerun-if-changed={}", scanner.display());
        }
        println!("cargo:rerun-if-changed={}", dir.join("parser.c").display());
        ts_build.compile(&format!("dae_ts_{grammar}"));
    }

    #[cfg(target_os = "linux")]
    cc::Build::new()
        .file("cpp/wayland/ext-background-effect-v1-protocol.c")
        .compile("dae_wayland_protocol");

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=AppKit");
        cc::Build::new()
            .file("cpp/macos_backdrop.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_backdrop");
        cc::Build::new()
            .file("cpp/macos_menu.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_menu");
        cc::Build::new()
            .file("cpp/macos_platform.mm")
            .flag("-fobjc-arc")
            .compile("dae_macos_platform");
    }
}
