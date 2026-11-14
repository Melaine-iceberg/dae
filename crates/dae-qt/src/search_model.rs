//! Search result models.
//!
//! `SearchModel` holds the ranked filename results from
//! `dae_core::fs::search::search_directory`; `ContentSearchModel` flattens the
//! grouped content-search response into header rows and match rows, because a
//! QML `ListView` has one delegate per row.
//!
//! Like the listing model, a new search bumps a generation and the result is
//! dropped if a newer one arrived first. The core's generation counter already
//! stops the walk; this one stops an answer that was queued on the Qt thread
//! before the query changed.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};
use dae_core::fs::search;
use dae_core::fs::vfs;

use crate::model::{category_of, extension_of, icon_version_of, thumbnail_for, type_label};
use crate::selection::{SelectableRow, Selection};
use dae_core::types::{
    ContentSearchFile, EntryKind, SearchEntry, SearchResponse, display_name_from_path,
};

use crate::format;
use crate::runtime;

const SEARCH_NAME: i32 = 0x0200;
const SEARCH_PATH: i32 = 0x0201;
const SEARCH_RELATIVE: i32 = 0x0202;
const SEARCH_SIZE: i32 = 0x0203;
const SEARCH_MODIFIED: i32 = 0x0204;
const SEARCH_IS_DIR: i32 = 0x0205;
const SEARCH_HIDDEN: i32 = 0x0206;
const SEARCH_READ_ONLY: i32 = 0x0207;
const SEARCH_ICON: i32 = 0x0208;
/// Search rows share `FileList.qml`'s delegate, which requires the Git role.
/// A result can sit several levels below the search root, so it never carries
/// a badge; the role exists to keep the delegate's contract uniform.
const SEARCH_GIT: i32 = 0x0209;
/// `FileList.qml`'s delegate requires these three as well, and a delegate with
/// one uninitialised required property is not created at all — which is how the
/// results list used to look: a status line counting 200 matches over an empty
/// body. They are the same values the listing model answers.
const SEARCH_CATEGORY: i32 = 0x020A;
const SEARCH_EXTENSION: i32 = 0x020B;
const SEARCH_THUMBNAIL: i32 = 0x020C;
/// Whether this result is selected. The listing delegate is shared, so the
/// search model answers the same role the listing model does.
const SEARCH_SELECTED: i32 = 0x020D;
/// The listing delegate is shared with the result list, and once the listing
/// grew tag, creation-time and section columns the delegate requires their
/// roles. A result carries no tags and no section, so these answer empty; the
/// delegate paints nothing for them.
const SEARCH_CREATED: i32 = 0x020E;
const SEARCH_CREATED_FALLBACK: i32 = 0x020F;
const SEARCH_TAGS: i32 = 0x0210;
const SEARCH_TAG_IDS: i32 = 0x0211;
const SEARCH_TAG_COLORS: i32 = 0x0212;
const SEARCH_GROUP: i32 = 0x0213;
/// Shared with the listing delegate (§7.1.6, §7.2): the type label, the raw
/// modification stamp the relative label is computed from, and the neighbour
/// flags that decide a selection block's outer corners.
const SEARCH_TYPE_NAME: i32 = 0x0214;
const SEARCH_MODIFIED_AT: i32 = 0x0215;
const SEARCH_SELECTED_PREV: i32 = 0x0216;
const SEARCH_SELECTED_NEXT: i32 = 0x0217;
/// Search rows carry no creation stamp; the role exists so the shared
/// delegate's required property can be created.
const SEARCH_CREATED_AT: i32 = 0x0218;

const CONTENT_IS_HEADER: i32 = 0x0300;
const CONTENT_NAME: i32 = 0x0301;
const CONTENT_PATH: i32 = 0x0302;
const CONTENT_RELATIVE: i32 = 0x0303;
const CONTENT_LINE_NUMBER: i32 = 0x0304;
const CONTENT_LINE_TEXT: i32 = 0x0305;
const CONTENT_LINE_HTML: i32 = 0x0306;
const CONTENT_MATCH_COUNT: i32 = 0x0307;

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
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
        /// Results currently selected. The shared delegate reads the
        /// `selected` role; this is what the status bar counts.
        #[qproperty(i32, selected_count, cxx_name = "selectedCount")]
        #[qproperty(bool, busy)]
        #[qproperty(bool, finished)]
        #[qproperty(bool, truncated)]
        #[qproperty(QString, status)]
        #[qproperty(QString, query)]
        #[qproperty(QString, path)]
        type SearchModel = super::SearchModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Searches `query` under `scope` (`current`, `favorites`, `recent`
        /// or `volumes`). The other three narrow by the same buckets the
        /// listing filter offers; `query` may also carry `tag:` tokens.
        #[qinvokable]
        fn search(
            self: Pin<&mut SearchModel>,
            path: &QString,
            query: &QString,
            scope: &QString,
            kind: &QString,
            modified: &QString,
            size: &QString,
        );

        #[qinvokable]
        fn cancel(self: Pin<&mut SearchModel>);

        #[qinvokable]
        fn reload(self: Pin<&mut SearchModel>);

        #[qinvokable]
        #[cxx_name = "indexOfPath"]
        fn index_of_path(&self, path: &QString) -> i32;

        #[qinvokable]
        #[cxx_name = "pathAt"]
        fn path_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "relativePathAt"]
        fn relative_path_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "isDirectoryAt"]
        fn is_directory_at(&self, row: i32) -> bool;

        #[qinvokable]
        fn dispose(self: Pin<&mut SearchModel>);

        /// Selection, in the same shape the listing model offers: the shared
        /// delegate reads the `selected` role, and a result list is clicked,
        /// Shift-clicked, Ctrl-clicked and cleared exactly like a listing.
        #[qinvokable]
        #[cxx_name = "selectOnly"]
        fn select_only(self: Pin<&mut SearchModel>, row: i32);

        #[qinvokable]
        #[cxx_name = "toggleAt"]
        fn toggle_at(self: Pin<&mut SearchModel>, row: i32);

        #[qinvokable]
        #[cxx_name = "selectRange"]
        fn select_range(self: Pin<&mut SearchModel>, first: i32, last: i32);

        #[qinvokable]
        #[cxx_name = "selectAll"]
        fn select_all_rows(self: Pin<&mut SearchModel>);

        #[qinvokable]
        #[cxx_name = "clearSelection"]
        fn clear_selection(self: Pin<&mut SearchModel>);

        #[qinvokable]
        #[cxx_name = "selectedPaths"]
        fn selected_paths(&self) -> QStringList;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut SearchModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut SearchModel>);

        #[inherit]
        #[rust_name = "model_index"]
        unsafe fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[rust_name = "data_changed"]
        unsafe fn dataChanged(
            self: Pin<&mut SearchModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, finished)]
        #[qproperty(bool, truncated)]
        #[qproperty(i32, file_count, cxx_name = "fileCount")]
        #[qproperty(i32, match_count, cxx_name = "matchCount")]
        #[qproperty(QString, status)]
        #[qproperty(QString, query)]
        #[qproperty(QString, path)]
        type ContentSearchModel = super::ContentSearchModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[qinvokable]
        fn search(
            self: Pin<&mut ContentSearchModel>,
            path: &QString,
            query: &QString,
            is_regex: bool,
            case_sensitive: bool,
            file_filter: &QString,
        );

        #[qinvokable]
        fn cancel(self: Pin<&mut ContentSearchModel>);

        #[qinvokable]
        fn reload(self: Pin<&mut ContentSearchModel>);

        #[qinvokable]
        #[cxx_name = "pathAt"]
        fn path_at(&self, row: i32) -> QString;

        /// The directory that contains `row`'s file, for the reveal button.
        #[qinvokable]
        #[cxx_name = "directoryAt"]
        fn directory_at(&self, row: i32) -> QString;

        #[qinvokable]
        fn dispose(self: Pin<&mut ContentSearchModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut ContentSearchModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut ContentSearchModel>);
    }

    impl cxx_qt::Threading for SearchModel {}
    impl cxx_qt::Threading for ContentSearchModel {}
}

struct SearchRow {
    name: String,
    path: String,
    relative_path: String,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    hidden: bool,
    read_only: bool,
    selected: bool,
}

impl SelectableRow for SearchRow {
    fn is_selected(&self) -> bool {
        self.selected
    }

    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    fn selection_key(&self) -> &str {
        // A result can sit anywhere below the search root, so its identity is
        // the whole path.
        &self.path
    }
}

impl SearchRow {
    fn from_entry(entry: SearchEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            relative_path: entry.relative_path,
            kind: entry.kind,
            size: entry.size,
            modified_at: entry.modified_at,
            hidden: entry.hidden,
            read_only: entry.read_only,
            selected: false,
        }
    }
}

/// One name-search request, kept so `reload()` re-runs exactly what was shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SearchArgs {
    path: String,
    query: String,
    scope: String,
    kind: String,
    modified: String,
    size: String,
}

#[derive(Default)]
pub struct SearchModelRust {
    loaded: i32,
    selected_count: i32,
    busy: bool,
    finished: bool,
    truncated: bool,
    status: QString,
    query: QString,
    path: QString,
    rows: Vec<SearchRow>,
    selection: Selection,
    generation: u64,
    /// Everything needed to re-run the current query after the files changed.
    reload_args: Option<SearchArgs>,
}

impl qobject::SearchModel {
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
            SEARCH_NAME => QVariant::from(&QString::from(row.name.as_str())),
            SEARCH_PATH => QVariant::from(&QString::from(row.path.as_str())),
            SEARCH_RELATIVE => QVariant::from(&QString::from(row.relative_path.as_str())),
            SEARCH_SIZE => {
                let text = if row.kind == EntryKind::Directory {
                    "—".to_owned()
                } else {
                    format::size(row.size.unwrap_or(0))
                };
                QVariant::from(&QString::from(text))
            }
            SEARCH_MODIFIED => QVariant::from(&QString::from(format::timestamp(
                row.modified_at.unwrap_or(0),
            ))),
            SEARCH_IS_DIR => QVariant::from(&(row.kind == EntryKind::Directory)),
            SEARCH_HIDDEN => QVariant::from(&row.hidden),
            SEARCH_READ_ONLY => QVariant::from(&row.read_only),
            SEARCH_GIT => QVariant::from(&QString::default()),
            SEARCH_SELECTED => QVariant::from(&row.selected),
            SEARCH_CATEGORY => QVariant::from(&QString::from(category_of(row.kind, &row.name))),
            SEARCH_EXTENSION => QVariant::from(&QString::from(extension_of(&row.name))),
            SEARCH_THUMBNAIL => {
                // A result two levels below the root is still a local file, so
                // the thumbnail provider can answer for it exactly as it does
                // for a listing row.
                let remote = !vfs::is_local_path(&row.path);
                if remote {
                    QVariant::from(&QString::default())
                } else {
                    let version = icon_version_of(row.modified_at, row.size);
                    QVariant::from(&QString::from(
                        thumbnail_for(&row.path, &row.name, row.kind, &version).as_str(),
                    ))
                }
            }
            SEARCH_ICON => {
                let version = format!("{}-{}", row.modified_at.unwrap_or(0), row.size.unwrap_or(0));
                // Network rows only carry a usable extension in their name;
                // passing the full remote path to the OS icon lookup would
                // never match anything. Directories have no fallback icon name
                // that the provider can resolve, so they stay URL-less.
                let remote = !vfs::is_local_path(&row.path);
                if remote && row.kind == EntryKind::Directory {
                    return QVariant::from(&QString::default());
                }
                let icon_path = if remote {
                    row.name.as_str()
                } else {
                    row.path.as_str()
                };
                QVariant::from(&QString::from(
                    crate::icons::source_for(icon_path, row.kind == EntryKind::Directory, &version)
                        .as_str(),
                ))
            }
            SEARCH_CREATED => QVariant::from(&QString::default()),
            SEARCH_CREATED_FALLBACK => QVariant::from(&false),
            SEARCH_TAGS | SEARCH_TAG_IDS | SEARCH_TAG_COLORS | SEARCH_GROUP => {
                QVariant::from(&QString::default())
            }
            SEARCH_TYPE_NAME => {
                QVariant::from(&QString::from(type_label(row.kind, &row.name)))
            }
            SEARCH_MODIFIED_AT => QVariant::from(&(row.modified_at.unwrap_or(0) as f64)),
            SEARCH_CREATED_AT => QVariant::from(&0.0),
            SEARCH_SELECTED_PREV => {
                let previous = index.row() > 0
                    && self
                        .rows
                        .get(index.row() as usize - 1)
                        .is_some_and(|row| row.selected);
                QVariant::from(&previous)
            }
            SEARCH_SELECTED_NEXT => {
                let next = self
                    .rows
                    .get(index.row() as usize + 1)
                    .is_some_and(|row| row.selected);
                QVariant::from(&next)
            }
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        roles(&[
            (SEARCH_NAME, "fileName"),
            (SEARCH_PATH, "filePath"),
            (SEARCH_RELATIVE, "relativePath"),
            (SEARCH_SIZE, "sizeText"),
            (SEARCH_MODIFIED, "modifiedText"),
            (SEARCH_IS_DIR, "isDir"),
            (SEARCH_HIDDEN, "hidden"),
            (SEARCH_READ_ONLY, "readOnly"),
            (SEARCH_ICON, "iconSource"),
            (SEARCH_GIT, "gitStatus"),
            (SEARCH_CATEGORY, "category"),
            (SEARCH_EXTENSION, "extension"),
            (SEARCH_THUMBNAIL, "thumbnailSource"),
            (SEARCH_SELECTED, "selected"),
            (SEARCH_CREATED, "createdText"),
            (SEARCH_CREATED_FALLBACK, "createdFallback"),
            (SEARCH_TAGS, "tagNames"),
            (SEARCH_TAG_IDS, "tagIds"),
            (SEARCH_TAG_COLORS, "tagColors"),
            (SEARCH_GROUP, "groupLabel"),
            (SEARCH_TYPE_NAME, "typeName"),
            (SEARCH_MODIFIED_AT, "modifiedAt"),
            (SEARCH_SELECTED_PREV, "previousSelected"),
            (SEARCH_SELECTED_NEXT, "nextSelected"),
            (SEARCH_CREATED_AT, "createdAt"),
        ])
    }

    fn search(
        mut self: Pin<&mut Self>,
        path: &QString,
        query: &QString,
        scope: &QString,
        kind: &QString,
        modified: &QString,
        size: &QString,
    ) {
        let args = SearchArgs {
            path: path.to_string(),
            query: query.to_string(),
            scope: scope.to_string(),
            kind: kind.to_string(),
            modified: modified.to_string(),
            size: size.to_string(),
        };
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);

        {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = generation;
            rust.path = QString::from(args.path.as_str());
            rust.query = QString::from(args.query.as_str());
            rust.reload_args = Some(args.clone());
        }

        self.as_mut().reset_rows();
        if args.query.trim().is_empty() {
            search::cancel_search();
            self.as_mut().set_loaded(0);
            self.as_mut().set_busy(false);
            self.as_mut().set_finished(true);
            self.as_mut().set_truncated(false);
            self.as_mut().set_status(QString::default());
            return;
        }

        self.as_mut().set_busy(true);
        self.as_mut().set_finished(false);
        self.as_mut().set_truncated(false);
        self.as_mut().set_status(QString::from("Searching…"));

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let request = match build_request(&args) {
                Ok(request) => request,
                Err(message) => {
                    let _ = thread.queue(move |mut model| {
                        model.as_mut().set_busy(false);
                        model.as_mut().set_finished(true);
                        model.as_mut().set_status(QString::from(message.as_str()));
                    });
                    return;
                }
            };
            let result = search::search(request).await;
            let _ = thread.queue(move |mut model| model.as_mut().apply_search(generation, result));
        });
    }

    fn cancel(mut self: Pin<&mut Self>) {
        search::cancel_search();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().reset_rows();
        self.as_mut().rust_mut().reload_args = None;
        self.as_mut().set_loaded(0);
        self.as_mut().set_busy(false);
        self.as_mut().set_finished(true);
        self.as_mut().set_truncated(false);
        self.as_mut().set_status(QString::default());
    }

    fn reload(mut self: Pin<&mut Self>) {
        let Some(args) = self.reload_args.clone() else {
            return;
        };
        self.as_mut().search(
            &QString::from(args.path.as_str()),
            &QString::from(args.query.as_str()),
            &QString::from(args.scope.as_str()),
            &QString::from(args.kind.as_str()),
            &QString::from(args.modified.as_str()),
            &QString::from(args.size.as_str()),
        );
    }

    fn index_of_path(&self, path: &QString) -> i32 {
        let path = path.to_string();
        self.rows
            .iter()
            .position(|row| row.path == path)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.path.as_str()))
            .unwrap_or_default()
    }

    fn name_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.name.as_str()))
            .unwrap_or_default()
    }

    fn relative_path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.relative_path.as_str()))
            .unwrap_or_default()
    }

    fn is_directory_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|entry| entry.kind == EntryKind::Directory)
            .unwrap_or(false)
    }

    /// Republishes the selected count and repaints the spans whose flags moved.
    /// Same contract as the listing model's: one `dataChanged` per contiguous
    /// run, widened by one row on each side because the delegate merges a
    /// block's corners from the neighbour flags.
    fn publish_selection(mut self: Pin<&mut Self>, runs: Vec<crate::selection::Run>) {
        self.as_mut().publish_selection_count();
        let len = self.rows.len();
        if runs.is_empty() || len == 0 {
            return;
        }
        unsafe {
            let parent = QModelIndex::default();
            let mut roles = cxx_qt_lib::QList::<i32>::default();
            roles.append(SEARCH_SELECTED);
            roles.append(SEARCH_SELECTED_PREV);
            roles.append(SEARCH_SELECTED_NEXT);
            for (first, last) in runs {
                let first = first.saturating_sub(1);
                let last = (last + 1).min(len - 1);
                let top = self.as_mut().model_index(first as i32, 0, &parent);
                let bottom = self.as_mut().model_index(last as i32, 0, &parent);
                self.as_mut().data_changed(&top, &bottom, &roles);
            }
        }
    }

    fn publish_selection_count(mut self: Pin<&mut Self>) {
        let count = self.selection.count();
        if self.selected_count != count {
            self.as_mut().set_selected_count(count);
        }
    }

    fn select_only(mut self: Pin<&mut Self>, row: i32) {
        // A negative row is past the last one too: both clear the selection,
        // which is what a click on a result that is no longer there means.
        let row = usize::try_from(row).unwrap_or(usize::MAX);
        let runs = {
            let SearchModelRust {
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
            let SearchModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.toggle(rows, row)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_range(mut self: Pin<&mut Self>, first: i32, last: i32) {
        let runs = {
            let SearchModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_range(rows, first, last)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_all_rows(mut self: Pin<&mut Self>) {
        let runs = {
            let SearchModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_all(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn clear_selection(mut self: Pin<&mut Self>) {
        let runs = {
            let SearchModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.clear(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn selected_paths(&self) -> QStringList {
        let mut paths = QStringList::default();
        for row in self.rows.iter().filter(|row| row.selected) {
            paths.append(QString::from(row.path.as_str()));
        }
        paths
    }

    fn dispose(mut self: Pin<&mut Self>) {
        self.as_mut().cancel();
        self.as_mut().set_status(QString::from("Closed"));
    }

    fn reset_rows(mut self: Pin<&mut Self>) {
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let SearchModelRust {
                    rows, selection, ..
                } = &mut *self.as_mut().rust_mut();
                rows.clear();
                selection.reset();
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().publish_selection_count();
    }

    fn apply_search(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<SearchResponse, dae_core::FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }

        self.as_mut().set_busy(false);
        match result {
            Ok(response) => {
                let rows: Vec<SearchRow> = response
                    .entries
                    .into_iter()
                    .map(SearchRow::from_entry)
                    .collect();
                let count = rows.len() as i32;
                unsafe {
                    self.as_mut().begin_reset_model();
                    {
                        let SearchModelRust {
                            rows: stored,
                            selection,
                            ..
                        } = &mut *self.as_mut().rust_mut();
                        *stored = rows;
                        // A new result set is a new list of rows: the flags of
                        // the old one address results that are gone.
                        selection.reset();
                    }
                    self.as_mut().end_reset_model();
                }
                self.as_mut().publish_selection_count();
                self.as_mut().set_loaded(count);
                self.as_mut().set_truncated(response.truncated);
                let mut status = if count == 0 {
                    "No matches".to_owned()
                } else {
                    format!("Results: {count}")
                };
                if response.truncated {
                    status.push_str(" · Truncated");
                }
                self.as_mut().set_status(QString::from(status.as_str()));
            }
            Err(error) => {
                self.as_mut().set_status(QString::from(
                    friendly_search_error(&error.to_string()).as_str(),
                ));
                self.as_mut().set_truncated(false);
            }
        }
        self.as_mut().set_finished(true);
    }
}

/// Translates one request into the roots and bounds the core search takes.
/// Scopes name where to look; a scope that resolves to nothing is reported
/// instead of quietly searching the current directory.
fn build_request(args: &SearchArgs) -> Result<search::SearchRequest, String> {
    let mut roots: Vec<String> = match args.scope.as_str() {
        "favorites" => dae_core::fs::sidebar::load_favorites()
            .unwrap_or_default()
            .into_iter()
            .map(|favorite| favorite.path)
            .collect(),
        "recent" => dae_core::fs::recents::list_recents()
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                if item.kind == EntryKind::Directory {
                    item.path
                } else {
                    item.path
                        .rsplit_once(['/', '\\'])
                        .map(|(parent, _)| parent.to_owned())
                        .unwrap_or(item.path)
                }
            })
            .collect(),
        "volumes" => dae_core::fs::sidebar::volume_roots(),
        // `current` and anything a stale document spells fall back to the
        // directory on screen.
        _ => vec![args.path.clone()],
    };

    roots.retain(|root| !root.is_empty() && vfs::is_local_path(root));
    roots.sort_unstable();
    roots.dedup();
    if roots.is_empty() {
        return Err("没有可搜索的本地位置".to_owned());
    }

    let (size_min, size_max) = match args.size.as_str() {
        "small" => (Some(0), Some(1024 * 1024)),
        "medium" => (Some(1024 * 1024), Some(100 * 1024 * 1024)),
        "large" => (Some(100 * 1024 * 1024), None),
        _ => (None, None),
    };

    Ok(search::SearchRequest {
        text: args.query.clone(),
        roots,
        tags: Vec::new(),
        filters: search::SearchFilters {
            kind: args.kind.clone(),
            modified_after: modified_cutoff(&args.modified),
            size_min,
            size_max,
        },
    })
}

/// The same buckets the listing filter uses; "today" is local midnight rather
/// than a rolling 24 hours.
fn modified_cutoff(bucket: &str) -> Option<u64> {
    use chrono::TimeZone;
    let now = chrono::Local::now();
    let millis = match bucket {
        "today" => now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|midnight| chrono::Local.from_local_datetime(&midnight).earliest())
            .map(|midnight| midnight.timestamp_millis().max(0) as u64),
        "week" => Some((now - chrono::Duration::days(7)).timestamp_millis().max(0) as u64),
        "month" => Some((now - chrono::Duration::days(30)).timestamp_millis().max(0) as u64),
        _ => None,
    };
    millis
}

enum ContentRow {
    Header {
        name: String,
        path: String,
        relative_path: String,
        match_count: i32,
    },
    Match {
        name: String,
        path: String,
        relative_path: String,
        line_number: i32,
        line_text: String,
        line_html: String,
    },
}

impl ContentRow {
    fn path(&self) -> &str {
        match self {
            Self::Header { path, .. } | Self::Match { path, .. } => path,
        }
    }
}

#[derive(Default)]
pub struct ContentSearchModelRust {
    loaded: i32,
    busy: bool,
    finished: bool,
    truncated: bool,
    file_count: i32,
    match_count: i32,
    status: QString,
    query: QString,
    path: QString,
    rows: Vec<ContentRow>,
    generation: u64,
    reload_args: Option<(String, String, bool, bool, Option<String>)>,
}

impl qobject::ContentSearchModel {
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
        match (role, row) {
            (CONTENT_IS_HEADER, ContentRow::Header { .. }) => QVariant::from(&true),
            (CONTENT_IS_HEADER, ContentRow::Match { .. }) => QVariant::from(&false),
            (CONTENT_NAME, ContentRow::Header { name, .. } | ContentRow::Match { name, .. }) => {
                QVariant::from(&QString::from(name.as_str()))
            }
            (CONTENT_PATH, ContentRow::Header { path, .. } | ContentRow::Match { path, .. }) => {
                QVariant::from(&QString::from(path.as_str()))
            }
            (
                CONTENT_RELATIVE,
                ContentRow::Header { relative_path, .. } | ContentRow::Match { relative_path, .. },
            ) => QVariant::from(&QString::from(relative_path.as_str())),
            (
                CONTENT_LINE_NUMBER,
                ContentRow::Header { .. } | ContentRow::Match { line_number: 0, .. },
            ) => QVariant::from(&0),
            (CONTENT_LINE_NUMBER, ContentRow::Match { line_number, .. }) => {
                QVariant::from(line_number)
            }
            (CONTENT_LINE_TEXT, ContentRow::Match { line_text, .. }) => {
                QVariant::from(&QString::from(line_text.as_str()))
            }
            (CONTENT_LINE_TEXT, ContentRow::Header { .. }) => QVariant::from(&QString::default()),
            (CONTENT_LINE_HTML, ContentRow::Match { line_html, .. }) => {
                QVariant::from(&QString::from(line_html.as_str()))
            }
            (CONTENT_LINE_HTML, ContentRow::Header { .. }) => QVariant::from(&QString::default()),
            (CONTENT_MATCH_COUNT, ContentRow::Header { match_count, .. }) => {
                QVariant::from(match_count)
            }
            (CONTENT_MATCH_COUNT, ContentRow::Match { .. }) => QVariant::from(&0),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        roles(&[
            (CONTENT_IS_HEADER, "isHeader"),
            (CONTENT_NAME, "fileName"),
            (CONTENT_PATH, "filePath"),
            (CONTENT_RELATIVE, "relativePath"),
            (CONTENT_LINE_NUMBER, "lineNumber"),
            (CONTENT_LINE_TEXT, "lineText"),
            (CONTENT_LINE_HTML, "lineHtml"),
            (CONTENT_MATCH_COUNT, "matchCount"),
        ])
    }

    fn search(
        mut self: Pin<&mut Self>,
        path: &QString,
        query: &QString,
        is_regex: bool,
        case_sensitive: bool,
        file_filter: &QString,
    ) {
        let requested_path = path.to_string();
        let requested_query = query.to_string();
        let filter = file_filter.to_string();
        let filter = (!filter.trim().is_empty()).then_some(filter);
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);

        {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = generation;
            rust.path = QString::from(requested_path.as_str());
            rust.query = QString::from(requested_query.as_str());
            rust.reload_args = Some((
                requested_path.clone(),
                requested_query.clone(),
                is_regex,
                case_sensitive,
                filter.clone(),
            ));
        }

        self.as_mut().reset_rows();
        if requested_query.trim().is_empty() {
            search::cancel_search();
            self.as_mut().set_loaded(0);
            self.as_mut().set_busy(false);
            self.as_mut().set_finished(true);
            self.as_mut().set_truncated(false);
            self.as_mut().set_file_count(0);
            self.as_mut().set_match_count(0);
            self.as_mut().set_status(QString::default());
            return;
        }

        self.as_mut().set_busy(true);
        self.as_mut().set_finished(false);
        self.as_mut().set_truncated(false);
        self.as_mut().set_file_count(0);
        self.as_mut().set_match_count(0);
        self.as_mut()
            .set_status(QString::from("Searching file contents…"));

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = search::search_file_contents(
                requested_path,
                requested_query,
                is_regex,
                case_sensitive,
                filter,
            )
            .await;
            let _ = thread.queue(move |mut model| model.as_mut().apply_search(generation, result));
        });
    }

    fn cancel(mut self: Pin<&mut Self>) {
        search::cancel_search();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().reset_rows();
        self.as_mut().rust_mut().reload_args = None;
        self.as_mut().set_loaded(0);
        self.as_mut().set_busy(false);
        self.as_mut().set_finished(true);
        self.as_mut().set_truncated(false);
        self.as_mut().set_file_count(0);
        self.as_mut().set_match_count(0);
        self.as_mut().set_status(QString::default());
    }

    fn reload(mut self: Pin<&mut Self>) {
        let Some((path, query, is_regex, case_sensitive, filter)) = self.reload_args.clone() else {
            return;
        };
        let path = QString::from(path.as_str());
        let query = QString::from(query.as_str());
        let filter = filter
            .map(|value| QString::from(value.as_str()))
            .unwrap_or_default();
        self.as_mut()
            .search(&path, &query, is_regex, case_sensitive, &filter);
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.path()))
            .unwrap_or_default()
    }

    fn directory_at(&self, row: i32) -> QString {
        let Some(path) = self.rows.get(row as usize) else {
            return QString::default();
        };
        QString::from(parent_directory(path.path()).as_str())
    }

    fn dispose(mut self: Pin<&mut Self>) {
        self.as_mut().cancel();
        self.as_mut().set_status(QString::from("Closed"));
    }

    fn reset_rows(mut self: Pin<&mut Self>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows.clear();
            self.as_mut().end_reset_model();
        }
    }

    fn apply_search(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<dae_core::types::ContentSearchResponse, dae_core::FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }

        self.as_mut().set_busy(false);
        match result {
            Ok(response) => {
                let (rows, files, matches) = flatten_content(response.files);
                let count = rows.len() as i32;
                unsafe {
                    self.as_mut().begin_reset_model();
                    self.as_mut().rust_mut().rows = rows;
                    self.as_mut().end_reset_model();
                }
                self.as_mut().set_loaded(count);
                self.as_mut().set_file_count(files);
                self.as_mut().set_match_count(matches);
                self.as_mut().set_truncated(response.truncated);
                let mut status = format!("Files: {files} · Matches: {matches}");
                if files == 0 {
                    status = "No matching content".to_owned();
                } else if response.truncated {
                    status.push_str(" · Truncated");
                }
                self.as_mut().set_status(QString::from(status.as_str()));
            }
            Err(error) => {
                self.as_mut().set_status(QString::from(
                    friendly_search_error(&error.to_string()).as_str(),
                ));
                self.as_mut().set_truncated(false);
            }
        }
        self.as_mut().set_finished(true);
    }
}

fn roles(pairs: &[(i32, &str)]) -> QHash<QHashPair_i32_QByteArray> {
    let mut names = QHash::<QHashPair_i32_QByteArray>::default();
    for (role, name) in pairs {
        names.insert(*role, QByteArray::from(*name));
    }
    names
}

fn flatten_content(files: Vec<ContentSearchFile>) -> (Vec<ContentRow>, i32, i32) {
    let file_count = files.len() as i32;
    let mut match_count = 0i32;
    let mut rows = Vec::new();
    for file in files {
        let name = display_name_from_path(&file.path);
        let file_matches = file.matches.len() as i32;
        match_count += file_matches;
        rows.push(ContentRow::Header {
            name: name.clone(),
            path: file.path.clone(),
            relative_path: file.relative_path.clone(),
            match_count: file_matches,
        });
        for found in file.matches {
            rows.push(ContentRow::Match {
                name: name.clone(),
                path: file.path.clone(),
                relative_path: file.relative_path.clone(),
                line_number: found.line_number.min(i32::MAX as u64) as i32,
                line_html: highlighted_line(&found.line_text, &found.ranges),
                line_text: found.line_text,
            });
        }
    }
    (rows, file_count, match_count)
}

/// Escapes one line and wraps the backend's character ranges in a span.
///
/// The ranges are character offsets (see `content_search::build_match`), so
/// the line is walked by `char` rather than by byte or QML code unit.
fn highlighted_line(text: &str, ranges: &[(usize, usize)]) -> String {
    let mut html = String::with_capacity(text.len() + ranges.len() * 48);
    let mut in_match = false;
    let mut next = 0usize;
    for (index, character) in text.chars().enumerate() {
        while next < ranges.len() && index >= ranges[next].1 {
            if in_match {
                html.push_str("</span>");
                in_match = false;
            }
            next += 1;
        }
        if next < ranges.len() && index == ranges[next].0 && !in_match {
            // A solid fill with explicit ink reads the same on the dark and
            // light themes; Qt's rich-text parser does not honour rgba().
            html.push_str("<span style=\"background-color: #f2c14e; color: #1c1f24\">");
            in_match = true;
        }
        escape_html_char(character, &mut html);
    }
    if in_match {
        html.push_str("</span>");
    }
    html
}

fn escape_html_char(character: char, out: &mut String) {
    match character {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        _ => out.push(character),
    }
}

fn parent_directory(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(0) => path[..1].to_owned(),
        Some(index) => {
            let parent = &trimmed[..index];
            if parent.len() == 2 && parent.ends_with(':') {
                format!("{parent}\\")
            } else {
                parent.to_owned()
            }
        }
        None => String::new(),
    }
}

fn friendly_search_error(message: &str) -> String {
    if message.starts_with("fs.invalid_search_query") {
        return "Invalid search expression".to_owned();
    }
    if message.starts_with("fs.content_search_local_only") {
        return "Content search is only available in local directories".to_owned();
    }
    if message.starts_with("fs.search_local_only") {
        return "Search is only available in local directories".to_owned();
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_core::types::{ContentSearchMatch, ContentSearchResponse};

    #[test]
    fn ranges_become_spans_by_character_not_byte() {
        assert_eq!(
            highlighted_line("你好 hello", &[(3, 8)]),
            "你好 <span style=\"background-color: #f2c14e; color: #1c1f24\">hello</span>"
        );
    }

    #[test]
    fn match_text_is_escaped() {
        assert_eq!(
            highlighted_line("<a & b>", &[(0, 1)]),
            "<span style=\"background-color: #f2c14e; color: #1c1f24\">&lt;</span>a &amp; b&gt;"
        );
    }

    #[test]
    fn grouped_response_flattens_headers_and_matches() {
        let response = ContentSearchResponse {
            files: vec![ContentSearchFile {
                path: "C:/tree/src/main.rs".to_owned(),
                relative_path: "src/main.rs".to_owned(),
                matches: vec![
                    ContentSearchMatch {
                        line_number: 3,
                        line_text: "fn main() {}".to_owned(),
                        ranges: vec![(3, 7)],
                    },
                    ContentSearchMatch {
                        line_number: 9,
                        line_text: "main();".to_owned(),
                        ranges: vec![(0, 4)],
                    },
                ],
            }],
            truncated: false,
        };

        let (rows, files, matches) = flatten_content(response.files);
        assert_eq!((files, matches), (1, 2));
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], ContentRow::Header { match_count: 2, .. }));
        assert_eq!(rows[1].path(), "C:/tree/src/main.rs");
        assert_eq!(rows[2].path(), "C:/tree/src/main.rs");
    }

    #[test]
    fn local_only_search_errors_are_friendly() {
        assert_eq!(
            friendly_search_error("fs.search_local_only"),
            "Search is only available in local directories"
        );
        assert_eq!(
            friendly_search_error("fs.content_search_local_only"),
            "Content search is only available in local directories"
        );
    }

    #[test]
    fn parent_directory_keeps_drive_roots_spelled_as_roots() {
        assert_eq!(parent_directory("C:/tree/src/main.rs"), "C:/tree/src");
        assert_eq!(parent_directory("C:/main.rs"), "C:\\");
        assert_eq!(parent_directory("main.rs"), "");
    }
}
