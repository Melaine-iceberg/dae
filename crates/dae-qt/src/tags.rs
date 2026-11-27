//! The tag UI's one entry point.
//!
//! The definitions, their counts and every write go through this singleton.
//! It is deliberately not a model: the tag list is a handful of rows, and QML
//! already parses JSON for the palette, so the definitions travel as one
//! `tagsJson` string that rebinds as a whole. Listings listen to
//! `tagsChanged(paths)` and repaint only the rows the editor touched.
//!
//! The storage itself — `tags.toml`, the per-entry stream/xattr and the
//! per-volume index — is `dae_core::fs::tags`; this file only carries its
//! answers across the bridge.
//!
//! Ported from CXX-Qt to Qt Bridge. Element, property and slot names are
//! unchanged (`TagStore.tagsJson`, `TagStore.setTags(paths, ids)`); the manual
//! [`QmlElement`] impl pins the module name so the QML tree keeps importing
//! `dae 1.0`.

use std::path::Path;

use dae_core::fs::tags as tag_core;
use qtbridge::{QmlElement, qobject};

/// `tagsJson` is a JSON array of `{ id, name, color, count }`, so the sidebar,
/// the editor and the context menu render from one value and cannot disagree
/// about a definition.
pub struct TagStore {
    tags_json: String,
    /// The sentence the editor shows after a write that could not store the
    /// tags with the file ("this location keeps them in dae only"), or the
    /// empty string.
    note: String,
}

impl Default for TagStore {
    fn default() -> Self {
        Self {
            tags_json: snapshot(),
            note: String::new(),
        }
    }
}

/// The definitions with their counts, as the one string QML reads.
fn snapshot() -> String {
    let definitions = tag_core::definitions();
    let counts = tag_core::counts();
    let rows: Vec<serde_json::Value> = definitions
        .iter()
        .map(|definition| {
            serde_json::json!({
                "id": definition.id,
                "name": definition.name,
                "color": definition.color,
                "count": counts.get(&definition.id).copied().unwrap_or(0),
            })
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_owned())
}

fn store() -> Option<tag_core::TagStore> {
    tag_core::TagStore::app()
}

fn failure(message: String) -> String {
    crate::errors::friendly_error(&message)
}

fn write_failed_message(count: usize) -> String {
    if count == 1 {
        "1 item could not be tagged".to_owned()
    } else {
        format!("{count} items could not be tagged")
    }
}

const MISSING_CONFIG_DIR: &str = "Could not locate the configuration directory";

/// What the note says after a write that could not put the tags on the file.
const INDEX_ONLY_NOTE: &str =
    "Tags cannot be saved with files at this location and only apply within dae";

#[qobject(NoQmlElement, ConvertToCamelCase)]
impl TagStore {
    qproperty!("tagsJson", Member = tags_json, Notify = tags_json_changed);
    qproperty!("note", Member = note, Notify = note_changed);

    /// Emitted after a definition or a count changed. Listings answer it by
    /// re-reading `tags.toml`; the sidebar answers by re-reading `tagsJson`.
    #[qsignal]
    fn definitions_changed(&mut self);

    /// The paths whose ids were just written, so each visible listing can
    /// repaint those rows (or re-sort, when it is ordered by tag).
    #[qsignal]
    fn tags_changed(&mut self, paths: Vec<String>);

    #[qsignal]
    fn tags_json_changed(&mut self);

    #[qsignal]
    fn note_changed(&mut self);

    #[qslot]
    fn reload(&mut self) {
        self.tags_json = snapshot();
        self.tags_json_changed();
    }

    /// Creates a definition and returns its id, or an empty string with
    /// `note` carrying the reason.
    #[qslot]
    fn create_tag(&mut self, name: String, color: String) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        match store.create_tag(&name, &color) {
            Ok((definition, _)) => {
                self.tags_json = snapshot();
                self.tags_json_changed();
                self.note.clear();
                self.note_changed();
                self.definitions_changed();
                definition.id
            }
            Err(error) => {
                self.note = failure(error.to_string());
                self.note_changed();
                String::new()
            }
        }
    }

    #[qslot]
    fn update_tag(&mut self, id: String, name: String, color: String) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        match store.update_tag(&id, &name, &color) {
            Ok(_) => {
                self.tags_json = snapshot();
                self.tags_json_changed();
                self.note.clear();
                self.note_changed();
                self.definitions_changed();
                String::new()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    #[qslot]
    fn remove_tag(&mut self, id: String) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        match store.remove_tag(&id) {
            Ok(_) => {
                self.tags_json = snapshot();
                self.tags_json_changed();
                self.note.clear();
                self.note_changed();
                self.definitions_changed();
                String::new()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    /// Replaces the tags of every path with `ids`. Empty `ids` clears.
    /// Returns the empty string on success.
    #[qslot]
    fn set_tags(&mut self, paths: Vec<String>, ids: Vec<String>) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        if paths.is_empty() {
            return String::new();
        }
        match store.set_tags(&paths, &ids) {
            Ok(report) => {
                let note = if report.file_store_unavailable {
                    INDEX_ONLY_NOTE.to_owned()
                } else {
                    String::new()
                };
                let mut message = note;
                if !report.failed.is_empty() {
                    message = write_failed_message(report.failed.len());
                }
                self.note = message;
                self.note_changed();
                self.tags_json = snapshot();
                self.tags_json_changed();
                self.definitions_changed();
                self.tags_changed(paths);
                String::new()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    /// Adds `id` to every path that lacks it, removes it from every path
    /// that has it — one call for a mixed selection.
    #[qslot]
    fn toggle_tag(&mut self, paths: Vec<String>, id: String) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        if paths.is_empty() || id.is_empty() {
            return String::new();
        }
        // Read once for the batch, then write each path with the one id
        // flipped. A mixed selection converges on "all tagged" or "all clear"
        // depending on the first path, which is what a single toggle on a
        // mixed selection has to mean.
        let current = store.tags_for(&paths);
        let adding = current
            .first()
            .is_none_or(|entry| !entry.tag_ids.contains(&id));

        let mut failed = 0;
        let mut index_only = false;
        for (path, entry) in paths.iter().zip(current.iter()) {
            let mut ids = entry.tag_ids.clone();
            if adding {
                if !ids.contains(&id) {
                    ids.push(id.clone());
                }
            } else {
                ids.retain(|tag| *tag != id);
            }
            match store.set_tags(std::slice::from_ref(path), &ids) {
                Ok(report) => index_only |= report.file_store_unavailable,
                Err(_) => failed += 1,
            }
        }

        self.note = if failed > 0 {
            write_failed_message(failed)
        } else if index_only {
            INDEX_ONLY_NOTE.to_owned()
        } else {
            String::new()
        };
        self.note_changed();
        self.tags_json = snapshot();
        self.tags_json_changed();
        self.definitions_changed();
        self.tags_changed(paths);
        String::new()
    }

    #[qslot]
    fn clear_tags(&mut self, paths: Vec<String>) -> String {
        let Some(store) = store() else {
            return failure(MISSING_CONFIG_DIR.to_owned());
        };
        if paths.is_empty() {
            return String::new();
        }
        match store.set_tags(&paths, &[]) {
            Ok(_) => {
                self.note.clear();
                self.note_changed();
                self.tags_json = snapshot();
                self.tags_json_changed();
                self.definitions_changed();
                self.tags_changed(paths);
                String::new()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    /// The definitions every path in `paths` carries, as the same JSON shape
    /// as `tagsJson`. The multi-selection details pane shows what the
    /// selection has in common, not one arbitrary member's tags.
    #[qslot]
    fn common_tags_json(&mut self, paths: Vec<String>) -> String {
        let Some(store) = store() else {
            return "[]".to_owned();
        };
        if paths.is_empty() {
            return "[]".to_owned();
        }
        let per_path = store.tags_for(&paths);
        let Some(first) = per_path.first() else {
            return "[]".to_owned();
        };
        let common: Vec<String> = first
            .tag_ids
            .iter()
            .filter(|id| per_path.iter().all(|entry| entry.tag_ids.contains(id)))
            .cloned()
            .collect();
        let rows: Vec<serde_json::Value> = store
            .definitions()
            .iter()
            .filter(|definition| common.contains(&definition.id))
            .map(|definition| {
                serde_json::json!({
                    "id": definition.id,
                    "name": definition.name,
                    "color": definition.color,
                })
            })
            .collect();
        serde_json::to_string(&rows).unwrap_or("[]".to_owned())
    }

    /// Writes the definitions to `path` in the same `tags.toml` format the
    /// config directory holds. Answers `{"ok":true}` or
    /// `{"ok":false,"error":"…"}`.
    #[qslot]
    fn export_definitions(&mut self, path: String) -> String {
        let Some(store) = store() else {
            return json_error(MISSING_CONFIG_DIR);
        };
        export_reply(&store, &path)
    }

    /// Merges a `tags.toml` document at `path` into the definitions: a
    /// known id overwrites name and colour, an unknown one is appended.
    /// Answers `{"ok":true,"added":n,"updated":n,"skipped":[…]}` or
    /// `{"ok":false,"error":"…"}`.
    #[qslot]
    fn import_definitions(&mut self, path: String) -> String {
        let Some(store) = store() else {
            return json_error(MISSING_CONFIG_DIR);
        };
        let reply = import_reply(&store, &path);
        if reply_ok(&reply) {
            self.tags_json = snapshot();
            self.tags_json_changed();
            self.note.clear();
            self.note_changed();
            self.definitions_changed();
        }
        reply
    }

    /// Reconciles the tag index with the files it names: drops entries for
    /// deleted files, adopts ids from the file's own stream, follows
    /// renames, and quarantines shard documents that no longer parse.
    /// Answers `{"ok":true,"entries":n,"droppedMissing":n,…}` or
    /// `{"ok":false,"error":"…"}`.
    #[qslot]
    fn rebuild_index(&mut self) -> String {
        let Some(store) = store() else {
            return json_error(MISSING_CONFIG_DIR);
        };
        let reply = rebuild_reply(&store);
        if reply_ok(&reply) {
            // Counts and names can move, so the sidebar and every open listing
            // re-read the definitions after a rebuild.
            self.tags_json = snapshot();
            self.tags_json_changed();
            self.note.clear();
            self.note_changed();
            self.definitions_changed();
        }
        reply
    }
}

impl QmlElement for TagStore {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "TagStore";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}

/// The three replies below are free functions over a store so the JSON
/// contract can be tested without a QML engine; the invokables only add the
/// signals. Every reply is `{"ok":true,…}` or `{"ok":false,"error":"…"}`.
fn export_reply(store: &tag_core::TagStore, path: &str) -> String {
    match store.export_to(Path::new(path)) {
        Ok(()) => json_ok(serde_json::json!({ "ok": true })),
        Err(error) => json_error(&failure(error.to_string())),
    }
}

fn import_reply(store: &tag_core::TagStore, path: &str) -> String {
    match store.import_from(Path::new(path)) {
        Ok(report) => json_ok(serde_json::json!({
            "ok": true,
            "added": report.added,
            "updated": report.updated,
            "skipped": report.skipped,
        })),
        Err(error) => json_error(&failure(error.to_string())),
    }
}

fn rebuild_reply(store: &tag_core::TagStore) -> String {
    match store.repair_index() {
        Ok(report) => json_ok(serde_json::json!({
            "ok": true,
            "entries": report.entries,
            "droppedMissing": report.dropped_missing,
            "droppedDuplicate": report.dropped_duplicate,
            "adoptedFromFile": report.adopted_from_file,
            "corruptShards": report.corrupt_shards,
        })),
        Err(error) => json_error(&failure(error.to_string())),
    }
}

fn reply_ok(reply: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(reply)
        .ok()
        .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// The one success shape the settings page parses. `json!` rather than a
/// struct because each call answers different counters, and a second mirror
/// of the core's report types would be a second definition to keep in sync.
fn json_ok(value: serde_json::Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":true}".to_owned())
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_is_a_json_array_of_definitions() {
        // The process-wide store reads the real config directory; the test
        // only asserts the shape, not the user's tags.
        let json = snapshot();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(parsed.is_array());
    }

    #[test]
    fn the_import_and_export_replies_are_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = tag_core::TagStore::new(dir.path().join("config"));
        store.create_tag("Work", "").expect("create");

        let export = dir.path().join("library.toml");
        assert!(reply_ok(&export_reply(&store, &export.to_string_lossy())));
        assert!(export.exists());

        // A path whose parent does not exist is an error, and the error still
        // arrives as the one reply shape.
        let unwritable = dir.path().join("no/such/directory/x.toml");
        let reply = export_reply(&store, &unwritable.to_string_lossy());
        let parsed: serde_json::Value = serde_json::from_str(&reply).expect("json");
        assert_eq!(parsed["ok"], false);
        assert!(
            parsed["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );

        let incoming = dir.path().join("incoming.toml");
        std::fs::write(&incoming, "[[tags]]\nid = \"urgent\"\nname = \"Urgent\"\n").expect("write");
        let reply = import_reply(&store, &incoming.to_string_lossy());
        let parsed: serde_json::Value = serde_json::from_str(&reply).expect("json");
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["added"], 1);
        assert_eq!(parsed["updated"], 0);
        assert_eq!(store.definitions().len(), 2);
    }

    #[test]
    fn the_rebuild_reply_reports_the_repair_counters() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = tag_core::TagStore::new(dir.path().join("config"));
        let (work, _) = store.create_tag("Work", "").expect("create");
        let file = dir.path().join("note.txt");
        std::fs::write(&file, b"hello").expect("write");
        let path = file.to_string_lossy().into_owned();
        store.set_tags(&[path], &[work.id]).expect("set");
        std::fs::remove_file(&file).expect("delete");

        let reply = rebuild_reply(&store);
        let parsed: serde_json::Value = serde_json::from_str(&reply).expect("json");
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["entries"], 0);
        assert_eq!(parsed["droppedMissing"], 1);
    }
}
