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
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant,
};
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

        include!("cxx-qt-lib/core/qlist/qlist_i32.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
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

        /// A folder that the user asked to expand could not be read. The row
        /// goes back to its closed state and keeps its chevron, but it has no
        /// children to show, so the shell has to say why: a silent no-op reads
        /// as a broken chevron.
        #[qsignal]
        #[cxx_name = "readFailed"]
        fn read_failed(self: Pin<&mut SidebarTreeModel>, path: &QString, message: &QString);

        #[inherit]
        #[rust_name = "begin_insert_rows"]
        unsafe fn beginInsertRows(
            self: Pin<&mut SidebarTreeModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_insert_rows"]
        unsafe fn endInsertRows(self: Pin<&mut SidebarTreeModel>);

        #[inherit]
        #[rust_name = "begin_remove_rows"]
        unsafe fn beginRemoveRows(
            self: Pin<&mut SidebarTreeModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_remove_rows"]
        unsafe fn endRemoveRows(self: Pin<&mut SidebarTreeModel>);

        #[inherit]
        #[rust_name = "model_index"]
        unsafe fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[rust_name = "data_changed"]
        unsafe fn dataChanged(
            self: Pin<&mut SidebarTreeModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

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

#[derive(Clone, PartialEq, Eq)]
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
        // Turn the chevron now rather than when the read finishes: the folder
        // is open from the moment it is clicked, and a blocking read on a slow
        // disk would otherwise leave the click looking ignored.
        self.as_mut().rebuild();
        if self.children.contains_key(&path) || self.loading_paths.contains(&path) {
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
        let failed = result.is_err();

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
                let message = crate::errors::friendly_error(&error);
                if is_root {
                    // The volume row's tree stays open on an empty list so the
                    // status below it can say why nothing arrived.
                    self.as_mut()
                        .rust_mut()
                        .children
                        .insert(path.clone(), Vec::new());
                    self.as_mut().rust_mut().loaded_paths.insert(path.clone());
                    self.as_mut().set_loaded(true);
                    self.as_mut()
                        .set_status(QString::from("Could not read folder"));
                } else {
                    // A failed folder is not an open folder. Close the branch
                    // and leave it out of `loaded_paths`, so its chevron stays
                    // and the next click retries the read instead of reusing
                    // an empty child list.
                    self.as_mut().rust_mut().expanded_paths.remove(&path);
                    self.as_mut().read_failed(
                        &QString::from(path.as_str()),
                        &QString::from(message.as_str()),
                    );
                }
                log::warn!(target: "dae::sidebar", "sidebar tree read failed: {message}");
            }
        }

        if is_root || self.expanded_paths.contains(&path) || failed {
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
        self.as_mut().apply_rows(rows);
    }

    /// Replaces the flattened rows without resetting the model.
    ///
    /// A reset throws away every delegate in the tree and builds them again,
    /// which is the flash the sidebar showed whenever a folder was expanded.
    /// Expansion and collapse only touch one contiguous run: the row whose
    /// `expanded` flag flipped, and the subtree introduced or dropped below
    /// it. Matching by path keeps the other rows in place; only the toggled
    /// row is repainted, and only the run is inserted or removed.
    fn apply_rows(mut self: Pin<&mut Self>, next: Vec<TreeRow>) {
        let previous = self.as_mut().rust_mut().rows.clone();
        let (edit_at, previous_mid_end, next_mid_end) = row_delta(&previous, &next);

        // Repaint the rows that stayed where they were but changed — the
        // toggled row's `expanded` flag. This runs before the run below moves
        // indexes, so the row numbers are still the old ones.
        let mut changed: Option<(usize, usize)> = None;
        for index in 0..edit_at {
            if previous[index] == next[index] {
                continue;
            }
            self.as_mut().rust_mut().rows[index] = next[index].clone();
            changed = Some(match changed {
                Some((first, _)) => (first, index),
                None => (index, index),
            });
        }
        if let Some((first, last)) = changed {
            self.as_mut().emit_rows_changed(first as i32, last as i32);
        }

        unsafe {
            if edit_at < previous_mid_end {
                self.as_mut().begin_remove_rows(
                    &QModelIndex::default(),
                    edit_at as i32,
                    (previous_mid_end - 1) as i32,
                );
                self.as_mut()
                    .rust_mut()
                    .rows
                    .drain(edit_at..previous_mid_end);
                self.as_mut().end_remove_rows();
            }
            if edit_at < next_mid_end {
                self.as_mut().begin_insert_rows(
                    &QModelIndex::default(),
                    edit_at as i32,
                    (next_mid_end - 1) as i32,
                );
                self.as_mut().rust_mut().rows.splice(
                    edit_at..edit_at,
                    next[edit_at..next_mid_end].iter().cloned(),
                );
                self.as_mut().end_insert_rows();
            }
        }

        // The shared tail cannot change its paths; a change there (an
        // ancestor's `mayHaveChildren`) is still a repaint, at the indexes the
        // insert or removal above left them.
        let mut changed: Option<(usize, usize)> = None;
        for index in next_mid_end..next.len() {
            if self.as_mut().rust_mut().rows[index] == next[index] {
                continue;
            }
            self.as_mut().rust_mut().rows[index] = next[index].clone();
            changed = Some(match changed {
                Some((first, _)) => (first, index),
                None => (index, index),
            });
        }
        if let Some((first, last)) = changed {
            self.as_mut().emit_rows_changed(first as i32, last as i32);
        }

        self.as_mut().set_count(next.len() as i32);
    }

    /// Tells the view that `first..=last` kept its rows but changed their
    /// values. The tree only has the six roles below, so they are all sent.
    fn emit_rows_changed(mut self: Pin<&mut Self>, first: i32, last: i32) {
        unsafe {
            let parent = QModelIndex::default();
            let top = self.as_mut().model_index(first, 0, &parent);
            let bottom = self.as_mut().model_index(last, 0, &parent);
            let mut roles = QList::<i32>::default();
            for role in [
                ROLE_NAME,
                ROLE_PATH,
                ROLE_DEPTH,
                ROLE_EXPANDED,
                ROLE_MAY_HAVE_CHILDREN,
                ROLE_LOADED,
            ] {
                roles.append(role);
            }
            self.as_mut().data_changed(&top, &bottom, &roles);
        }
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

/// The edit `apply_rows` has to make: repaint rows `..edit_at`, then replace
/// `edit_at..previous_end` in the old list with `edit_at..next_end` from the
/// new one. Rows are matched by path, not by their full contents, so the
/// folder whose `expanded` flag flipped stays in place and the subtree below
/// it is the only run inserted or removed.
fn row_delta(previous: &[TreeRow], next: &[TreeRow]) -> (usize, usize, usize) {
    let prefix = previous
        .iter()
        .zip(next.iter())
        .take_while(|(before, after)| before.path == after.path)
        .count();
    // A shared tail must not overlap the prefix when the lists share a tail
    // that is longer than what is left after the prefix.
    let max_suffix = previous.len().min(next.len()) - prefix;
    let suffix = previous
        .iter()
        .rev()
        .zip(next.iter().rev())
        .take(max_suffix)
        .take_while(|(before, after)| before.path == after.path)
        .count();

    let previous_end = previous.len() - suffix;
    let next_end = next.len() - suffix;
    // The toggled row keeps its path; the children are what appears below it.
    // Matching the leading paths inside the changed middle keeps that row
    // (and repaints it) instead of removing and re-inserting it together with
    // its subtree.
    let in_place = previous[prefix..previous_end]
        .iter()
        .zip(next[prefix..next_end].iter())
        .take_while(|(before, after)| before.path == after.path)
        .count();

    (prefix + in_place, previous_end, next_end)
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
            created_at: None,
            created_fallback: false,
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

    fn row(name: &str, expanded: bool) -> TreeRow {
        TreeRow {
            name: name.to_owned(),
            path: format!("C:/root/{name}"),
            depth: 0,
            expanded,
            may_have_children: true,
            loaded: true,
        }
    }

    /// Opening a folder inserts only the subtree below it; the rows above and
    /// the siblings after it keep their delegates.
    #[test]
    fn expanding_a_folder_edits_the_run_below_its_row() {
        let a = row("a", false);
        let b = row("b", false);
        let c = row("c", false);
        let previous = vec![a.clone(), b, c.clone()];
        let next = vec![a, row("b", true), row("b1", false), row("b2", false), c];

        let (edit_at, previous_end, next_end) = row_delta(&previous, &next);
        assert_eq!(edit_at, 2, "b keeps its row and is only repainted");
        assert_eq!(previous_end, 2, "nothing above the open folder is removed");
        assert_eq!(next_end, 4, "the two children are the insertion");
    }

    /// Folding it removes exactly that run again.
    #[test]
    fn collapsing_a_folder_removes_the_run_below_its_row() {
        let a = row("a", false);
        let b = row("b", true);
        let c = row("c", false);
        let previous = vec![a.clone(), b, row("b1", false), row("b2", false), c.clone()];
        let next = vec![a, row("b", false), c];

        let (edit_at, previous_end, next_end) = row_delta(&previous, &next);
        assert_eq!(edit_at, 2, "b keeps its row and is only repainted");
        assert_eq!(previous_end, 4, "the two children are the removal");
        assert_eq!(next_end, 2, "nothing is inserted");
    }

    /// A flag-only change never touches the row count.
    #[test]
    fn a_flag_only_change_repaints_the_row_in_place() {
        let previous = vec![row("a", false), row("b", false)];
        let next = vec![row("a", false), row("b", true)];

        let (edit_at, previous_end, next_end) = row_delta(&previous, &next);
        assert_eq!((edit_at, previous_end, next_end), (2, 2, 2));
    }

    /// Identical lists produce an empty edit at the end, not a reset.
    #[test]
    fn an_unchanged_list_has_nothing_to_edit() {
        let rows = vec![row("a", false), row("b", false)];
        let (edit_at, previous_end, next_end) = row_delta(&rows, &rows);
        assert_eq!((edit_at, previous_end, next_end), (2, 2, 2));
    }
}
