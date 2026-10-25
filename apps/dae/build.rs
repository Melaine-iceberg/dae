use cxx_qt_build::{CxxQtBuilder, QResource, QResources};
use qt_build_utils::QResourceFile;

/// The view layer lives at the repository root (`qml/`) so it stays readable
/// next to the decision docs. The resource aliases keep the import paths
/// relative to `qml/` even though the files come from `../../qml/`.
const QML_FILES: &[(&str, &str)] = &[
    ("../../qml/Main.qml", "qml/Main.qml"),
    ("../../qml/ShellWindow.qml", "qml/ShellWindow.qml"),
    ("../../qml/Tab.qml", "qml/Tab.qml"),
    ("../../qml/TabStrip.qml", "qml/TabStrip.qml"),
    ("../../qml/TerminalPanel.qml", "qml/TerminalPanel.qml"),
    ("../../qml/TitleBar.qml", "qml/TitleBar.qml"),
    ("../../qml/FileList.qml", "qml/FileList.qml"),
    ("../../qml/Sidebar.qml", "qml/Sidebar.qml"),
    ("../../qml/ContextMenu.qml", "qml/ContextMenu.qml"),
    ("../../qml/NamePrompt.qml", "qml/NamePrompt.qml"),
    ("../../qml/SettingsPanel.qml", "qml/SettingsPanel.qml"),
    ("../../qml/Toast.qml", "qml/Toast.qml"),
    ("../../qml/TearOffWindow.qml", "qml/TearOffWindow.qml"),
    ("../../qml/Shell.js", "qml/Shell.js"),
];

fn main() {
    let files = QML_FILES
        .iter()
        .map(|(path, alias)| QResourceFile::new(path).alias((*alias).to_owned()));
    let qml = QResource::new().prefix("/qt/qml/dae").files(files);

    CxxQtBuilder::new()
        .qt_module("Quick")
        .qrc_resources(QResources::new().resource(qml))
        .build();
}
