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
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QVariant,
};
use dae_core::error::FileSystemError;
use dae_core::fs::trash_view::{self, TrashEntry};
use dae_core::types::EntryKind;

use crate::format;
use crate::runtime;
use crate::selection::{Run, SelectableRow, Selection};

const ROLE_ENTRY_ID: i32 = 0x0300;
const ROLE_NAME: i32 = 0x0301;
const ROLE_ORIGINAL_PARENT: i32 = 0x0302;
const ROLE_DELETED_TEXT: i32 = 0x0303;
const ROLE_SIZE_TEXT: i32 = 0x0304;
const ROLE_IS_DIR: i32 = 0x0305;
const ROLE_CATEGORY: i32 = 0x0306;
const ROLE_SELECTED: i32 = 0x0307;
const ROLE_SELECTED_PREV: i32 = 0x0308;
const ROLE_SELECTED_NEXT: i32 = 0x0309;

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

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/core/qlist/qlist_i32.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    unsafe extern "RustQt" {
        /// The recycle bin, reachable from QML as `TrashModel`.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        #[qproperty(bool, loaded)]
        /// Maintained by the model so the view never recounts a selection map.
        #[qproperty(i32, selected_count, cxx_name = "selectedCount")]
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

        /// The system trash id of `row`, or an empty string. Ids remain the
        /// command boundary: restore and purge address trash entries, not rows.
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(&self, row: i32) -> QString;

        /// The preview pane reads one row at a time and cannot reach a
        /// delegate's `required` properties once that row has been recycled,
        /// so the fields it paints are asked for by row instead.
        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "originalParentAt"]
        fn original_parent_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "deletedTextAt"]
        fn deleted_text_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "sizeTextAt"]
        fn size_text_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "isDirAt"]
        fn is_dir_at(&self, row: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "isSelectedAt"]
        fn is_selected_at(&self, row: i32) -> bool;

        /// Selection uses the same row-based contract as the directory and
        /// search models, so a range never round-trips one id per row.
        #[qinvokable]
        #[cxx_name = "selectOnly"]
        fn select_only(self: Pin<&mut TrashModel>, row: i32);

        #[qinvokable]
        #[cxx_name = "toggleAt"]
        fn toggle_at(self: Pin<&mut TrashModel>, row: i32);

        #[qinvokable]
        #[cxx_name = "selectRange"]
        fn select_range(self: Pin<&mut TrashModel>, first: i32, last: i32);

        #[qinvokable]
        #[cxx_name = "selectAll"]
        fn select_all_rows(self: Pin<&mut TrashModel>);

        #[qinvokable]
        #[cxx_name = "clearSelection"]
        fn clear_selection(self: Pin<&mut TrashModel>);

        /// The selected system trash ids, in display order.
        #[qinvokable]
        #[cxx_name = "selectedIds"]
        fn selected_ids(&self) -> QStringList;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut TrashModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut TrashModel>);

        #[inherit]
        #[rust_name = "model_index"]
        unsafe fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[rust_name = "data_changed"]
        unsafe fn dataChanged(
            self: Pin<&mut TrashModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
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
    category: String,
    selected: bool,
}

impl TrashRow {
    fn from_entry(entry: &TrashEntry) -> Self {
        let kind = if entry.is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
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
            category: crate::model::category_of(kind, &entry.name).to_owned(),
            selected: false,
        }
    }
}

impl SelectableRow for TrashRow {
    fn is_selected(&self) -> bool {
        self.selected
    }

    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    fn selection_key(&self) -> &str {
        &self.entry_id
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
    selection: Selection,
    count: i32,
    selected_count: i32,
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
            ROLE_CATEGORY => QVariant::from(&QString::from(row.category.as_str())),
            ROLE_SELECTED => QVariant::from(&row.selected),
            ROLE_SELECTED_PREV => {
                let selected = index.row() > 0
                    && self
                        .rows
                        .get(index.row() as usize - 1)
                        .is_some_and(|row| row.selected);
                QVariant::from(&selected)
            }
            ROLE_SELECTED_NEXT => {
                let selected = self
                    .rows
                    .get(index.row() as usize + 1)
                    .is_some_and(|row| row.selected);
                QVariant::from(&selected)
            }
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
        roles.insert(ROLE_CATEGORY, QByteArray::from("category"));
        roles.insert(ROLE_SELECTED, QByteArray::from("selected"));
        roles.insert(ROLE_SELECTED_PREV, QByteArray::from("rowAboveSelected"));
        roles.insert(ROLE_SELECTED_NEXT, QByteArray::from("rowBelowSelected"));
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

    fn name_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.name.as_str()))
            .unwrap_or_default()
    }

    fn original_parent_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.original_parent.as_str()))
            .unwrap_or_default()
    }

    fn deleted_text_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.deleted_text.as_str()))
            .unwrap_or_default()
    }

    fn size_text_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.size_text.as_str()))
            .unwrap_or_default()
    }

    fn is_dir_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|entry| entry.is_directory)
            .unwrap_or(false)
    }

    fn is_selected_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .is_some_and(|entry| entry.selected)
    }

    /// Repaints only the flags that moved, widened by one row on each side so
    /// the selection block's neighbours update their outer corners as well.
    fn publish_selection(mut self: Pin<&mut Self>, runs: Vec<Run>) {
        let count = self.selection.count();
        if self.selected_count != count {
            self.as_mut().set_selected_count(count);
        }
        let len = self.rows.len();
        if runs.is_empty() || len == 0 {
            return;
        }
        unsafe {
            let parent = QModelIndex::default();
            let mut roles = QList::<i32>::default();
            roles.append(ROLE_SELECTED);
            roles.append(ROLE_SELECTED_PREV);
            roles.append(ROLE_SELECTED_NEXT);
            for (first, last) in runs {
                let first = first.saturating_sub(1);
                let last = (last + 1).min(len - 1);
                let top = self.as_mut().model_index(first as i32, 0, &parent);
                let bottom = self.as_mut().model_index(last as i32, 0, &parent);
                self.as_mut().data_changed(&top, &bottom, &roles);
            }
        }
    }

    fn select_only(mut self: Pin<&mut Self>, row: i32) {
        let row = usize::try_from(row).unwrap_or(usize::MAX);
        let runs = {
            let TrashModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_only(rows, row)
        };
        self.as_mut().publish_selection(runs);
    }

    fn toggle_at(mut self: Pin<&mut Self>, row: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let runs = {
            let TrashModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.toggle(rows, row)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_range(mut self: Pin<&mut Self>, first: i32, last: i32) {
        let runs = {
            let TrashModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_range(rows, first, last)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_all_rows(mut self: Pin<&mut Self>) {
        let runs = {
            let TrashModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_all(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn clear_selection(mut self: Pin<&mut Self>) {
        let runs = {
            let TrashModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.clear(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn selected_ids(&self) -> QStringList {
        let mut ids = QStringList::default();
        for row in self.rows.iter().filter(|row| row.selected) {
            ids.append(QString::from(row.entry_id.as_str()));
        }
        ids
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
            {
                let TrashModelRust {
                    rows: stored,
                    selection,
                    ..
                } = &mut *self.as_mut().rust_mut();
                *stored = rows;
                // A restored or purged id no longer exists. A refresh is a
                // new snapshot, so no selection is allowed to leak into it.
                selection.reset();
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_selected_count(0);
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
        assert_eq!(file.size_text, "2.0 KiB");
    }

    #[test]
    fn the_summary_counts_entries_and_their_bytes() {
        assert_eq!(summary(0, 0), "Trash is empty");
        assert_eq!(summary(1, 0), "Items: 1");
        assert_eq!(summary(2, 2_048), "Items: 2 · 2.0 KiB");
    }

    #[test]
    fn selection_uses_trash_ids_so_equal_names_cannot_collide() {
        let rows = (0..3)
            .map(|index| {
                let mut item = entry(1, Some(1), false);
                item.id = format!("trash-{index}");
                item.name = "report.txt".to_owned();
                TrashRow::from_entry(&item)
            })
            .collect::<Vec<_>>();
        let mut rows = rows;
        let mut selection = Selection::default();

        // Two files deleted from different folders can share a display name.
        // Selecting the middle one must still address exactly its own id.
        selection.select_only(&mut rows, 1);
        assert_eq!(selection.count(), 1);
        assert_eq!(selection.selected_keys(&rows), vec!["trash-1".to_owned()]);
        assert!(rows[1].selected);
        assert!(!rows[0].selected && !rows[2].selected);

        // A Shift range is one model call and flags every row in it.
        selection.select_range(&mut rows, 0, 2);
        assert_eq!(selection.count(), 3);
        assert_eq!(
            selection.selected_keys(&rows),
            vec![
                "trash-0".to_owned(),
                "trash-1".to_owned(),
                "trash-2".to_owned()
            ]
        );
    }
}
