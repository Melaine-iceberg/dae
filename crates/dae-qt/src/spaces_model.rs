//! Spaces, shared by the sidebar, the overview and the command palette.
//!
//! `dae_core::fs::spaces` owns the file format and its seed data; this module
//! is the QML-facing list model plus the small command surface. The store is a
//! few kilobytes of JSON, so reads and writes happen on the Qt thread: a
//! created space must have an id to open in the same event, and the write is
//! cheaper than the round-trip a queued command would need.
//!
//! One singleton (`Spaces`) serves every window, the same way `ExplorerPrefs`
//! does. `revision` makes QML bindings that call [invokables](Self::name_of_id)
//! re-evaluate after a rename; `changed` is what the sidebar and the space view
//! listen to instead of polling.

use core::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};
use dae_core::error::FileSystemError;
use dae_core::fs::spaces::{self, Space};

const ROLE_ID: i32 = 0x0500;
const ROLE_NAME: i32 = 0x0501;
const ROLE_IS_PRESET: i32 = 0x0502;
const ROLE_ITEM_COUNT: i32 = 0x0503;

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
        /// The user's spaces, reachable from QML as `Spaces`.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        #[qproperty(i32, revision)]
        #[qproperty(QString, error)]
        type Spaces = super::SpacesRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Re-reads the store. Called when a view opens; mutations refresh
        /// themselves.
        #[qinvokable]
        fn refresh(self: Pin<&mut Spaces>);

        /// Creates a space and returns its id, or an empty string with
        /// `error` set. QML opens the returned id in the same gesture.
        #[qinvokable]
        #[cxx_name = "createSpace"]
        fn create_space(self: Pin<&mut Spaces>, name: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "renameSpace"]
        fn rename_space(self: Pin<&mut Spaces>, id: &QString, name: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "deleteSpace"]
        fn delete_space(self: Pin<&mut Spaces>, id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "addItem"]
        fn add_item(self: Pin<&mut Spaces>, id: &QString, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "removeItem"]
        fn remove_item(self: Pin<&mut Spaces>, id: &QString, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "nameOfId"]
        fn name_of_id(&self, id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "isPresetOfId"]
        fn is_preset_of_id(&self, id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "isPresetAt"]
        fn is_preset_at(&self, row: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "itemCountAt"]
        fn item_count_at(&self, row: i32) -> i32;

        /// The pinned paths of `id`, in insertion order. The space view pairs
        /// these with `itemNames` to build its small JS list.
        #[qinvokable]
        #[cxx_name = "itemPaths"]
        fn item_paths(&self, id: &QString) -> QStringList;

        #[qinvokable]
        #[cxx_name = "itemNames"]
        fn item_names(&self, id: &QString) -> QStringList;

        /// Emitted after a successful mutation. Renames and item edits do not
        /// change the row count, so a reset alone is not enough for the views.
        #[qsignal]
        fn changed(self: Pin<&mut Spaces>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut Spaces>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut Spaces>);
    }
}

struct SpaceRow {
    id: String,
    name: String,
    is_preset: bool,
    item_count: i32,
    items: Vec<(String, String)>,
}

impl SpaceRow {
    fn from_space(space: &Space) -> Self {
        Self {
            id: space.id.clone(),
            name: space.name.clone(),
            is_preset: space.is_preset,
            item_count: space.items.len() as i32,
            items: space
                .items
                .iter()
                .map(|item| (item.path.clone(), item.name.clone()))
                .collect(),
        }
    }
}

pub struct SpacesRust {
    rows: Vec<SpaceRow>,
    count: i32,
    revision: i32,
    error: QString,
}

impl Default for SpacesRust {
    /// Loads the store at construction so QML bindings that read
    /// [name_of_id](qobject::Spaces::name_of_id) or `count` are usable before
    /// the first `refresh()`. The document is tiny and `list_spaces` seeds the
    /// preset rows on first use, so this has no async counterpart.
    fn default() -> Self {
        let mut model = Self {
            rows: Vec::new(),
            count: 0,
            revision: 1,
            error: QString::default(),
        };
        match spaces::list_spaces() {
            Ok(spaces) => {
                model.rows = spaces.iter().map(SpaceRow::from_space).collect();
                model.count = model.rows.len() as i32;
            }
            Err(error) => {
                model.error = QString::from(friendly(&error).as_str());
            }
        }
        model
    }
}

impl qobject::Spaces {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            self.rows.len() as i32
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if index.column() != 0 || index.parent().is_valid() {
            return QVariant::default();
        }
        let Some(row) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role {
            ROLE_ID => QVariant::from(&QString::from(row.id.as_str())),
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_IS_PRESET => QVariant::from(&row.is_preset),
            ROLE_ITEM_COUNT => QVariant::from(&row.item_count),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_ID, QByteArray::from("spaceId"));
        roles.insert(ROLE_NAME, QByteArray::from("name"));
        roles.insert(ROLE_IS_PRESET, QByteArray::from("isPreset"));
        roles.insert(ROLE_ITEM_COUNT, QByteArray::from("itemCount"));
        roles
    }

    fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().reload();
    }

    fn create_space(mut self: Pin<&mut Self>, name: &QString) -> QString {
        match spaces::create_space(name.to_string()) {
            Ok(space) => {
                self.as_mut().reload();
                self.as_mut().changed();
                QString::from(space.id.as_str())
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
                QString::default()
            }
        }
    }

    fn rename_space(mut self: Pin<&mut Self>, id: &QString, name: &QString) -> bool {
        match spaces::rename_space(id.to_string(), name.to_string()) {
            Ok(_) => {
                self.as_mut().reload();
                self.as_mut().changed();
                true
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
                false
            }
        }
    }

    fn delete_space(mut self: Pin<&mut Self>, id: &QString) -> bool {
        match spaces::delete_space(id.to_string()) {
            Ok(()) => {
                self.as_mut().reload();
                self.as_mut().changed();
                true
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
                false
            }
        }
    }

    fn add_item(mut self: Pin<&mut Self>, id: &QString, path: &QString) -> bool {
        match spaces::add_space_item(id.to_string(), path.to_string()) {
            Ok(_) => {
                self.as_mut().reload();
                self.as_mut().changed();
                true
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
                false
            }
        }
    }

    fn remove_item(mut self: Pin<&mut Self>, id: &QString, path: &QString) -> bool {
        match spaces::remove_space_item(id.to_string(), path.to_string()) {
            Ok(_) => {
                self.as_mut().reload();
                self.as_mut().changed();
                true
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
                false
            }
        }
    }

    fn name_of_id(&self, id: &QString) -> QString {
        self.rows
            .iter()
            .find(|row| row.id == id.to_string())
            .map(|row| QString::from(row.name.as_str()))
            .unwrap_or_default()
    }

    fn is_preset_of_id(&self, id: &QString) -> bool {
        self.rows
            .iter()
            .find(|row| row.id == id.to_string())
            .map(|row| row.is_preset)
            .unwrap_or(false)
    }

    fn id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.id.as_str()))
            .unwrap_or_default()
    }

    fn name_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.name.as_str()))
            .unwrap_or_default()
    }

    fn is_preset_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|row| row.is_preset)
            .unwrap_or(false)
    }

    fn item_count_at(&self, row: i32) -> i32 {
        self.rows
            .get(row as usize)
            .map(|row| row.item_count)
            .unwrap_or(0)
    }

    fn item_paths(&self, id: &QString) -> QStringList {
        let mut paths = QStringList::default();
        if let Some(row) = self.rows.iter().find(|row| row.id == id.to_string()) {
            for (path, _) in &row.items {
                paths.append(QString::from(path.as_str()));
            }
        }
        paths
    }

    fn item_names(&self, id: &QString) -> QStringList {
        let mut names = QStringList::default();
        if let Some(row) = self.rows.iter().find(|row| row.id == id.to_string()) {
            for (_, name) in &row.items {
                names.append(QString::from(name.as_str()));
            }
        }
        names
    }

    fn reload(mut self: Pin<&mut Self>) {
        match spaces::list_spaces() {
            Ok(spaces) => {
                let rows: Vec<SpaceRow> = spaces.iter().map(SpaceRow::from_space).collect();
                let count = rows.len() as i32;
                let revision = self.revision.wrapping_add(1);
                self.as_mut().replace_rows(rows);
                self.as_mut().set_count(count);
                self.as_mut().set_revision(revision);
                self.as_mut().set_error(QString::default());
            }
            Err(error) => {
                self.as_mut().replace_rows(Vec::new());
                self.as_mut().set_count(0);
                self.as_mut()
                    .set_error(QString::from(friendly(&error).as_str()));
            }
        }
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<SpaceRow>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
    }
}

/// The core errors are precise but speak the file-layer vocabulary. The shell
/// shows these in toasts, so only the two user-correctable cases are mapped;
/// everything else stays verbatim for the log and the report.
fn friendly(error: &FileSystemError) -> String {
    let message = error.to_string();
    if message.contains("Space name must not be empty") {
        "Space name cannot be empty".to_owned()
    } else if message.contains("Preset spaces cannot be deleted") {
        "Preset spaces cannot be deleted".to_owned()
    } else if message.contains("No space with id") {
        "Space not found".to_owned()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_core::fs::spaces::SpaceItem;

    fn space(id: &str, name: &str, preset: bool, items: usize) -> Space {
        Space {
            id: id.to_owned(),
            name: name.to_owned(),
            is_preset: preset,
            items: (0..items)
                .map(|n| SpaceItem {
                    path: format!("C:/item-{n}"),
                    name: format!("item-{n}"),
                    added_at: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn a_row_carries_the_count_and_the_paired_names() {
        let row = SpaceRow::from_space(&space("work", "工作", true, 2));
        assert_eq!(row.id, "work");
        assert_eq!(row.item_count, 2);
        assert!(row.is_preset);
        assert_eq!(row.items[1], ("C:/item-1".to_owned(), "item-1".to_owned()));
    }

    #[test]
    fn the_user_correctable_errors_are_localized() {
        let empty = FileSystemError::InvalidInput("Space name must not be empty".to_owned());
        assert_eq!(friendly(&empty), "Space name cannot be empty");

        let preset = FileSystemError::InvalidInput("Preset spaces cannot be deleted".to_owned());
        assert_eq!(friendly(&preset), "Preset spaces cannot be deleted");

        let missing = FileSystemError::NotFound("No space with id nope".to_owned());
        assert_eq!(friendly(&missing), "Space not found");
    }
}
