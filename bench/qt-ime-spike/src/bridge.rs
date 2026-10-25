//! QML-facing wrapper around `cpp/ime_probe.h`.
//!
//! Every method is a thin forwarder. The scripted session lives in QML because
//! that is where `TextInput.preeditText`, `displayText` and focus changes are
//! observable; the probe owns the native side (IMM32, SendInput, screen grab)
//! and the JSON result is written here so QML does not need file APIs.

use cxx_qt_lib::QString;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("ime_probe.h");

        #[namespace = "dae_ime_probe"]
        #[rust_name = "attach_probe"]
        fn attach() -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "focus_native"]
        fn focus_window() -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "is_foreground_native"]
        fn is_foreground() -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "ensure_native_mode_native"]
        fn ensure_native_mode() -> UniquePtr<CxxString>;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "send_key_native"]
        fn send_key(virtual_key: i32) -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "click_item_native"]
        fn click_item(name: &QString) -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "snapshot_native"]
        fn snapshot() -> UniquePtr<CxxString>;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "clear_events_native"]
        fn clear_events();

        #[namespace = "dae_ime_probe"]
        #[rust_name = "grab_native"]
        fn grab(path: &QString) -> bool;

        #[namespace = "dae_ime_probe"]
        #[rust_name = "event_log_native"]
        fn event_log() -> UniquePtr<CxxString>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        type ImeProbe = super::ImeProbeRust;

        #[qinvokable]
        fn attach(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "focusWindow"]
        fn focus_window(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "isForeground"]
        fn is_foreground(&self) -> bool;

        #[qinvokable]
        #[cxx_name = "ensureNativeMode"]
        fn ensure_native_mode(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "sendKey"]
        fn send_key(&self, virtual_key: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "clickItem"]
        fn click_item(&self, name: &QString) -> bool;

        #[qinvokable]
        fn snapshot(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "clearEvents"]
        fn clear_events(&self);

        #[qinvokable]
        fn grab(&self, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "eventLog"]
        fn event_log(&self) -> QString;

        #[qinvokable]
        #[cxx_name = "writeResults"]
        fn write_results(&self, path: &QString, json: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "appendLog"]
        fn append_log(&self, path: &QString, line: &QString) -> bool;
    }
}

#[derive(Default)]
pub struct ImeProbeRust;

impl qobject::ImeProbe {
    fn attach(&self) -> bool {
        qobject::attach_probe()
    }

    fn focus_window(&self) -> bool {
        qobject::focus_native()
    }

    fn is_foreground(&self) -> bool {
        qobject::is_foreground_native()
    }

    fn ensure_native_mode(&self) -> QString {
        convert(qobject::ensure_native_mode_native())
    }

    fn send_key(&self, virtual_key: i32) -> bool {
        qobject::send_key_native(virtual_key)
    }

    fn click_item(&self, name: &QString) -> bool {
        qobject::click_item_native(name)
    }

    fn snapshot(&self) -> QString {
        convert(qobject::snapshot_native())
    }

    fn clear_events(&self) {
        qobject::clear_events_native();
    }

    fn grab(&self, path: &QString) -> bool {
        let value = path.to_string();
        if let Some(parent) = std::path::Path::new(&value).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        qobject::grab_native(path)
    }

    fn event_log(&self) -> QString {
        convert(qobject::event_log_native())
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
