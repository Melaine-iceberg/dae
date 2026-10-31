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

    let mut builder = CxxQtBuilder::new()
        .qt_module("Quick")
        .qrc_resources(QResources::new().resource(qml).resource(translations))
        // Always compiled: it is the reference that keeps the generated units
        // out of the archive's dead objects. See the file itself.
        .cpp_file("cpp/qml_cache_init.cpp");

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
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{QML_FILES, QML_PREFIX};

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

            if let Err(error) = run(&mut command) {
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
        if let Err(error) = run(&mut command) {
            return degraded(
                strict,
                &format!("qmlcachegen failed to build the loader: {error}"),
            );
        }
        units.push(loader);

        units
    }

    /// Runs one `qmlcachegen` invocation, folding its stderr into the message so
    /// a compile error in a binding is reported by the build script instead of
    /// as a bare exit code.
    fn run(command: &mut Command) -> Result<(), String> {
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

    /// `qmlcachegen` from the Qt the rest of the build uses: `DAE_QMLCACHEGEN`
    /// first, then the `bin` directory `qmake` reports (`QMAKE`, the same
    /// override `crates/dae-qt/build.rs` honours), then `PATH`. Resolving it
    /// through the same Qt matters: the units are compiled against the headers
    /// cxx-qt links, and a units/tool version mismatch fails at link time with
    /// errors that name neither.
    fn find_qmlcachegen() -> Option<PathBuf> {
        if let Some(path) = env::var_os("DAE_QMLCACHEGEN") {
            let path = PathBuf::from(path);
            return path.is_file().then_some(path);
        }

        let qmake = env::var_os("QMAKE")
            .map(PathBuf::from)
            .or_else(|| which("qmake6").or_else(|| which("qmake")));
        if let Some(qmake) = qmake {
            if let Ok(output) = Command::new(&qmake)
                .args(["-query", "QT_INSTALL_BINS"])
                .output()
            {
                if output.status.success() {
                    let bins = String::from_utf8_lossy(&output.stdout);
                    let name = if cfg!(windows) {
                        "qmlcachegen.exe"
                    } else {
                        "qmlcachegen"
                    };
                    let candidate = Path::new(bins.trim()).join(name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }

        which(if cfg!(windows) {
            "qmlcachegen.exe"
        } else {
            "qmlcachegen"
        })
    }

    fn which(name: &str) -> Option<PathBuf> {
        env::split_paths(&env::var_os("PATH")?)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// `<target>/cxxqt/qml_modules`: where `dae-qt`'s build script exports the
    /// generated `qmldir` and `plugin.qmltypes`. Resolved the way cxx-qt-build
    /// resolves it, because the build script of a dependency has already run by
    /// the time this one does.
    fn qml_modules() -> Option<PathBuf> {
        let base = env::var_os("CXX_QT_EXPORT_DIR")
            .map(PathBuf::from)
            .or_else(cargo_target_dir)?;
        Some(base.join("qml_modules"))
    }

    fn cargo_target_dir() -> Option<PathBuf> {
        if let Some(dir) = env::var_os("CARGO_TARGET_DIR") {
            return Some(PathBuf::from(dir).join("cxxqt"));
        }
        let mut dir = PathBuf::from(env::var_os("OUT_DIR")?);
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
}
