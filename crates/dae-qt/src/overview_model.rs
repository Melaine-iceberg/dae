//! The overview page's rows and its project scan.
//!
//! The overview combines four independent sources: known places and custom
//! favorites, the recent list, the user's spaces, and the git repositories
//! behind recent folders. The first three are cheap and synchronous, so the
//! page can paint immediately; the project scan is spawned and replaces the
//! row list when it answers. A generation counter keeps a slow scan from
//! overwriting a newer refresh (a tab switch or a favorites change).
//!
//! Sections and row anatomy are flattened into one model on purpose: the page
//! is one scroll, and a single `ListView` with section headers avoids four
//! nested scroll areas with four independent viewport heights.

use core::pin::Pin;
use std::collections::HashSet;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::git::{self, GitEntryStatusKind};
use dae_core::fs::recents::{self, RecentItem};
use dae_core::fs::sidebar::{self, PlaceKind, SystemPlace};
use dae_core::fs::spaces;
use dae_core::fs::vfs;
use dae_core::types::{EntryKind, display_name_from_path};

use crate::format;
use crate::runtime;

const SECTION_PROJECTS: &str = "Projects";
const SECTION_FAVORITES: &str = "Favorites";
const SECTION_RECENTS: &str = "Recent";
const SECTION_SPACES: &str = "Spaces";

const MAX_RECENTS: usize = 8;
const MAX_PROJECTS: usize = 4;
const MAX_CANDIDATES: usize = 8;

const ROLE_SECTION: i32 = 0x0600;
const ROLE_KIND: i32 = 0x0601;
const ROLE_NAME: i32 = 0x0602;
const ROLE_PATH: i32 = 0x0603;
const ROLE_DETAIL: i32 = 0x0604;
const ROLE_ICON: i32 = 0x0605;
const ROLE_IS_DIR: i32 = 0x0606;
const ROLE_BRANCH: i32 = 0x0607;
const ROLE_CHANGE_TEXT: i32 = 0x0608;
const ROLE_TIME_TEXT: i32 = 0x0609;

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
        /// The overview's flattened, sectioned rows.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        #[qproperty(bool, loaded)]
        #[qproperty(bool, project_busy, cxx_name = "projectBusy")]
        #[qproperty(QString, status)]
        type OverviewModel = super::OverviewModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Rebuilds the cheap sections, then scans the recent work trees.
        #[qinvokable]
        fn refresh(self: Pin<&mut OverviewModel>);

        #[qinvokable]
        #[cxx_name = "kindAt"]
        fn kind_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "pathAt"]
        fn path_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "detailAt"]
        fn detail_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "sectionAt"]
        fn section_at(&self, row: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "isDirAt"]
        fn is_dir_at(&self, row: i32) -> bool;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut OverviewModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut OverviewModel>);
    }

    impl cxx_qt::Threading for OverviewModel {}
}

#[derive(Clone)]
struct OverviewRow {
    section: &'static str,
    kind: &'static str,
    name: String,
    path: String,
    detail: String,
    icon: &'static str,
    is_directory: bool,
    branch: String,
    change_text: String,
    time_text: String,
}

#[derive(Clone)]
struct ProjectCandidate {
    path: String,
    accessed_at: u64,
}

#[derive(Default)]
pub struct OverviewModelRust {
    rows: Vec<OverviewRow>,
    base_rows: Vec<OverviewRow>,
    count: i32,
    loaded: bool,
    project_busy: bool,
    status: QString,
    generation: u64,
}

impl qobject::OverviewModel {
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
            ROLE_SECTION => QVariant::from(&QString::from(row.section)),
            ROLE_KIND => QVariant::from(&QString::from(row.kind)),
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_PATH => QVariant::from(&QString::from(row.path.as_str())),
            ROLE_DETAIL => QVariant::from(&QString::from(row.detail.as_str())),
            ROLE_ICON => QVariant::from(&QString::from(row.icon)),
            ROLE_IS_DIR => QVariant::from(&row.is_directory),
            ROLE_BRANCH => QVariant::from(&QString::from(row.branch.as_str())),
            ROLE_CHANGE_TEXT => QVariant::from(&QString::from(row.change_text.as_str())),
            ROLE_TIME_TEXT => QVariant::from(&QString::from(row.time_text.as_str())),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_SECTION, QByteArray::from("section"));
        roles.insert(ROLE_KIND, QByteArray::from("kind"));
        roles.insert(ROLE_NAME, QByteArray::from("name"));
        roles.insert(ROLE_PATH, QByteArray::from("path"));
        roles.insert(ROLE_DETAIL, QByteArray::from("detail"));
        roles.insert(ROLE_ICON, QByteArray::from("icon"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles.insert(ROLE_BRANCH, QByteArray::from("branch"));
        roles.insert(ROLE_CHANGE_TEXT, QByteArray::from("changeText"));
        roles.insert(ROLE_TIME_TEXT, QByteArray::from("timeText"));
        roles
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let generation = self.generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;

        let (rows, candidates) = self.as_ref().build_base();
        let count = rows.len() as i32;
        self.as_mut().rust_mut().base_rows = rows.clone();
        self.as_mut().replace_rows(rows);
        self.as_mut().set_count(count);
        self.as_mut().set_loaded(true);
        self.as_mut().set_project_busy(false);
        self.as_mut().set_status(QString::default());

        if candidates.is_empty() {
            return;
        }

        self.as_mut().set_project_busy(true);
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let projects = scan_projects(candidates).await;
            let _ = thread.queue(move |mut model| {
                model.as_mut().apply_projects(generation, projects);
            });
        });
    }

    fn kind_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.kind))
            .unwrap_or_default()
    }

    fn name_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.name.as_str()))
            .unwrap_or_default()
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.path.as_str()))
            .unwrap_or_default()
    }

    fn detail_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.detail.as_str()))
            .unwrap_or_default()
    }

    fn section_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|row| QString::from(row.section))
            .unwrap_or_default()
    }

    fn is_dir_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|row| row.is_directory)
            .unwrap_or(false)
    }

    fn apply_projects(mut self: Pin<&mut Self>, generation: u64, projects: Vec<OverviewRow>) {
        if self.generation != generation {
            return;
        }

        let mut rows = projects;
        rows.extend(self.base_rows.iter().cloned());
        let count = rows.len() as i32;
        self.as_mut().replace_rows(rows);
        self.as_mut().set_count(count);
        self.as_mut().set_project_busy(false);
    }

    fn build_base(&self) -> (Vec<OverviewRow>, Vec<ProjectCandidate>) {
        let mut rows = Vec::new();
        let mut candidates = Vec::new();
        let mut seen_paths = HashSet::new();

        let places = sidebar::system_places();
        for place in &places {
            push_place(&mut rows, &mut seen_paths, place);
        }

        match sidebar::load_favorites() {
            Ok(favorites) => {
                for favorite in favorites {
                    let key = normalize_path(&favorite.path);
                    if !seen_paths.insert(key) {
                        continue;
                    }
                    rows.push(OverviewRow {
                        section: SECTION_FAVORITES,
                        kind: "favorite",
                        name: favorite.name,
                        path: favorite.path.clone(),
                        detail: favorite.path.clone(),
                        icon: "star",
                        is_directory: true,
                        branch: String::new(),
                        change_text: String::new(),
                        time_text: String::new(),
                    });
                    if candidates.len() < MAX_CANDIDATES && vfs::is_local_path(&favorite.path) {
                        candidates.push(ProjectCandidate {
                            path: favorite.path,
                            accessed_at: 0,
                        });
                    }
                }
            }
            Err(error) => {
                report_aux_error("overview", &error.to_string());
            }
        }

        match recents::list_recents() {
            Ok(items) => {
                let mut shown = 0;
                for item in items {
                    if shown >= MAX_RECENTS {
                        break;
                    }
                    rows.push(recent_row(&item));
                    shown += 1;
                    if item.kind == EntryKind::Directory
                        && candidates.len() < MAX_CANDIDATES
                        && vfs::is_local_path(&item.path)
                    {
                        candidates.push(ProjectCandidate {
                            path: item.path,
                            accessed_at: item.accessed_at,
                        });
                    }
                }
            }
            Err(error) => {
                report_aux_error("overview", &error.to_string());
            }
        }

        match spaces::list_spaces() {
            Ok(spaces) => {
                for space in spaces {
                    rows.push(OverviewRow {
                        section: SECTION_SPACES,
                        kind: "space",
                        name: space.name,
                        path: space.id,
                        detail: if space.items.is_empty() {
                            "Empty space".to_owned()
                        } else {
                            let count = space.items.len();
                            format!("{count} item{}", if count == 1 { "" } else { "s" })
                        },
                        icon: "space",
                        is_directory: true,
                        branch: String::new(),
                        change_text: String::new(),
                        time_text: String::new(),
                    });
                }
            }
            Err(error) => {
                report_aux_error("overview", &error.to_string());
            }
        }

        (rows, candidates)
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<OverviewRow>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
    }
}

fn push_place(rows: &mut Vec<OverviewRow>, seen: &mut HashSet<String>, place: &SystemPlace) {
    if !seen.insert(normalize_path(&place.path)) {
        return;
    }
    rows.push(OverviewRow {
        section: SECTION_FAVORITES,
        kind: "favorite",
        name: place_name(place.kind).to_owned(),
        path: place.path.clone(),
        detail: place.path.clone(),
        icon: place_icon(place.kind),
        is_directory: true,
        branch: String::new(),
        change_text: String::new(),
        time_text: String::new(),
    });
}

fn recent_row(item: &RecentItem) -> OverviewRow {
    let is_directory = item.kind == EntryKind::Directory;
    OverviewRow {
        section: SECTION_RECENTS,
        kind: "recent",
        name: item.name.clone(),
        path: item.path.clone(),
        detail: parent_path(&item.path),
        icon: if is_directory { "folder" } else { "file" },
        is_directory,
        branch: String::new(),
        change_text: String::new(),
        time_text: format::day_label(item.accessed_at),
    }
}

/// A broken favorites/recents/spaces file must not hide the other sections.
/// The overview keeps rendering and the failure stays in the log rather than
/// turning the landing page into an error screen.
fn report_aux_error(source: &str, error: &str) {
    eprintln!("dae: overview {source} failed: {error}");
}

async fn scan_projects(candidates: Vec<ProjectCandidate>) -> Vec<OverviewRow> {
    let mut projects = Vec::new();
    let mut seen_roots = HashSet::new();

    for candidate in candidates.into_iter().take(MAX_CANDIDATES) {
        let Ok(Some(status)) = git::get_git_status(candidate.path.clone()).await else {
            continue;
        };
        let root = status.root.clone();
        if !seen_roots.insert(normalize_path(&root)) {
            continue;
        }

        let root_status = if same_path(&root, &candidate.path) {
            Some(status)
        } else {
            git::get_git_status(root.clone()).await.ok().flatten()
        };
        let (branch, entries, directory_untracked) = match root_status {
            Some(status) => (status.branch, status.entries, status.directory_untracked),
            None => continue,
        };
        if branch.is_empty() {
            continue;
        }

        let (added, modified, untracked) = count_entries(&entries);
        let change_text = if directory_untracked {
            "Untracked".to_owned()
        } else if added + modified + untracked == 0 {
            "Clean".to_owned()
        } else {
            let count = added + modified + untracked;
            format!("{count} change{}", if count == 1 { "" } else { "s" })
        };

        projects.push(OverviewRow {
            section: SECTION_PROJECTS,
            kind: "project",
            name: display_name_from_path(&root),
            path: root.clone(),
            detail: root,
            icon: "project",
            is_directory: true,
            branch,
            change_text,
            time_text: if candidate.accessed_at == 0 {
                String::new()
            } else {
                format::day_label(candidate.accessed_at)
            },
        });

        if projects.len() >= MAX_PROJECTS {
            break;
        }
    }

    projects
}

fn count_entries(entries: &[dae_core::fs::git::GitEntryStatus]) -> (usize, usize, usize) {
    let mut added = 0;
    let mut modified = 0;
    let mut untracked = 0;
    for entry in entries {
        match entry.kind {
            GitEntryStatusKind::Added => added += 1,
            GitEntryStatusKind::Modified => modified += 1,
            GitEntryStatusKind::Untracked => untracked += 1,
        }
    }
    (added, modified, untracked)
}

fn place_name(kind: PlaceKind) -> &'static str {
    match kind {
        PlaceKind::Home => "Home",
        PlaceKind::Desktop => "Desktop",
        PlaceKind::Documents => "Documents",
        PlaceKind::Downloads => "Downloads",
        PlaceKind::Pictures => "Pictures",
        PlaceKind::Music => "Music",
        PlaceKind::Videos => "Videos",
    }
}

fn place_icon(kind: PlaceKind) -> &'static str {
    match kind {
        PlaceKind::Home => "home",
        PlaceKind::Desktop => "desktop",
        PlaceKind::Documents => "documents",
        PlaceKind::Downloads => "downloads",
        PlaceKind::Pictures => "pictures",
        PlaceKind::Music => "music",
        PlaceKind::Videos => "videos",
    }
}

fn parent_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(0) => normalized[..1].to_owned(),
        Some(index) if index >= 2 && normalized.as_bytes()[index - 1] == b':' => {
            normalized[..index + 1].to_owned()
        }
        Some(index) => normalized[..index].to_owned(),
        None => String::new(),
    }
}

fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn same_path(left: &str, right: &str) -> bool {
    normalize_path(left) == normalize_path(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_core::fs::recents::RecentSource;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }

    fn recent(path: &str, kind: EntryKind) -> RecentItem {
        RecentItem {
            path: path.to_owned(),
            name: display_name_from_path(path),
            kind,
            source: RecentSource::Visited,
            accessed_at: now_millis(),
        }
    }

    #[test]
    fn a_recent_directory_points_at_its_parent() {
        let row = recent_row(&recent("C:/Users/me/Documents", EntryKind::Directory));
        assert_eq!(row.kind, "recent");
        assert_eq!(row.detail, "C:/Users/me");
        assert_eq!(row.icon, "folder");
        assert!(row.is_directory);
        assert_eq!(row.time_text, "Today");
    }

    #[test]
    fn a_recent_file_keeps_its_parent_and_file_icon() {
        let row = recent_row(&recent("C:/Users/me/note.txt", EntryKind::File));
        assert_eq!(row.icon, "file");
        assert!(!row.is_directory);
        assert_eq!(row.detail, "C:/Users/me");
    }

    #[test]
    fn a_drive_root_keeps_its_slash() {
        assert_eq!(parent_path("C:/Users"), "C:/");
        assert_eq!(parent_path("C:\\Users"), "C:/");
        assert_eq!(parent_path("C:/"), "C:/");
        assert_eq!(parent_path("/home/me"), "/home");
        assert_eq!(parent_path("/home"), "/");
    }

    #[test]
    fn the_path_comparison_ignores_separator_style() {
        assert!(same_path("C:/Users/me", "C:\\Users\\me"));
    }

    #[test]
    fn entries_are_counted_by_kind() {
        use dae_core::fs::git::GitEntryStatus;

        let entries = vec![
            GitEntryStatus {
                name: "a".to_owned(),
                kind: GitEntryStatusKind::Added,
            },
            GitEntryStatus {
                name: "b".to_owned(),
                kind: GitEntryStatusKind::Modified,
            },
            GitEntryStatus {
                name: "c".to_owned(),
                kind: GitEntryStatusKind::Untracked,
            },
        ];
        assert_eq!(count_entries(&entries), (1, 1, 1));
    }
}
