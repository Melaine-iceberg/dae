use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    ("../../qml/Locator.qml", "qml/Locator.qml"),
    (
        "../../qml/TransferConflictDialog.qml",
        "qml/TransferConflictDialog.qml",
    ),
    ("../../qml/ConflictSide.qml", "qml/ConflictSide.qml"),
    ("../../qml/TransferQueue.qml", "qml/TransferQueue.qml"),
    ("../../qml/StatusCenter.qml", "qml/StatusCenter.qml"),
    ("../../qml/QuickPreview.qml", "qml/QuickPreview.qml"),
    ("../../qml/GitPanel.qml", "qml/GitPanel.qml"),
    ("../../qml/Tab.qml", "qml/Tab.qml"),
    ("../../qml/TopBar.qml", "qml/TopBar.qml"),
    ("../../qml/ActionBar.qml", "qml/ActionBar.qml"),
    ("../../qml/TransferChip.qml", "qml/TransferChip.qml"),
    ("../../qml/TerminalPanel.qml", "qml/TerminalPanel.qml"),
    ("../../qml/ResizeHandles.qml", "qml/ResizeHandles.qml"),
    ("../../qml/FileList.qml", "qml/FileList.qml"),
    ("../../qml/ListRow.qml", "qml/ListRow.qml"),
    ("../../qml/ScrollBar.qml", "qml/ScrollBar.qml"),
    ("../../qml/FileIcon.qml", "qml/FileIcon.qml"),
    ("../../qml/FileGrid.qml", "qml/FileGrid.qml"),
    ("../../qml/ColumnView.qml", "qml/ColumnView.qml"),
    ("../../qml/EmptyState.qml", "qml/EmptyState.qml"),
    ("../../qml/Shadow.qml", "qml/Shadow.qml"),
    ("../../qml/M3Progress.qml", "qml/M3Progress.qml"),
    ("../../qml/M3Loader.qml", "qml/M3Loader.qml"),
    ("../../qml/InkRipple.qml", "qml/InkRipple.qml"),
    ("../../qml/ButtonGroup.qml", "qml/ButtonGroup.qml"),
    ("../../qml/M3Switch.qml", "qml/M3Switch.qml"),
    ("../../qml/M3Slider.qml", "qml/M3Slider.qml"),
    // The nine-patch shadow, one texture per theme (proposal §6.5).
    ("../../Assets/shadow-light.png", "qml/assets/shadow-light.png"),
    ("../../Assets/shadow-dark.png", "qml/assets/shadow-dark.png"),
    ("../../qml/DisplayMenu.qml", "qml/DisplayMenu.qml"),
    ("../../qml/FilterMenu.qml", "qml/FilterMenu.qml"),
    ("../../qml/ContentResults.qml", "qml/ContentResults.qml"),
    ("../../qml/SearchFilterBar.qml", "qml/SearchFilterBar.qml"),
    ("../../qml/Inspector.qml", "qml/Inspector.qml"),
    ("../../qml/MediaPreview.qml", "qml/MediaPreview.qml"),
    ("../../qml/Sidebar.qml", "qml/Sidebar.qml"),
    ("../../qml/Glyph.qml", "qml/Glyph.qml"),
    ("../../qml/Glass.qml", "qml/Glass.qml"),
    ("../../qml/material-marks.json", "qml/material-marks.json"),
    ("../../qml/SidebarTree.qml", "qml/SidebarTree.qml"),
    ("../../qml/OverviewView.qml", "qml/OverviewView.qml"),
    ("../../qml/CommandPalette.qml", "qml/CommandPalette.qml"),
    ("../../qml/Menu.qml", "qml/Menu.qml"),
    ("../../qml/MenuList.qml", "qml/MenuList.qml"),
    ("../../qml/ToolTip.qml", "qml/ToolTip.qml"),
    ("../../qml/NamePrompt.qml", "qml/NamePrompt.qml"),
    ("../../qml/Dialog.qml", "qml/Dialog.qml"),
    ("../../qml/DialogButton.qml", "qml/DialogButton.qml"),
    ("../../qml/DialogHeader.qml", "qml/DialogHeader.qml"),
    ("../../qml/PropertiesDialog.qml", "qml/PropertiesDialog.qml"),
    ("../../qml/BulkRenameDialog.qml", "qml/BulkRenameDialog.qml"),
    ("../../qml/MoveToDialog.qml", "qml/MoveToDialog.qml"),
    ("../../qml/LocksmithDialog.qml", "qml/LocksmithDialog.qml"),
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
    ("../../qml/Localize.js", "qml/Localize.js"),
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
/// explicit override first, then the directories `qmake` reports (`QMAKE`,
/// the same override `crates/dae-qt/build.rs` honours), then `PATH`. Resolving
/// through the same Qt matters: generated C++ is compiled against the headers
/// cxx-qt links, and a tool/headers version mismatch fails with errors that
/// name neither.
///
/// `QT_INSTALL_BINS` alone is not enough. Qt 6 keeps the code generators in
/// `libexec` on Unix — `qmlcachegen`, `qmltyperegistrar`, `qmllint` — and only
/// the Windows packages put them next to `qmake`. A lookup that asked `bin`
/// and then `PATH` therefore found nothing on Linux (`libexec` is never on
/// `PATH`), which surfaced as `DAE_QML_AOT=1` failing the release build with
/// "qmlcachegen was not found" on a machine where qmake had just been used to
/// compile the C++ bridge.
fn find_qt_tool(override_var: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(override_var) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let qmake = std::env::var_os("QMAKE")
        .map(PathBuf::from)
        .or_else(|| which("qmake6").or_else(|| which("qmake")));
    if let Some(qmake) = qmake {
        // `QT_INSTALL_*` is the Qt being built against, `QT_HOST_*` the one
        // providing the tools; for a native build they are the same prefix,
        // and asking both costs one `qmake -query` each.
        for key in [
            "QT_INSTALL_BINS",
            "QT_HOST_BINS",
            "QT_INSTALL_LIBEXECS",
            "QT_HOST_LIBEXECS",
        ] {
            let output = match Command::new(&qmake).args(["-query", key]).output() {
                Ok(output) if output.status.success() => output,
                _ => continue,
            };
            let dir = String::from_utf8_lossy(&output.stdout);
            let dir = dir.trim();
            if dir.is_empty() {
                continue;
            }
            let candidate = Path::new(dir).join(exe_name(name));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // Last structured chance before `PATH`: the Qt root the CI action and the
    // packaging steps export, probed with both layouts. This is what covers a
    // build script that runs with `QMAKE` unset and `qmake6` off `PATH`.
    for key in ["QT_ROOT_DIR", "QTDIR"] {
        let Some(root) = std::env::var_os(key) else {
            continue;
        };
        for sub in ["bin", "libexec"] {
            let candidate = Path::new(&root).join(sub).join(exe_name(name));
            if candidate.is_file() {
                return Some(candidate);
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

fn query_qmake(key: &str) -> Option<PathBuf> {
    let qmake = std::env::var_os("QMAKE")
        .map(PathBuf::from)
        .or_else(|| which("qmake6").or_else(|| which("qmake")))?;
    let output = Command::new(qmake).args(["-query", key]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Synthesizes `<out_dir>/modules/dae/{qmldir,plugin.qmltypes}` from the
/// `#[qobject]` Rust declarations so `qmlcachegen`, `qmltc`, and `qmllint`
/// resolve `import dae` cleanly.
fn prepare_dae_module(out_dir: &Path) -> Result<PathBuf, String> {
    let dir = out_dir.join("modules").join("dae");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;

    const BASE_QMLTYPES: &str = "import QtQuick.tooling 1.2\n\nModule {\n}\n";
    let prepared = with_port_placeholders(BASE_QMLTYPES, &ported_qml_elements());
    std::fs::write(dir.join("plugin.qmltypes"), prepared)
        .map_err(|error| format!("could not write synthesized plugin.qmltypes: {error}"))?;

    let qmldir = "module dae\ntypeinfo plugin.qmltypes\ndepends QtQuick auto\n";
    std::fs::write(dir.join("qmldir"), qmldir)
        .map_err(|error| format!("could not write synthesized qmldir: {error}"))?;
    Ok(out_dir.join("modules"))
}

/// One type that Qt Bridge registers at runtime, as its `QmlElement` impl and
/// its `#[qobject]` impl declare it.
struct PortedElement {
    uri: String,
    name: String,
    major: u8,
    minor: u8,
    singleton: bool,
    prototype: String,
    /// Properties, as (QML name, QML type, optional NOTIFY signal name).
    properties: Vec<(String, String, Option<String>)>,
    /// Slots and signals, as (QML name, parameter count).
    slots: Vec<(String, usize)>,
    signals: Vec<(String, usize)>,
}

/// The ported types that are missing from cxx-qt's `plugin.qmltypes`.
///
/// Qt Bridge registers at runtime and emits no type information, so every
/// module that moves off CXX-Qt disappears from the metadata that
/// `qmlcachegen`, `qmltc` and `qmllint` resolve `import dae` through — while the
/// QML surface stays identical. The declarations are read back from the Rust
/// sources: the URI, name, version and singleton flag from the `impl
/// QmlElement` block, the invokables from the `#[qslot]`/`#[qsignal]` markers.
/// Porting a module therefore needs no second list here, and a placeholder
/// cannot disagree with what the type actually registers.
fn ported_qml_elements() -> Vec<PortedElement> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sources = manifest.join("../../crates/dae-qt/src");
    let Ok(entries) = std::fs::read_dir(&sources) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .collect();
    files.sort();

    let mut elements = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        const HEADER: &str = "impl QmlElement for ";
        if !text.contains(HEADER) {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let qobject_blocks = split_qobject_blocks(&text);

        let mut rest = text.as_str();
        while let Some(index) = rest.find(HEADER) {
            rest = &rest[index + HEADER.len()..];
            let Some(open) = rest.find('{') else { break };
            let name = rest[..open].trim().to_owned();
            let body = &rest[open + 1..];
            // The impl's own closing brace is the first `}` in column 0.
            let Some(close) = body.find("\n}") else { break };
            let body = &body[..close];
            rest = &rest[open + 1 + close..];

            let (Some(uri), Some(major), Some(minor), Some(singleton)) = (
                rust_string_const(body, "URI"),
                rust_number_const(body, "MAJOR_VERSION"),
                rust_number_const(body, "MINOR_VERSION"),
                rust_bool_const(body, "IS_SINGLETON"),
            ) else {
                continue;
            };

            let scope = qobject_blocks
                .iter()
                .find(|block| {
                    block.contains(&format!("impl {name} "))
                        || block.contains(&format!("impl {name}\n"))
                        || block.contains(&format!("impl {name}\r"))
                        || block.contains(&format!("struct {name} "))
                        || block.contains(&format!("struct {name}\n"))
                        || block.contains(&format!("struct {name}\r"))
                })
                .map(String::as_str)
                .unwrap_or(text.as_str());
            let qobject_attr = scope
                .lines()
                .find(|line| line.trim_start().starts_with("#[qobject"))
                .unwrap_or("");
            // `ConvertToCamelCase` renames the members, not the properties.
            let camel_case = qobject_attr.contains("ConvertToCamelCase");
            let prototype = if qobject_attr.contains("Base = QAbstractItemModel") {
                "QAbstractItemModel".to_owned()
            } else {
                "QObject".to_owned()
            };

            elements.push(PortedElement {
                uri,
                name: name.clone(),
                major,
                minor,
                singleton,
                prototype,
                properties: bridged_properties(scope, &text, &name, camel_case),
                slots: bridged_members(scope, "#[qslot]", camel_case),
                signals: bridged_members(scope, "#[qsignal]", camel_case),
            });
        }
    }
    elements
}

fn split_qobject_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("#[qobject") {
        rest = &rest[index..];
        let Some(open) = rest.find('{') else { break };
        let mut depth = 0usize;
        let mut end = None;
        for (offset, byte) in rest[open..].bytes().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        blocks.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    blocks
}

/// The `#[qslot]`/`#[qsignal]` functions of a file, as (QML name, arity).
///
/// One file carries one ported type, so the markers do not need to be tied back
/// to an impl block. The QML name follows the same rule the macro applies: with
/// `ConvertToCamelCase` the identifier is camelCased, otherwise it is exported
/// verbatim.
fn bridged_members(text: &str, marker: &str, camel_case: bool) -> Vec<(String, usize)> {
    let mut members = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find(marker) {
        rest = &rest[index + marker.len()..];
        let Some(fn_index) = rest.find("fn ") else { break };
        rest = &rest[fn_index + 3..];
        let Some(open) = rest.find('(') else { break };
        let name = rest[..open].trim();
        let name = if camel_case {
            snake_to_camel(name)
        } else {
            name.to_owned()
        };
        let params = &rest[open + 1..];
        // Match the closing parenthesis: a parameter type can nest parentheses
        // (`Option<...>` no, but a closure or tuple in a default could).
        let mut depth = 1usize;
        let mut end = None;
        for (offset, byte) in params.bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        let arity = params[..end]
            .split(',')
            .filter(|param| {
                let param = param.trim();
                !matches!(param, "" | "&self" | "&mut self" | "self" | "mut self")
            })
            .count();
        rest = &params[end..];
        members.push((name, arity));
    }
    members
}

/// The macro's own rule (`qtbridge-gen`'s `case_conv`): the first token keeps a
/// lowercase head, every later token capitalises its head.
fn snake_to_camel(name: &str) -> String {
    let mut camel = String::with_capacity(name.len());
    for token in name.split('_') {
        let mut chars = token.chars();
        let Some(first) = chars.next() else { continue };
        let first = if camel.is_empty() {
            first.to_ascii_lowercase()
        } else {
            first.to_ascii_uppercase()
        };
        camel.push(first);
        camel.push_str(chars.as_str());
    }
    camel
}

/// The `qproperty!` declarations of a file, as (QML name, QML type).
///
/// The QML name is the string literal; the type comes from the struct field the
/// declaration names with `Member = …`, because the macro does not repeat it.
/// A field whose Rust type has no QML spelling yet is declared `var`: the value
/// still resolves at run time, only static type checks are skipped.
fn bridged_properties(
    scope: &str,
    full_text: &str,
    struct_name: &str,
    camel_case: bool,
) -> Vec<(String, String, Option<String>)> {
    let field_types = struct_field_types(full_text, struct_name);
    let mut properties = Vec::new();
    let mut rest = scope;
    while let Some(index) = rest.find("qproperty!(") {
        rest = &rest[index + "qproperty!(".len()..];
        let Some(opening_quote) = rest.find('"') else {
            break;
        };
        let after = &rest[opening_quote + 1..];
        let Some(closing_quote) = after.find('"') else {
            break;
        };
        let name = after[..closing_quote].to_owned();
        let arguments = &after[closing_quote + 1..];
        let arguments = &arguments[..arguments.find(')').unwrap_or(arguments.len())];
        let field = arguments.find("Member = ").and_then(|at| {
            let field = &arguments[at + "Member = ".len()..];
            field
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .next()
        });
        let read_fn = arguments.find("Read = ").and_then(|at| {
            let func = &arguments[at + "Read = ".len()..];
            func.split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .next()
        });
        let notify = arguments.find("Notify = ").and_then(|at| {
            let sig = &arguments[at + "Notify = ".len()..];
            let sig = sig
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .next()?;
            Some(if camel_case {
                snake_to_camel(sig)
            } else {
                sig.to_owned()
            })
        });
        let qml_type = field
            .and_then(|field| field_types.get(field))
            .map(|ty| qml_type(ty))
            .or_else(|| read_fn.and_then(|func| getter_return_type(scope, func)).map(|ty| qml_type(&ty)))
            .unwrap_or_else(|| "var".to_owned());
        properties.push((name, qml_type, notify));
        rest = &after[closing_quote..];
    }
    properties
}

fn getter_return_type(scope: &str, func: &str) -> Option<String> {
    let needle = format!("fn {func}(");
    let at = scope.find(&needle)?;
    let sig = &scope[at + needle.len()..];
    let arrow = sig.find("->")?;
    let brace = sig[arrow..].find('{')?;
    Some(sig[arrow + 2..arrow + brace].trim().to_owned())
}

/// The fields of `struct <name> { … }`, as Rust type text.
fn struct_field_types(text: &str, struct_name: &str) -> HashMap<String, String> {
    let header = format!("struct {struct_name} {{");
    let mut types = HashMap::new();
    let Some(start) = text.find(&header) else {
        return types;
    };
    let body = &text[start + header.len()..];
    let mut depth = 1usize;
    let mut end = None;
    for (offset, byte) in body.bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return types;
    };
    for line in body[..end].lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("#[") {
            continue;
        }
        let Some((name, ty)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let ty = ty.trim().trim_end_matches(',').trim();
        if !name.is_empty() && !ty.is_empty() {
            types.insert(name.to_owned(), ty.to_owned());
        }
    }
    types
}

/// The QML spelling of a Rust type the bridge can carry. Anything else stays
/// `var`.
fn qml_type(rust: &str) -> String {
    match rust {
        "String" => "QString",
        "bool" => "bool",
        "i32" => "int",
        "i64" => "qlonglong",
        "u32" => "uint",
        "u64" => "qulonglong",
        "f32" => "float",
        "f64" => "double",
        "Vec<String>" => "QStringList",
        _ => "var",
    }
    .to_owned()
}

fn rust_const_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("const {key}:");
    let line = body.lines().find(|line| line.trim_start().starts_with(&prefix))?;
    Some(line.split_once('=')?.1.trim().trim_end_matches(';').trim())
}

fn rust_string_const(body: &str, key: &str) -> Option<String> {
    let value = rust_const_value(body, key)?;
    let quoted = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(quoted.to_owned())
}

fn rust_number_const(body: &str, key: &str) -> Option<u8> {
    rust_const_value(body, key)?.parse().ok()
}

fn rust_bool_const(body: &str, key: &str) -> Option<bool> {
    match rust_const_value(body, key)? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Appends the placeholders for `elements` to a `plugin.qmltypes` document.
///
/// `isTypeOpaque: true` says "this type's property list is not described here":
/// a binding to one resolves at run time instead of being compiled, which is
/// exactly the situation — Qt Bridge builds the metaobject at run time. qmllint
/// still looks call sites up, so the invokables and signals are listed; their
/// parameter types are `var`, because the Rust signatures are not available here
/// and only the arity has to match for the lookup to resolve.
///
/// A singleton is not instantiable, so it mirrors what cxx-qt wrote for the ones
/// it still owns.
fn with_port_placeholders(text: &str, elements: &[PortedElement]) -> String {
    if elements.is_empty() {
        return text.to_owned();
    }
    let mut blocks = String::new();
    for element in elements {
        let PortedElement {
            uri,
            name,
            major,
            minor,
            singleton,
            prototype,
            properties,
            slots,
            signals,
        } = element;
        let creatable = !singleton;
        blocks.push_str(&format!(
            "    Component {{\n\
             \x20       name: \"{name}\"\n\
             \x20       accessSemantics: \"reference\"\n\
             \x20       prototype: \"{prototype}\"\n\
             \x20       exports: [\"{uri}/{name} {major}.{minor}\"]\n\
             \x20       isCreatable: {creatable}\n\
             \x20       isSingleton: {singleton}\n"
        ));
        for (index, (name, qml_type, notify)) in properties.iter().enumerate() {
            let mut chars = name.chars();
            let setter = match chars.next() {
                Some(first) => format!("set{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => "set".to_owned(),
            };
            if let Some(notify) = notify {
                blocks.push_str(&format!(
                    "        Property {{ name: \"{name}\"; type: \"{qml_type}\"; read: \"{name}\"; write: \"{setter}\"; notify: \"{notify}\"; index: {index} }}\n"
                ));
            } else {
                blocks.push_str(&format!(
                    "        Property {{ name: \"{name}\"; type: \"{qml_type}\"; read: \"{name}\"; index: {index}; isReadonly: true }}\n"
                ));
            }
        }
        for (name, arity) in slots {
            blocks.push_str("        Method {\n");
            blocks.push_str(&format!("            name: \"{name}\"\n"));
            blocks.push_str("            type: \"var\"\n");
            push_parameters(&mut blocks, *arity);
            blocks.push_str("        }\n");
        }
        for (name, arity) in signals {
            if *arity == 0 {
                blocks.push_str(&format!("        Signal {{ name: \"{name}\" }}\n"));
            } else {
                blocks.push_str("        Signal {\n");
                blocks.push_str(&format!("            name: \"{name}\"\n"));
                push_parameters(&mut blocks, *arity);
                blocks.push_str("        }\n");
            }
        }
        blocks.push_str("    }\n");
    }
    // The document is one `Module { … }`; the placeholders belong inside it.
    match text.rfind('}') {
        Some(index) => format!("{}{}{}", &text[..index], blocks, &text[index..]),
        None => text.to_owned(),
    }
}

fn push_parameters(block: &mut String, arity: usize) {
    for index in 0..arity {
        block.push_str(&format!(
            "            Parameter {{ name: \"arg{index}\"; type: \"var\" }}\n"
        ));
    }
}

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

fn compile_resources(out_dir: &Path, manifest: &Path) -> PathBuf {
    let rcc = find_qt_tool("DAE_RCC", "rcc")
        .expect("rcc was not found (set QMAKE or DAE_RCC)");

    const TRANSLATIONS: &[(&str, &str)] = &[
        ("../../qml/i18n/dae_en.ts", "dae_en.ts"),
        ("../../qml/i18n/dae_zh_CN.ts", "dae_zh_CN.ts"),
        ("../../qml/i18n/dae_zh_TW.ts", "dae_zh_TW.ts"),
    ];
    const FONTS: &[(&str, &str)] = &[
        ("../../Assets/fonts/GoogleSansFlex.ttf", "GoogleSansFlex.ttf"),
        ("../../Assets/fonts/GoogleSansCodeVF.ttf", "GoogleSansCodeVF.ttf"),
        (
            "../../Assets/fonts/SymbolsNerdFontMono-Regular.ttf",
            "SymbolsNerdFontMono-Regular.ttf",
        ),
        (
            "../../Assets/fonts/LICENSE-GoogleSansFlex.txt",
            "LICENSE-GoogleSansFlex.txt",
        ),
        (
            "../../Assets/fonts/LICENSE-GoogleSansCode.txt",
            "LICENSE-GoogleSansCode.txt",
        ),
        (
            "../../Assets/fonts/LICENSE-NerdFontsSymbols.txt",
            "LICENSE-NerdFontsSymbols.txt",
        ),
    ];

    let mut qrc = String::from("<RCC>\n");
    let mut push_group = |prefix: &str, entries: &[(&str, &str)]| {
        qrc.push_str(&format!("  <qresource prefix=\"{prefix}\">\n"));
        for (source, alias) in entries {
            println!("cargo:rerun-if-changed={source}");
            let full = std::fs::canonicalize(manifest.join(source))
                .unwrap_or_else(|error| panic!("missing resource {source}: {error}"));
            let normalized = full.to_string_lossy().replace('\\', "/");
            // Strip Windows UNC prefix `//?/` if canonicalize produced one.
            let clean = normalized.strip_prefix("//?/").unwrap_or(&normalized);
            qrc.push_str(&format!("    <file alias=\"{alias}\">{clean}</file>\n"));
        }
        qrc.push_str("  </qresource>\n");
    };

    push_group(QML_PREFIX, QML_FILES);
    push_group("/i18n", TRANSLATIONS);
    push_group("/fonts", FONTS);
    qrc.push_str("</RCC>\n");

    let qrc_path = out_dir.join("dae_resources.qrc");
    std::fs::write(&qrc_path, qrc).expect("write dae_resources.qrc");

    let cpp_out = out_dir.join("qrc_dae_resources.cpp");
    let mut command = Command::new(&rcc);
    command
        .arg("--name")
        .arg("dae_resources")
        .arg("-o")
        .arg(&cpp_out)
        .arg(&qrc_path);
    run_tool(&mut command).expect("rcc failed on dae_resources.qrc");
    cpp_out
}

fn main() {
    for var in ["QMAKE", "DAE_RCC", "QT_ROOT_DIR", "QTDIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    // The shell draws its own chrome and never sets a QWindow icon, so the
    // taskbar button and Alt+Tab fell back to the generic application icon.
    // Embed the icon as resource 1; `set_window_icon` in
    // `crates/dae-qt/cpp/window.h` hands it to every window, and Explorer uses
    // it for the portable exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../packaging/icons/icon.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/icons/icon.ico");
        resource
            .compile()
            .expect("compile packaging/icons/icon.ico into dae.exe");
    }

    // The C++ type descriptions for the two modules registered by hand at run
    // time (`dae.terminal`, `dae.activation`). They are not documents, so they
    // are not in `QML_FILES`, but both QML compilers read them through `-I qml`;
    // without this a change would not rerun the build script.
    for path in [
        "../../qml/dae/terminal/qmldir",
        "../../qml/dae/terminal/terminal.qmltypes",
        "../../qml/dae/activation/qmldir",
        "../../qml/dae/activation/activation.qmltypes",
        "../../qml/dae/m3/qmldir",
        "../../qml/dae/m3/m3.qmltypes",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=cpp/qml_cache_init.cpp");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let modules = prepare_dae_module(&out_dir).expect("prepare synthesized dae QML module");

    // Runs first because it is the cheapest guard: a Qt upgrade that breaks
    // type compilation fails here instead of after the AOT and C++ build.
    typec::compile(&modules);
    lint::check(&modules);

    let rcc_cpp = compile_resources(&out_dir, &manifest);
    let units = aot::compile(&modules);

    let headers = query_qmake("QT_INSTALL_HEADERS")
        .expect("qmake -query QT_INSTALL_HEADERS failed (set QMAKE or add Qt to PATH)");
    let libs = query_qmake("QT_INSTALL_LIBS")
        .expect("qmake -query QT_INSTALL_LIBS failed");

    let mut cc = cc::Build::new();
    cc.cpp(true)
        .std("c++17")
        .flag_if_supported("/Zc:__cplusplus")
        .flag_if_supported("/permissive-");
    if cfg!(target_env = "msvc") {
        cc.flag("-utf-8");
    }
    for module in ["", "QtCore", "QtGui", "QtQml", "QtQuick"] {
        let dir = if module.is_empty() {
            headers.clone()
        } else {
            headers.join(module)
        };
        if dir.exists() {
            cc.include(dir);
        }
    }
    #[cfg(target_os = "macos")]
    cc.flag("-F").flag(libs.to_string_lossy().as_ref());

    cc.file("cpp/qml_cache_init.cpp").file(&rcc_cpp);
    if !units.is_empty() {
        cc.define("DAE_QML_AOT", None);
        for unit in &units {
            cc.file(unit);
        }
    }
    apply_cxx_profile(&mut cc);
    cc.compile("dae_app_qml");

    println!("cargo:rustc-link-search=native={}", libs.display());
    for lib in ["Qt6Core", "Qt6Qml"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
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

    use super::{QML_FILES, QML_PREFIX, run_tool};

    /// Name the loader registers under. Only has to be unique; it ends up in
    /// the generated `qInitResources_<name>` symbols.
    const RESOURCE_NAME: &str = "dae_qmlcache";

    /// The generated units, plus the loader that points at them. Empty when the
    /// step is skipped or degraded, which just means the engine compiles the
    /// sources at run time as before.
    pub(super) fn compile(modules: &Path) -> Vec<PathBuf> {
        // Before the gate below, not after: a skipped build still has to react
        // to `DAE_QML_AOT=1` on the next invocation, and cargo only watches what
        // the script prints this time.
        for name in ["DAE_QML_AOT", "DAE_QMLCACHEGEN", "QMAKE"] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        match env::var("DAE_QML_AOT").ok().as_deref() {
            // Explicitly off; also keeps a debug loop from warning every build.
            Some("0") => Vec::new(),
            // Explicitly on, in any profile, and a failure is fatal so CI can
            // tell "shipped with units" from "shipped without".
            Some("1") => compile_units(true, modules),
            // An unrecognised value behaves like unset rather than silently
            // disabling the step for a release.
            _ if env::var("PROFILE").as_deref() == Ok("release") => compile_units(false, modules),
            // Debug builds skip it: the generated C++ is ~18 MB, and
            // recompiling it after every QML edit costs more than the launch it
            // would speed up.
            _ => Vec::new(),
        }
    }

    fn compile_units(strict: bool, modules: &Path) -> Vec<PathBuf> {
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

        // The synthesized `dae` `qmldir`/`plugin.qmltypes` let the compiler see
        // the Rust types so bindings on `dae.*` compile directly.
        let qmldir = modules.join("dae").join("qmldir");

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
            command
                .arg("--resource-path")
                .arg(&resource_path)
                .arg("-I")
                .arg(modules)
                .arg("-i")
                .arg(&qmldir)
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

    use super::{QML_FILES, QML_PREFIX, find_qt_tool, run_tool};

    /// Documents `qmltc` cannot compile today. `MediaPreview.qml` binds a
    /// `MediaPlayer`; Qt 6.12's own QtMultimedia qmltypes marks
    /// `QAudioBufferOutput` (the type behind `audioBufferOutput`)
    /// `isTypeOpaque: true`, and qmltc refuses to generate a setter for an
    /// opaque property type. The defect is in Qt's metadata, not this tree.
    /// Drop the entry once the type is exported.
    const SKIPPED: &[&str] = &["qml/MediaPreview.qml"];

    pub(super) fn compile(modules: &Path) {
        // Printed even when the step is skipped: a build that did not parse
        // this environment this time must still react when it is set.
        for name in ["DAE_QML_TYPEC", "DAE_QMLTC", "QMAKE"] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        if env::var("DAE_QML_TYPEC").ok().as_deref() != Some("1") {
            return;
        }
        if let Err(reason) = check(modules) {
            panic!("DAE_QML_TYPEC=1 was requested but QML type compilation failed: {reason}");
        }
    }

    /// Runs `qmltc` over every document and reports the first failure.
    fn check(modules: &Path) -> Result<(), String> {
        let qmltc = find_qt_tool("DAE_QMLTC", "qmltc")
            .ok_or_else(|| "qmltc was not found (set QMAKE or DAE_QMLTC)".to_owned())?;

        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let qml_dir = manifest.join("../../qml");
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("qmltc");
        fs::create_dir_all(out_dir.join("units"))
            .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;

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
                .arg(modules)
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

/// QML lint, wired as a build-time check.
///
/// `qmllint` resolves every id, binding, property and signal against the type
/// metadata, which is exactly the layer qmlcachegen does *not* report: a
/// binding that no longer resolves compiles to a fallback lookup and stays
/// silent. The tree is at zero warnings, so the check is a hard one: any new
/// unqualified access, missing property or unresolved import fails the build.
///
/// `DAE_QML_LINT=1` runs it. Unset or `0` skips it: it is not part of a normal
/// build, and CI (the Qt job) sets the variable. It is separate from `mod typec`
/// because qmllint also covers `MediaPreview.qml`, which qmltc cannot compile
/// yet, and because the two tools catch different regressions.
mod lint {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{QML_FILES, find_qt_tool};

    pub(super) fn check(modules: &Path) {
        for name in ["DAE_QML_LINT", "DAE_QMLLINT", "QMAKE"] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        if env::var("DAE_QML_LINT").ok().as_deref() != Some("1") {
            return;
        }
        if let Err(reason) = run(modules) {
            panic!("DAE_QML_LINT=1 was requested but QML linting failed: {reason}");
        }
    }

    fn run(modules: &Path) -> Result<(), String> {
        let qmllint = find_qt_tool("DAE_QMLLINT", "qmllint")
            .ok_or_else(|| "qmllint was not found (set QMAKE or DAE_QMLLINT)".to_owned())?;
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let qml_dir = manifest.join("../../qml");

        for (source, alias) in QML_FILES {
            // The `.js` files are imported by name, not linted as documents,
            // and `qmldir` is not a document at all.
            if !alias.ends_with(".qml") {
                continue;
            }

            let mut command = Command::new(&qmllint);
            command
                // The synthesized Rust QML types (`plugin.qmltypes`).
                .arg("-I")
                .arg(modules)
                // The declarations for the two hand-registered modules
                // (`dae.terminal`, `dae.activation`).
                .arg("-I")
                .arg(&qml_dir)
                // `Theme` is declared as a singleton in `qml/qmldir`; without
                // this qmllint reports the pragma as undeclared.
                .arg("-i")
                .arg(qml_dir.join("qmldir"))
                // Absolute, so the file's own directory matches the `-i`
                // above: qmllint compares the singleton declaration against
                // the qmldir of the directory it sees the file in.
                .arg(manifest.join(source));

            // `--max-warnings 0` makes qmllint exit 127 on `ShellWindow.qml`
            // (Qt 6.12) instead of reporting a count, so the warnings are
            // read from the report instead. It exits 0 even when it warned.
            let output = command
                .output()
                .map_err(|error| format!("could not run qmllint on {source}: {error}"))?;
            let report = String::from_utf8_lossy(&output.stderr).into_owned();
            if !output.status.success() {
                return Err(format!("qmllint failed on {source}: {}", report.trim()));
            }
            if report
                .lines()
                .any(|line| line.starts_with("Warning:") || line.starts_with("Error:"))
            {
                return Err(format!(
                    "qmllint reported warnings for {source}:\n{}",
                    report.trim()
                ));
            }
            println!("cargo:rerun-if-changed={source}");
        }
        Ok(())
    }
}
