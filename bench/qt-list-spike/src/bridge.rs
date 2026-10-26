//! The cxx-qt bridge for gate 1.
//!
//! Two QObjects:
//!
//! * `SpikeListModel` — a real `QAbstractListModel` over the rows that
//!   `dae_core::fs::listing` emits. The reader thread calls `BatchTarget` from
//!   `dae-core`; this file is only the shell half of that seam: it queues the
//!   batch onto the Qt thread and turns it into `beginInsertRows`.
//! * `FrameProbe` — records GUI event-loop busy time, render-thread
//!   sync/paint time, the `frameSwapped` interval and process memory while QML
//!   scrolls the list, then writes one JSON result and quits.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant, QVector,
};
use dae_core::fs::entry_codec::ListingRow;
use dae_core::fs::listing::{self, Batch, BatchTarget};
use dae_core::host::{Event, Host};
use dae_core::types::{DirectoryEntry, EntryKind};

const ROLE_NAME: i32 = 0x0100;
const ROLE_SIZE: i32 = 0x0101;
const ROLE_MODIFIED: i32 = 0x0102;
const ROLE_IS_DIR: i32 = 0x0103;
const ROLE_HIDDEN: i32 = 0x0104;
/// The same `image://fileicon/type/...` URL the product model emits. The
/// fixture is files-only, so every row shares one request per extension; a
/// directory fixture would ask by path instead.
const ROLE_ICON: i32 = 0x0105;

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

        include!("cxx-qt-lib/qvector.h");
        type QVector_i32 = cxx_qt_lib::QVector<i32>;

        include!("frame_probe.h");
        #[namespace = "dae_spike_probe"]
        #[rust_name = "attach_frame_probe"]
        fn dae_spike_attach();
        #[namespace = "dae_spike_probe"]
        #[rust_name = "begin_frame_probe"]
        fn dae_spike_begin();
        #[namespace = "dae_spike_probe"]
        #[rust_name = "end_frame_probe"]
        fn dae_spike_end();
        #[namespace = "dae_spike_probe"]
        fn dae_spike_count(series: i32) -> i32;
        #[namespace = "dae_spike_probe"]
        fn dae_spike_percentile(series: i32, percentile: f64) -> f64;
        #[namespace = "dae_spike_probe"]
        fn dae_spike_max(series: i32) -> f64;
        #[namespace = "dae_spike_probe"]
        fn dae_spike_mean(series: i32) -> f64;
        #[namespace = "dae_spike_probe"]
        fn dae_spike_gui_busy_ns() -> i64;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
        #[qproperty(bool, finished)]
        #[qproperty(i32, batches)]
        #[qproperty(f64, load_ms)]
        #[qproperty(i64, last_patch_us)]
        #[qproperty(QString, status)]
        type SpikeListModel = super::SpikeListModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[qinvokable]
        fn start(self: Pin<&mut SpikeListModel>, path: &QString, stream_id: &QString);

        #[qinvokable]
        #[cxx_name = "applyPatches"]
        fn apply_patches(self: Pin<&mut SpikeListModel>, first: i32, count: i32);

        #[inherit]
        #[rust_name = "begin_insert_rows"]
        unsafe fn beginInsertRows(
            self: Pin<&mut SpikeListModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_insert_rows"]
        unsafe fn endInsertRows(self: Pin<&mut SpikeListModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut SpikeListModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut SpikeListModel>);

        #[inherit]
        fn index(
            self: &SpikeListModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[rust_name = "data_changed"]
        fn dataChanged(
            self: Pin<&mut SpikeListModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QVector_i32,
        );
    }

    impl cxx_qt::Threading for SpikeListModel {}

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        type FrameProbe = super::FrameProbeRust;

        #[qinvokable]
        #[cxx_name = "onFrame"]
        fn on_frame(self: Pin<&mut FrameProbe>);

        #[qinvokable]
        fn begin(self: Pin<&mut FrameProbe>);

        #[qinvokable]
        #[cxx_name = "sampleMemory"]
        fn sample_memory(self: Pin<&mut FrameProbe>, label: &QString);

        #[qinvokable]
        #[cxx_name = "notePatch"]
        fn note_patch(self: Pin<&mut FrameProbe>, micros: i64);

        #[qinvokable]
        fn finish(
            self: Pin<&mut FrameProbe>,
            output: &QString,
            rows: i32,
            load_ms: f64,
            batches: i32,
            delegates_created: i32,
            patches_enabled: bool,
            jumps_enabled: bool,
        ) -> QString;
    }
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

/// The path is the listing base plus the name, and the model never needs the
/// spelled-out path. Keeping only the name is what the PSS number is about:
/// 120k of these live in the model for the whole run.
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
pub struct SpikeListModelRust {
    loaded: i32,
    finished: bool,
    batches: i32,
    load_ms: f64,
    last_patch_us: i64,
    status: QString,
    rows: Vec<StoredRow>,
    started: Option<Instant>,
}

impl qobject::SpikeListModel {
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
            ROLE_SIZE => QVariant::from(&QString::from(super::format::size(row.row.size))),
            ROLE_MODIFIED => QVariant::from(&QString::from(super::format::timestamp(
                row.row.modified_at,
            ))),
            ROLE_IS_DIR => QVariant::from(&row.row.is_dir),
            ROLE_HIDDEN => QVariant::from(&row.row.hidden),
            ROLE_ICON => {
                let extension = Path::new(row.name.as_str())
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                QVariant::from(&QString::from(
                    format!("image://fileicon/type/{extension}?size=36").as_str(),
                ))
            }
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
        roles.insert(ROLE_ICON, QByteArray::from("iconSource"));
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
        self.as_mut().set_last_patch_us(-1);
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

    /// A real file manager patches rows while the user scrolls (sizes settle,
    /// timestamps move). This emits one `dataChanged` over a contiguous range,
    /// the shape the listing watcher also uses, and records how long the model
    /// side took.
    fn apply_patches(mut self: Pin<&mut Self>, first: i32, count: i32) {
        if count <= 0 {
            return;
        }
        let start = first.max(0) as usize;
        let end = start.saturating_add(count as usize).min(self.rows.len());
        if start >= end {
            self.as_mut().set_last_patch_us(0);
            return;
        }

        let began = Instant::now();
        {
            let mut rust = self.as_mut().rust_mut();
            for (offset, stored) in rust.rows[start..end].iter_mut().enumerate() {
                stored.row.size = stored.row.size.wrapping_add(1 + offset as u64);
                stored.row.modified_at = stored.row.modified_at.wrapping_add(1_000);
            }
        }

        let top = self.index(start as i32, 0, &QModelIndex::default());
        let bottom = self.index(end as i32 - 1, 0, &QModelIndex::default());
        let mut roles = QVector::<i32>::default();
        roles.append(ROLE_SIZE);
        roles.append(ROLE_MODIFIED);
        self.as_mut().data_changed(&top, &bottom, &roles);
        self.as_mut()
            .set_last_patch_us(began.elapsed().as_micros() as i64);
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
            self.as_mut().set_load_ms(millis);
            self.as_mut().set_finished(true);
            self.as_mut().set_status(QString::from(format!(
                "{total} rows / {batches} batches / {millis:.1} ms"
            )));
        }
    }

    fn mark_failed(mut self: Pin<&mut Self>, message: String) {
        eprintln!("spike: listing failed: {message}");
        self.as_mut().set_finished(true);
        self.as_mut().set_status(QString::from(message));
    }
}

struct NoopHost;

impl Host for NoopHost {
    fn emit(&self, _event: Event) {}
}

/// The shell end of `dae_core::fs::listing::BatchTarget`.
///
/// `open_streamed_listing` returns the head directly and streams the rest, so
/// the head has to be inserted before the first streamed batch. Batches that
/// race the head are parked in `pending` while `head_ready` is false; both the
/// release and the normal path queue while holding the same lock, which keeps
/// the QObject-side delivery order equal to the reader's order.
struct QtBatchTarget {
    thread: cxx_qt::CxxQtThread<qobject::SpikeListModel>,
    state: Mutex<TargetState>,
}

#[derive(Default)]
struct TargetState {
    head_ready: bool,
    pending: Vec<Batch>,
}

impl QtBatchTarget {
    fn new(thread: cxx_qt::CxxQtThread<qobject::SpikeListModel>) -> Self {
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

struct MemorySample {
    label: String,
    at_ms: u64,
    snapshot: super::memory::Snapshot,
}

pub struct FrameProbeRust {
    recording: bool,
    wall_us: Vec<u64>,
    cpu_us: Vec<u64>,
    gui_us: Vec<u64>,
    last_wall: Option<Instant>,
    last_cpu: Option<u64>,
    last_gui_busy: Option<i64>,
    memory: Vec<MemorySample>,
    patch_us: Vec<i64>,
    began: Option<Instant>,
    started: Instant,
}

impl Default for FrameProbeRust {
    fn default() -> Self {
        Self {
            recording: false,
            wall_us: Vec::new(),
            cpu_us: Vec::new(),
            gui_us: Vec::new(),
            last_wall: None,
            last_cpu: None,
            last_gui_busy: None,
            memory: Vec::new(),
            patch_us: Vec::new(),
            began: None,
            started: Instant::now(),
        }
    }
}

impl qobject::FrameProbe {
    fn on_frame(mut self: Pin<&mut Self>) {
        let now = Instant::now();
        let cpu = super::memory::process_cpu_time();
        let gui_busy = qobject::dae_spike_gui_busy_ns();
        if self.recording {
            let previous_wall = self.last_wall;
            let previous_cpu = self.last_cpu;
            let previous_gui = self.last_gui_busy;
            if let (Some(previous_wall), Some(cpu), Some(previous_cpu)) =
                (previous_wall, cpu, previous_cpu)
            {
                let wall = now.duration_since(previous_wall).as_micros() as u64;
                if wall > 0 {
                    let mut rust = self.as_mut().rust_mut();
                    rust.wall_us.push(wall);
                    rust.cpu_us.push(cpu.saturating_sub(previous_cpu) / 10);
                    if let Some(previous_gui) = previous_gui {
                        let busy = gui_busy.saturating_sub(previous_gui).max(0) as u64;
                        rust.gui_us.push(busy / 1_000);
                    }
                }
            }
        }
        let mut rust = self.as_mut().rust_mut();
        rust.last_wall = Some(now);
        rust.last_cpu = cpu;
        rust.last_gui_busy = Some(gui_busy);
    }

    fn begin(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.recording = true;
            rust.wall_us.clear();
            rust.cpu_us.clear();
            rust.gui_us.clear();
            rust.patch_us.clear();
            rust.last_wall = None;
            rust.last_cpu = None;
            rust.last_gui_busy = None;
            rust.began = Some(Instant::now());
        }
        qobject::attach_frame_probe();
        qobject::begin_frame_probe();
        self.as_mut().sample_memory(&QString::from("begin"));
    }

    fn sample_memory(mut self: Pin<&mut Self>, label: &QString) {
        let snapshot = super::memory::sample();
        let at_ms = self.started.elapsed().as_millis() as u64;
        self.as_mut().rust_mut().memory.push(MemorySample {
            label: label.to_string(),
            at_ms,
            snapshot,
        });
    }

    fn note_patch(mut self: Pin<&mut Self>, micros: i64) {
        self.as_mut().rust_mut().patch_us.push(micros);
    }

    fn finish(
        mut self: Pin<&mut Self>,
        output: &QString,
        rows: i32,
        load_ms: f64,
        batches: i32,
        delegates_created: i32,
        patches_enabled: bool,
        jumps_enabled: bool,
    ) -> QString {
        self.as_mut().rust_mut().recording = false;
        qobject::end_frame_probe();
        self.as_mut().sample_memory(&QString::from("end"));

        let pinned = self.as_ref();
        let rust = pinned.rust();
        let seconds = rust
            .began
            .map(|began| began.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let wall_ms: Vec<f64> = rust.wall_us.iter().map(|us| *us as f64 / 1000.0).collect();
        let cpu_ms: Vec<f64> = rust.cpu_us.iter().map(|us| *us as f64 / 1000.0).collect();
        let gui_ms: Vec<f64> = rust.gui_us.iter().map(|us| *us as f64 / 1000.0).collect();
        let wall = Stats::from(&wall_ms);
        let cpu = Stats::from(&cpu_ms);
        let gui = Stats::from(&gui_ms);
        let sync = cpp_stats(0);
        let paint = cpp_stats(1);
        let render = cpp_stats(2);
        let render_count = qobject::dae_spike_count(2);
        let patches: Vec<f64> = rust.patch_us.iter().map(|us| *us as f64 / 1000.0).collect();
        let patch = Stats::from(&patches);

        // GUI-thread busy time and render-thread work are disjoint: the GUI
        // thread blocks while the render thread syncs/paints. Their p95s are
        // added to get a conservative per-frame cost, not two samples from one
        // distribution.
        let work = Stats {
            p50: gui.p50 + render.p50,
            p95: gui.p95 + render.p95,
            p99: gui.p99 + render.p99,
            max: gui.max + render.max,
            mean: gui.mean + render.mean,
        };

        let pss_mb = rust
            .memory
            .iter()
            .rev()
            .find(|sample| sample.label == "end")
            .map(|sample| sample.snapshot.pss as f64 / 1_048_576.0)
            .unwrap_or(0.0);
        let scroll_pass = render_count > 0 && work.p95 <= 8.0;
        let pss_pass = pss_mb <= 250.0;

        let samples: Vec<serde_json::Value> = rust
            .memory
            .iter()
            .map(|sample| {
                let snapshot = sample.snapshot;
                serde_json::json!({
                    "label": &sample.label,
                    "at_ms": sample.at_ms,
                    "pss_mb": round(snapshot.pss as f64 / 1_048_576.0),
                    "working_set_mb": round(snapshot.working_set as f64 / 1_048_576.0),
                    "private_mb": round(snapshot.private as f64 / 1_048_576.0),
                    "peak_working_set_mb": round(snapshot.peak_working_set as f64 / 1_048_576.0),
                })
            })
            .collect();

        let result = serde_json::json!({
            "kind": "qt-list-spike",
            "qt_version": std::env::var("DAE_QT_VERSION").unwrap_or_else(|_| "unknown".into()),
            "rhi_backend": std::env::var("QSG_RHI_BACKEND").unwrap_or_else(|_| "default".into()),
            "vsync_off": std::env::var_os("QSG_NO_VSYNC").is_some(),
            "rows": rows,
            "delegates_created": delegates_created,
            "workload": {"patches": patches_enabled, "jumps": jumps_enabled},
            "load": {"batches": batches, "ms": load_ms},
            "frame": {
                "count": wall_ms.len(),
                "seconds": round(seconds),
                "fps": if seconds > 0.0 { round(wall_ms.len() as f64 / seconds) } else { 0.0 },
                "cadence_ms": wall.to_json(),
                "gui_ms": gui.to_json(),
                "cpu_ms": cpu.to_json(),
                "render": {
                    "count": render_count,
                    "sync_ms": sync.to_json(),
                    "paint_ms": paint.to_json(),
                    "total_ms": render.to_json(),
                },
                "work_ms": work.to_json(),
            },
            "patch": {
                "count": patches.len(),
                "p50_us": round(patch.p50 * 1000.0),
                "p95_us": round(patch.p95 * 1000.0),
                "max_us": round(patch.max * 1000.0),
            },
            "memory": samples,
            "gate": {
                "scroll_p95_ms": 8.0,
                "scroll_p95": round(work.p95),
                "scroll_cadence_p95": round(wall.p95),
                "scroll_gui_p95": round(gui.p95),
                "scroll_render_p95": round(render.p95),
                "scroll_pass": scroll_pass,
                "pss_mb": 250.0,
                "pss": round(pss_mb),
                "pss_pass": pss_pass,
                "pass": scroll_pass && pss_pass,
            },
        });

        let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into());
        let output = output.to_string();
        let path = Path::new(&output);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(path, &text) {
            eprintln!("could not write {}: {error}", path.display());
        }
        println!("{text}");

        QString::from(format!(
            "rows={rows} frames={} work_p95={:.2}ms (gui {:.2} + render {:.2}) cadence_p95={:.2}ms pss={:.1}MB => {}",
            wall_ms.len(),
            work.p95,
            gui.p95,
            render.p95,
            wall.p95,
            pss_mb,
            if scroll_pass && pss_pass {
                "PASS"
            } else {
                "FAIL"
            }
        ))
    }
}

#[derive(Default)]
struct Stats {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    mean: f64,
}

impl Stats {
    fn from(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|left, right| left.total_cmp(right));
        let percentile = |p: f64| {
            let index = ((sorted.len() as f64 * p).ceil() as usize)
                .saturating_sub(1)
                .min(sorted.len() - 1);
            sorted[index]
        };
        Self {
            p50: percentile(0.50),
            p95: percentile(0.95),
            p99: percentile(0.99),
            max: *sorted.last().expect("non-empty"),
            mean: values.iter().sum::<f64>() / values.len() as f64,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "p50": round(self.p50),
            "p95": round(self.p95),
            "p99": round(self.p99),
            "max": round(self.max),
            "mean": round(self.mean),
        })
    }
}

fn cpp_stats(series: i32) -> Stats {
    Stats {
        p50: qobject::dae_spike_percentile(series, 0.50),
        p95: qobject::dae_spike_percentile(series, 0.95),
        p99: qobject::dae_spike_percentile(series, 0.99),
        max: qobject::dae_spike_max(series),
        mean: qobject::dae_spike_mean(series),
    }
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
