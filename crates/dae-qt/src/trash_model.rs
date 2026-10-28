//! The recycle bin's rows.
//!
//! `dae_core::fs::trash_view` lists the system trash; this module is the Qt
//! half: it runs the scan off the Qt thread, keeps one row per entry, and
//! exposes the strings a delegate paints. Restore, purge and empty are
//! commands rather than model methods and live on `Workspace`, because they
//! run as tracked operations with progress and a toast.
//!
//! A refresh bumps a generation. The scan is slow enough that a second refresh
//! (a tab switch, an operation finishing) can overtake the first, and the older
//! answer must not replace the newer list.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::error::FileSystemError;
use dae_core::fs::trash_view::{self, TrashEntry};

use crate::format;
use crate::runtime;

const ROLE_ENTRY_ID: i32 = 0x0300;
const ROLE_NAME: i32 = 0x0301;
const ROLE_ORIGINAL_PARENT: i32 = 0x0302;
const ROLE_DELETED_TEXT: i32 = 0x0303;
const ROLE_SIZE_TEXT: i32 = 0x0304;
const ROLE_IS_DIR: i32 = 0x0305;

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

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
    }

    unsafe extern "RustQt" {
        /// The recycle bin, reachable from QML as `TrashModel`.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        #[qproperty(bool, loaded)]
        #[qproperty(QString, status)]
        #[qproperty(QString, error)]
        type TrashModel = super::TrashModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Re-scans the system trash off the Qt thread.
        #[qinvokable]
        fn refresh(self: Pin<&mut TrashModel>);

        /// The system trash id of `row`, or an empty string. Selection is kept
        /// as ids in QML, so range selection needs the model to turn a row back
        /// into an id.
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(&self, row: i32) -> QString;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut TrashModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut TrashModel>);
    }

    impl cxx_qt::Threading for TrashModel {}
}

struct TrashRow {
    entry_id: String,
    name: String,
    original_parent: String,
    deleted_text: String,
    size_text: String,
    is_directory: bool,
}

impl TrashRow {
    fn from_entry(entry: &TrashEntry) -> Self {
        Self {
            entry_id: entry.id.clone(),
            name: entry.name.clone(),
            original_parent: entry.original_parent.clone(),
            deleted_text: deleted_text(entry.time_deleted),
            size_text: entry
                .size_bytes
                .map(format::size)
                .unwrap_or_else(|| "—".to_owned()),
            is_directory: entry.is_directory,
        }
    }
}

/// The deletion timestamp as the same `YYYY-MM-DD HH:MM` the listing uses.
/// Entries without a usable timestamp (some platforms cannot recover one)
/// show a dash rather than 1970.
fn deleted_text(time_deleted: i64) -> String {
    if time_deleted <= 0 {
        return "—".to_owned();
    }
    format::timestamp(time_deleted as u64 * 1000)
}

#[derive(Default)]
pub struct TrashModelRust {
    rows: Vec<TrashRow>,
    count: i32,
    loaded: bool,
    status: QString,
    error: QString,
    generation: u64,
}

impl qobject::TrashModel {
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
            ROLE_ENTRY_ID => QVariant::from(&QString::from(row.entry_id.as_str())),
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_ORIGINAL_PARENT => QVariant::from(&QString::from(row.original_parent.as_str())),
            ROLE_DELETED_TEXT => QVariant::from(&QString::from(row.deleted_text.as_str())),
            ROLE_SIZE_TEXT => QVariant::from(&QString::from(row.size_text.as_str())),
            ROLE_IS_DIR => QVariant::from(&row.is_directory),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_ENTRY_ID, QByteArray::from("entryId"));
        roles.insert(ROLE_NAME, QByteArray::from("fileName"));
        roles.insert(ROLE_ORIGINAL_PARENT, QByteArray::from("originalParent"));
        roles.insert(ROLE_DELETED_TEXT, QByteArray::from("deletedText"));
        roles.insert(ROLE_SIZE_TEXT, QByteArray::from("sizeText"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let generation = self.generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().set_loaded(false);
        self.as_mut().set_error(QString::default());
        self.as_mut().set_status(QString::from("Loading Trash…"));

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = trash_view::list_trash().await;
            let _ = thread.queue(move |mut model| {
                model.as_mut().apply_result(generation, result);
            });
        });
    }

    fn id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.entry_id.as_str()))
            .unwrap_or_default()
    }

    fn apply_result(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<Vec<TrashEntry>, FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }

        match result {
            Ok(entries) => {
                let total_bytes: u64 = entries.iter().filter_map(|entry| entry.size_bytes).sum();
                let rows: Vec<TrashRow> = entries.iter().map(TrashRow::from_entry).collect();
                let status = summary(rows.len(), total_bytes);
                let count = rows.len() as i32;
                self.as_mut().replace_rows(rows);
                self.as_mut().set_count(count);
                self.as_mut().set_status(QString::from(status.as_str()));
            }
            Err(error) => {
                self.as_mut().replace_rows(Vec::new());
                self.as_mut().set_count(0);
                self.as_mut()
                    .set_status(QString::from("Could not read Trash"));
                self.as_mut()
                    .set_error(QString::from(error.to_string().as_str()));
            }
        }
        self.as_mut().set_loaded(true);
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<TrashRow>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
    }
}

fn summary(count: usize, total_bytes: u64) -> String {
    if count == 0 {
        return "Trash is empty".to_owned();
    }
    if total_bytes > 0 {
        format!("Items: {count} · {}", format::size(total_bytes))
    } else {
        format!("Items: {count}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time_deleted: i64, size_bytes: Option<u64>, is_directory: bool) -> TrashEntry {
        TrashEntry {
            id: "id-1".to_owned(),
            name: "note.txt".to_owned(),
            original_parent: "C:/Users/me/Documents".to_owned(),
            time_deleted,
            is_directory,
            size_bytes,
        }
    }

    #[test]
    fn an_unknown_deletion_time_is_a_dash() {
        assert_eq!(deleted_text(0), "—");
        assert_eq!(deleted_text(-1), "—");
    }

    #[test]
    fn a_known_deletion_time_uses_the_listing_format() {
        // 2026-10-25T00:00:00Z
        assert_eq!(deleted_text(1_792_886_400), "2026-10-25 00:00");
    }

    #[test]
    fn a_directory_has_no_size_and_a_file_does() {
        let directory = TrashRow::from_entry(&entry(1, None, true));
        assert_eq!(directory.size_text, "—");
        assert!(directory.is_directory);

        let file = TrashRow::from_entry(&entry(1, Some(2_048), false));
        assert_eq!(file.size_text, "2.0 KB");
    }

    #[test]
    fn the_summary_counts_entries_and_their_bytes() {
        assert_eq!(summary(0, 0), "Trash is empty");
        assert_eq!(summary(1, 0), "Items: 1");
        assert_eq!(summary(2, 2_048), "Items: 2 · 2.0 KB");
    }
}
