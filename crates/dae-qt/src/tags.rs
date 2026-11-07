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

use std::path::Path;
use std::pin::Pin;

use cxx_qt_lib::{QString, QStringList};
use dae_core::fs::tags as tag_core;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    unsafe extern "RustQt" {
        /// QML usage: `TagStore.tagsJson`, `TagStore.setTags(paths, ids)`.
        ///
        /// `tagsJson` is a JSON array of `{ id, name, color, count }`, so the
        /// sidebar, the editor and the context menu render from one value and
        /// cannot disagree about a definition.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, tags_json, cxx_name = "tagsJson")]
        /// The sentence the editor shows after a write that could not store
        /// the tags with the file ("this location keeps them in dae only"), or
        /// the empty string.
        #[qproperty(QString, note)]
        type TagStore = super::TagStoreRust;

        #[qinvokable]
        fn reload(self: Pin<&mut TagStore>);

        /// Creates a definition and returns its id, or an empty string with
        /// `note` carrying the reason.
        #[qinvokable]
        #[cxx_name = "createTag"]
        fn create_tag(self: Pin<&mut TagStore>, name: &QString, color: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "updateTag"]
        fn update_tag(
            self: Pin<&mut TagStore>,
            id: &QString,
            name: &QString,
            color: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "removeTag"]
        fn remove_tag(self: Pin<&mut TagStore>, id: &QString) -> QString;

        /// Replaces the tags of every path with `ids`. Empty `ids` clears.
        /// Returns the empty string on success.
        #[qinvokable]
        #[cxx_name = "setTags"]
        fn set_tags(self: Pin<&mut TagStore>, paths: &QStringList, ids: &QStringList) -> QString;

        /// Adds `id` to every path that lacks it, removes it from every path
        /// that has it — one call for a mixed selection.
        #[qinvokable]
        #[cxx_name = "toggleTag"]
        fn toggle_tag(self: Pin<&mut TagStore>, paths: &QStringList, id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "clearTags"]
        fn clear_tags(self: Pin<&mut TagStore>, paths: &QStringList) -> QString;

        /// The definitions every path in `paths` carries, as the same JSON
        /// shape as `tagsJson`. The multi-selection details pane shows what
        /// the selection has in common, not one arbitrary member's tags.
        #[qinvokable]
        #[cxx_name = "commonTagsJson"]
        fn common_tags_json(self: Pin<&mut TagStore>, paths: &QStringList) -> QString;

        /// Writes the definitions to `path` in the same `tags.toml` format the
        /// config directory holds. Answers `{"ok":true}` or
        /// `{"ok":false,"error":"…"}`.
        #[qinvokable]
        #[cxx_name = "exportDefinitions"]
        fn export_definitions(self: Pin<&mut TagStore>, path: &QString) -> QString;

        /// Merges a `tags.toml` document at `path` into the definitions: a
        /// known id overwrites name and colour, an unknown one is appended.
        /// Answers `{"ok":true,"added":n,"updated":n,"skipped":[…]}` or
        /// `{"ok":false,"error":"…"}`.
        #[qinvokable]
        #[cxx_name = "importDefinitions"]
        fn import_definitions(self: Pin<&mut TagStore>, path: &QString) -> QString;

        /// Reconciles the tag index with the files it names: drops entries for
        /// deleted files, adopts ids from the file's own stream, follows
        /// renames, and quarantines shard documents that no longer parse.
        /// Answers `{"ok":true,"entries":n,"droppedMissing":n,…}` or
        /// `{"ok":false,"error":"…"}`.
        #[qinvokable]
        #[cxx_name = "rebuildIndex"]
        fn rebuild_index(self: Pin<&mut TagStore>) -> QString;

        /// Emitted after a definition or a count changed. Listings answer it
        /// by re-reading `tags.toml`; the sidebar answers by re-reading
        /// `tagsJson`.
        #[qsignal]
        #[cxx_name = "definitionsChanged"]
        fn definitions_changed(self: Pin<&mut TagStore>);

        /// The paths whose ids were just written, so each visible listing can
        /// repaint those rows (or re-sort, when it is ordered by tag).
        #[qsignal]
        #[cxx_name = "tagsChanged"]
        fn tags_changed(self: Pin<&mut TagStore>, paths: &QStringList);
    }
}

pub struct TagStoreRust {
    tags_json: QString,
    note: QString,
}

impl Default for TagStoreRust {
    fn default() -> Self {
        Self {
            tags_json: snapshot(),
            note: QString::default(),
        }
    }
}

/// The definitions with their counts, as the one string QML reads.
fn snapshot() -> QString {
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
    QString::from(
        serde_json::to_string(&rows)
            .unwrap_or_else(|_| "[]".to_owned())
            .as_str(),
    )
}

fn store() -> Option<tag_core::TagStore> {
    tag_core::TagStore::app()
}

fn failure(message: String) -> QString {
    QString::from(message.as_str())
}

fn paths_list(paths: &QStringList) -> Vec<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

fn ids_list(ids: &QStringList) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// What the note says after a write that could not put the tags on the file.
const INDEX_ONLY_NOTE: &str = "此位置无法随文件保存标签，标签只在 dae 内生效";

impl qobject::TagStore {
    fn reload(mut self: Pin<&mut Self>) {
        self.as_mut().set_tags_json(snapshot());
    }

    fn create_tag(mut self: Pin<&mut Self>, name: &QString, color: &QString) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        match store.create_tag(&name.to_string(), &color.to_string()) {
            Ok((definition, _)) => {
                self.as_mut().set_tags_json(snapshot());
                self.as_mut().set_note(QString::default());
                self.as_mut().definitions_changed();
                QString::from(definition.id.as_str())
            }
            Err(error) => {
                self.as_mut().set_note(failure(error.to_string()));
                QString::default()
            }
        }
    }

    fn update_tag(
        mut self: Pin<&mut Self>,
        id: &QString,
        name: &QString,
        color: &QString,
    ) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        match store.update_tag(&id.to_string(), &name.to_string(), &color.to_string()) {
            Ok(_) => {
                self.as_mut().set_tags_json(snapshot());
                self.as_mut().set_note(QString::default());
                self.as_mut().definitions_changed();
                QString::default()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    fn remove_tag(mut self: Pin<&mut Self>, id: &QString) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        match store.remove_tag(&id.to_string()) {
            Ok(_) => {
                self.as_mut().set_tags_json(snapshot());
                self.as_mut().set_note(QString::default());
                self.as_mut().definitions_changed();
                QString::default()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    fn set_tags(mut self: Pin<&mut Self>, paths: &QStringList, ids: &QStringList) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        let paths = paths_list(paths);
        if paths.is_empty() {
            return QString::default();
        }
        match store.set_tags(&paths, &ids_list(ids)) {
            Ok(report) => {
                let note = if report.file_store_unavailable {
                    INDEX_ONLY_NOTE.to_owned()
                } else {
                    String::new()
                };
                let mut message = QString::from(note.as_str());
                if !report.failed.is_empty() {
                    message = failure(format!("{} 项无法写入标签", report.failed.len()));
                }
                self.as_mut().set_note(message);
                self.as_mut().set_tags_json(snapshot());
                self.as_mut().definitions_changed();
                self.as_mut().tags_changed(&qstringlist(&paths));
                QString::default()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    fn toggle_tag(mut self: Pin<&mut Self>, paths: &QStringList, id: &QString) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        let paths = paths_list(paths);
        let id = id.to_string();
        if paths.is_empty() || id.is_empty() {
            return QString::default();
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

        self.as_mut().set_note(QString::from(
            if failed > 0 {
                format!("{failed} 项无法写入标签")
            } else if index_only {
                INDEX_ONLY_NOTE.to_owned()
            } else {
                String::new()
            }
            .as_str(),
        ));
        self.as_mut().set_tags_json(snapshot());
        self.as_mut().definitions_changed();
        self.as_mut().tags_changed(&qstringlist(&paths));
        QString::default()
    }

    fn common_tags_json(self: Pin<&mut Self>, paths: &QStringList) -> QString {
        let paths = paths_list(paths);
        let Some(store) = store() else {
            return QString::from("[]");
        };
        if paths.is_empty() {
            return QString::from("[]");
        }
        let per_path = store.tags_for(&paths);
        let Some(first) = per_path.first() else {
            return QString::from("[]");
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
        QString::from(
            serde_json::to_string(&rows)
                .unwrap_or("[]".to_owned())
                .as_str(),
        )
    }

    fn clear_tags(mut self: Pin<&mut Self>, paths: &QStringList) -> QString {
        let Some(store) = store() else {
            return failure("无法定位配置目录".to_owned());
        };
        let paths = paths_list(paths);
        if paths.is_empty() {
            return QString::default();
        }
        match store.set_tags(&paths, &[]) {
            Ok(_) => {
                self.as_mut().set_note(QString::default());
                self.as_mut().set_tags_json(snapshot());
                self.as_mut().definitions_changed();
                self.as_mut().tags_changed(&qstringlist(&paths));
                QString::default()
            }
            Err(error) => failure(error.to_string()),
        }
    }

    fn export_definitions(self: Pin<&mut Self>, path: &QString) -> QString {
        let Some(store) = store() else {
            return json_error("无法定位配置目录");
        };
        export_reply(&store, &path.to_string())
    }

    fn import_definitions(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let Some(store) = store() else {
            return json_error("无法定位配置目录");
        };
        let reply = import_reply(&store, &path.to_string());
        if reply_ok(&reply) {
            self.as_mut().set_tags_json(snapshot());
            self.as_mut().set_note(QString::default());
            self.as_mut().definitions_changed();
        }
        reply
    }

    fn rebuild_index(mut self: Pin<&mut Self>) -> QString {
        let Some(store) = store() else {
            return json_error("无法定位配置目录");
        };
        let reply = rebuild_reply(&store);
        if reply_ok(&reply) {
            // Counts and names can move, so the sidebar and every open listing
            // re-read the definitions after a rebuild.
            self.as_mut().set_tags_json(snapshot());
            self.as_mut().set_note(QString::default());
            self.as_mut().definitions_changed();
        }
        reply
    }
}

/// The three replies below are free functions over a store so the JSON
/// contract can be tested without a QML engine; the invokables only add the
/// signals. Every reply is `{"ok":true,…}` or `{"ok":false,"error":"…"}`.
fn export_reply(store: &tag_core::TagStore, path: &str) -> QString {
    match store.export_to(Path::new(path)) {
        Ok(()) => json_ok(serde_json::json!({ "ok": true })),
        Err(error) => json_error(&error.to_string()),
    }
}

fn import_reply(store: &tag_core::TagStore, path: &str) -> QString {
    match store.import_from(Path::new(path)) {
        Ok(report) => json_ok(serde_json::json!({
            "ok": true,
            "added": report.added,
            "updated": report.updated,
            "skipped": report.skipped,
        })),
        Err(error) => json_error(&error.to_string()),
    }
}

fn rebuild_reply(store: &tag_core::TagStore) -> QString {
    match store.repair_index() {
        Ok(report) => json_ok(serde_json::json!({
            "ok": true,
            "entries": report.entries,
            "droppedMissing": report.dropped_missing,
            "droppedDuplicate": report.dropped_duplicate,
            "adoptedFromFile": report.adopted_from_file,
            "corruptShards": report.corrupt_shards,
        })),
        Err(error) => json_error(&error.to_string()),
    }
}

fn reply_ok(reply: &QString) -> bool {
    serde_json::from_str::<serde_json::Value>(&reply.to_string())
        .ok()
        .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// The one success shape the settings page parses. `json!` rather than a
/// struct because each call answers different counters, and a second mirror
/// of the core's report types would be a second definition to keep in sync.
fn json_ok(value: serde_json::Value) -> QString {
    QString::from(
        serde_json::to_string(&value)
            .unwrap_or_else(|_| "{\"ok\":true}".to_owned())
            .as_str(),
    )
}

fn json_error(message: &str) -> QString {
    QString::from(
        serde_json::json!({ "ok": false, "error": message })
            .to_string()
            .as_str(),
    )
}

/// Rebuilds the `QStringList` the signal carries from the paths the core
/// reported.
fn qstringlist(paths: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for path in paths {
        list.append(QString::from(path.as_str()));
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_is_a_json_array_of_definitions() {
        // The process-wide store reads the real config directory; the test
        // only asserts the shape, not the user's tags.
        let json = snapshot().to_string();
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
        let parsed: serde_json::Value = serde_json::from_str(&reply.to_string()).expect("json");
        assert_eq!(parsed["ok"], false);
        assert!(
            parsed["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );

        let incoming = dir.path().join("incoming.toml");
        std::fs::write(&incoming, "[[tags]]\nid = \"urgent\"\nname = \"Urgent\"\n").expect("write");
        let reply = import_reply(&store, &incoming.to_string_lossy());
        let parsed: serde_json::Value = serde_json::from_str(&reply.to_string()).expect("json");
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
        let parsed: serde_json::Value = serde_json::from_str(&reply.to_string()).expect("json");
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["entries"], 0);
        assert_eq!(parsed["droppedMissing"], 1);
    }
}
