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
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::search;
use dae_core::fs::vfs;

use crate::model::{category_of, extension_of, icon_version_of, thumbnail_for};
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
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
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

        #[qinvokable]
        fn search(self: Pin<&mut SearchModel>, path: &QString, query: &QString);

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

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut SearchModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut SearchModel>);
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
        }
    }
}

#[derive(Default)]
pub struct SearchModelRust {
    loaded: i32,
    busy: bool,
    finished: bool,
    truncated: bool,
    status: QString,
    query: QString,
    path: QString,
    rows: Vec<SearchRow>,
    generation: u64,
    reload_args: Option<(String, String)>,
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
        ])
    }

    fn search(mut self: Pin<&mut Self>, path: &QString, query: &QString) {
        let requested_path = path.to_string();
        let requested_query = query.to_string();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);

        {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = generation;
            rust.path = QString::from(requested_path.as_str());
            rust.query = QString::from(requested_query.as_str());
            rust.reload_args = Some((requested_path.clone(), requested_query.clone()));
        }

        self.as_mut().reset_rows();
        if requested_query.trim().is_empty() {
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
            let result = search::search_directory(requested_path, requested_query).await;
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
        let Some((path, query)) = self.reload_args.clone() else {
            return;
        };
        self.as_mut().search(
            &QString::from(path.as_str()),
            &QString::from(query.as_str()),
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
                    self.as_mut().rust_mut().rows = rows;
                    self.as_mut().end_reset_model();
                }
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
