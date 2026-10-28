//! The sidebar's lazy directory tree.
//!
//! Each volume row can expand into a tree of the folders below it. Nothing is
//! read until a node is expanded, so opening the sidebar stays cheap even when
//! the user has several large drives. Expansion lives here rather than in the
//! delegate: `ListView` recycles delegates, and a delegate-local `expanded`
//! flag would attach itself to whichever volume happened to reuse the object.
//!
//! Reads go through `LocalBackend::read_dir` on the blocking pool. The volumes
//! section is local by construction; network and cloud trees remain flat in
//! the sidebar until those backends get their own listers.

use core::pin::Pin;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::local::LocalBackend;
use dae_core::fs::vfs::FileSystemBackend;
use dae_core::types::EntryKind;

use crate::runtime;

const ROLE_NAME: i32 = 0x0700;
const ROLE_PATH: i32 = 0x0701;
const ROLE_DEPTH: i32 = 0x0702;
const ROLE_EXPANDED: i32 = 0x0703;
const ROLE_MAY_HAVE_CHILDREN: i32 = 0x0704;
const ROLE_LOADED: i32 = 0x0705;

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
        /// A flattened view of one expanded volume tree.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, loaded)]
        #[qproperty(QString, status)]
        type SidebarTreeModel = super::SidebarTreeModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Starts over at `root`; the root itself is implicit (the volume row)
        /// and its direct children are the first rows.
        #[qinvokable]
        fn load(self: Pin<&mut SidebarTreeModel>, root: &QString, show_hidden: bool);

        /// Expands a node and lazily reads its child folders, or collapses it.
        #[qinvokable]
        fn toggle(self: Pin<&mut SidebarTreeModel>, path: &QString);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut SidebarTreeModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut SidebarTreeModel>);
    }

    impl cxx_qt::Threading for SidebarTreeModel {}
}

#[derive(Clone)]
struct TreeChild {
    name: String,
    path: String,
}

#[derive(Clone)]
struct TreeRow {
    name: String,
    path: String,
    depth: i32,
    expanded: bool,
    may_have_children: bool,
    loaded: bool,
}

pub struct SidebarTreeModelRust {
    root: String,
    show_hidden: bool,
    expanded_paths: HashSet<String>,
    children: HashMap<String, Vec<TreeChild>>,
    loaded_paths: HashSet<String>,
    loading_paths: HashSet<String>,
    rows: Vec<TreeRow>,
    count: i32,
    busy: bool,
    loaded: bool,
    status: QString,
    generation: u64,
}

impl Default for SidebarTreeModelRust {
    fn default() -> Self {
        Self {
            root: String::new(),
            // The listing's default: hidden entries are visible until the
            // user turns them off in Settings.
            show_hidden: true,
            expanded_paths: HashSet::new(),
            children: HashMap::new(),
            loaded_paths: HashSet::new(),
            loading_paths: HashSet::new(),
            rows: Vec::new(),
            count: 0,
            busy: false,
            loaded: false,
            status: QString::default(),
            generation: 0,
        }
    }
}

impl qobject::SidebarTreeModel {
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
            ROLE_PATH => QVariant::from(&QString::from(row.path.as_str())),
            ROLE_DEPTH => QVariant::from(&row.depth),
            ROLE_EXPANDED => QVariant::from(&row.expanded),
            ROLE_MAY_HAVE_CHILDREN => QVariant::from(&row.may_have_children),
            ROLE_LOADED => QVariant::from(&row.loaded),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_NAME, QByteArray::from("treeName"));
        roles.insert(ROLE_PATH, QByteArray::from("treePath"));
        roles.insert(ROLE_DEPTH, QByteArray::from("depth"));
        roles.insert(ROLE_EXPANDED, QByteArray::from("expanded"));
        roles.insert(ROLE_MAY_HAVE_CHILDREN, QByteArray::from("mayHaveChildren"));
        roles.insert(ROLE_LOADED, QByteArray::from("loaded"));
        roles
    }

    fn load(mut self: Pin<&mut Self>, root: &QString, show_hidden: bool) {
        let generation = self.generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().rust_mut().root = root.to_string();
        self.as_mut().rust_mut().show_hidden = show_hidden;
        self.as_mut().rust_mut().expanded_paths.clear();
        self.as_mut().rust_mut().children.clear();
        self.as_mut().rust_mut().loaded_paths.clear();
        self.as_mut().rust_mut().loading_paths.clear();
        self.as_mut().set_rows(Vec::new());
        self.as_mut().set_loaded(false);
        self.as_mut().set_busy(false);
        self.as_mut().set_status(QString::default());

        let root = root.to_string();
        if root.is_empty() {
            return;
        }

        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from("Loading folder…"));
        self.as_mut().request_children(root, generation);
    }

    fn toggle(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if path.is_empty() || path == self.root {
            return;
        }

        if self.as_mut().rust_mut().expanded_paths.remove(&path) {
            self.as_mut().rebuild();
            return;
        }

        self.as_mut().rust_mut().expanded_paths.insert(path.clone());
        if self.children.contains_key(&path) {
            self.as_mut().rebuild();
            return;
        }
        if self.loading_paths.contains(&path) {
            return;
        }

        let generation = self.generation;
        self.as_mut().set_busy(true);
        self.as_mut().request_children(path, generation);
    }

    fn request_children(mut self: Pin<&mut Self>, path: String, generation: u64) {
        self.as_mut().rust_mut().loading_paths.insert(path.clone());
        let show_hidden = self.show_hidden;
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let target = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                let backend = LocalBackend;
                backend
                    .read_dir(&target)
                    .map(|view| children_from(view.entries, show_hidden))
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);

            let _ = thread.queue(move |mut model| {
                model.as_mut().apply_children(generation, path, result);
            });
        });
    }

    fn apply_children(
        mut self: Pin<&mut Self>,
        generation: u64,
        path: String,
        result: Result<Vec<TreeChild>, String>,
    ) {
        if self.generation != generation {
            return;
        }

        self.as_mut().rust_mut().loading_paths.remove(&path);
        let root = self.root.clone();
        let is_root = path == root;

        match result {
            Ok(children) => {
                let count = children.len();
                self.as_mut()
                    .rust_mut()
                    .children
                    .insert(path.clone(), children);
                self.as_mut().rust_mut().loaded_paths.insert(path.clone());
                if is_root {
                    self.as_mut().set_loaded(true);
                    self.as_mut()
                        .set_status(QString::from(tree_status(count).as_str()));
                }
            }
            Err(error) => {
                self.as_mut()
                    .rust_mut()
                    .children
                    .insert(path.clone(), Vec::new());
                self.as_mut().rust_mut().loaded_paths.insert(path.clone());
                if is_root {
                    self.as_mut().set_loaded(true);
                    self.as_mut()
                        .set_status(QString::from("Could not read folder"));
                    eprintln!("dae: sidebar tree read failed: {error}");
                }
            }
        }

        if is_root || self.expanded_paths.contains(&path) {
            self.as_mut().rebuild();
        }
        if self.loading_paths.is_empty() {
            self.as_mut().set_busy(false);
        }
    }

    fn rebuild(mut self: Pin<&mut Self>) {
        let rows = flatten(
            &self.root,
            &self.expanded_paths,
            &self.children,
            &self.loaded_paths,
        );
        let count = rows.len() as i32;
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_count(count);
    }

    fn set_rows(mut self: Pin<&mut Self>, rows: Vec<TreeRow>) {
        let count = rows.len() as i32;
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_count(count);
    }
}

fn children_from(
    entries: Vec<dae_core::types::DirectoryEntry>,
    show_hidden: bool,
) -> Vec<TreeChild> {
    let mut children: Vec<TreeChild> = entries
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::Directory && (show_hidden || !entry.hidden))
        .map(|entry| TreeChild {
            name: entry.name,
            path: entry.path,
        })
        .collect();
    children.sort_by(|left, right| natural_cmp(&left.name, &right.name));
    children
}

fn tree_status(count: usize) -> String {
    if count == 0 {
        "No subfolders".to_owned()
    } else {
        format!("Subfolders: {count}")
    }
}

fn flatten(
    root: &str,
    expanded: &HashSet<String>,
    children: &HashMap<String, Vec<TreeChild>>,
    loaded: &HashSet<String>,
) -> Vec<TreeRow> {
    fn collect(
        parent: &str,
        depth: i32,
        expanded: &HashSet<String>,
        children: &HashMap<String, Vec<TreeChild>>,
        loaded: &HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        let Some(nodes) = children.get(parent) else {
            return;
        };
        for child in nodes {
            let is_expanded = expanded.contains(&child.path);
            let is_loaded = loaded.contains(&child.path);
            let has_children = children
                .get(&child.path)
                .map(|grandchildren| !grandchildren.is_empty())
                .unwrap_or(false);
            rows.push(TreeRow {
                name: child.name.clone(),
                path: child.path.clone(),
                depth,
                expanded: is_expanded,
                may_have_children: !is_loaded || has_children,
                loaded: is_loaded,
            });
            if is_expanded {
                collect(&child.path, depth + 1, expanded, children, loaded, rows);
            }
        }
    }

    let mut rows = Vec::new();
    collect(root, 0, expanded, children, loaded, &mut rows);
    rows
}

/// Case-insensitive name order. The backend already returns a stable order;
/// this only makes the sidebar's tree predictable across filesystems and
/// locale-independent, which the collator version in the old shell was not.
fn natural_cmp(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(name: &str, path: &str) -> TreeChild {
        TreeChild {
            name: name.to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn only_folders_survive_and_hidden_ones_follow_the_preference() {
        use dae_core::types::DirectoryEntry;

        let entry = |name: &str, kind: EntryKind, hidden: bool| DirectoryEntry {
            name: name.to_owned(),
            path: format!("C:/root/{name}"),
            kind,
            modified_at: None,
            size: None,
            hidden,
            read_only: false,
        };
        let entries = vec![
            entry("b", EntryKind::Directory, false),
            entry("a", EntryKind::Directory, false),
            entry(".hidden", EntryKind::Directory, true),
            entry("file.txt", EntryKind::File, false),
        ];

        let visible = children_from(entries.clone(), false);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "a");

        let all = children_from(entries, true);
        assert_eq!(all.len(), 3);
        assert!(!all.iter().any(|child| child.name == "file.txt"));
    }

    #[test]
    fn the_flattened_rows_follow_expansion_depth_first() {
        let mut children = HashMap::new();
        children.insert(
            "C:/root".to_owned(),
            vec![child("a", "C:/root/a"), child("b", "C:/root/b")],
        );
        children.insert("C:/root/a".to_owned(), vec![child("a1", "C:/root/a/a1")]);
        let mut expanded = HashSet::new();
        expanded.insert("C:/root/a".to_owned());
        let mut loaded = HashSet::new();
        loaded.insert("C:/root".to_owned());
        loaded.insert("C:/root/a".to_owned());

        let rows = flatten("C:/root", &expanded, &children, &loaded);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["a", "a1", "b"]);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].path, "C:/root/a/a1");
        assert!(rows[0].expanded);
        assert!(!rows[2].expanded);
    }
}
