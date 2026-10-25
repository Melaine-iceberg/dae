//! The listing model for gate 4.
//!
//! The same real `QAbstractListModel` gate 1 measured, trimmed to the model
//! half: rows arrive from `dae_core::fs::listing::open_streamed_listing`
//! through the `BatchTarget` seam and become `beginInsertRows`. Gate 4 shares
//! one instance between the source window and the torn-off window, which is
//! what a tab tear-off does in the shell: the view moves, the data does not.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::entry_codec::ListingRow;
use dae_core::fs::listing::{self, Batch, BatchTarget};
use dae_core::host::{Event, Host};
use dae_core::types::{DirectoryEntry, EntryKind};

const ROLE_NAME: i32 = 0x0100;
const ROLE_SIZE: i32 = 0x0101;
const ROLE_MODIFIED: i32 = 0x0102;
const ROLE_IS_DIR: i32 = 0x0103;
const ROLE_HIDDEN: i32 = 0x0104;

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
        #[qproperty(i32, batches)]
        #[qproperty(f64, load_ms)]
        #[qproperty(QString, status)]
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
        fn start(self: Pin<&mut DirectoryListModel>, path: &QString, stream_id: &QString);

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
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut DirectoryListModel>);
    }

    impl cxx_qt::Threading for DirectoryListModel {}
}

#[derive(Clone, Copy)]
struct Row {
    size: u64,
    modified_at: u64,
    is_dir: bool,
    hidden: bool,
}

impl Row {
    fn from_listing(row: &ListingRow) -> Self {
        Self {
            size: row.size.unwrap_or(0),
            modified_at: row.modified_at.unwrap_or(0),
            is_dir: row.kind == EntryKind::Directory,
            hidden: row.hidden,
        }
    }

    fn from_entry(entry: &DirectoryEntry) -> Self {
        Self {
            size: entry.size.unwrap_or(0),
            modified_at: entry.modified_at.unwrap_or(0),
            is_dir: entry.kind == EntryKind::Directory,
            hidden: entry.hidden,
        }
    }
}

struct StoredRow {
    name: String,
    row: Row,
}

impl StoredRow {
    fn from_entry(entry: &DirectoryEntry) -> Self {
        Self {
            name: entry.name.clone(),
            row: Row::from_entry(entry),
        }
    }

    fn from_listing(row: ListingRow) -> Self {
        let row_data = Row::from_listing(&row);
        Self {
            name: row.name,
            row: row_data,
        }
    }
}

#[derive(Default)]
pub struct DirectoryListModelRust {
    loaded: i32,
    finished: bool,
    batches: i32,
    load_ms: f64,
    status: QString,
    rows: Vec<StoredRow>,
    started: Option<Instant>,
}

impl qobject::DirectoryListModel {
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
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_SIZE => QVariant::from(&QString::from(crate::format::size(row.row.size))),
            ROLE_MODIFIED => QVariant::from(&QString::from(crate::format::timestamp(
                row.row.modified_at,
            ))),
            ROLE_IS_DIR => QVariant::from(&row.row.is_dir),
            ROLE_HIDDEN => QVariant::from(&row.row.hidden),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_NAME, QByteArray::from("fileName"));
        roles.insert(ROLE_SIZE, QByteArray::from("sizeText"));
        roles.insert(ROLE_MODIFIED, QByteArray::from("modifiedText"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles.insert(ROLE_HIDDEN, QByteArray::from("hidden"));
        roles
    }

    fn start(mut self: Pin<&mut Self>, path: &QString, stream_id: &QString) {
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let mut rust = self.as_mut().rust_mut();
                rust.rows.clear();
                rust.started = Some(Instant::now());
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_loaded(0);
        self.as_mut().set_finished(false);
        self.as_mut().set_batches(0);
        self.as_mut().set_load_ms(-1.0);
        self.as_mut().set_status(QString::from("reading"));

        let thread = self.as_mut().qt_thread();
        let target = Arc::new(QtBatchTarget::new(thread));
        let worker = Arc::clone(&target);
        let path = PathBuf::from(path.to_string());
        let stream_id = stream_id.to_string();

        std::thread::spawn(move || {
            let host: Arc<dyn Host> = Arc::new(NoopHost);
            match listing::open_streamed_listing(host, path, stream_id, None, worker.clone()) {
                Ok(view) => {
                    let done = view.stream_id.is_none();
                    let rows = view.entries.iter().map(StoredRow::from_entry).collect();
                    worker.deliver_head(rows, done);
                }
                Err(error) => worker.fail(format!("{error}")),
            }
        });
    }

    fn append_rows(mut self: Pin<&mut Self>, rows: Vec<StoredRow>, done: bool) {
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

        let batches = self.batches + 1;
        self.as_mut().set_batches(batches);

        if done {
            let total = first + added;
            let millis = self
                .started
                .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(-1.0);
            // Status before `finished`: QML reads the status in
            // `onFinishedChanged`, which is emitted synchronously.
            self.as_mut().set_status(QString::from(format!(
                "{total} rows / {batches} batches / {millis:.1} ms"
            )));
            self.as_mut().set_load_ms(millis);
            self.as_mut().set_finished(true);
        }
    }

    fn mark_failed(mut self: Pin<&mut Self>, message: String) {
        eprintln!("gate4: listing failed: {message}");
        self.as_mut().set_finished(true);
        self.as_mut().set_status(QString::from(message));
    }
}

struct NoopHost;

impl Host for NoopHost {
    fn emit(&self, _event: Event) {}
}

/// The shell end of `dae_core::fs::listing::BatchTarget`, unchanged from gate 1:
/// the head is delivered before any streamed batch, and a batch that races the
/// head waits in `pending`.
struct QtBatchTarget {
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
    state: Mutex<TargetState>,
}

#[derive(Default)]
struct TargetState {
    head_ready: bool,
    pending: Vec<Batch>,
}

impl QtBatchTarget {
    fn new(thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>) -> Self {
        Self {
            thread,
            state: Mutex::new(TargetState::default()),
        }
    }

    fn queue_rows(&self, rows: Vec<StoredRow>, done: bool) -> bool {
        self.thread
            .queue(move |mut model| model.as_mut().append_rows(rows, done))
            .is_ok()
    }

    fn queue_batch(&self, batch: Batch) -> bool {
        let rows = batch
            .rows
            .into_iter()
            .map(StoredRow::from_listing)
            .collect();
        self.queue_rows(rows, batch.done)
    }

    fn deliver_head(&self, rows: Vec<StoredRow>, done: bool) {
        let mut state = self.state.lock().expect("target state poisoned");
        self.queue_rows(rows, done);
        state.head_ready = true;
        for batch in state.pending.drain(..) {
            self.queue_batch(batch);
        }
    }

    fn fail(&self, message: String) {
        let _ = self
            .thread
            .queue(move |mut model| model.as_mut().mark_failed(message));
    }
}

impl BatchTarget for QtBatchTarget {
    fn deliver(&self, batch: Batch) -> bool {
        let mut state = self.state.lock().expect("target state poisoned");
        if !state.head_ready {
            state.pending.push(batch);
            return true;
        }
        self.queue_batch(batch)
    }
}
