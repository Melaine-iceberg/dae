//! QML-facing wrapper around `cpp/window_probe.h` and the ported backdrop
//! decision in `material.rs`.
//!
//! The scripted run lives in QML because that is where the timer steps and the
//! assertions read best; the probe owns the native side (DWM, hit tests,
//! SendInput, screen grabs) and the JSON result is written here so QML does not
//! need file APIs.

use cxx_qt_lib::QString;

use crate::material;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("window_probe.h");

        #[namespace = "dae_window_probe"]
        #[rust_name = "enable_alpha_native"]
        fn enable_alpha();

        #[namespace = "dae_window_probe"]
        #[rust_name = "attach_native"]
        fn attach() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "apply_backdrop_native"]
        fn apply_backdrop(material: i32, dark: bool) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "backdrop_state_native"]
        fn backdrop_state_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "window_info_native"]
        fn window_info_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "chrome_regions_native"]
        fn chrome_regions_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "hit_test_native"]
        fn hit_test_at(x: i32, y: i32) -> i32;

        #[namespace = "dae_window_probe"]
        #[rust_name = "chrome_stats_native"]
        fn chrome_stats_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "clear_chrome_stats_native"]
        fn clear_chrome_stats();

        #[namespace = "dae_window_probe"]
        #[rust_name = "cursor_position_native"]
        fn cursor_position_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "move_cursor_native"]
        fn move_cursor(x: i32, y: i32) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "mouse_down_native"]
        fn mouse_down() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "mouse_up_native"]
        fn mouse_up() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "click_native"]
        fn click(x: i32, y: i32) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "set_foreground_native"]
        fn set_foreground() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "is_foreground_native"]
        fn is_foreground() -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "capture_native"]
        fn capture(path: &QString, x: i32, y: i32, width: i32, height: i32) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "capture_window_native"]
        fn capture_window(path: &QString) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "begin_samples_native"]
        fn begin_samples(x: i32, y: i32, width: i32, height: i32);

        #[namespace = "dae_window_probe"]
        #[rust_name = "sample_native"]
        fn take_sample(tag: &QString, path: &QString) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "sample_report_native"]
        fn sample_report_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "top_windows_native"]
        fn top_windows_json() -> UniquePtr<CxxString>;

        #[namespace = "dae_window_probe"]
        #[rust_name = "flyout_begin_native"]
        fn flyout_begin(x: i32, y: i32, width: i32, height: i32);

        #[namespace = "dae_window_probe"]
        #[rust_name = "maximize_native_fn"]
        fn maximize_native(via_qt: bool) -> bool;

        #[namespace = "dae_window_probe"]
        #[rust_name = "flyout_report_native"]
        fn flyout_report_json() -> UniquePtr<CxxString>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        type WindowProbe = super::WindowProbeRust;

        #[qinvokable]
        fn attach(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "detectMaterial"]
        fn detect_material(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "materialReport"]
        fn material_report(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "applyMaterial"]
        fn apply_material(&self, material: &QString, dark: bool) -> bool;

        #[qinvokable]
        #[cxx_name = "backdropState"]
        fn backdrop_state(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "windowInfo"]
        fn window_info(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "chromeRegions"]
        fn chrome_regions(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "hitTest"]
        fn hit_test(&self, x: i32, y: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "chromeStats"]
        fn chrome_stats(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "clearStats"]
        fn clear_stats(&self);

        #[qinvokable]
        #[cxx_name = "cursorPosition"]
        fn cursor_position(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "moveCursor"]
        fn move_cursor(&self, x: i32, y: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "mouseDown"]
        fn mouse_down(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "mouseUp"]
        fn mouse_up(&self) -> bool;

        #[qinvokable]
        fn click(&self, x: i32, y: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "setForeground"]
        fn set_foreground(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "isForeground"]
        fn is_foreground(&self) -> bool;

        #[qinvokable]
        fn capture(
            &self,
            path: &QString,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "captureWindow"]
        fn capture_window(&self, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "beginSamples"]
        fn begin_samples(&self, x: i32, y: i32, width: i32, height: i32);

        #[qinvokable]
        fn sample(&self, tag: &QString, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "sampleReport"]
        fn sample_report(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "topWindows"]
        fn top_windows(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "flyoutBegin"]
        fn flyout_begin(&self, x: i32, y: i32, width: i32, height: i32);

        #[qinvokable]
        #[cxx_name = "flyoutReport"]
        fn flyout_report(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "maximizeWindow"]
        fn maximize_window(&self, via_qt: bool) -> bool;

        #[qinvokable]
        #[cxx_name = "writeResults"]
        fn write_results(&self, path: &QString, json: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "appendLog"]
        fn append_log(&self, path: &QString, line: &QString) -> bool;
    }
}

#[derive(Default)]
pub struct WindowProbeRust;

impl qobject::WindowProbe {
    fn attach(&self) -> bool {
        qobject::attach_native()
    }

    fn detect_material(&self) -> QString {
        QString::from(material::startup().as_str())
    }

    fn material_report(&self) -> QString {
        convert_str(&material::report().to_string())
    }

    fn apply_material(&self, material: &QString, dark: bool) -> bool {
        let code = match material.to_string().as_str() {
            "mica" => 0,
            "acrylic" => 1,
            "none" => 2,
            other => {
                eprintln!("window-spike: unknown material {other:?}");
                return false;
            }
        };
        qobject::apply_backdrop_native(code, dark)
    }

    fn backdrop_state(&self) -> QString {
        convert(qobject::backdrop_state_native())
    }

    fn window_info(&self) -> QString {
        convert(qobject::window_info_native())
    }

    fn chrome_regions(&self) -> QString {
        convert(qobject::chrome_regions_native())
    }

    fn hit_test(&self, x: i32, y: i32) -> i32 {
        qobject::hit_test_native(x, y)
    }

    fn chrome_stats(&self) -> QString {
        convert(qobject::chrome_stats_native())
    }

    fn clear_stats(&self) {
        qobject::clear_chrome_stats_native();
    }

    fn cursor_position(&self) -> QString {
        convert(qobject::cursor_position_native())
    }

    fn move_cursor(&self, x: i32, y: i32) -> bool {
        qobject::move_cursor_native(x, y)
    }

    fn mouse_down(&self) -> bool {
        qobject::mouse_down_native()
    }

    fn mouse_up(&self) -> bool {
        qobject::mouse_up_native()
    }

    fn click(&self, x: i32, y: i32) -> bool {
        qobject::click_native(x, y)
    }

    fn set_foreground(&self) -> bool {
        qobject::set_foreground_native()
    }

    fn is_foreground(&self) -> bool {
        qobject::is_foreground_native()
    }

    fn capture(&self, path: &QString, x: i32, y: i32, width: i32, height: i32) -> bool {
        qobject::capture_native(path, x, y, width, height)
    }

    fn capture_window(&self, path: &QString) -> bool {
        qobject::capture_window_native(path)
    }

    fn begin_samples(&self, x: i32, y: i32, width: i32, height: i32) {
        qobject::begin_samples_native(x, y, width, height);
    }

    fn sample(&self, tag: &QString, path: &QString) -> bool {
        qobject::sample_native(tag, path)
    }

    fn sample_report(&self) -> QString {
        convert(qobject::sample_report_native())
    }

    fn top_windows(&self) -> QString {
        convert(qobject::top_windows_native())
    }

    fn flyout_begin(&self, x: i32, y: i32, width: i32, height: i32) {
        qobject::flyout_begin_native(x, y, width, height);
    }

    fn flyout_report(&self) -> QString {
        convert(qobject::flyout_report_native())
    }

    fn maximize_window(&self, via_qt: bool) -> bool {
        qobject::maximize_native_fn(via_qt)
    }

    fn write_results(&self, path: &QString, json: &QString) -> bool {
        let path = path.to_string();
        let Some(parent) = std::path::Path::new(&path).parent() else {
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
        if let Some(parent) = std::path::Path::new(&path).parent() {
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

fn convert_str(value: &str) -> QString {
    QString::from(value)
}

/// Called from `main` before the QML engine creates the first window.
pub fn enable_alpha() {
    qobject::enable_alpha_native();
}
