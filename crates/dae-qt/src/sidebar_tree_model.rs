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

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use dae_core::fs::local::LocalBackend;
use dae_core::fs::vfs::FileSystemBackend;
use dae_core::types::EntryKind;
use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QmlElement, QmlObject, qobject};

use crate::runtime;

const ROLE_NAME: i32 = 0x0700;
const ROLE_PATH: i32 = 0x0701;
const ROLE_DEPTH: i32 = 0x0702;
const ROLE_EXPANDED: i32 = 0x0703;
const ROLE_MAY_HAVE_CHILDREN: i32 = 0x0704;
const ROLE_LOADED: i32 = 0x0705;

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

#[qobject(Base = QAbstractItemModel, NoQmlElement, ConvertToCamelCase)]
mod rows {
    use super::*;

    /// A flattened view of one expanded volume tree.
    pub struct SidebarTreeModel {
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
        status: String,
        generation: u64,
    }

    impl Default for SidebarTreeModel {
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
                status: String::new(),
                generation: 0,
            }
        }
    }

    impl QAbstractItemModel for SidebarTreeModel {
        fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
            if parent.is_valid() || column != 0 || row < 0 || row as usize >= self.rows.len() {
                return QModelIndex::default();
            }
            self.create_index(row, column, 0)
        }

        fn parent(&self, _child: &QModelIndex) -> QModelIndex {
            QModelIndex::default()
        }

        fn row_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() {
                0
            } else {
                self.rows.len() as i32
            }
        }

        fn column_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() { 0 } else { 1 }
        }

        fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
            if !index.is_valid() || index.column() != 0 || index.row() < 0 {
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

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
            roles.insert(ROLE_NAME, QByteArray::from("treeName"));
            roles.insert(ROLE_PATH, QByteArray::from("treePath"));
            roles.insert(ROLE_DEPTH, QByteArray::from("depth"));
            roles.insert(ROLE_EXPANDED, QByteArray::from("expanded"));
            roles.insert(ROLE_MAY_HAVE_CHILDREN, QByteArray::from("mayHaveChildren"));
            roles.insert(ROLE_LOADED, QByteArray::from("loaded"));
            roles
        }

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }

    impl SidebarTreeModel {
        qproperty!("count", Member = count, Notify = count_changed);
        qproperty!("busy", Member = busy, Notify = busy_changed);
        qproperty!("loaded", Member = loaded, Notify = loaded_changed);
        qproperty!("status", Member = status, Notify = status_changed);

        #[qsignal]
        fn count_changed(&mut self);
        #[qsignal]
        fn busy_changed(&mut self);
        #[qsignal]
        fn loaded_changed(&mut self);
        #[qsignal]
        fn status_changed(&mut self);

        /// A folder that the user asked to expand could not be read. The row
        /// goes back to its closed state and keeps its chevron, but it has no
        /// children to show, so the shell has to say why: a silent no-op reads
        /// as a broken chevron.
        #[qsignal]
        fn read_failed(&mut self, path: String, message: String);

        /// Starts over at `root`; the root itself is implicit (the volume row)
        /// and its direct children are the first rows.
        #[qslot]
        fn load(&mut self, root: String, show_hidden: bool) {
            let generation = self.generation.wrapping_add(1);
            self.generation = generation;
            self.root = root.clone();
            self.show_hidden = show_hidden;
            self.expanded_paths.clear();
            self.children.clear();
            self.loaded_paths.clear();
            self.loading_paths.clear();
            self.set_rows(Vec::new());
            self.loaded = false;
            self.loaded_changed();
            self.busy = false;
            self.busy_changed();
            self.status.clear();
            self.status_changed();

            if root.is_empty() {
                return;
            }

            self.busy = true;
            self.busy_changed();
            self.status = "Loading folder…".to_owned();
            self.status_changed();
            self.request_children(root, generation);
        }

        /// Expands a node and lazily reads its child folders, or collapses it.
        #[qslot]
        fn toggle(&mut self, path: String) {
            if path.is_empty() || path == self.root {
                return;
            }

            if self.expanded_paths.remove(&path) {
                self.rebuild();
                return;
            }

            self.expanded_paths.insert(path.clone());
            // Turn the chevron now rather than when the read finishes: the folder
            // is open from the moment it is clicked, and a blocking read on a slow
            // disk would otherwise leave the click looking ignored.
            self.rebuild();
            if self.children.contains_key(&path) || self.loading_paths.contains(&path) {
                return;
            }

            let generation = self.generation;
            self.busy = true;
            self.busy_changed();
            self.request_children(path, generation);
        }

        fn request_children(&mut self, path: String, generation: u64) {
            self.loading_paths.insert(path.clone());
            let show_hidden = self.show_hidden;
            let handle = self.get_thread_handle();
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

                handle.invoke_mut(move |model| {
                    model.apply_children(generation, path, result);
                });
            });
        }

        fn apply_children(
            &mut self,
            generation: u64,
            path: String,
            result: Result<Vec<TreeChild>, String>,
        ) {
            if self.generation != generation {
                return;
            }

            self.loading_paths.remove(&path);
            let is_root = path == self.root;
            let failed = result.is_err();

            match result {
                Ok(children) => {
                    let count = children.len();
                    self.children.insert(path.clone(), children);
                    self.loaded_paths.insert(path.clone());
                    if is_root {
                        self.loaded = true;
                        self.loaded_changed();
                        self.status = tree_status(count);
                        self.status_changed();
                    }
                }
                Err(error) => {
                    let message = crate::errors::friendly_error(&error);
                    if is_root {
                        // The volume row's tree stays open on an empty list so the
                        // status below it can say why nothing arrived.
                        self.children.insert(path.clone(), Vec::new());
                        self.loaded_paths.insert(path.clone());
                        self.loaded = true;
                        self.loaded_changed();
                        self.status = "Could not read folder".to_owned();
                        self.status_changed();
                    } else {
                        // A failed folder is not an open folder. Close the branch
                        // and leave it out of `loaded_paths`, so its chevron stays
                        // and the next click retries the read instead of reusing
                        // an empty child list.
                        self.expanded_paths.remove(&path);
                        self.read_failed(path.clone(), message.clone());
                    }
                    log::warn!(target: "dae::sidebar", "sidebar tree read failed: {message}");
                }
            }

            if is_root || self.expanded_paths.contains(&path) || failed {
                self.rebuild();
            }
            if self.loading_paths.is_empty() {
                self.busy = false;
                self.busy_changed();
            }
        }

        fn rebuild(&mut self) {
            let rows = flatten(
                &self.root,
                &self.expanded_paths,
                &self.children,
                &self.loaded_paths,
            );
            self.apply_rows(rows);
        }

        /// Replaces the flattened rows without resetting the model.
        ///
        /// A reset throws away every delegate in the tree and builds them again,
        /// which is the flash the sidebar showed whenever a folder was expanded.
        /// Expansion and collapse only touch one contiguous run: the row whose
        /// `expanded` flag flipped, and the subtree introduced or dropped below
        /// it. Matching by path keeps the other rows in place; only the toggled
        /// row is repainted, and only the run is inserted or removed.
        fn apply_rows(&mut self, next: Vec<TreeRow>) {
            let previous = self.rows.clone();
            let (edit_at, previous_mid_end, next_mid_end) = row_delta(&previous, &next);

            // Repaint the rows that stayed where they were but changed — the
            // toggled row's `expanded` flag. This runs before the run below moves
            // indexes, so the row numbers are still the old ones.
            let mut changed: Option<(usize, usize)> = None;
            for index in 0..edit_at {
                if previous[index] == next[index] {
                    continue;
                }
                self.rows[index] = next[index].clone();
                changed = Some(match changed {
                    Some((first, _)) => (first, index),
                    None => (index, index),
                });
            }
            if let Some((first, last)) = changed {
                self.emit_rows_changed(first as i32, last as i32);
            }

            if edit_at < previous_mid_end {
                self.begin_remove_rows(
                    &QModelIndex::default(),
                    edit_at as i32,
                    (previous_mid_end - 1) as i32,
                );
                self.rows.drain(edit_at..previous_mid_end);
                self.end_remove_rows();
            }
            if edit_at < next_mid_end {
                self.begin_insert_rows(
                    &QModelIndex::default(),
                    edit_at as i32,
                    (next_mid_end - 1) as i32,
                );
                self.rows
                    .splice(edit_at..edit_at, next[edit_at..next_mid_end].iter().cloned());
                self.end_insert_rows();
            }

            // The shared tail cannot change its paths; a change there (an
            // ancestor's `mayHaveChildren`) is still a repaint, at the indexes the
            // insert or removal above left them.
            let mut changed: Option<(usize, usize)> = None;
            for index in next_mid_end..next.len() {
                if self.rows[index] == next[index] {
                    continue;
                }
                self.rows[index] = next[index].clone();
                changed = Some(match changed {
                    Some((first, _)) => (first, index),
                    None => (index, index),
                });
            }
            if let Some((first, last)) = changed {
                self.emit_rows_changed(first as i32, last as i32);
            }

            self.count = next.len() as i32;
            self.count_changed();
        }

        /// Tells the view that `first..=last` kept its rows but changed their
        /// values.
        fn emit_rows_changed(&mut self, first: i32, last: i32) {
            let parent = QModelIndex::default();
            let top = self.index(first, 0, &parent);
            let bottom = self.index(last, 0, &parent);
            self.data_changed(&top, &bottom);
        }

        fn set_rows(&mut self, rows: Vec<TreeRow>) {
            let count = rows.len() as i32;
            self.begin_reset_model();
            self.rows = rows;
            self.end_reset_model();
            self.count = count;
            self.count_changed();
        }
    }
}

pub use rows::SidebarTreeModel;

impl QmlElement for SidebarTreeModel {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "SidebarTreeModel";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
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
