use std::path::{Path, PathBuf};
use std::process::Command;

use cxx_qt_build::{CxxQtBuilder, QResource, QResources};
use qt_build_utils::QResourceFile;

/// Where the QML tree lives inside the binary. Every file below is mounted
/// under this prefix, and the ahead-of-time compiler has to be told exactly the
/// path the engine will ask for (`qrc:/qt/qml/dae/qml/Main.qml`) or it registers
/// units nobody looks up.
const QML_PREFIX: &str = "/qt/qml/dae";

/// The view layer lives at the repository root (`qml/`) so it stays readable
/// next to the decision docs. The resource aliases keep the import paths
/// relative to `qml/` even though the files come from `../../qml/`.
const QML_FILES: &[(&str, &str)] = &[
    // The colour singleton is declared in `qmldir`; without the declaration
    // the engine reads `pragma Singleton` and then refuses to expose the type.
    ("../../qml/Theme.qml", "qml/Theme.qml"),
    ("../../qml/qmldir", "qml/qmldir"),
    ("../../qml/Main.qml", "qml/Main.qml"),
    ("../../qml/ShellWindow.qml", "qml/ShellWindow.qml"),
    // Built by `ShellWindow.qml` for its on-demand dialogs and views.
    ("../../qml/LazySurface.qml", "qml/LazySurface.qml"),
    ("../../qml/PathBreadcrumbs.qml", "qml/PathBreadcrumbs.qml"),
    (
        "../../qml/TransferConflictDialog.qml",
        "qml/TransferConflictDialog.qml",
    ),
    ("../../qml/ConflictSide.qml", "qml/ConflictSide.qml"),
    ("../../qml/TransferQueue.qml", "qml/TransferQueue.qml"),
    ("../../qml/GitPanel.qml", "qml/GitPanel.qml"),
    ("../../qml/Tab.qml", "qml/Tab.qml"),
    ("../../qml/TabStrip.qml", "qml/TabStrip.qml"),
    ("../../qml/TerminalPanel.qml", "qml/TerminalPanel.qml"),
    ("../../qml/TitleBar.qml", "qml/TitleBar.qml"),
    ("../../qml/ResizeHandles.qml", "qml/ResizeHandles.qml"),
    ("../../qml/FileList.qml", "qml/FileList.qml"),
    ("../../qml/FileGrid.qml", "qml/FileGrid.qml"),
    ("../../qml/ColumnView.qml", "qml/ColumnView.qml"),
    ("../../qml/EmptyState.qml", "qml/EmptyState.qml"),
    ("../../qml/ThemedIcon.qml", "qml/ThemedIcon.qml"),
    ("../../qml/DisplayMenu.qml", "qml/DisplayMenu.qml"),
    ("../../qml/FilterMenu.qml", "qml/FilterMenu.qml"),
    ("../../qml/ContentResults.qml", "qml/ContentResults.qml"),
    ("../../qml/SearchBar.qml", "qml/SearchBar.qml"),
    ("../../qml/SearchFilterBar.qml", "qml/SearchFilterBar.qml"),
    ("../../qml/PreviewPanel.qml", "qml/PreviewPanel.qml"),
    ("../../qml/MediaPreview.qml", "qml/MediaPreview.qml"),
    ("../../qml/Sidebar.qml", "qml/Sidebar.qml"),
    ("../../qml/Glyph.qml", "qml/Glyph.qml"),
    ("../../qml/SidebarTree.qml", "qml/SidebarTree.qml"),
    ("../../qml/OverviewView.qml", "qml/OverviewView.qml"),
    ("../../qml/SpaceView.qml", "qml/SpaceView.qml"),
    ("../../qml/CommandPalette.qml", "qml/CommandPalette.qml"),
    ("../../qml/ContextMenu.qml", "qml/ContextMenu.qml"),
    ("../../qml/MenuList.qml", "qml/MenuList.qml"),
    ("../../qml/NamePrompt.qml", "qml/NamePrompt.qml"),
    ("../../qml/DialogButton.qml", "qml/DialogButton.qml"),
    ("../../qml/DialogHeader.qml", "qml/DialogHeader.qml"),
    ("../../qml/PropertiesDialog.qml", "qml/PropertiesDialog.qml"),
    ("../../qml/BulkRenameDialog.qml", "qml/BulkRenameDialog.qml"),
    ("../../qml/MoveToDialog.qml", "qml/MoveToDialog.qml"),
    ("../../qml/OpenWithDialog.qml", "qml/OpenWithDialog.qml"),
    ("../../qml/SettingsPanel.qml", "qml/SettingsPanel.qml"),
    ("../../qml/ConnectDialog.qml", "qml/ConnectDialog.qml"),
    (
        "../../qml/CloudAccountDialog.qml",
        "qml/CloudAccountDialog.qml",
    ),
    ("../../qml/Toast.qml", "qml/Toast.qml"),
    ("../../qml/UpdateDialog.qml", "qml/UpdateDialog.qml"),
    ("../../qml/TrashView.qml", "qml/TrashView.qml"),
    ("../../qml/ConfirmDialog.qml", "qml/ConfirmDialog.qml"),
    (
        "../../qml/ArchivePasswordDialog.qml",
        "qml/ArchivePasswordDialog.qml",
    ),
    ("../../qml/TearOffWindow.qml", "qml/TearOffWindow.qml"),
    ("../../qml/DialogFocus.js", "qml/DialogFocus.js"),
    ("../../qml/Shell.js", "qml/Shell.js"),
    ("../../qml/Shortcut.js", "qml/Shortcut.js"),
];

/// Adds the LLVM profile flags `scripts/opt/pgo.mjs` asks for to every C++
/// translation unit this package compiles (the QML cache anchor and the AOT
/// units). The cxx-qt generated C++ lives in `crates/dae-qt` and receives the
/// same treatment there.
///
/// Only clang can contribute to the `.profdata` rustc consumes: MSVC and GCC
/// have their own profilers whose output `llvm-profdata` cannot merge. The
/// compiler's LLVM major also has to match rustc's, because the profraw format
/// is versioned. Anything else is warned about and skipped — the Rust half
/// still gets its profile, which is where the measured startup cost is.
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
        // A file name, not a directory: clang writes the path verbatim (rustc
        // adds the default name itself). `%m`/`%p` keep concurrent runs apart,
        // exactly like rustc's own `default_%m_%p.profraw`.
        let file = std::path::PathBuf::from(directory).join("cxx-%m-%p.profraw");
        cc.flag(format!(
            "-fprofile-instr-generate={}",
            file.to_string_lossy().replace('\\', "/")
        ));
    }
    if let Some(profile) = use_profile {
        cc.flag(format!(
            "-fprofile-instr-use={}",
            std::path::PathBuf::from(profile)
                .to_string_lossy()
                .replace('\\', "/")
        ));
        // The profile is shared with the Rust half; functions this build never
        // trained (or trained on another platform) are expected, not a defect.
        cc.flag("-Wno-profile-instr-unprofiled");
    }
}

fn compiler_llvm_major(compiler: &cc::Tool) -> Option<u32> {
    let output = std::process::Command::new(compiler.path())
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let rest = text.split("clang version ").nth(1)?;
    rest.split('.').next()?.parse().ok()
}

fn rustc_llvm_major() -> Option<u32> {
    let rustc = std::env::var_os("RUSTC")?;
    let output = std::process::Command::new(rustc).arg("-vV").output().ok()?;
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

/// Resolves a Qt host tool the same way for both QML compile steps: an
/// explicit override first, then the `bin` directory `qmake` reports (`QMAKE`,
/// the same override `crates/dae-qt/build.rs` honours), then `PATH`. Resolving
/// through the same Qt matters: generated C++ is compiled against the headers
/// cxx-qt links, and a tool/headers version mismatch fails with errors that
/// name neither.
fn find_qt_tool(override_var: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(override_var) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let qmake = std::env::var_os("QMAKE")
        .map(PathBuf::from)
        .or_else(|| which("qmake6").or_else(|| which("qmake")));
    if let Some(qmake) = qmake {
        if let Ok(output) = Command::new(&qmake)
            .args(["-query", "QT_INSTALL_BINS"])
            .output()
        {
            if output.status.success() {
                let bins = String::from_utf8_lossy(&output.stdout);
                let candidate = Path::new(bins.trim()).join(exe_name(name));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    which(&exe_name(name))
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// `<target>/cxxqt/qml_modules`: where `dae-qt`'s build script exports the
/// generated `qmldir` and `plugin.qmltypes`. Resolved the way cxx-qt-build
/// resolves it, because the build script of a dependency has already run by the
/// time this one does.
fn qml_modules() -> Option<PathBuf> {
    let base = std::env::var_os("CXX_QT_EXPORT_DIR")
        .map(PathBuf::from)
        .or_else(cargo_target_dir)?;
    Some(base.join("qml_modules"))
}

fn cargo_target_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        return Some(PathBuf::from(dir).join("cxxqt"));
    }
    let mut dir = PathBuf::from(std::env::var_os("OUT_DIR")?);
    loop {
        let marker = dir.join(".rustc_info.json").exists()
            || dir.join("CACHEDIR.TAG").exists()
            || (dir.file_name().is_some_and(|name| name == "target")
                && dir
                    .parent()
                    .is_some_and(|parent| parent.join("Cargo.toml").exists()));
        if marker {
            return Some(dir.join("cxxqt"));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Runs one compiler invocation, folding its stderr into the message so a
/// compile error in a binding is reported by the build script instead of as a
/// bare exit code.
fn run_tool(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("could not run {:?}: {error}", command.get_program()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr.to_owned()
    })
}

fn main() {
    let files = QML_FILES
        .iter()
        .map(|(path, alias)| QResourceFile::new(path).alias((*alias).to_owned()));
    let qml = QResource::new().prefix(QML_PREFIX).files(files);

    // The translator reads these XML documents directly (see
    // `crates/dae-qt/cpp/translator.cpp`), so no `lrelease` step is needed.
    let translations = QResource::new().prefix("/i18n").files([
        QResourceFile::new("../../qml/i18n/dae_en.ts").alias("dae_en.ts".to_owned()),
        QResourceFile::new("../../qml/i18n/dae_zh_CN.ts").alias("dae_zh_CN.ts".to_owned()),
    ]);

    // The C++ type descriptions for the two modules registered by hand at run
    // time (`dae.terminal`, `dae.activation`). They are not documents, so they
    // are not in `QML_FILES`, but both QML compilers read them through `-I qml`;
    // without this a change would not rerun the build script.
    for path in [
        "../../qml/dae/terminal/qmldir",
        "../../qml/dae/terminal/terminal.qmltypes",
        "../../qml/dae/activation/qmldir",
        "../../qml/dae/activation/activation.qmltypes",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    let mut builder = CxxQtBuilder::new()
        .qt_module("Quick")
        .qrc_resources(QResources::new().resource(qml).resource(translations))
        // Always compiled: it is the reference that keeps the generated units
        // out of the archive's dead objects. See the file itself.
        .cpp_file("cpp/qml_cache_init.cpp");

    // Applies to the anchor above, and (because cxx-qt-build shares one
    // `cc::Build`) to the AOT units added below.
    builder = unsafe { builder.cc_builder(|cc| apply_cxx_profile(cc)) };

    // Runs first because it is the cheapest guard: a Qt upgrade that breaks
    // type compilation fails here instead of after the AOT and C++ build.
    typec::compile();

    // The `.qml` files above stay in the resource: the units are a cache the
    // engine prefers for a matching `qrc:` URL, not a replacement for the file
    // the URL names, and a unit whose lookup signature no longer matches falls
    // back to that file instead of failing.
    let units = aot::compile();
    if !units.is_empty() {
        // Marks the anchor above so it actually references the loader. It also
        // reaches the bridge sources, which do not test it.
        builder = unsafe {
            builder.cc_builder(|cc| {
                cc.define("DAE_QML_AOT", None);
            })
        };
        builder = builder.cpp_files(units);
    }

    builder.build();
}

/// QML ahead-of-time compilation.
///
/// `rcc` only ships the sources, so `QQmlApplicationEngine` parses and compiles
/// the whole tree on the first launch of a build, and again whenever the
/// per-user `qmlcache` directory is cold — 53 files, 2.0 MB of bytecode,
/// measured at 1115–1137 ms to the first window against 849–874 ms with that
/// cache warm. `qmlcachegen` turns every file into a C++ translation unit;
/// the loader registers those units with
/// `QQmlPrivate::QmlUnitCacheHookRegistration`, keyed by resource path, so the
/// engine finds a native unit instead of compiling the source.
///
/// This is the half `qt_add_qml_module` runs inside CMake, reproduced by hand
/// because cxx-qt-build has no equivalent: one invocation per file with
/// `--resource-path`, then one more whose *output file name* ends in
/// `qmlcache_loader.cpp` (that name is what selects loader mode in
/// `qmlcachegen`) and whose positional arguments are the resource paths it
/// should register.
///
/// Release builds only: the generated C++ is ~18 MB, and recompiling it after
/// every QML edit costs far more than the debug launch it would speed up.
/// `DAE_QML_AOT=1` forces it on (and makes a failure fatal), `=0` turns it off.
mod aot {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use super::{QML_FILES, QML_PREFIX, qml_modules, run_tool};

    /// Name the loader registers under. Only has to be unique; it ends up in
    /// the generated `qInitResources_<name>` symbols.
    const RESOURCE_NAME: &str = "dae_qmlcache";

    /// The generated units, plus the loader that points at them. Empty when the
    /// step is skipped or degraded, which just means the engine compiles the
    /// sources at run time as before.
    pub(super) fn compile() -> Vec<PathBuf> {
        // Before the gate below, not after: a skipped build still has to react
        // to `DAE_QML_AOT=1` on the next invocation, and cargo only watches what
        // the script prints this time.
        for name in [
            "DAE_QML_AOT",
            "DAE_QMLCACHEGEN",
            "QMAKE",
            "CARGO_TARGET_DIR",
            "CXX_QT_EXPORT_DIR",
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        match env::var("DAE_QML_AOT").ok().as_deref() {
            // Explicitly off; also keeps a debug loop from warning every build.
            Some("0") => Vec::new(),
            // Explicitly on, in any profile, and a failure is fatal so CI can
            // tell "shipped with units" from "shipped without".
            Some("1") => compile_units(true),
            // An unrecognised value behaves like unset rather than silently
            // disabling the step for a release.
            _ if env::var("PROFILE").as_deref() == Ok("release") => compile_units(false),
            // Debug builds skip it: the generated C++ is ~18 MB, and
            // recompiling it after every QML edit costs more than the launch it
            // would speed up.
            _ => Vec::new(),
        }
    }

    fn compile_units(strict: bool) -> Vec<PathBuf> {
        let Some(qmlcachegen) = find_qmlcachegen() else {
            return degraded(
                strict,
                "qmlcachegen was not found (set QMAKE or DAE_QMLCACHEGEN)",
            );
        };

        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        // Every source is addressed relative to this crate's manifest, and they
        // all live in one directory; `-I` there is what lets a component resolve
        // its siblings without a full qrc mapping (`qt_add_qml_module` passes
        // its binary directory for the same reason).
        let qml_dir = manifest.join("../../qml");

        // The `dae` module's generated `qmldir`/`plugin.qmltypes` are what let
        // the compiler see the Rust types, so bindings on `dae.*` properties and
        // invokables compile to C++ instead of falling back to lookups.
        let modules = qml_modules();
        let module_dir = modules.as_ref().map(|dir| dir.join("dae"));
        let qmldir = module_dir.as_ref().map(|dir| dir.join("qmldir"));
        if let Some(qmldir) = qmldir.as_ref().filter(|path| path.exists()) {
            println!("cargo:rerun-if-changed={}", qmldir.display());
        }

        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("qmlcache");
        if let Err(error) = fs::create_dir_all(&out_dir) {
            return degraded(
                strict,
                &format!("could not create {}: {error}", out_dir.display()),
            );
        }

        let mut units = Vec::with_capacity(QML_FILES.len() + 1);
        let mut resource_paths = Vec::with_capacity(QML_FILES.len());
        for (source, alias) in QML_FILES {
            // `qml/qmldir` is a resource the engine reads, not a document: the
            // type declaration has no compilation unit, and handing it to
            // `qmlcachegen` leaves a unit in the list with no file behind it.
            if !(alias.ends_with(".qml") || alias.ends_with(".js")) {
                continue;
            }

            let resource_path = format!("{QML_PREFIX}/{alias}");
            // `qrc:/` URLs have slashes in them and the alias is unique, so the
            // flattened alias is already a unique file name.
            let unit = out_dir.join(format!("{}.cpp", alias.replace('/', "_")));

            let mut command = Command::new(&qmlcachegen);
            command.arg("--resource-path").arg(&resource_path);
            if let Some(modules) = modules.as_ref() {
                command.arg("-I").arg(modules);
            }
            if let Some(qmldir) = qmldir.as_ref().filter(|path| path.exists()) {
                command.arg("-i").arg(qmldir);
            }
            command
                .arg("-I")
                .arg(&qml_dir)
                .arg("-o")
                .arg(&unit)
                .arg(source);

            if let Err(error) = run_tool(&mut command) {
                return degraded(strict, &format!("qmlcachegen failed on {source}: {error}"));
            }
            // A zero exit that wrote nothing would reach the C++ compiler as a
            // missing file, so treat the missing output as the failure it is.
            if !unit.is_file() {
                return degraded(strict, &format!("qmlcachegen wrote no unit for {source}"));
            }
            println!("cargo:rerun-if-changed={source}");

            units.push(unit);
            resource_paths.push(resource_path);
        }

        // The loader is what the engine sees: it maps each `qrc:` path to the
        // `qmlData` and `aotBuiltFunctions` the per-file units above define.
        // The output name has to end in `qmlcache_loader.cpp`: that suffix is
        // what selects loader mode in `qmlcachegen`.
        let loader = out_dir.join("dae_qmlcache_loader.cpp");
        let mut command = Command::new(&qmlcachegen);
        command
            .arg("--resource-name")
            .arg(RESOURCE_NAME)
            .arg("-o")
            .arg(&loader)
            .args(&resource_paths);
        if let Err(error) = run_tool(&mut command) {
            return degraded(
                strict,
                &format!("qmlcachegen failed to build the loader: {error}"),
            );
        }
        units.push(loader);

        units
    }

    /// A step that did not run. Never fatal unless it was explicitly requested:
    /// the shell still works without it, it just compiles the QML at run time,
    /// and a build must not be blocked by an optimizer (same rule as
    /// `docs/pgo-bolt.md`).
    fn degraded(strict: bool, reason: &str) -> Vec<PathBuf> {
        assert!(
            !strict,
            "DAE_QML_AOT=1 was requested but QML ahead-of-time compilation failed: {reason}"
        );
        println!("cargo:warning=QML ahead-of-time compilation skipped: {reason}");
        Vec::new()
    }

    /// `qmlcachegen` from the Qt the rest of the build uses; see
    /// [`super::find_qt_tool`] for why it is resolved that way.
    fn find_qmlcachegen() -> Option<PathBuf> {
        super::find_qt_tool("DAE_QMLCACHEGEN", "qmlcachegen")
    }
}

/// QML type compilation, wired as a build-time check only.
///
/// `qmltc` is Qt's QML type compiler: it turns a document into a C++ class and
/// resolves every property, method and signal statically. Unlike the
/// `qmlcachegen` units in `mod aot`, that output is not part of the binary:
/// linking it needs one translation unit for the whole tree plus private Qt
/// APIs, and the runtime experiment that did so was rejected
/// (`docs/qml-aot.md` §10). What remains is the guard — a Qt upgrade that
/// breaks type compilation fails before the change is merged.
///
/// `DAE_QML_TYPEC=1` runs it and makes a failure fatal. Unset or `0` skips it;
/// unlike `DAE_QML_AOT`, there is no default-on for release because nothing
/// ships the generated code and an unused check should not add time or a new
/// failure mode to a release build. The CI Qt job sets the variable.
mod typec {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{QML_FILES, QML_PREFIX, find_qt_tool, qml_modules, run_tool};

    /// Documents `qmltc` cannot compile today. `MediaPreview.qml` binds a
    /// `MediaPlayer`; Qt 6.12's own QtMultimedia qmltypes marks
    /// `QAudioBufferOutput` (the type behind `audioBufferOutput`)
    /// `isTypeOpaque: true`, and qmltc refuses to generate a setter for an
    /// opaque property type. The defect is in Qt's metadata, not this tree.
    /// Drop the entry once the type is exported.
    const SKIPPED: &[&str] = &["qml/MediaPreview.qml"];

    pub(super) fn compile() {
        // Printed even when the step is skipped: a build that did not parse
        // this environment this time must still react when it is set.
        for name in [
            "DAE_QML_TYPEC",
            "DAE_QMLTC",
            "QMAKE",
            "CARGO_TARGET_DIR",
            "CXX_QT_EXPORT_DIR",
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        if env::var("DAE_QML_TYPEC").ok().as_deref() != Some("1") {
            return;
        }
        if let Err(reason) = check() {
            panic!("DAE_QML_TYPEC=1 was requested but QML type compilation failed: {reason}");
        }
    }

    /// Runs `qmltc` over every document and reports the first failure.
    fn check() -> Result<(), String> {
        let qmltc = find_qt_tool("DAE_QMLTC", "qmltc")
            .ok_or_else(|| "qmltc was not found (set QMAKE or DAE_QMLTC)".to_owned())?;

        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let qml_dir = manifest.join("../../qml");
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("qmltc");
        fs::create_dir_all(out_dir.join("units"))
            .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;

        let module_dir = prepare_module(&out_dir)?;
        let qrc = write_qrc(&out_dir, &manifest)?;

        let mut compiled = 0usize;
        for (source, alias) in QML_FILES {
            // `qmldir` is a module declaration and the `.js` files are not
            // documents: qmltc rejects a non-QML input, and neither has a
            // generated type to check.
            if !alias.ends_with(".qml") || SKIPPED.contains(alias) {
                continue;
            }

            // Flattened alias, exactly as in `aot`: unique because the qrc
            // path is unique. The generated *class* name comes from the QML
            // file, not from this path.
            let unit = out_dir.join("units").join(alias.replace('/', "_"));
            let impl_file = unit.with_extension("cpp");
            let header = unit.with_extension("h");

            let mut command = Command::new(&qmltc);
            command
                .arg("-I")
                .arg(&module_dir)
                // The module declarations for the two hand-registered
                // C++ modules (`dae.terminal`, `dae.activation`).
                .arg("-I")
                .arg(&qml_dir)
                .arg("--resource")
                .arg(&qrc)
                .arg("--impl")
                .arg(&impl_file)
                .arg("--header")
                .arg(&header)
                .arg(source);

            run_tool(&mut command).map_err(|error| format!("qmltc failed on {source}: {error}"))?;
            // A zero exit that wrote nothing is still a broken step, not a
            // pass: the check exists to prove the document generated code.
            if !impl_file.is_file() || !header.is_file() {
                return Err(format!("qmltc wrote no unit for {source}"));
            }
            println!("cargo:rerun-if-changed={source}");
            compiled += 1;
        }

        if compiled == 0 {
            return Err("no QML document was type-compiled".to_owned());
        }
        Ok(())
    }

    /// The `dae` module as `qmltc` needs to see it.
    ///
    /// cxx-qt's generated `plugin.qmltypes` declares every built-in value type
    /// it only references from a property (`QString`, `bool`, `double`, …) as
    /// an `isTypeOpaque: true` placeholder. qmllint accepts that; qmltc does
    /// not, and reports "Type of property 'x' is unknown" for every property of
    /// every `dae` type. Those placeholders describe types Qt already
    /// registers, so drop the blocks and let the built-ins resolve normally.
    /// The generated file itself is left alone: qmlcachegen and qmllint read
    /// it, and it is cxx-qt's to write.
    ///
    /// `depends QtQuick` is what resolves `QAbstractListModel`, the prototype
    /// of every model type here; cxx-qt cannot infer it from the prototype
    /// names, so it is added in `crates/dae-qt/build.rs` as well.
    fn prepare_module(out_dir: &Path) -> Result<PathBuf, String> {
        let modules =
            qml_modules().ok_or_else(|| "could not locate target/cxxqt/qml_modules".to_owned())?;
        let source = modules.join("dae");
        let qmltypes = source.join("plugin.qmltypes");
        let text = fs::read_to_string(&qmltypes).map_err(|error| {
            format!(
                "could not read {} (did dae-qt build?): {error}",
                qmltypes.display()
            )
        })?;
        println!("cargo:rerun-if-changed={}", qmltypes.display());
        println!("cargo:rerun-if-changed={}", source.join("qmldir").display());

        let dir = out_dir.join("modules").join("dae");
        fs::create_dir_all(&dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
        fs::write(dir.join("plugin.qmltypes"), strip_opaque(&text))
            .map_err(|error| format!("could not write the sanitised qmltypes: {error}"))?;
        // No `plugin`/`classname`/`prefer` lines: this copy exists for qmltc's
        // type resolver only, and it must not try to load a plugin.
        let qmldir = "module dae\ntypeinfo plugin.qmltypes\ndepends QtQuick auto\n";
        fs::write(dir.join("qmldir"), qmldir)
            .map_err(|error| format!("could not write the sanitised qmldir: {error}"))?;
        Ok(out_dir.join("modules"))
    }

    /// Drops every `Component { … }` block carrying `isTypeOpaque: true`.
    /// Everything else is copied verbatim, including blocks whose contents are
    /// not placeholders.
    fn strip_opaque(text: &str) -> String {
        let mut kept = String::with_capacity(text.len());
        let mut block: Option<(String, usize)> = None;

        for line in text.lines() {
            if let Some((mut body, mut depth)) = block.take() {
                body.push('\n');
                body.push_str(line);
                depth += line.matches('{').count();
                depth = depth.saturating_sub(line.matches('}').count());
                if depth == 0 {
                    if !body.contains("isTypeOpaque: true") {
                        kept.push_str(&body);
                        kept.push('\n');
                    }
                } else {
                    block = Some((body, depth));
                }
                continue;
            }

            if line.trim_start().starts_with("Component {") {
                let depth = line.matches('{').count();
                let depth = depth.saturating_sub(line.matches('}').count());
                if depth == 0 {
                    if !line.contains("isTypeOpaque: true") {
                        kept.push_str(line);
                        kept.push('\n');
                    }
                } else {
                    block = Some((line.to_owned(), depth));
                }
                continue;
            }

            kept.push_str(line);
            kept.push('\n');
        }

        // An unterminated block means the qmltypes is malformed. Keep it so the
        // compiler reports the real problem instead of losing types silently.
        if let Some((tail, _)) = block {
            kept.push_str(&tail);
            kept.push('\n');
        }
        kept
    }

    /// `qmltc` registers each generated class under the `qrc:` URL the mapper
    /// hands it. `qt_add_qml_module` passes its generated `.qrc`; this writes
    /// the same mapping from `QML_FILES`, the list the resource build uses, so
    /// a file cannot be compiled under one path and mounted under another.
    fn write_qrc(out_dir: &Path, manifest: &Path) -> Result<PathBuf, String> {
        let mut qrc = String::from("<RCC>\n");
        qrc.push_str(&format!("<qresource prefix=\"{QML_PREFIX}/\">\n"));
        for (source, alias) in QML_FILES {
            if !alias.ends_with(".qml") {
                continue;
            }
            let path = manifest.join(source);
            qrc.push_str(&format!(
                "<file alias=\"{alias}\">{}</file>\n",
                path.to_string_lossy().replace('\\', "/")
            ));
        }
        qrc.push_str("</qresource>\n</RCC>\n");

        let path = out_dir.join("files.qrc");
        fs::write(&path, qrc)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(path)
    }
}
