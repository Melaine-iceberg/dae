//! The rows behind the macOS/Linux "Open With" picker.
//!
//! `dae_core::fs::open_with` asks LaunchServices or the freedesktop
//! application registry what can open an item; this model is the Qt half. It
//! runs that query off the Qt thread, flattens the three answers (default,
//! recommended, other) into one list with section headers, and applies the
//! filter in Rust so headers never survive when their section is empty.
//!
//! Windows has no custom picker: `SHOpenWithDialog` already provides one and
//! also owns the association UI, so QML keeps the native command there. The
//! model still compiles on Windows and reports the core's unsupported error,
//! which keeps the dialog itself platform-neutral.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::error::FileSystemError;
use dae_core::fs::open_with::{self, OpenWithApp, OpenWithChoices};

use crate::errors::friendly_error;
use crate::runtime;

const ROLE_KIND: i32 = 0x0400;
const ROLE_LABEL: i32 = 0x0401;
const ROLE_APP_ID: i32 = 0x0402;
const ROLE_ICON_NAME: i32 = 0x0403;
const ROLE_IS_DEFAULT: i32 = 0x0404;

const OTHER_APP_THRESHOLD: usize = 8;

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
        /// The flattened picker model, reachable from QML as `OpenWithModel`.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(QString, path)]
        #[qproperty(bool, loading)]
        #[qproperty(bool, loaded)]
        #[qproperty(QString, error)]
        #[qproperty(QString, suggested_app_id, cxx_name = "suggestedAppId")]
        #[qproperty(bool, needs_filter, cxx_name = "needsFilter")]
        #[qproperty(bool, has_other, cxx_name = "hasOther")]
        #[qproperty(i32, total_count, cxx_name = "totalCount")]
        #[qproperty(bool, empty)]
        type OpenWithModel = super::OpenWithModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Asks the desktop what can open `path`. Resets the filter and any
        /// previous answer, and ignores a reply overtaken by a newer `load`.
        #[qinvokable]
        fn load(self: Pin<&mut OpenWithModel>, path: &QString);

        /// Filters the loaded rows by display name. Headers are rebuilt, so an
        /// empty section does not leave its title behind.
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut OpenWithModel>, text: &QString);

        /// Re-runs the query for the current path after a failure.
        #[qinvokable]
        fn retry(self: Pin<&mut OpenWithModel>);

        /// The application id of `row`, or an empty string for a section
        /// header. QML keeps selection as an id so a reset or filter cannot
        /// silently point at a different application.
        #[qinvokable]
        #[cxx_name = "appIdAt"]
        fn app_id_at(&self, row: i32) -> QString;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut OpenWithModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut OpenWithModel>);
    }

    impl cxx_qt::Threading for OpenWithModel {}
}

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

pub struct OpenWithModelRust {
    rows: Vec<OpenWithRow>,
    choices: OpenWithChoices,
    filter: String,
    path: QString,
    loading: bool,
    loaded: bool,
    error: QString,
    suggested_app_id: QString,
    needs_filter: bool,
    has_other: bool,
    total_count: i32,
    empty: bool,
    generation: u64,
}

impl Default for OpenWithModelRust {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            choices: OpenWithChoices::default(),
            filter: String::new(),
            path: QString::default(),
            loading: false,
            loaded: false,
            error: QString::default(),
            suggested_app_id: QString::default(),
            needs_filter: false,
            has_other: false,
            total_count: 0,
            empty: true,
            generation: 0,
        }
    }
}

impl qobject::OpenWithModel {
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

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_KIND, QByteArray::from("kind"));
        roles.insert(ROLE_LABEL, QByteArray::from("label"));
        roles.insert(ROLE_APP_ID, QByteArray::from("appId"));
        roles.insert(ROLE_ICON_NAME, QByteArray::from("iconName"));
        roles.insert(ROLE_IS_DEFAULT, QByteArray::from("isDefault"));
        roles
    }

    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if path.trim().is_empty() {
            self.as_mut().apply_error("No item to open".to_owned());
            return;
        }

        let generation = self.generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().rust_mut().filter.clear();
        self.as_mut().rust_mut().choices = OpenWithChoices::default();
        self.as_mut().set_path(QString::from(path.as_str()));
        self.as_mut().set_loading(true);
        self.as_mut().set_loaded(false);
        self.as_mut().set_error(QString::default());
        self.as_mut().set_suggested_app_id(QString::default());
        self.as_mut().set_needs_filter(false);
        self.as_mut().set_has_other(false);
        self.as_mut().set_total_count(0);
        self.as_mut().set_empty(false);
        self.as_mut().replace_rows(Vec::new());

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = open_with::list_open_with_apps(path).await;
            let _ = thread.queue(move |mut model| {
                model.as_mut().apply_result(generation, result);
            });
        });
    }

    fn retry(mut self: Pin<&mut Self>) {
        let path = self.path.clone();
        if path.is_empty() {
            return;
        }
        self.as_mut().load(&path);
    }

    fn app_id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .filter(|row| row.kind == RowKind::App)
            .map(|row| QString::from(row.app_id.as_str()))
            .unwrap_or_default()
    }

    fn set_filter(mut self: Pin<&mut Self>, text: &QString) {
        let filter = text.to_string();
        if self.filter == filter {
            return;
        }
        self.as_mut().rust_mut().filter = filter.clone();
        let prepared = prepare_rows(&self.choices, &filter);
        self.as_mut().apply_prepared(prepared);
    }

    fn apply_result(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<OpenWithChoices, FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }

        self.as_mut().set_loading(false);
        self.as_mut().set_loaded(true);

        match result {
            Ok(choices) => {
                self.as_mut().set_error(QString::default());
                let prepared = prepare_rows(&choices, &self.filter);
                self.as_mut().rust_mut().choices = choices;
                self.as_mut().apply_prepared(prepared);
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut().apply_error(message);
            }
        }
    }

    fn apply_error(mut self: Pin<&mut Self>, message: String) {
        self.as_mut().rust_mut().choices = OpenWithChoices::default();
        self.as_mut().set_loading(false);
        self.as_mut().set_loaded(true);
        self.as_mut().set_error(QString::from(message.as_str()));
        self.as_mut().set_suggested_app_id(QString::default());
        self.as_mut().set_needs_filter(false);
        self.as_mut().set_has_other(false);
        self.as_mut().set_total_count(0);
        self.as_mut().set_empty(true);
        self.as_mut().replace_rows(Vec::new());
    }

    fn apply_prepared(mut self: Pin<&mut Self>, prepared: PreparedRows) {
        self.as_mut()
            .set_suggested_app_id(QString::from(prepared.suggested_app_id.as_str()));
        self.as_mut().set_needs_filter(prepared.needs_filter);
        self.as_mut().set_has_other(prepared.has_other);
        self.as_mut().set_total_count(prepared.total_count);
        self.as_mut().set_empty(prepared.rows.is_empty());
        self.as_mut().replace_rows(prepared.rows);
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<OpenWithRow>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
    }
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
