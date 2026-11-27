//! The rows behind the macOS/Linux "Open With" picker.
//!
//! `dae_core::fs::open_with` asks LaunchServices or the freedesktop
//! application registry what can open an item; this model is the Qt half. It
//! runs that query off the Qt thread, flattens the three answers (default,
//! recommended, other) into one list with section headers, and applies the
//! filter in Rust so headers never survive when their section is empty.
//!
//! On Windows the same model feeds the context menu's Explorer-style "Open
//! with" submenu (recommended handlers only); the picker dialog itself stays
//! macOS/Linux, because the native `SHOpenWithDialog` owns the association UI
//! on Windows.

use dae_core::error::FileSystemError;
use dae_core::fs::open_with::{self, OpenWithApp, OpenWithChoices};
use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QmlElement, QmlObject, qobject};

use crate::errors::friendly_error;
use crate::runtime;

const ROLE_KIND: i32 = 0x0400;
const ROLE_LABEL: i32 = 0x0401;
const ROLE_APP_ID: i32 = 0x0402;
const ROLE_ICON_NAME: i32 = 0x0403;
const ROLE_IS_DEFAULT: i32 = 0x0404;

const OTHER_APP_THRESHOLD: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Header,
    App,
}

struct OpenWithRow {
    kind: RowKind,
    label: String,
    app_id: String,
    icon_name: String,
    is_default: bool,
}

struct PreparedRows {
    rows: Vec<OpenWithRow>,
    suggested_app_id: String,
    needs_filter: bool,
    has_other: bool,
    total_count: i32,
}

macro_rules! notifying_setters {
    ($($setter:ident($ty:ty) => $field:ident, $notify:ident;)*) => {
        $(fn $setter(&mut self, value: $ty) {
            self.$field = value;
            self.$notify();
        })*
    };
}

#[qobject(Base = QAbstractItemModel, NoQmlElement, ConvertToCamelCase)]
mod rows {
    use super::*;

    /// The flattened picker model, reachable from QML as `OpenWithModel`.
    pub struct OpenWithModel {
        rows: Vec<OpenWithRow>,
        choices: OpenWithChoices,
        filter: String,
        path: String,
        loading: bool,
        loaded: bool,
        error: String,
        suggested_app_id: String,
        needs_filter: bool,
        has_other: bool,
        total_count: i32,
        empty: bool,
        /// The painted row count, headers included, for callers that read the
        /// model row by row instead of through a view (the context submenu).
        count: i32,
        generation: u64,
    }

    impl Default for OpenWithModel {
        fn default() -> Self {
            Self {
                rows: Vec::new(),
                choices: OpenWithChoices::default(),
                filter: String::new(),
                path: String::new(),
                loading: false,
                loaded: false,
                error: String::new(),
                suggested_app_id: String::new(),
                needs_filter: false,
                has_other: false,
                total_count: 0,
                empty: true,
                count: 0,
                generation: 0,
            }
        }
    }

    impl QAbstractItemModel for OpenWithModel {
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
                ROLE_KIND => QVariant::from(&QString::from(match row.kind {
                    RowKind::Header => "header",
                    RowKind::App => "app",
                })),
                ROLE_LABEL => QVariant::from(&QString::from(row.label.as_str())),
                ROLE_APP_ID => QVariant::from(&QString::from(row.app_id.as_str())),
                ROLE_ICON_NAME => QVariant::from(&QString::from(row.icon_name.as_str())),
                ROLE_IS_DEFAULT => QVariant::from(&row.is_default),
                _ => QVariant::default(),
            }
        }

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
            roles.insert(ROLE_KIND, QByteArray::from("kind"));
            roles.insert(ROLE_LABEL, QByteArray::from("label"));
            roles.insert(ROLE_APP_ID, QByteArray::from("appId"));
            roles.insert(ROLE_ICON_NAME, QByteArray::from("iconName"));
            roles.insert(ROLE_IS_DEFAULT, QByteArray::from("isDefault"));
            roles
        }

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }

    impl OpenWithModel {
        qproperty!("path", Member = path, Notify = path_changed);
        qproperty!("loading", Member = loading, Notify = loading_changed);
        qproperty!("loaded", Member = loaded, Notify = loaded_changed);
        qproperty!("error", Member = error, Notify = error_changed);
        qproperty!("suggestedAppId", Member = suggested_app_id, Notify = suggested_app_id_changed);
        qproperty!("needsFilter", Member = needs_filter, Notify = needs_filter_changed);
        qproperty!("hasOther", Member = has_other, Notify = has_other_changed);
        qproperty!("totalCount", Member = total_count, Notify = total_count_changed);
        qproperty!("empty", Member = empty, Notify = empty_changed);
        qproperty!("count", Member = count, Notify = count_changed);

        #[qsignal]
        fn path_changed(&mut self);
        #[qsignal]
        fn loading_changed(&mut self);
        #[qsignal]
        fn loaded_changed(&mut self);
        #[qsignal]
        fn error_changed(&mut self);
        #[qsignal]
        fn suggested_app_id_changed(&mut self);
        #[qsignal]
        fn needs_filter_changed(&mut self);
        #[qsignal]
        fn has_other_changed(&mut self);
        #[qsignal]
        fn total_count_changed(&mut self);
        #[qsignal]
        fn empty_changed(&mut self);
        #[qsignal]
        fn count_changed(&mut self);

        notifying_setters! {
            set_path(String) => path, path_changed;
            set_loading(bool) => loading, loading_changed;
            set_loaded(bool) => loaded, loaded_changed;
            set_error(String) => error, error_changed;
            set_suggested_app_id(String) => suggested_app_id, suggested_app_id_changed;
            set_needs_filter(bool) => needs_filter, needs_filter_changed;
            set_has_other(bool) => has_other, has_other_changed;
            set_total_count(i32) => total_count, total_count_changed;
            set_empty(bool) => empty, empty_changed;
            set_count(i32) => count, count_changed;
        }

        /// Asks the desktop what can open `path`. Resets the filter and any
        /// previous answer, and ignores a reply overtaken by a newer `load`.
        #[qslot]
        fn load(&mut self, path: String) {
            if path.trim().is_empty() {
                self.apply_error("No item to open".to_owned());
                return;
            }

            let generation = self.generation.wrapping_add(1);
            self.generation = generation;
            self.filter.clear();
            self.choices = OpenWithChoices::default();
            self.set_path(path.clone());
            self.set_loading(true);
            self.set_loaded(false);
            self.set_error(String::new());
            self.set_suggested_app_id(String::new());
            self.set_needs_filter(false);
            self.set_has_other(false);
            self.set_total_count(0);
            self.set_empty(false);
            self.replace_rows(Vec::new());

            let handle = self.get_thread_handle();
            runtime::spawn(async move {
                let result = open_with::list_open_with_apps(path).await;
                handle.invoke_mut(move |model| {
                    model.apply_result(generation, result);
                });
            });
        }

        /// Re-runs the query for the current path after a failure.
        #[qslot]
        fn retry(&mut self) {
            let path = self.path.clone();
            if path.is_empty() {
                return;
            }
            self.load(path);
        }

        /// The application id of `row`, or an empty string for a section
        /// header. QML keeps selection as an id so a reset or filter cannot
        /// silently point at a different application.
        #[qslot]
        fn app_id_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .filter(|row| row.kind == RowKind::App)
                .map(|row| row.app_id.clone())
                .unwrap_or_default()
        }

        /// The display name of `row`, for callers that read the model row by
        /// row instead of through a view (the context submenu).
        #[qslot]
        fn label_at(&self, row: i32) -> String {
            self.rows
                .get(row as usize)
                .map(|row| row.label.clone())
                .unwrap_or_default()
        }

        /// The `image://` URL of `row`'s icon, or empty when the entry names
        /// none. Windows keys icons on the handler's executable; the other
        /// shells on a launcher's icon name.
        #[qslot]
        fn icon_at(&self, row: i32) -> String {
            let Some(row) = self.rows.get(row as usize) else {
                return String::new();
            };
            if row.kind != RowKind::App || row.icon_name.is_empty() {
                return String::new();
            }
            if cfg!(target_os = "windows") {
                crate::icons::source_for(&row.icon_name, false, "")
            } else {
                crate::icons::named_source(&row.icon_name)
            }
        }

        /// Filters the loaded rows by display name. Headers are rebuilt, so an
        /// empty section does not leave its title behind.
        #[qslot]
        fn set_filter(&mut self, filter: String) {
            if self.filter == filter {
                return;
            }
            self.filter = filter.clone();
            let prepared = prepare_rows(&self.choices, &filter);
            self.apply_prepared(prepared);
        }

        fn apply_result(
            &mut self,
            generation: u64,
            result: Result<OpenWithChoices, FileSystemError>,
        ) {
            if self.generation != generation {
                return;
            }

            match result {
                Ok(choices) => {
                    self.set_error(String::new());
                    let prepared = prepare_rows(&choices, &self.filter);
                    self.choices = choices;
                    self.apply_prepared(prepared);
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.apply_error(message);
                    return;
                }
            }
            // Published last: `loadedChanged` handlers read the rows synchronously,
            // so the answer has to be in the model before the flag flips.
            self.set_loading(false);
            self.set_loaded(true);
        }

        fn apply_error(&mut self, message: String) {
            self.choices = OpenWithChoices::default();
            self.set_error(message);
            self.set_suggested_app_id(String::new());
            self.set_needs_filter(false);
            self.set_has_other(false);
            self.set_total_count(0);
            self.set_empty(true);
            self.replace_rows(Vec::new());
            self.set_loading(false);
            self.set_loaded(true);
        }

        fn apply_prepared(&mut self, prepared: PreparedRows) {
            self.set_suggested_app_id(prepared.suggested_app_id);
            self.set_needs_filter(prepared.needs_filter);
            self.set_has_other(prepared.has_other);
            self.set_total_count(prepared.total_count);
            self.set_empty(prepared.rows.is_empty());
            self.replace_rows(prepared.rows);
        }

        fn replace_rows(&mut self, rows: Vec<OpenWithRow>) {
            let count = rows.len().min(i32::MAX as usize) as i32;
            self.begin_reset_model();
            self.rows = rows;
            self.end_reset_model();
            self.set_count(count);
        }
    }
}

pub use rows::OpenWithModel;

impl QmlElement for OpenWithModel {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "OpenWithModel";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}

fn prepare_rows(choices: &OpenWithChoices, filter: &str) -> PreparedRows {
    let needle = filter.trim().to_lowercase();
    let matches =
        |app: &OpenWithApp| needle.is_empty() || app.name.to_lowercase().contains(&needle);
    let default_id = choices.default.as_ref().map(|app| app.id.as_str());

    let mut rows = Vec::new();
    if let Some(app) = choices.default.as_ref().filter(|app| matches(app)) {
        rows.push(OpenWithRow {
            kind: RowKind::Header,
            label: "Current default".to_owned(),
            app_id: String::new(),
            icon_name: String::new(),
            is_default: false,
        });
        rows.push(app_row(app, true));
    }

    let recommended: Vec<&OpenWithApp> = choices
        .recommended
        .iter()
        .filter(|app| matches(app))
        .collect();
    if !recommended.is_empty() {
        rows.push(OpenWithRow {
            kind: RowKind::Header,
            label: "Recommended apps".to_owned(),
            app_id: String::new(),
            icon_name: String::new(),
            is_default: false,
        });
        rows.extend(
            recommended
                .into_iter()
                .map(|app| app_row(app, Some(app.id.as_str()) == default_id)),
        );
    }

    let other: Vec<&OpenWithApp> = choices.other.iter().filter(|app| matches(app)).collect();
    if !other.is_empty() {
        rows.push(OpenWithRow {
            kind: RowKind::Header,
            label: "Other apps".to_owned(),
            app_id: String::new(),
            icon_name: String::new(),
            is_default: false,
        });
        rows.extend(
            other
                .into_iter()
                .map(|app| app_row(app, Some(app.id.as_str()) == default_id)),
        );
    }

    let suggested_app_id = choices
        .default
        .as_ref()
        .or_else(|| choices.recommended.first())
        .or_else(|| choices.other.first())
        .map(|app| app.id.clone())
        .unwrap_or_default();

    PreparedRows {
        rows,
        suggested_app_id,
        needs_filter: !choices.other.is_empty()
            && choices.recommended.len() + choices.other.len() > OTHER_APP_THRESHOLD,
        has_other: !choices.other.is_empty(),
        total_count: (choices.recommended.len() + choices.other.len()) as i32,
    }
}

fn app_row(app: &OpenWithApp, is_default: bool) -> OpenWithRow {
    OpenWithRow {
        kind: RowKind::App,
        label: app.name.clone(),
        app_id: app.id.clone(),
        icon_name: app.icon_name.clone().unwrap_or_default(),
        is_default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> OpenWithApp {
        OpenWithApp {
            id: id.to_owned(),
            name: name.to_owned(),
            icon_name: Some(format!("{id}-icon")),
        }
    }

    fn choices() -> OpenWithChoices {
        OpenWithChoices {
            default: Some(app("text-editor", "Text Editor")),
            recommended: vec![app("text-editor", "Text Editor"), app("vim", "Vim")],
            other: vec![app("gedit", "gedit"), app("code", "Code")],
        }
    }

    fn labels(prepared: &PreparedRows) -> Vec<&str> {
        prepared.rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn sections_put_the_current_default_first() {
        let prepared = prepare_rows(&choices(), "");
        assert_eq!(
            labels(&prepared),
            vec![
                "Current default",
                "Text Editor",
                "Recommended apps",
                "Text Editor",
                "Vim",
                "Other apps",
                "gedit",
                "Code",
            ]
        );
        assert!(prepared.rows[1].is_default);
        assert_eq!(prepared.suggested_app_id, "text-editor");
        assert_eq!(prepared.total_count, 4);
        assert!(prepared.has_other);
        assert!(!prepared.needs_filter);
    }

    #[test]
    fn the_default_is_marked_again_in_recommended() {
        let prepared = prepare_rows(&choices(), "");
        let defaults: Vec<bool> = prepared
            .rows
            .iter()
            .filter(|row| row.kind == RowKind::App)
            .map(|row| row.is_default)
            .collect();
        assert_eq!(defaults, vec![true, true, false, false, false]);
    }

    #[test]
    fn filtering_rebuilds_headers_instead_of_leaving_them_behind() {
        let prepared = prepare_rows(&choices(), "  co  ");
        assert_eq!(labels(&prepared), vec!["Other apps", "Code"]);
        assert_eq!(prepared.suggested_app_id, "text-editor");
        assert!(!prepared.rows.is_empty());
    }

    #[test]
    fn a_filter_that_matches_nothing_leaves_no_headers() {
        let prepared = prepare_rows(&choices(), "zzz");
        assert!(prepared.rows.is_empty());
        assert!(!prepared.needs_filter);
        assert!(prepared.has_other);
    }

    #[test]
    fn only_a_large_other_section_asks_for_the_filter_box() {
        let mut many = choices();
        many.other = (0..8)
            .map(|index| app(&format!("other-{index}"), &format!("Other {index}")))
            .collect();
        let prepared = prepare_rows(&many, "");
        assert!(prepared.needs_filter);
    }

    #[test]
    fn macos_style_choices_without_other_still_select_the_default() {
        let macos = OpenWithChoices {
            default: Some(app("com.apple.Preview", "Preview")),
            recommended: vec![app("com.apple.Preview", "Preview")],
            other: Vec::new(),
        };
        let prepared = prepare_rows(&macos, "");
        assert_eq!(prepared.suggested_app_id, "com.apple.Preview");
        assert!(!prepared.has_other);
        assert!(!prepared.needs_filter);
    }

    #[test]
    fn an_icon_name_is_carried_through_to_the_delegate() {
        let prepared = prepare_rows(&choices(), "");
        assert_eq!(prepared.rows[1].icon_name, "text-editor-icon");
        assert_eq!(prepared.rows[0].icon_name, "");
    }
}
