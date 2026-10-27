//! The installed apps' right-click commands, as a QML model.
//!
//! `dae_core::fs::shell_commands` owns the three platform mechanisms (Windows
//! verbs, macOS services, KDE service menus); this module is the Qt half: it
//! runs the discovery off the Qt thread, keeps one row per command, and exposes
//! the strings the menu paints — including the icon, which arrives as a `data:`
//! URL so the menu can hand it straight to an `Image`.
//!
//! Invocation is a command on `Workspace`, not a model method: it is an
//! operation with an outcome the window reports in a toast, and the model has
//! no business owning that.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};
use dae_core::fs::shell_commands::{self, ShellCommand};

use crate::runtime;

const ROLE_COMMAND_ID: i32 = 0x0400;
const ROLE_LABEL: i32 = 0x0401;
const ROLE_ICON: i32 = 0x0402;
const ROLE_GROUP: i32 = 0x0403;
const ROLE_SEPARATOR_BEFORE: i32 = 0x0404;
const ROLE_DISABLED: i32 = 0x0405;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
    }

    unsafe extern "RustQt" {
        /// The application-extensions section, reachable from QML as
        /// `ShellCommandModel`. One instance per window: a second window has
        /// its own right-click selection and its own list.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        /// Bumped on every row replacement. The menu watches this rather than
        /// `count`, because a fresh answer can carry the same number of rows
        /// with different labels or ids.
        #[qproperty(i32, revision)]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        type ShellCommandModel = super::ShellCommandModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Asks the platform what applies to this selection. `primary` is the
        /// right-clicked entry, which decides the file/folder menu; `paths` is
        /// what a command would run on. A remote selection answers with no rows
        /// at all, because none of the three mechanisms can address one.
        #[qinvokable]
        fn load(self: Pin<&mut ShellCommandModel>, paths: &QStringList, primary: &QString);

        /// Empties the list without asking the platform. The menu calls it when
        /// it closes, so a stale answer cannot surface on the next right-click
        /// before the fresh one arrives.
        #[qinvokable]
        fn clear(self: Pin<&mut ShellCommandModel>);

        /// The command id of `row`, or an empty string. The menu builds its
        /// submenu from the rows, so it needs to turn a row back into the id
        /// `Workspace.invokeShellCommand` takes.
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(&self, row: i32) -> QString;

        /// Whether `row` wants a separator drawn above it.
        #[qinvokable]
        #[cxx_name = "separatorBeforeAt"]
        fn separator_before_at(&self, row: i32) -> bool;

        /// The menu text, exactly as the owning application worded it.
        #[qinvokable]
        #[cxx_name = "labelAt"]
        fn label_at(&self, row: i32) -> QString;

        /// The owning app's name when it contributed more than one command, so
        /// the menu can draw one header per app instead of repeating it.
        #[qinvokable]
        #[cxx_name = "groupAt"]
        fn group_at(&self, row: i32) -> QString;

        /// A 16–32 px icon as a `data:` URL, or empty when the provider has
        /// none (macOS services) and the menu draws its own glyph.
        #[qinvokable]
        #[cxx_name = "iconAt"]
        fn icon_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "disabledAt"]
        fn disabled_at(&self, row: i32) -> bool;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut ShellCommandModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut ShellCommandModel>);
    }

    impl cxx_qt::Threading for ShellCommandModel {}
}

#[derive(Default)]
pub struct ShellCommandModelRust {
    rows: Vec<ShellCommand>,
    count: i32,
    revision: i32,
    loading: bool,
    error: QString,
    /// Bumped by every `load`/`clear`, so the slower of two overlapping
    /// discoveries cannot overwrite the newer answer.
    generation: u64,
}

impl qobject::ShellCommandModel {
    fn load(mut self: Pin<&mut Self>, paths: &QStringList, primary: &QString) {
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;

        let primary = primary.to_string();
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        if primary.is_empty() {
            self.as_mut().clear();
            return;
        }

        self.as_mut().set_loading(true);
        self.as_mut().set_error(QString::default());

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = shell_commands::list_shell_commands(paths, primary).await;
            let _ = thread.queue(move |mut model| model.as_mut().apply(generation, result));
        });
    }

    fn apply(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<Vec<ShellCommand>, dae_core::error::FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }
        match result {
            Ok(rows) => {
                self.as_mut().replace_rows(rows);
                self.as_mut().set_error(QString::default());
            }
            Err(error) => {
                // A command list that cannot be read is not worth a toast: the
                // menu simply has no extensions section, and the error stays
                // here for a future diagnostic view.
                self.as_mut().replace_rows(Vec::new());
                self.as_mut()
                    .set_error(QString::from(error.to_string().as_str()));
            }
        }
        self.as_mut().set_loading(false);
    }

    fn clear(mut self: Pin<&mut Self>) {
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().replace_rows(Vec::new());
        self.as_mut().set_loading(false);
        self.as_mut().set_error(QString::default());
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<ShellCommand>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
        let count = self.rows.len().min(i32::MAX as usize) as i32;
        self.as_mut().set_count(count);
        let revision = self.revision.wrapping_add(1);
        self.as_mut().set_revision(revision);
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len().min(i32::MAX as usize) as i32
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role {
            ROLE_COMMAND_ID => QVariant::from(&QString::from(row.id.as_str())),
            ROLE_LABEL => QVariant::from(&QString::from(row.label.as_str())),
            ROLE_ICON => QVariant::from(&QString::from(row.icon_data_url.as_deref().unwrap_or(""))),
            ROLE_GROUP => QVariant::from(&QString::from(row.group.as_deref().unwrap_or(""))),
            ROLE_SEPARATOR_BEFORE => QVariant::from(&row.separator_before),
            ROLE_DISABLED => QVariant::from(&row.disabled),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_COMMAND_ID, QByteArray::from("commandId"));
        roles.insert(ROLE_LABEL, QByteArray::from("label"));
        roles.insert(ROLE_ICON, QByteArray::from("iconSource"));
        roles.insert(ROLE_GROUP, QByteArray::from("group"));
        roles.insert(ROLE_SEPARATOR_BEFORE, QByteArray::from("separatorBefore"));
        roles.insert(ROLE_DISABLED, QByteArray::from("disabled"));
        roles
    }

    fn id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|command| QString::from(command.id.as_str()))
            .unwrap_or_default()
    }

    fn separator_before_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|command| command.separator_before)
            .unwrap_or(false)
    }

    fn label_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|command| QString::from(command.label.as_str()))
            .unwrap_or_default()
    }

    fn group_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|command| QString::from(command.group.as_deref().unwrap_or("")))
            .unwrap_or_default()
    }

    fn icon_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|command| QString::from(command.icon_data_url.as_deref().unwrap_or("")))
            .unwrap_or_default()
    }

    fn disabled_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|command| command.disabled)
            .unwrap_or(false)
    }
}
