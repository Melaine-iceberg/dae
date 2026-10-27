//! The directory listing model, productised from gate 1.
//!
//! `DirectoryListModel` is a real `QAbstractListModel` over the rows that
//! `dae_core::fs::listing` emits. The reader thread calls the core's
//! [`BatchTarget`] seam; this module is the shell half that queues each batch
//! onto the Qt thread and turns it into `beginInsertRows`.
//!
//! The spike had no navigation, so it also had no stale-batch problem. Here a
//! new `load()` bumps a generation and cancels the previous stream: a batch
//! from the old directory that was already queued is dropped instead of being
//! appended to the new one (which would show the wrong files under the right
//! path).
//!
//! The core also arms a watcher for the displayed directory, so a change made
//! outside the shell (Explorer, a terminal, another app) reaches this model as
//! a reload instead of waiting for F5.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::entry_codec::{self, ListingRow};
use dae_core::fs::listing::{self, Batch, BatchTarget};
use dae_core::fs::vfs::{self, Scheme};
use dae_core::fs::watch;
use dae_core::host::{Event, Host};
use dae_core::types::{DirectoryEntry, EntryKind};

const ROLE_NAME: i32 = 0x0100;
const ROLE_PATH: i32 = 0x0101;
const ROLE_SIZE: i32 = 0x0102;
const ROLE_MODIFIED: i32 = 0x0103;
const ROLE_IS_DIR: i32 = 0x0104;
const ROLE_HIDDEN: i32 = 0x0105;
const ROLE_READ_ONLY: i32 = 0x0106;
/// A listing row's relative path is its name; search rows carry the path below
/// the search root. Exposing it here lets one delegate paint both models.
const ROLE_RELATIVE: i32 = 0x0107;
/// The `image://fileicon/...` URL for this row's OS icon.
const ROLE_ICON: i32 = 0x0108;

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
        #[qproperty(bool, finished)]
        #[qproperty(QString, status)]
        #[qproperty(QString, path)]
        /// True when this model was loaded through a network backend. The
        /// shell reads it to keep local-only actions off `smb://`, `sftp://`
        /// and `cloud://` paths instead of letting them fail on click.
        #[qproperty(bool, remote)]
        type DirectoryListModel = super::DirectoryListModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[qinvokable]
        fn load(self: Pin<&mut DirectoryListModel>, path: &QString);

        #[qinvokable]
        fn reload(self: Pin<&mut DirectoryListModel>);

        /// Emitted before a load clears the rows. The view reads its scroll
        /// position here, because the reset that follows discards it.
        #[qsignal]
        #[cxx_name = "aboutToReload"]
        fn about_to_reload(self: Pin<&mut DirectoryListModel>);

        /// The row whose full path is `path`, or -1. QML uses this to keep a
        /// renamed or newly created entry selected after the reload.
        #[qinvokable]
        #[cxx_name = "indexOfPath"]
        fn index_of_path(&self, path: &QString) -> i32;

        /// The full path of `row`, or an empty string. Selection ranges are
        /// computed in QML from row indices, which only the model can turn
        /// back into paths.
        #[qinvokable]
        #[cxx_name = "pathAt"]
        fn path_at(&self, row: i32) -> QString;

        /// Whether `row` is a directory, for Enter/activation.
        #[qinvokable]
        #[cxx_name = "isDirectoryAt"]
        fn is_directory_at(&self, row: i32) -> bool;

        /// Stops the stream and drops every row. A closed tab calls this
        /// before it is released, so a 120k-row directory is not kept alive by
        /// a model nobody shows.
        #[qinvokable]
        fn dispose(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_insert_rows"]
        unsafe fn beginInsertRows(
            self: Pin<&mut DirectoryListModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_insert_rows"]
        unsafe fn endInsertRows(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_remove_rows"]
        unsafe fn beginRemoveRows(
            self: Pin<&mut DirectoryListModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_remove_rows"]
        unsafe fn endRemoveRows(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut DirectoryListModel>);
    }

    impl cxx_qt::Threading for DirectoryListModel {}
}

/// `PartialEq` is what lets a refresh keep the rows it already has: the diff
/// compares old and new entries field by field, so an unchanged row costs no
/// model signal at all.
#[derive(Clone, PartialEq)]
struct StoredRow {
    name: String,
    /// Kept only for the rare row that is not under the listing base. Rows that
    /// are (the local reader's normal shape) rebuild their path from the base,
    /// which is what keeps a 120k-row model cheap.
    path: Option<String>,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    hidden: bool,
    read_only: bool,
}

impl StoredRow {
    fn from_entry(entry: &DirectoryEntry, base: &str) -> Self {
        let path = if entry.path.starts_with(base) {
            None
        } else {
            Some(entry.path.clone())
        };
        Self {
            name: entry.name.clone(),
            path,
            kind: entry.kind,
            size: entry.size,
            modified_at: entry.modified_at,
            hidden: entry.hidden,
            read_only: entry.read_only,
        }
    }

    fn from_listing(row: ListingRow) -> Self {
        Self {
            name: row.name,
            path: row.path,
            kind: row.kind,
            size: row.size,
            modified_at: row.modified_at,
            hidden: row.hidden,
            read_only: row.read_only,
        }
    }

    fn full_path(&self, base: &str) -> String {
        match &self.path {
            Some(path) => path.clone(),
            None => format!("{base}{}", self.name),
        }
    }
}

/// Rows collected while the directory already on screen is re-read. The old
/// rows stay visible until this is complete, so the view never shows an empty
/// model between the two listings.
#[derive(Default)]
struct RefreshStage {
    rows: Vec<StoredRow>,
    base: String,
    path: String,
}

pub struct DirectoryListModelRust {
    loaded: i32,
    finished: bool,
    status: QString,
    path: QString,
    rows: Vec<StoredRow>,
    base: String,
    /// `Some` while a same-directory refresh runs; the rows above are the
    /// previous listing until the stage is swapped in.
    refresh: Option<RefreshStage>,
    stream_id: Option<String>,
    generation: u64,
    /// Rows came from a network backend. Their `name` is the only part that
    /// carries a file extension, so the icon URL is built from it rather than
    /// from the full path. Exposed to QML so local-only actions can be gated.
    remote: bool,
    /// This model's identity in the core watcher registry. One id for the
    /// model's whole life: navigation re-targets the observation rather than
    /// minting a new one, so a slow arm cannot leave a stale watcher behind.
    watcher_id: String,
    /// A change arrived while a read was in flight. Re-reading now would cancel
    /// that read, so the reload waits for it to finish instead.
    reload_pending: bool,
    /// `dispose` ran and released the watcher; a reload already queued on the
    /// Qt thread must not revive a closed model.
    closed: bool,
}

impl Default for DirectoryListModelRust {
    fn default() -> Self {
        Self {
            loaded: 0,
            finished: false,
            status: QString::default(),
            path: QString::default(),
            rows: Vec::new(),
            base: String::new(),
            refresh: None,
            stream_id: None,
            generation: 0,
            remote: false,
            watcher_id: next_watcher_id(),
            reload_pending: false,
            closed: false,
        }
    }
}

impl Drop for DirectoryListModelRust {
    fn drop(&mut self) {
        // `dispose` normally released this already; a model the shell let go
        // without disposing must not leave an OS watcher running.
        watch::unwatch_directory(self.watcher_id.clone());
    }
}

impl qobject::DirectoryListModel {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        // A list model has one level: any valid parent has no children.
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
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_PATH => QVariant::from(&QString::from(row.full_path(&self.base))),
            ROLE_RELATIVE => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_SIZE => QVariant::from(&QString::from(crate::format::size(row.size.unwrap_or(0)))),
            ROLE_MODIFIED => QVariant::from(&QString::from(crate::format::timestamp(
                row.modified_at.unwrap_or(0),
            ))),
            ROLE_IS_DIR => QVariant::from(&(row.kind == EntryKind::Directory)),
            ROLE_HIDDEN => QVariant::from(&row.hidden),
            ROLE_READ_ONLY => QVariant::from(&row.read_only),
            ROLE_ICON => {
                // The desktop icon lookup cannot answer for a remote
                // directory (a bare name is not a local path), so leave the
                // URL empty and let the delegate paint its fallback square
                // instead of asking the provider for an icon it cannot make.
                if self.remote && row.kind == EntryKind::Directory {
                    return QVariant::from(&QString::default());
                }
                let path = if self.remote {
                    row.name.clone()
                } else {
                    row.full_path(&self.base)
                };
                let version = icon_version(row);
                QVariant::from(&QString::from(
                    crate::icons::source_for(&path, row.kind == EntryKind::Directory, &version)
                        .as_str(),
                ))
            }
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_NAME, QByteArray::from("fileName"));
        roles.insert(ROLE_PATH, QByteArray::from("filePath"));
        roles.insert(ROLE_RELATIVE, QByteArray::from("relativePath"));
        roles.insert(ROLE_SIZE, QByteArray::from("sizeText"));
        roles.insert(ROLE_MODIFIED, QByteArray::from("modifiedText"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles.insert(ROLE_HIDDEN, QByteArray::from("hidden"));
        roles.insert(ROLE_READ_ONLY, QByteArray::from("readOnly"));
        roles.insert(ROLE_ICON, QByteArray::from("iconSource"));
        roles
    }

    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let requested = path.to_string();
        if requested.trim().is_empty() {
            self.as_mut().set_status(QString::from("无路径"));
            return;
        }

        // Reloading the directory already on screen keeps its rows until the
        // replacement has arrived; the reset is deferred to the swap so the
        // user never sees an empty list. Navigating elsewhere still clears at
        // once, or the old rows would appear under the new path.
        let refreshing = !self.path.as_slice().is_empty() && self.path.to_string() == requested;
        if !refreshing {
            // A view can only remember where it was scrolled before the reset
            // below; afterwards the rows — and the position with them — are gone.
            self.as_mut().about_to_reload();
        }

        let generation = self.generation.wrapping_add(1);
        if let Some(stream_id) = self.as_mut().rust_mut().stream_id.take() {
            listing::cancel_directory_listing(stream_id);
        }

        // A scheme prefix sends this load through the vfs backend instead of
        // the local streaming reader. Remote backends list in one piece.
        let scheme = vfs::scheme_of(&requested);
        let remote = matches!(scheme, Ok(scheme) if scheme != Scheme::Local);

        if refreshing {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = generation;
            rust.refresh = Some(RefreshStage::default());
            rust.reload_pending = false;
            rust.closed = false;
        } else {
            unsafe {
                self.as_mut().begin_reset_model();
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.generation = generation;
                    rust.rows.clear();
                    rust.base.clear();
                    rust.refresh = None;
                    rust.reload_pending = false;
                    rust.closed = false;
                }
                self.as_mut().end_reset_model();
            }
        }
        self.as_mut().set_remote(remote);

        self.as_mut().set_finished(false);
        if !refreshing {
            self.as_mut().set_loaded(0);
            self.as_mut().set_path(QString::from(requested.as_str()));
            self.as_mut().set_status(QString::from("读取中…"));
        }

        // An unknown scheme is a failure of this path, not of the previous
        // directory: the model has already adopted `requested` above.
        if let Err(error) = scheme {
            self.as_mut().rust_mut().refresh = None;
            let message = crate::errors::friendly_error(&error.to_string());
            self.as_mut().set_status(QString::from(message.as_str()));
            self.as_mut().set_finished(true);
            return;
        }

        let stream_id = next_stream_id();
        self.as_mut().rust_mut().stream_id = Some(stream_id.clone());

        let thread = self.as_mut().qt_thread();
        let target = Arc::new(QtBatchTarget::new(thread.clone(), generation));
        let watch_host: Arc<dyn Host> = Arc::new(ListingHost::new(thread));
        let worker = Arc::clone(&target);
        let requested_path = requested.clone();
        let watcher_id = self.watcher_id.clone();

        std::thread::spawn(move || {
            if remote {
                match vfs::resolve(&requested_path) {
                    Ok(backend) => {
                        // Armed before the read, like the local arm: a change
                        // landing during enumeration is either in the view or
                        // reported as an event.
                        watch::arm_polling_watcher(
                            Arc::clone(&watch_host),
                            &requested_path,
                            backend.clone(),
                            &watcher_id,
                        );
                        match backend.read_dir(&requested_path) {
                            Ok(view) => {
                                let base = entry_codec::base_of(&view.path);
                                let rows = view
                                    .entries
                                    .iter()
                                    .map(|entry| StoredRow::from_entry(entry, &base))
                                    .collect();
                                worker.deliver_head(base, view.path, rows, true);
                            }
                            Err(error) => {
                                worker.fail(crate::errors::friendly_error(&error.to_string()))
                            }
                        }
                    }
                    Err(error) => worker.fail(crate::errors::friendly_error(&error.to_string())),
                }
                return;
            }

            match listing::open_streamed_listing(
                watch_host,
                PathBuf::from(&requested_path),
                stream_id,
                Some(watcher_id),
                worker.clone(),
            ) {
                Ok(view) => {
                    let done = view.stream_id.is_none();
                    let base = entry_codec::base_of(&view.path);
                    let rows = view
                        .entries
                        .iter()
                        .map(|entry| StoredRow::from_entry(entry, &base))
                        .collect();
                    worker.deliver_head(base, view.path, rows, done);
                }
                Err(error) => worker.fail(crate::errors::friendly_error(&error.to_string())),
            }
        });
    }

    fn reload(mut self: Pin<&mut Self>) {
        let path = self.path.clone();
        if !path.as_slice().is_empty() {
            self.as_mut().load(&path);
        }
    }

    fn index_of_path(&self, path: &QString) -> i32 {
        let path = path.to_string();
        self.rows
            .iter()
            .position(|row| row.full_path(&self.base) == path)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.full_path(&self.base).as_str()))
            .unwrap_or_default()
    }

    fn is_directory_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|entry| entry.kind == EntryKind::Directory)
            .unwrap_or(false)
    }

    fn dispose(mut self: Pin<&mut Self>) {
        watch::unwatch_directory(self.watcher_id.clone());
        self.as_mut().rust_mut().closed = true;
        self.as_mut().rust_mut().reload_pending = false;
        self.as_mut().rust_mut().refresh = None;
        if let Some(stream_id) = self.as_mut().rust_mut().stream_id.take() {
            listing::cancel_directory_listing(stream_id);
        }
        // A batch already queued on the Qt thread carries the old generation
        // and is dropped instead of appending to a model its window let go.
        let generation = self.generation.wrapping_add(1);
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let mut rust = self.as_mut().rust_mut();
                rust.generation = generation;
                rust.rows.clear();
                rust.base.clear();
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_remote(false);
        self.as_mut().set_loaded(0);
        self.as_mut().set_status(QString::from("已关闭"));
        self.as_mut().set_finished(true);
    }

    fn append_head(
        mut self: Pin<&mut Self>,
        generation: u64,
        base: String,
        path: String,
        rows: Vec<StoredRow>,
        done: bool,
    ) {
        if self.generation != generation {
            return;
        }

        if self.refresh.is_some() {
            if let Some(stage) = self.as_mut().rust_mut().refresh.as_mut() {
                stage.base = base;
                stage.path = path;
                stage.rows.extend(rows);
            }
            if done {
                self.as_mut().finish_refresh(generation);
            }
            return;
        }

        self.as_mut().set_path(QString::from(path.as_str()));
        self.as_mut().rust_mut().base = base;
        self.as_mut().append_rows(generation, rows, done);
    }

    fn append_rows(mut self: Pin<&mut Self>, generation: u64, rows: Vec<StoredRow>, done: bool) {
        if self.generation != generation {
            return;
        }

        if self.refresh.is_some() {
            if let Some(stage) = self.as_mut().rust_mut().refresh.as_mut() {
                stage.rows.extend(rows);
            }
            if done {
                self.as_mut().finish_refresh(generation);
            }
            return;
        }

        let added = rows.len() as i32;
        let first = self.loaded;
        if added > 0 {
            let last = first + added - 1;
            unsafe {
                self.as_mut()
                    .begin_insert_rows(&QModelIndex::default(), first, last);
                self.as_mut().rust_mut().rows.extend(rows);
                self.as_mut().end_insert_rows();
            }
            self.as_mut().set_loaded(first + added);
        }

        if done {
            let pending = self.reload_pending;
            self.as_mut().rust_mut().stream_id = None;
            if pending {
                self.as_mut().rust_mut().reload_pending = false;
            }
            let total = first + added;
            // Status before `finished`: QML reads the status in
            // `onFinishedChanged`, which is emitted synchronously.
            self.as_mut()
                .set_status(QString::from(format!("{total} 项")));
            self.as_mut().set_finished(true);
            // A change arrived during this read, so the rows just shown may
            // already be stale. Re-read now that the stream is over instead of
            // cancelling it mid-flight.
            if pending {
                self.as_mut().reload();
            }
        }
    }

    /// Applies a completed refresh. The new rows are already in hand, so the
    /// model can edit itself in place: the diff keeps every unchanged row and
    /// only inserts, removes or replaces the ones that differ. A full reset is
    /// reserved for a refresh that also changes the base or path, where the
    /// rows would address different files.
    fn finish_refresh(mut self: Pin<&mut Self>, generation: u64) {
        if self.generation != generation {
            self.as_mut().rust_mut().refresh = None;
            return;
        }
        let Some(stage) = self.as_mut().rust_mut().refresh.take() else {
            return;
        };

        let pending = self.reload_pending;
        let total = stage.rows.len() as i32;
        let path_changed = self.path.to_string() != stage.path;
        let base_changed = self.base != stage.base;

        self.as_mut().rust_mut().stream_id = None;
        self.as_mut().rust_mut().reload_pending = false;

        if path_changed || base_changed {
            if path_changed {
                self.as_mut().set_path(QString::from(stage.path.as_str()));
            }
            self.as_mut().about_to_reload();
            unsafe {
                self.as_mut().begin_reset_model();
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.rows = stage.rows;
                    rust.base = stage.base;
                }
                self.as_mut().end_reset_model();
            }
            self.as_mut().set_loaded(total);
        } else {
            self.as_mut().apply_refreshed_rows(stage.rows);
        }

        self.as_mut()
            .set_status(QString::from(format!("{total} 项")));
        self.as_mut().set_finished(true);
        // A change arrived during this read; re-read now that the stream is
        // over instead of having shown a mix of the two listings.
        if pending {
            self.as_mut().reload();
        }
    }

    /// Turns `fresh` into the smallest set of row operations that reaches it
    /// from the rows on screen. Unchanged rows keep their delegates, so a
    /// create/delete does not rebuild the whole view.
    fn apply_refreshed_rows(mut self: Pin<&mut Self>, mut fresh: Vec<StoredRow>) {
        let (prefix, removed, added) = row_edit(&self.rows, &fresh);
        let middle: Vec<StoredRow> = fresh.drain(prefix..prefix + added).collect();

        unsafe {
            if removed > 0 {
                self.as_mut().begin_remove_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + removed - 1) as i32,
                );
                self.as_mut()
                    .rust_mut()
                    .rows
                    .drain(prefix..prefix + removed);
                self.as_mut().end_remove_rows();
            }
            if added > 0 {
                self.as_mut().begin_insert_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + added - 1) as i32,
                );
                self.as_mut().rust_mut().rows.splice(prefix..prefix, middle);
                self.as_mut().end_insert_rows();
            }
        }

        let total = self.rows.len() as i32;
        self.as_mut().set_loaded(total);
    }

    /// A watcher reported a change in `changed`. `changed` is re-checked
    /// because an arm that lost a navigation can still have an event in flight.
    fn reload_after_change(mut self: Pin<&mut Self>, changed: &str) {
        if self.closed {
            return;
        }
        if self.stream_id.is_some() {
            // The displayed path may still be the requested spelling while the
            // head is in flight; the reload waits for it to adopt the
            // canonical one instead of being dropped by the comparison below.
            self.as_mut().rust_mut().reload_pending = true;
            return;
        }
        if self.path.to_string() != changed {
            return;
        }
        self.as_mut().reload();
    }

    fn mark_failed(mut self: Pin<&mut Self>, generation: u64, message: String) {
        if self.generation != generation {
            return;
        }
        self.as_mut().rust_mut().stream_id = None;
        self.as_mut().rust_mut().reload_pending = false;
        // A failed refresh keeps the rows already on screen; a failed
        // navigation had already cleared them.
        self.as_mut().rust_mut().refresh = None;
        self.as_mut().set_status(QString::from(message));
        self.as_mut().set_finished(true);
    }
}

/// Forwards the directory watcher's events into the model's Qt thread.
///
/// The core arms a watcher for the directory this listing displays (see
/// `dae_core::fs::listing::open_streamed_listing`), and the watcher reports
/// changes here. A burst — a copy, an extraction — is coalesced: the reload is
/// delivered once the burst has been quiet for [`CHANGE_DEBOUNCE`], so a
/// directory being written into is not reset once per file.
struct ListingHost {
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
    /// Created on the first event, so a load whose watcher was already armed
    /// (the steady-state refresh) costs nothing.
    changes: Mutex<Option<mpsc::Sender<String>>>,
}

impl ListingHost {
    fn new(thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>) -> Self {
        Self {
            thread,
            changes: Mutex::new(None),
        }
    }
}

impl Host for ListingHost {
    fn emit(&self, event: Event) {
        let Event::DirectoryChanged { path, .. } = event else {
            return;
        };

        let mut changes = self.changes.lock().expect("listing host lock poisoned");
        let sender = changes.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel();
            let thread = self.thread.clone();
            std::thread::spawn(move || forward_changes(receiver, thread));
            sender
        });
        let _ = sender.send(path);
    }
}

/// How long a watcher burst has to go quiet before its reload is delivered.
/// Matches the web shell's debounce, and stays below the interval at which a
/// person notices the list is stale.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);

fn forward_changes(
    changes: mpsc::Receiver<String>,
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
) {
    let mut pending = match changes.recv() {
        Ok(path) => path,
        Err(_) => return,
    };

    loop {
        match changes.recv_timeout(CHANGE_DEBOUNCE) {
            Ok(path) => pending = path,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let changed = pending;
                let _ = thread.queue(move |mut model| model.as_mut().reload_after_change(&changed));
                pending = match changes.recv() {
                    Ok(path) => path,
                    Err(_) => return,
                };
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The shell end of `dae_core::fs::listing::BatchTarget`.
///
/// `open_streamed_listing` returns the head directly and streams the rest, so
/// the head must be inserted before the first streamed batch. A batch that
/// races the head waits in `pending` while `head_ready` is false; the release
/// and the normal path both queue under the same lock, which keeps delivery
/// order equal to the reader's order.
struct QtBatchTarget {
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
    generation: u64,
    state: Mutex<TargetState>,
}

#[derive(Default)]
struct TargetState {
    head_ready: bool,
    pending: Vec<Batch>,
}

impl QtBatchTarget {
    fn new(thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>, generation: u64) -> Self {
        Self {
            thread,
            generation,
            state: Mutex::new(TargetState::default()),
        }
    }

    fn queue_rows(&self, rows: Vec<StoredRow>, done: bool) -> bool {
        let generation = self.generation;
        self.thread
            .queue(move |mut model| model.as_mut().append_rows(generation, rows, done))
            .is_ok()
    }

    fn queue_batch(&self, batch: Batch) -> bool {
        let done = batch.done;
        let rows = batch
            .rows
            .into_iter()
            .map(StoredRow::from_listing)
            .collect();
        self.queue_rows(rows, done)
    }

    fn deliver_head(&self, base: String, path: String, rows: Vec<StoredRow>, done: bool) {
        let mut state = self.state.lock().expect("listing target poisoned");
        let generation = self.generation;
        let _ = self.thread.queue(move |mut model| {
            model
                .as_mut()
                .append_head(generation, base, path, rows, done)
        });
        state.head_ready = true;
        for batch in state.pending.drain(..) {
            self.queue_batch(batch);
        }
    }

    fn fail(&self, message: String) {
        let generation = self.generation;
        let _ = self
            .thread
            .queue(move |mut model| model.as_mut().mark_failed(generation, message));
    }
}

impl BatchTarget for QtBatchTarget {
    fn deliver(&self, batch: Batch) -> bool {
        let mut state = self.state.lock().expect("listing target poisoned");
        if !state.head_ready {
            state.pending.push(batch);
            return true;
        }
        self.queue_batch(batch)
    }
}

fn next_stream_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("qt-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// A distinct registry key per model, so two tabs showing the same directory
/// each keep their own observation (one arm must not steal the other's).
fn next_watcher_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("qt-view-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The smallest single block of rows that has to change to turn `old` into
/// `new`: everything before the common prefix and after the common suffix is
/// kept. Returns `(prefix, removed, added)`.
///
/// A create or delete therefore costs one insert or one remove — the rest of
/// the view keeps its delegates — instead of the whole-model reset that made
/// the list flicker. A change spread across the listing widens the block, but
/// it is still a legal edit: the rows in it are removed and re-inserted.
fn row_edit(old: &[StoredRow], new: &[StoredRow]) -> (usize, usize, usize) {
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(old, new)| old == new)
        .count();
    // The suffix must not overlap the prefix when the lists share a tail that
    // is longer than what is left after the prefix.
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(old, new)| old == new)
        .count();

    (
        prefix,
        old.len() - prefix - suffix,
        new.len() - prefix - suffix,
    )
}

/// The listing's `mtime-size`, used to version a per-file icon URL. Type icons
/// are shared and deliberately not versioned.
fn icon_version(row: &StoredRow) -> String {
    format!("{}-{}", row.modified_at.unwrap_or(0), row.size.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> DirectoryEntry {
        DirectoryEntry {
            name: path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
            path: path.to_owned(),
            kind: EntryKind::File,
            modified_at: Some(1_000),
            size: Some(2_048),
            hidden: true,
            read_only: true,
        }
    }

    #[test]
    fn rows_under_the_base_keep_only_the_name() {
        let row = StoredRow::from_entry(&entry("K:/dae/\u{6587}\u{4ef6}.txt"), "K:/dae/");
        assert!(row.path.is_none());
        assert_eq!(row.full_path("K:/dae/"), "K:/dae/\u{6587}\u{4ef6}.txt");
    }

    #[test]
    fn rows_outside_the_base_keep_their_spelling() {
        let row = StoredRow::from_entry(&entry("K:/elsewhere/file.txt"), "K:/dae/");
        assert_eq!(row.path.as_deref(), Some("K:/elsewhere/file.txt"));
    }

    fn rows(names: &[&str]) -> Vec<StoredRow> {
        names
            .iter()
            .map(|name| StoredRow::from_entry(&entry(&format!("K:/dae/{name}")), "K:/dae/"))
            .collect()
    }

    #[test]
    fn an_unchanged_listing_edits_nothing() {
        let old = rows(&["a.txt", "b.txt"]);
        assert_eq!(row_edit(&old, &old.clone()), (2, 0, 0));
    }

    #[test]
    fn a_new_entry_is_one_insert_at_its_sorted_position() {
        let old = rows(&["a.txt", "b.txt"]);
        let new = rows(&["a.txt", "b.txt", "c.txt"]);
        assert_eq!(row_edit(&old, &new), (2, 0, 1));

        let middle = rows(&["a.txt", "ab.txt", "b.txt"]);
        assert_eq!(row_edit(&old, &middle), (1, 0, 1));
    }

    #[test]
    fn a_deleted_entry_is_one_remove() {
        let old = rows(&["a.txt", "b.txt", "c.txt"]);
        let new = rows(&["a.txt", "c.txt"]);
        assert_eq!(row_edit(&old, &new), (1, 1, 0));
    }

    #[test]
    fn a_changed_entry_is_one_replace() {
        let old = rows(&["a.txt", "b.txt"]);
        let mut new = old.clone();
        new[1].size = Some(4_096);
        assert_eq!(row_edit(&old, &new), (1, 1, 1));
    }

    #[test]
    fn a_tail_shared_with_a_shorter_list_does_not_overlap_the_prefix() {
        let old = rows(&["a.txt", "b.txt"]);
        let new = rows(&["a.txt"]);
        assert_eq!(row_edit(&old, &new), (1, 1, 0));
    }
}
