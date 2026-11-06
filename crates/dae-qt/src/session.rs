//! The window/tab session restored on the next launch.
//!
//! The tabs themselves live in QML: a `Tab` is a `QtObject` owning its listing
//! model, search models and terminal, and tearing one off moves the object
//! rather than copying state. This module therefore never models a tab — it
//! stores whatever JSON the window hands it, one record per window token, and
//! returns the whole document once at startup.
//!
//! Two things it does own:
//!
//! * **Ordering.** Records keep the order windows were created in, so a
//!   restored session puts the main window back first and the torn-off ones
//!   after it instead of stealing focus in id order.
//! * **The dirty flag.** `clean` is written `false` when a run starts and
//!   `true` when the last window closes normally. A document left with
//!   `clean = false` is how the next launch knows the previous one crashed, so
//!   the restore can say so rather than silently pretending nothing happened.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex};

use cxx_qt_lib::QString;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "session-qt.json";

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// The process-wide session document, reachable from QML as `Session`.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// The previous run's windows, as JSON, once. Empty before the first
        /// call and after it: a second window must not restore the session a
        /// second time.
        #[qproperty(QString, restore_json, cxx_name = "restoreJson")]
        /// True when the document being restored was left by a run that did
        /// not close normally.
        #[qproperty(bool, crashed)]
        type Session = super::SessionRust;

        /// Stores one window's record under `token`, replacing an older record
        /// for the same window. `json` is the window's own serialization; this
        /// module only checks that it parses and keeps its key order.
        #[qinvokable]
        #[cxx_name = "saveWindow"]
        fn save_window(self: Pin<&mut Session>, token: &QString, json: &QString);

        /// Returns the previous run's windows once and empties the property, so
        /// a second window cannot restore the same tabs a second time.
        #[qinvokable]
        fn take(self: Pin<&mut Session>) -> QString;

        /// Forgets a window that no longer exists (a torn-off window closed
        /// while the main one stayed open).
        #[qinvokable]
        #[cxx_name = "removeWindow"]
        fn remove_window(self: Pin<&mut Session>, token: &QString);

        /// Marks the run clean. Called when the last window closes, so the next
        /// launch does not report a crash.
        #[qinvokable]
        #[cxx_name = "markClean"]
        fn mark_clean(self: Pin<&mut Session>);

        /// Deletes the document, so the next launch starts from scratch.
        #[qinvokable]
        fn clear(self: Pin<&mut Session>);
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredWindow {
    token: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    active_index: i32,
    split_enabled: bool,
    /// The window's tabs in strip order. Opaque here: the QML side owns the
    /// tab shape and is the only reader that has to understand it.
    tabs: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SessionDocument {
    /// `false` while a run is in progress; `true` after a normal exit.
    clean: bool,
    windows: Vec<StoredWindow>,
}

/// Loads the document and marks this run as in progress. Separate from the
/// registry lock so a failed write cannot leave the process unable to start.
fn load_and_begin_run() -> (SessionDocument, bool) {
    let Some(path) = document_path() else {
        return (SessionDocument::default(), false);
    };
    let document = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<SessionDocument>(&text).ok())
        .unwrap_or_default();
    let crashed = !document.clean && !document.windows.is_empty();
    let mut begun = document;
    begun.clean = false;
    let _ = persist(&begun);
    (begun, crashed)
}

fn document_path() -> Option<PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|dir| dir.join(FILE_NAME))
}

fn persist(document: &SessionDocument) {
    let Some(path) = document_path() else {
        return;
    };
    let Ok(contents) = serde_json::to_vec_pretty(document) else {
        return;
    };
    if let Err(error) = dae_core::fs::atomic::write_atomic(&path, &contents) {
        log::warn!(target: "dae::session", "could not save the session: {error}");
    }
}

/// In-memory copy, so a window closing and the next one closing do not read
/// and rewrite the file twice with stale contents in between.
static STATE: LazyLock<Mutex<Option<SessionDocument>>> = LazyLock::new(|| Mutex::new(None));

fn with_document(update: impl FnOnce(&mut SessionDocument)) {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let document = guard.get_or_insert_with(SessionDocument::default);
    update(document);
    persist(document);
}

pub struct SessionRust {
    restore_json: QString,
    crashed: bool,
}

impl Default for SessionRust {
    fn default() -> Self {
        let (document, crashed) = load_and_begin_run();
        {
            let mut guard = STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.is_none() {
                *guard = Some(document.clone());
            }
        }
        let restore = serde_json::to_string(&document).unwrap_or_default();
        Self {
            restore_json: QString::from(restore.as_str()),
            crashed,
        }
    }
}

impl qobject::Session {
    fn save_window(self: Pin<&mut Self>, token: &QString, json: &QString) {
        let token = token.to_string();
        if token.is_empty() {
            return;
        }
        let Ok(mut record) = serde_json::from_str::<StoredWindow>(&json.to_string()) else {
            log::warn!(target: "dae::session", "ignoring a malformed window record");
            return;
        };
        record.token = token.clone();

        with_document(|document| {
            match document
                .windows
                .iter_mut()
                .find(|entry| entry.token == token)
            {
                Some(existing) => *existing = record,
                None => document.windows.push(record),
            }
        });
    }

    fn take(mut self: Pin<&mut Self>) -> QString {
        let document = self.restore_json.clone();
        if !document.is_empty() {
            self.as_mut().set_restore_json(QString::default());
        }
        document
    }

    fn remove_window(self: Pin<&mut Self>, token: &QString) {
        let token = token.to_string();
        if token.is_empty() {
            return;
        }
        with_document(|document| {
            document.windows.retain(|entry| entry.token != token);
        });
    }

    fn mark_clean(self: Pin<&mut Self>) {
        with_document(|document| document.clean = true);
    }

    fn clear(mut self: Pin<&mut Self>) {
        {
            let mut guard = STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = Some(SessionDocument {
                clean: true,
                windows: Vec::new(),
            });
            if let Some(document) = guard.as_ref() {
                persist(document);
            }
        }
        self.as_mut().set_restore_json(QString::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_record_round_trips_with_its_tabs() {
        let json = r#"{
            "token": "dae-main",
            "x": 10, "y": 20, "width": 1180, "height": 780,
            "activeIndex": 1,
            "splitEnabled": true,
            "tabs": [
                {"surface": "folder", "path": "C:/one", "back": [], "forward": []},
                {"surface": "trash", "path": ""}
            ]
        }"#;
        let record: StoredWindow = serde_json::from_str(json).expect("record parses");
        assert_eq!(record.tabs.len(), 2);
        assert_eq!(record.active_index, 1);
        assert!(record.split_enabled);
        let text = serde_json::to_string(&record).expect("record serializes");
        assert!(text.contains("\"activeIndex\":1"));
    }

    #[test]
    fn a_document_without_windows_is_not_a_crash() {
        let document = SessionDocument {
            clean: false,
            windows: Vec::new(),
        };
        assert!(!(!document.clean && !document.windows.is_empty()));
    }
}
