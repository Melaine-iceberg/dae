//! The process-wide session document behind `Session.restoreJson`.
//!
//! One window record per window, written by the view that owns it; the file is
//! what the next launch restores. The JSON is opaque here: the shape belongs to
//! `qml/ShellWindow.qml`, and this module only keeps it ordered and durable.
//!
//! Ported from CXX-Qt to Qt Bridge. The element name, the module and every
//! property/slot name are unchanged, so no QML file knows about the move; the
//! manual [`QmlElement`] impl is what pins them.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use qtbridge::{QmlElement, qobject};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "session.json";

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

/// The process-wide session document, reachable from QML as `Session`.
pub struct Session {
    /// The previous run's windows, as JSON, once. Empty before the first call
    /// and after it: a second window must not restore the session a second
    /// time.
    restore_json: String,
    /// True when the document being restored was left by a run that did not
    /// close normally.
    crashed: bool,
}

impl Default for Session {
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
            restore_json: restore,
            crashed,
        }
    }
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
impl Session {
    qproperty!(
        "restoreJson",
        Member = restore_json,
        Notify = restore_json_changed
    );
    qproperty!("crashed", Member = crashed, Constant);

    #[qsignal]
    fn restore_json_changed(&mut self);

    /// Stores one window's record under `token`, replacing an older record
    /// for the same window. `json` is the window's own serialization; this
    /// module only checks that it parses and keeps its key order.
    #[qslot]
    fn save_window(&mut self, token: String, json: String) {
        if token.is_empty() {
            return;
        }
        let Ok(mut record) = serde_json::from_str::<StoredWindow>(&json) else {
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

    /// Returns the previous run's windows once and empties the property, so a
    /// second window cannot restore the same tabs a second time.
    #[qslot]
    fn take(&mut self) -> String {
        let document = std::mem::take(&mut self.restore_json);
        if !document.is_empty() {
            self.restore_json_changed();
        }
        document
    }

    /// Forgets a window that no longer exists (a torn-off window closed
    /// while the main one stayed open).
    #[qslot]
    fn remove_window(&mut self, token: String) {
        if token.is_empty() {
            return;
        }
        with_document(|document| {
            document.windows.retain(|entry| entry.token != token);
        });
    }

    /// Marks the run clean. Called when the last window closes, so the next
    /// launch does not report a crash.
    #[qslot]
    fn mark_clean(&mut self) {
        with_document(|document| document.clean = true);
    }

    /// Deletes the document, so the next launch starts from scratch.
    #[qslot]
    fn clear(&mut self) {
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
        self.restore_json.clear();
        self.restore_json_changed();
    }
}

impl QmlElement for Session {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Session";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
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
