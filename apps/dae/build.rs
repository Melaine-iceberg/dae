use std::collections::HashMap;
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
    ("../../qml/solar-marks.json", "qml/solar-marks.json"),
    ("../../qml/SidebarTree.qml", "qml/SidebarTree.qml"),
    ("../../qml/OverviewView.qml", "qml/OverviewView.qml"),
    ("../../qml/SpaceView.qml", "qml/SpaceView.qml"),
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

/// One type that Qt Bridge registers at runtime, as its `QmlElement` impl and
/// its `#[qobject]` impl declare it.
struct PortedElement {
    uri: String,
    name: String,
    major: u8,
    minor: u8,
    singleton: bool,
    /// Properties, as (QML name, QML type).
    properties: Vec<(String, String)>,
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
        // `ConvertToCamelCase` renames the members, not the properties.
        let camel_case = text
            .lines()
            .filter(|line| line.trim_start().starts_with("#[qobject("))
            .any(|line| line.contains("ConvertToCamelCase"));
        let slots = bridged_members(&text, "#[qslot]", camel_case);
        let signals = bridged_members(&text, "#[qsignal]", camel_case);

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
            elements.push(PortedElement {
                uri,
                name: name.clone(),
                major,
                minor,
                singleton,
                properties: bridged_properties(&text, &name),
                slots: slots.clone(),
                signals: signals.clone(),
            });
        }
    }
    elements
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
fn bridged_properties(text: &str, struct_name: &str) -> Vec<(String, String)> {
    let field_types = struct_field_types(text, struct_name);
    let mut properties = Vec::new();
    let mut rest = text;
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
        let qml_type = field
            .and_then(|field| field_types.get(field))
            .map_or_else(|| "var".to_owned(), |ty| qml_type(ty));
        properties.push((name, qml_type));
        rest = &after[closing_quote..];
    }
    properties
}

/// The fields of `struct <name> { … }`, as Rust type text.
fn struct_field_types(text: &str, struct_name: &str) -> HashMap<String, String> {
    let header = format!("struct {struct_name} {{");
    let mut types = HashMap::new();
    let Some(start) = text.find(&header) else {
        return types;
    };
    let body = &text[start + header.len()..];
    let Some(end) = body.find("\n}") else {
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
            properties,
            slots,
            signals,
        } = element;
        let creatable = !singleton;
        blocks.push_str(&format!(
            "    Component {{\n\
             \x20       name: \"{name}\"\n\
             \x20       accessSemantics: \"reference\"\n\
             \x20       prototype: \"QObject\"\n\
             \x20       exports: [\"{uri}/{name} {major}.{minor}\"]\n\
             \x20       isCreatable: {creatable}\n\
             \x20       isSingleton: {singleton}\n\
             \x20       isTypeOpaque: true\n"
        ));
        for (name, qml_type) in properties {
            blocks.push_str(&format!(
                "        Property {{ name: \"{name}\"; type: \"{qml_type}\" }}\n"
            ));
        }
        for (name, arity) in slots {
            blocks.push_str("        Method {\n");
            blocks.push_str(&format!("            name: \"{name}\"\n"));
            blocks.push_str("            type: \"var\"\n");
            push_parameters(&mut blocks, *arity);
            blocks.push_str("        }\n");
        }
        for (name, arity) in signals {
            blocks.push_str(&format!("        Signal {{ name: \"{name}\""));
            if *arity > 0 {
                blocks.push_str("; Parameter { name: \"arg0\"; type: \"var\" }");
                for index in 1..*arity {
                    blocks.push_str(&format!(
                        "; Parameter {{ name: \"arg{index}\"; type: \"var\" }}"
                    ));
                }
            }
            blocks.push_str(" }\n");
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

    let files = QML_FILES
        .iter()
        .map(|(path, alias)| QResourceFile::new(path).alias((*alias).to_owned()));
    let qml = QResource::new().prefix(QML_PREFIX).files(files);

    // The translator reads these XML documents directly (see
    // `crates/dae-qt/cpp/translator.cpp`), so no `lrelease` step is needed.
    let translations = QResource::new().prefix("/i18n").files([
        QResourceFile::new("../../qml/i18n/dae_en.ts").alias("dae_en.ts".to_owned()),
        QResourceFile::new("../../qml/i18n/dae_zh_CN.ts").alias("dae_zh_CN.ts".to_owned()),
        QResourceFile::new("../../qml/i18n/dae_zh_TW.ts").alias("dae_zh_TW.ts".to_owned()),
    ]);

    // The UI faces (proposal §6.2.1, §9.2). Noto Sans SC covers the Latin
    // alphabet and Simplified Chinese in one variable file, so every label
    // keeps one face across both scripts; GoogleSansCode NFM (the Nerd Font
    // Mono patch) covers hashes, paths, the terminal and code preview. Both
    // are OFL 1.1, which requires the licence to travel with the fonts, so it
    // is mounted beside them. `dae_qt_apply_fonts` loads these paths before
    // the first QML document exists.
    let fonts = QResource::new().prefix("/fonts").files([
        QResourceFile::new("../../Assets/fonts/NotoSansSC.ttf")
            .alias("NotoSansSC.ttf".to_owned()),
        QResourceFile::new("../../Assets/fonts/GoogleSansCodeNerdFontMono-Regular.ttf")
            .alias("GoogleSansCodeNerdFontMono-Regular.ttf".to_owned()),
        QResourceFile::new("../../Assets/fonts/GoogleSansCodeNerdFontMono-Bold.ttf")
            .alias("GoogleSansCodeNerdFontMono-Bold.ttf".to_owned()),
        QResourceFile::new("../../Assets/fonts/LICENSE-NotoSansSC.txt")
            .alias("LICENSE-NotoSansSC.txt".to_owned()),
        QResourceFile::new("../../Assets/fonts/LICENSE-GoogleSansCode.txt")
            .alias("LICENSE-GoogleSansCode.txt".to_owned()),
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
        .qrc_resources(
            QResources::new()
                .resource(qml)
                .resource(translations)
                .resource(fonts),
        )
        // Always compiled: it is the reference that keeps the generated units
        // out of the archive's dead objects. See the file itself.
        .cpp_file("cpp/qml_cache_init.cpp");

    // Applies to the anchor above, and (because cxx-qt-build shares one
    // `cc::Build`) to the AOT units added below.
    builder = unsafe { builder.cc_builder(|cc| apply_cxx_profile(cc)) };

    // Runs first because it is the cheapest guard: a Qt upgrade that breaks
    // type compilation fails here instead of after the AOT and C++ build.
    typec::compile();
    lint::check();

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

    use super::{
        QML_FILES, QML_PREFIX, find_qt_tool, ported_qml_elements, qml_modules, run_tool,
        with_port_placeholders,
    };

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

    /// The `dae` module as `qmltc` and `qmllint` need to see it.
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
    /// The same file is where the types that moved to Qt Bridge went missing
    /// from: those register at run time and write no metadata, so every ported
    /// type is added back as an opaque placeholder (see
    /// [`ported_qml_elements`]). Stripping first and appending after is what
    /// keeps the two apart — one pass would otherwise delete the placeholders it
    /// depends on.
    ///
    /// `depends QtQuick` is what resolves `QAbstractListModel`, the prototype
    /// of every model type here; cxx-qt cannot infer it from the prototype
    /// names, so it is added in `crates/dae-qt/build.rs` as well.
    pub(super) fn prepare_module(out_dir: &Path) -> Result<PathBuf, String> {
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
        let prepared = with_port_placeholders(&strip_opaque(&text), &ported_qml_elements());
        fs::write(dir.join("plugin.qmltypes"), prepared)
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
    use std::path::PathBuf;
    use std::process::Command;

    use super::{QML_FILES, find_qt_tool, typec};

    pub(super) fn check() {
        for name in [
            "DAE_QML_LINT",
            "DAE_QMLLINT",
            "QMAKE",
            "CARGO_TARGET_DIR",
            "CXX_QT_EXPORT_DIR",
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }

        if env::var("DAE_QML_LINT").ok().as_deref() != Some("1") {
            return;
        }
        if let Err(reason) = run() {
            panic!("DAE_QML_LINT=1 was requested but QML linting failed: {reason}");
        }
    }

    fn run() -> Result<(), String> {
        let qmllint = find_qt_tool("DAE_QMLLINT", "qmllint")
            .ok_or_else(|| "qmllint was not found (set QMAKE or DAE_QMLLINT)".to_owned())?;
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let qml_dir = manifest.join("../../qml");
        // The sanitised copy, not cxx-qt's own file: it is the one that carries
        // the ported types (see `typec::prepare_module`), and a type that only
        // exists there would otherwise read as an unresolved import here.
        let modules = typec::prepare_module(&PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")))?;

        for (source, alias) in QML_FILES {
            // The `.js` files are imported by name, not linted as documents,
            // and `qmldir` is not a document at all.
            if !alias.ends_with(".qml") {
                continue;
            }

            let mut command = Command::new(&qmllint);
            command
                // The Rust types, from cxx-qt's generated `plugin.qmltypes`.
                .arg("-I")
                .arg(&modules)
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
