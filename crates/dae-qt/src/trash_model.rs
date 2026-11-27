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

use dae_core::error::FileSystemError;
use dae_core::fs::trash_view::{self, TrashEntry};
use dae_core::types::EntryKind;
use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QmlElement, QmlObject, qobject};

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

#[qobject(Base = QAbstractItemModel, NoQmlElement, ConvertToCamelCase)]
mod rows {
    use super::*;

    #[derive(Default)]
    pub struct TrashModel {
        rows: Vec<TrashRow>,
        selection: Selection,
        count: i32,
        selected_count: i32,
        loaded: bool,
        status: String,
        error: String,
        generation: u64,
    }

    impl QAbstractItemModel for TrashModel {
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
                self.rows.len() as i32
            }
        }

        fn column_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() { 0 } else { 1 }
        }

        fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
            if !index.is_valid() || index.column() != 0 || index.row() < 0 {
                return QVariant::default();
            }
            let Some(row) = self.rows.get(index.row() as usize) else {
                return QVariant::default();
            };
            match role {
                ROLE_ENTRY_ID => QVariant::from(&QString::from(row.entry_id.as_str())),
                ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
                ROLE_ORIGINAL_PARENT => {
                    QVariant::from(&QString::from(row.original_parent.as_str()))
                }
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

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
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

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }

    impl TrashModel {
        qproperty!("count", Member = count, Notify = count_changed);
        qproperty!("loaded", Member = loaded, Notify = loaded_changed);
        qproperty!("selectedCount", Member = selected_count, Notify = selected_count_changed);
        qproperty!("status", Member = status, Notify = status_changed);
        qproperty!("error", Member = error, Notify = error_changed);

        #[qsignal]
        fn count_changed(&mut self);
        #[qsignal]
        fn loaded_changed(&mut self);
        #[qsignal]
        fn selected_count_changed(&mut self);
        #[qsignal]
        fn status_changed(&mut self);
        #[qsignal]
        fn error_changed(&mut self);

        /// Re-scans the system trash off the Qt thread.
        #[qslot]
        fn refresh(&mut self) {
            let generation = self.generation.wrapping_add(1);
            self.generation = generation;
            self.loaded = false;
            self.loaded_changed();
            self.error.clear();
            self.error_changed();
            self.status = "Loading Trash…".to_owned();
            self.status_changed();

            let handle = self.get_thread_handle();
            runtime::spawn(async move {
                let result = trash_view::list_trash().await;
                handle.invoke_mut(move |model| {
                    model.apply_result(generation, result);
                });
            });
        }

        /// The system trash id of `row`, or an empty string. Ids remain the
        /// command boundary: restore and purge address trash entries, not rows.
        #[qslot]
        fn id_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|entry| entry.entry_id.clone())
                .unwrap_or_default()
        }

        /// The preview pane reads one row at a time and cannot reach a
        /// delegate's `required` properties once that row has been recycled,
        /// so the fields it paints are asked for by row instead.
        #[qslot]
        fn name_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|entry| entry.name.clone())
                .unwrap_or_default()
        }

        #[qslot]
        fn original_parent_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|entry| entry.original_parent.clone())
                .unwrap_or_default()
        }

        #[qslot]
        fn deleted_text_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|entry| entry.deleted_text.clone())
                .unwrap_or_default()
        }

        #[qslot]
        fn size_text_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|entry| entry.size_text.clone())
                .unwrap_or_default()
        }

        #[qslot]
        fn is_dir_at(&self, row: i32) -> bool {
            self.rows
                .get(row as usize)
                .map(|entry| entry.is_directory)
                .unwrap_or(false)
        }

        #[qslot]
        fn is_selected_at(&self, row: i32) -> bool {
            self.rows
                .get(row as usize)
                .is_some_and(|entry| entry.selected)
        }

        /// Repaints only the flags that moved, widened by one row on each side so
        /// the selection block's neighbours update their outer corners as well.
        fn publish_selection(&mut self, runs: Vec<Run>) {
            let count = self.selection.count();
            if self.selected_count != count {
                self.selected_count = count;
                self.selected_count_changed();
            }
            let len = self.rows.len();
            if runs.is_empty() || len == 0 {
                return;
            }
            let parent = QModelIndex::default();
            for (first, last) in runs {
                let first = first.saturating_sub(1);
                let last = (last + 1).min(len - 1);
                let top = self.index(first as i32, 0, &parent);
                let bottom = self.index(last as i32, 0, &parent);
                self.data_changed(&top, &bottom);
            }
        }

        /// Selection uses the same row-based contract as the directory and
        /// search models, so a range never round-trips one id per row.
        #[qslot]
        fn select_only(&mut self, row: i32) {
            let row = usize::try_from(row).unwrap_or(usize::MAX);
            let runs = self.selection.select_only(&mut self.rows, row);
            self.publish_selection(runs);
        }

        #[qslot]
        fn toggle_at(&mut self, row: i32) {
            let Ok(row) = usize::try_from(row) else {
                return;
            };
            let runs = self.selection.toggle(&mut self.rows, row);
            self.publish_selection(runs);
        }

        #[qslot]
        fn select_range(&mut self, first: i32, last: i32) {
            let runs = self.selection.select_range(&mut self.rows, first, last);
            self.publish_selection(runs);
        }

        #[qslot]
        fn select_all(&mut self) {
            let runs = self.selection.select_all(&mut self.rows);
            self.publish_selection(runs);
        }

        #[qslot]
        fn clear_selection(&mut self) {
            let runs = self.selection.clear(&mut self.rows);
            self.publish_selection(runs);
        }

        /// The selected system trash ids, in display order.
        #[qslot]
        fn selected_ids(&self) -> Vec<String> {
            self.rows
                .iter()
                .filter(|row| row.selected)
                .map(|row| row.entry_id.clone())
                .collect()
        }

        fn apply_result(
            &mut self,
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
                    self.replace_rows(rows);
                    self.count = count;
                    self.count_changed();
                    self.status = status;
                    self.status_changed();
                }
                Err(error) => {
                    self.replace_rows(Vec::new());
                    self.count = 0;
                    self.count_changed();
                    self.status = "Could not read Trash".to_owned();
                    self.status_changed();
                    self.error = error.to_string();
                    self.error_changed();
                }
            }
            self.loaded = true;
            self.loaded_changed();
        }

        fn replace_rows(&mut self, rows: Vec<TrashRow>) {
            self.begin_reset_model();
            self.rows = rows;
            // A restored or purged id no longer exists. A refresh is a
            // new snapshot, so no selection is allowed to leak into it.
            self.selection.reset();
            self.end_reset_model();
            self.selected_count = 0;
            self.selected_count_changed();
        }
    }
}

pub use rows::TrashModel;

impl QmlElement for TrashModel {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "TrashModel";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
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
