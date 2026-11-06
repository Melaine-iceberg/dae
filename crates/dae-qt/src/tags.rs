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
}
