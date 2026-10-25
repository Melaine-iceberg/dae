//! The gate-4 probe: one clock for the tear-off, one PSS sampler for the
//! window's cost, plus the QML-facing wrapper around the secondary-window half
//! of `cpp/window_probe.h`.
//!
//! The latency is wall-clock, measured in Rust from the moment QML asks for the
//! window to the first `frameSwapped` the new window emits. The marks in
//! between (`created`, `shown`) separate "QML instantiated" from "native window
//! on screen", so a 149 ms total cannot hide a 140 ms hole in one phase.

use std::path::Path;
use std::pin::Pin;
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use crate::memory;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("window_probe.h");

        #[namespace = "dae_window_probe"]
        #[rust_name = "attach_secondary_native"]
        fn attach_secondary(title: &QString) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "forget_secondary_native"]
        fn forget_secondary();

        #[namespace = "dae_window_probe"]
        #[rust_name = "apply_secondary_backdrop_native"]
        fn apply_secondary_backdrop(material: i32, dark: bool) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_info_native"]
        fn secondary_info_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_backdrop_native"]
        fn secondary_backdrop_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_regions_native"]
        fn secondary_regions_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_stats_native"]
        fn secondary_stats_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "clear_secondary_stats_native"]
        fn clear_secondary_stats();

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_hit_test_native"]
        fn secondary_hit_test_at(x: i32, y: i32) -> i32;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_set_foreground_native"]
        fn set_secondary_foreground() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_is_foreground_native"]
        fn is_secondary_foreground() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "secondary_mouse_down_native"]
        fn secondary_mouse_down() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "tearoff_mouse_up_native"]
        fn mouse_up() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "tearoff_move_cursor_native"]
        fn move_cursor(x: i32, y: i32) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "capture_secondary_window_native"]
        fn capture_secondary_window(path: &QString) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "close_secondary_native"]
        fn close_secondary() -> bool;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[cxx_name = "TearOffProbe"]
        type TearOffProbe = super::TearOffProbeRust;

        #[qinvokable]
        #[cxx_name = "beginCycle"]
        fn begin_cycle(self: Pin<&mut TearOffProbe>, label: &QString);

        #[qinvokable]
        fn mark(self: Pin<&mut TearOffProbe>, name: &QString);

        #[qinvokable]
        #[cxx_name = "sampleMemory"]
        fn sample_memory(self: Pin<&mut TearOffProbe>, cycle: &QString, label: &QString);

        #[qinvokable]
        #[cxx_name = "metricsJson"]
        fn metrics_json(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "attachSecondary"]
        fn attach_secondary(&self, title: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "forgetSecondary"]
        fn forget_secondary(&self);

        #[qinvokable]
        #[cxx_name = "applySecondaryMaterial"]
        fn apply_secondary_material(&self, material: &QString, dark: bool) -> bool;

        #[qinvokable]
        #[cxx_name = "secondaryInfo"]
        fn secondary_info(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "secondaryBackdrop"]
        fn secondary_backdrop(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "secondaryRegions"]
        fn secondary_regions(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "secondaryStats"]
        fn secondary_stats(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "clearSecondaryStats"]
        fn clear_secondary_stats(&self);

        #[qinvokable]
        #[cxx_name = "secondaryHitTest"]
        fn secondary_hit_test(&self, x: i32, y: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "secondarySetForeground"]
        fn secondary_set_foreground(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "secondaryIsForeground"]
        fn secondary_is_foreground(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "secondaryMouseDown"]
        fn secondary_mouse_down(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "mouseUp"]
        fn mouse_up(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "moveCursor"]
        fn move_cursor(&self, x: i32, y: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "captureSecondaryWindow"]
        fn capture_secondary_window(&self, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "closeSecondary"]
        fn close_secondary(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "writeResults"]
        fn write_results(&self, path: &QString, json: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "appendLog"]
        fn append_log(&self, path: &QString, line: &QString) -> bool;
    }
}

#[derive(Clone)]
struct Mark {
    name: String,
    ms: f64,
}

struct Cycle {
    label: String,
    marks: Vec<Mark>,
}

struct MemorySample {
    cycle: String,
    label: String,
    at_ms: u64,
    snapshot: memory::Snapshot,
}

pub struct TearOffProbeRust {
    cycles: Vec<Cycle>,
    memory: Vec<MemorySample>,
    cycle_start: Option<Instant>,
    started: Instant,
}

impl Default for TearOffProbeRust {
    fn default() -> Self {
        Self {
            cycles: Vec::new(),
            memory: Vec::new(),
            cycle_start: None,
            started: Instant::now(),
        }
    }
}

impl qobject::TearOffProbe {
    fn begin_cycle(mut self: Pin<&mut Self>, label: &QString) {
        let label = label.to_string();
        let mut rust = self.as_mut().rust_mut();
        rust.cycles.push(Cycle {
            label,
            marks: Vec::new(),
        });
        rust.cycle_start = Some(Instant::now());
    }

    fn mark(mut self: Pin<&mut Self>, name: &QString) {
        let mut rust = self.as_mut().rust_mut();
        let Some(start) = rust.cycle_start else {
            return;
        };
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        if let Some(cycle) = rust.cycles.last_mut() {
            cycle.marks.push(Mark {
                name: name.to_string(),
                ms,
            });
        }
    }

    fn sample_memory(mut self: Pin<&mut Self>, cycle: &QString, label: &QString) {
        let snapshot = memory::sample();
        let at_ms = self.started.elapsed().as_millis() as u64;
        self.as_mut().rust_mut().memory.push(MemorySample {
            cycle: cycle.to_string(),
            label: label.to_string(),
            at_ms,
            snapshot,
        });
    }

    fn metrics_json(&self) -> QString {
        let rust = self.rust();
        let cycles: Vec<serde_json::Value> = rust
            .cycles
            .iter()
            .map(|cycle| {
                let mut marks = serde_json::Map::new();
                for mark in &cycle.marks {
                    marks.insert(mark.name.clone(), serde_json::json!(round(mark.ms)));
                }
                let latency = cycle
                    .marks
                    .iter()
                    .find(|mark| mark.name == "frame")
                    .map(|mark| mark.ms);
                serde_json::json!({
                    "label": cycle.label,
                    "latency_ms": latency.map(round),
                    "marks": marks,
                })
            })
            .collect();

        let memory: Vec<serde_json::Value> = rust
            .memory
            .iter()
            .map(|sample| {
                let snapshot = sample.snapshot;
                serde_json::json!({
                    "cycle": sample.cycle,
                    "label": sample.label,
                    "at_ms": sample.at_ms,
                    "pss_mb": round(snapshot.pss as f64 / 1_048_576.0),
                    "working_set_mb": round(snapshot.working_set as f64 / 1_048_576.0),
                    "private_mb": round(snapshot.private as f64 / 1_048_576.0),
                    "peak_working_set_mb": round(snapshot.peak_working_set as f64 / 1_048_576.0),
                })
            })
            .collect();

        QString::from(
            serde_json::json!({
                "cycles": cycles,
                "memory": memory,
            })
            .to_string(),
        )
    }

    fn attach_secondary(&self, title: &QString) -> bool {
        qobject::attach_secondary_native(title)
    }

    fn forget_secondary(&self) {
        qobject::forget_secondary_native();
    }

    fn apply_secondary_material(&self, material: &QString, dark: bool) -> bool {
        let code = match material.to_string().as_str() {
            "mica" => 0,
            "acrylic" => 1,
            "none" => 2,
            other => {
                eprintln!("gate4: unknown material {other:?}");
                return false;
            }
        };
        qobject::apply_secondary_backdrop_native(code, dark)
    }

    fn secondary_info(&self) -> QString {
        convert(qobject::secondary_info_native())
    }

    fn secondary_backdrop(&self) -> QString {
        convert(qobject::secondary_backdrop_native())
    }

    fn secondary_regions(&self) -> QString {
        convert(qobject::secondary_regions_native())
    }

    fn secondary_stats(&self) -> QString {
        convert(qobject::secondary_stats_native())
    }

    fn clear_secondary_stats(&self) {
        qobject::clear_secondary_stats_native();
    }

    fn secondary_hit_test(&self, x: i32, y: i32) -> i32 {
        qobject::secondary_hit_test_native(x, y)
    }

    fn secondary_set_foreground(&self) -> bool {
        qobject::secondary_set_foreground_native()
    }

    fn secondary_is_foreground(&self) -> bool {
        qobject::secondary_is_foreground_native()
    }

    fn secondary_mouse_down(&self) -> bool {
        qobject::secondary_mouse_down_native()
    }

    fn mouse_up(&self) -> bool {
        qobject::tearoff_mouse_up_native()
    }

    fn move_cursor(&self, x: i32, y: i32) -> bool {
        qobject::tearoff_move_cursor_native(x, y)
    }

    fn capture_secondary_window(&self, path: &QString) -> bool {
        qobject::capture_secondary_window_native(path)
    }

    fn close_secondary(&self) -> bool {
        qobject::close_secondary_native()
    }

    fn write_results(&self, path: &QString, json: &QString) -> bool {
        let path = path.to_string();
        let Some(parent) = Path::new(&path).parent() else {
            return false;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
        std::fs::write(path, json.to_string().as_bytes()).is_ok()
    }

    fn append_log(&self, path: &QString, line: &QString) -> bool {
        use std::io::Write;

        let path = path.to_string();
        if let Some(parent) = Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        else {
            return false;
        };
        writeln!(file, "{}", line.to_string()).is_ok()
    }
}

fn convert(value: cxx::UniquePtr<cxx::CxxString>) -> QString {
    match value.as_ref() {
        Some(text) => QString::from(std::str::from_utf8(text.as_bytes()).unwrap_or_default()),
        None => QString::from(""),
    }
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
