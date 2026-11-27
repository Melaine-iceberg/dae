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

use dae_core::fs::shell_commands::{self, ShellCommand};
use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QmlElement, QmlObject, qobject};

use crate::runtime;

const ROLE_COMMAND_ID: i32 = 0x0400;
const ROLE_LABEL: i32 = 0x0401;
const ROLE_ICON: i32 = 0x0402;
const ROLE_GROUP: i32 = 0x0403;
const ROLE_SEPARATOR_BEFORE: i32 = 0x0404;
const ROLE_DISABLED: i32 = 0x0405;

#[qobject(Base = QAbstractItemModel, NoQmlElement, ConvertToCamelCase)]
mod rows {
    use super::*;

    /// The application-extensions section, reachable from QML as
    /// `ShellCommandModel`. One instance per window: a second window has
    /// its own right-click selection and its own list.
    #[derive(Default)]
    pub struct ShellCommandModel {
        rows: Vec<ShellCommand>,
        count: i32,
        /// Bumped on every row replacement. The menu watches this rather than
        /// `count`, because a fresh answer can carry the same number of rows
        /// with different labels or ids.
        revision: i32,
        loading: bool,
        error: String,
        /// Bumped by every `load`/`clear`, so the slower of two overlapping
        /// discoveries cannot overwrite the newer answer.
        generation: u64,
    }

    impl QAbstractItemModel for ShellCommandModel {
        fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
            if parent.is_valid() || column != 0 || row < 0 || row as usize >= self.rows.len() {
                return QModelIndex::default();
            }
            self.create_index(row, column, 0)
        }

        fn parent(&self, _child: &QModelIndex) -> QModelIndex {
            QModelIndex::default()
        }

        fn row_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() {
                0
            } else {
                self.rows.len().min(i32::MAX as usize) as i32
            }
        }

        fn column_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() { 0 } else { 1 }
        }

        fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
            let Some(row) = self.rows.get(index.row() as usize) else {
                return QVariant::default();
            };
            match role {
                ROLE_COMMAND_ID => QVariant::from(&QString::from(row.id.as_str())),
                ROLE_LABEL => QVariant::from(&QString::from(row.label.as_str())),
                ROLE_ICON => {
                    QVariant::from(&QString::from(row.icon_data_url.as_deref().unwrap_or("")))
                }
                ROLE_GROUP => QVariant::from(&QString::from(row.group.as_deref().unwrap_or(""))),
                ROLE_SEPARATOR_BEFORE => QVariant::from(&row.separator_before),
                ROLE_DISABLED => QVariant::from(&row.disabled),
                _ => QVariant::default(),
            }
        }

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
            roles.insert(ROLE_COMMAND_ID, QByteArray::from("commandId"));
            roles.insert(ROLE_LABEL, QByteArray::from("label"));
            roles.insert(ROLE_ICON, QByteArray::from("iconSource"));
            roles.insert(ROLE_GROUP, QByteArray::from("group"));
            roles.insert(ROLE_SEPARATOR_BEFORE, QByteArray::from("separatorBefore"));
            roles.insert(ROLE_DISABLED, QByteArray::from("disabled"));
            roles
        }

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }

    impl ShellCommandModel {
        qproperty!("count", Member = count, Notify = count_changed);
        qproperty!("revision", Member = revision, Notify = revision_changed);
        qproperty!("loading", Member = loading, Notify = loading_changed);
        qproperty!("error", Member = error, Notify = error_changed);

        #[qsignal]
        fn count_changed(&mut self);
        #[qsignal]
        fn revision_changed(&mut self);
        #[qsignal]
        fn loading_changed(&mut self);
        #[qsignal]
        fn error_changed(&mut self);

        /// Asks the platform what applies to this selection. `primary` is the
        /// right-clicked entry, which decides the file/folder menu; `paths` is
        /// what a command would run on. A remote selection answers with no rows
        /// at all, because none of the three mechanisms can address one.
        #[qslot]
        fn load(&mut self, paths: Vec<String>, primary: String) {
            let generation = self.generation.wrapping_add(1);
            self.generation = generation;

            if primary.is_empty() {
                self.clear();
                return;
            }

            self.loading = true;
            self.loading_changed();
            self.error.clear();
            self.error_changed();

            let handle = self.get_thread_handle();
            runtime::spawn(async move {
                let result = shell_commands::list_shell_commands(paths, primary).await;
                handle.invoke_mut(move |model| model.apply(generation, result));
            });
        }

        fn apply(
            &mut self,
            generation: u64,
            result: Result<Vec<ShellCommand>, dae_core::error::FileSystemError>,
        ) {
            if self.generation != generation {
                return;
            }
            match result {
                Ok(rows) => {
                    self.replace_rows(rows);
                    self.error.clear();
                    self.error_changed();
                }
                Err(error) => {
                    // A command list that cannot be read is not worth a toast: the
                    // menu simply has no extensions section, and the error stays
                    // here for a future diagnostic view.
                    self.replace_rows(Vec::new());
                    self.error = error.to_string();
                    self.error_changed();
                }
            }
            self.loading = false;
            self.loading_changed();
        }

        /// Empties the list without asking the platform. The menu calls it when
        /// it closes, so a stale answer cannot surface on the next right-click
        /// before the fresh one arrives.
        #[qslot]
        fn clear(&mut self) {
            self.generation = self.generation.wrapping_add(1);
            self.replace_rows(Vec::new());
            self.loading = false;
            self.loading_changed();
            self.error.clear();
            self.error_changed();
        }

        fn replace_rows(&mut self, rows: Vec<ShellCommand>) {
            self.begin_reset_model();
            self.rows = rows;
            self.end_reset_model();
            self.count = self.rows.len().min(i32::MAX as usize) as i32;
            self.count_changed();
            self.revision = self.revision.wrapping_add(1);
            self.revision_changed();
        }

        /// The command id of `row`, or an empty string. The menu builds its
        /// submenu from the rows, so it needs to turn a row back into the id
        /// `Workspace.invokeShellCommand` takes.
        #[qslot]
        fn id_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|command| command.id.clone())
                .unwrap_or_default()
        }

        /// Whether `row` wants a separator drawn above it.
        #[qslot]
        fn separator_before_at(&self, row: i32) -> bool {
            self.rows
                .get(row as usize)
                .map(|command| command.separator_before)
                .unwrap_or(false)
        }

        /// The menu text, exactly as the owning application worded it.
        #[qslot]
        fn label_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|command| command.label.clone())
                .unwrap_or_default()
        }

        /// The owning app's name when it contributed more than one command, so
        /// the menu can draw one header per app instead of repeating it.
        #[qslot]
        fn group_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .and_then(|command| command.group.clone())
                .unwrap_or_default()
        }

        /// A 16–32 px icon as a `data:` URL, or empty when the provider has
        /// none (macOS services) and the menu draws its own glyph.
        #[qslot]
        fn icon_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .and_then(|command| command.icon_data_url.clone())
                .unwrap_or_default()
        }

        #[qslot]
        fn disabled_at(&self, row: i32) -> bool {
            self.rows
                .get(row as usize)
                .map(|command| command.disabled)
                .unwrap_or(false)
        }
    }
}

pub use rows::ShellCommandModel;

impl QmlElement for ShellCommandModel {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "ShellCommandModel";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}
