//! The directory listing model, productised from gate 1.
//!
//! `DirectoryListModel` is a real `QAbstractListModel` over the rows that
//! `dae_core::fs::listing` emits. The reader thread calls the core's
//! [`BatchTarget`] seam; this module is the shell half that queues each batch
//! onto the Qt thread and turns it into `beginInsertRows`.
//!
//! The spike had no navigation, so it also had no stale-batch problem. Here a
//! new `load()` bumps a generation and cancels the previous stream: a batch
//! from the old directory that was already queued is dropped instead of being
//! appended to the new one (which would show the wrong files under the right
//! path).
//!
//! The core also arms a watcher for the displayed directory, so a change made
//! outside the shell (Explorer, a terminal, another app) reaches this model as
//! a reload instead of waiting for F5.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QVariant,
};
use dae_core::fs::entry_codec::{self, ListingRow};
use dae_core::fs::git::{self, GitBranches, GitDirectoryStatus, GitEntryStatusKind};
use dae_core::fs::listing::{self, Batch, BatchTarget};
use dae_core::fs::vfs::{self, Scheme};
use dae_core::fs::watch;
use dae_core::host::{Event, Host};
use dae_core::types::{Breadcrumb, DirectoryEntry, EntryKind};

use crate::selection::{Band, Run, SelectableRow, Selection};

const ROLE_NAME: i32 = 0x0100;
const ROLE_PATH: i32 = 0x0101;
const ROLE_SIZE: i32 = 0x0102;
const ROLE_MODIFIED: i32 = 0x0103;
const ROLE_IS_DIR: i32 = 0x0104;
const ROLE_HIDDEN: i32 = 0x0105;
const ROLE_READ_ONLY: i32 = 0x0106;
/// A listing row's relative path is its name; search rows carry the path below
/// the search root. Exposing it here lets one delegate paint both models.
const ROLE_RELATIVE: i32 = 0x0107;
/// The `image://fileicon/...` URL for this row's OS icon.
const ROLE_ICON: i32 = 0x0108;
/// A coarse kind (`folder`, `image`, `video`, `code`, …) for the built-in
/// glyph set. The OS icon provider has no such notion, so the themed style
/// needs the model to answer it.
const ROLE_CATEGORY: i32 = 0x0109;
/// The file's extension without the dot, empty for directories. The themed
/// tile prints it and the type sort orders by it.
const ROLE_EXTENSION: i32 = 0x010A;
/// The `image://thumbnail/...` URL for this row's first frame, or empty when
/// the row has none the desktop can draw.
const ROLE_THUMBNAIL: i32 = 0x010B;
/// This row's working-tree state: `added`, `modified`, `untracked`, or empty
/// when the row is clean or the directory is not inside a worktree. The
/// delegate paints the letter; keeping the coarse kind here means the shell
/// does not need a second map keyed by file name.
const ROLE_GIT: i32 = 0x010C;
/// Whether this row is selected. A role rather than a JS lookup on the tab:
/// the delegate then re-evaluates only when its own row changes, and a click
/// emits one `dataChanged` for the rows that moved instead of one script call
/// per delegate on screen.
const ROLE_SELECTED: i32 = 0x010D;
/// The row's creation time, formatted for the details column. Empty when the
/// backend cannot report one.
const ROLE_CREATED: i32 = 0x010E;
/// `createdText` is the `ctime` fallback rather than a birth time, so the
/// column can say so.
const ROLE_CREATED_FALLBACK: i32 = 0x010F;
/// The tag names on this row, joined with `, ` for the details column.
const ROLE_TAGS: i32 = 0x0110;
/// The ids, joined with `, `. The context menu edits by id.
const ROLE_TAG_IDS: i32 = 0x0111;
/// The colours of [`ROLE_TAGS`] in the same order, joined with `, `, so the
/// delegate can paint one dot per tag without a second lookup.
const ROLE_TAG_COLORS: i32 = 0x0112;
/// The section this row belongs to under the current grouping; empty when
/// grouping is off.
const ROLE_GROUP: i32 = 0x0113;

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

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/core/qlist/qlist_i32.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
        /// Image files and file rows in the finished listing. The adaptive
        /// layout reads them once per directory; nothing else needs a second
        /// pass over the rows.
        #[qproperty(i32, images)]
        #[qproperty(i32, files)]
        /// Rows currently selected. Maintained as the selection changes, so the
        /// status bar reads a number instead of walking the selection.
        #[qproperty(i32, selected_count, cxx_name = "selectedCount")]
        #[qproperty(bool, finished)]
        #[qproperty(QString, status)]
        #[qproperty(QString, path)]
        /// The clickable path from the root to the shown directory, one entry
        /// per ancestor, spelled by the backend that read it: `C:`, `Users`,
        /// `Mt342`; `smb://nas.local`, `share`, `docs`; the cloud provider's
        /// own segments. The breadcrumb bar paints them and navigating asks the
        /// model to load the one that was clicked, so the ancestor list only
        /// has to be right here, once, instead of in a copy of the splitting
        /// rules per backend.
        ///
        /// Two parallel lists rather than one list of `{ name, path }` maps:
        /// cxx-qt can only export a property whose value type is comparable
        /// (`QVariant` and `QVariantList` are not), and `QStringList` is. They
        /// are always written together from one `Vec<Breadcrumb>`, so the
        /// pairing cannot drift.
        #[qproperty(QStringList, breadcrumb_names, cxx_name = "breadcrumbNames")]
        #[qproperty(QStringList, breadcrumb_paths, cxx_name = "breadcrumbPaths")]
        /// True when this model was loaded through a network backend. The
        /// shell reads it to keep local-only actions off `smb://`, `sftp://`
        /// and `cloud://` paths instead of letting them fail on click.
        #[qproperty(bool, remote)]
        /// The worktree root above the shown directory, or empty when the
        /// directory is not inside a Git worktree.
        #[qproperty(QString, git_root, cxx_name = "gitRoot")]
        #[qproperty(QString, git_branch, cxx_name = "gitBranch")]
        #[qproperty(bool, git_detached, cxx_name = "gitDetached")]
        #[qproperty(bool, git_has_upstream, cxx_name = "gitHasUpstream")]
        #[qproperty(i32, git_ahead, cxx_name = "gitAhead")]
        #[qproperty(i32, git_behind, cxx_name = "gitBehind")]
        /// True while a checkout/create/fetch/pull/push/sync command runs. The
        /// status-bar chip spins and the branch panel disables its actions.
        #[qproperty(bool, git_busy, cxx_name = "gitBusy")]
        #[qproperty(QStringList, git_remotes, cxx_name = "gitRemotes")]
        #[qproperty(QStringList, git_local_branches, cxx_name = "gitLocalBranches")]
        #[qproperty(QStringList, git_remote_branches, cxx_name = "gitRemoteBranches")]
        /// The current local branch, empty on a detached HEAD. The panel marks
        /// this row rather than comparing names itself.
        #[qproperty(QString, git_current_branch, cxx_name = "gitCurrentBranch")]
        type DirectoryListModel = super::DirectoryListModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[qinvokable]
        fn load(self: Pin<&mut DirectoryListModel>, path: &QString);

        #[qinvokable]
        fn reload(self: Pin<&mut DirectoryListModel>);

        /// Sets the display order. Re-sorts the rows already loaded and
        /// returns immediately; large listings sort on a worker and emit
        /// `sortApplied` when the permutation lands. Batches that arrive later
        /// are appended in stream order and the completed listing is sorted
        /// once more.
        #[qinvokable]
        #[cxx_name = "setOrder"]
        fn set_order(
            self: Pin<&mut DirectoryListModel>,
            sort_key: &QString,
            sort_order: &QString,
            folder_order: &QString,
        );

        /// Sections the listing by `group_key` (`name`, `modified`, `created`,
        /// `size`, `type`, `tag`, or empty for none); dates are bucketed by
        /// `group_date` (`year`, `month`, `day`). The grouping is part of the
        /// order, so changing it re-sorts.
        #[qinvokable]
        #[cxx_name = "setGroup"]
        fn set_group(self: Pin<&mut DirectoryListModel>, group_key: &QString, group_date: &QString);

        /// Reads the tags of the rows in `[first, last]`. The view calls this
        /// for the rows it is about to paint, so tagging never touches the
        /// enumeration path.
        #[qinvokable]
        #[cxx_name = "requestTags"]
        fn request_tags(self: Pin<&mut DirectoryListModel>, first: i32, last: i32);

        /// Records a measured byte total for one directory and repaints its
        /// size cell. Written by the shell after `Workspace.measureFolders`
        /// answers; a folder with no entry keeps showing the em dash.
        #[qinvokable]
        #[cxx_name = "setDirectorySize"]
        fn set_directory_size(self: Pin<&mut DirectoryListModel>, path: &QString, bytes: f64);

        /// Re-reads the tags of `paths` after the tag editor changed them, and
        /// repaints those rows. Also re-applies a tag sort or grouping.
        #[qinvokable]
        #[cxx_name = "refreshTags"]
        fn refresh_tags(self: Pin<&mut DirectoryListModel>, paths: &QStringList);

        /// Re-reads `tags.toml` after the tag manager created, renamed or
        /// removed a definition.
        #[qinvokable]
        #[cxx_name = "refreshTagDefinitions"]
        fn refresh_tag_definitions(self: Pin<&mut DirectoryListModel>);

        /// Sets the visibility filter. A filter change re-reads the directory:
        /// a listing that changed which rows exist has to be rebuilt, and the
        /// streamed reader is the one place that already knows how to do that
        /// without blocking the GUI thread.
        #[qinvokable]
        #[cxx_name = "setFilters"]
        fn set_filters(
            self: Pin<&mut DirectoryListModel>,
            show_hidden: bool,
            kind: &QString,
            modified: &QString,
            size: &QString,
        );

        /// Emitted before a load clears the rows. The view reads its scroll
        /// position here, because the reset that follows discards it.
        #[qsignal]
        #[cxx_name = "aboutToReload"]
        fn about_to_reload(self: Pin<&mut DirectoryListModel>);

        /// Emitted after a worker re-sort applies. Its reset dropped the
        /// viewport position that `aboutToReload` saved just before it, so the
        /// view restores it here (and not only on `finished`).
        #[qsignal]
        #[cxx_name = "sortApplied"]
        fn sort_applied(self: Pin<&mut DirectoryListModel>);

        /// The row whose full path is `path`, or -1. QML uses this to keep a
        /// renamed or newly created entry selected after the reload.
        #[qinvokable]
        #[cxx_name = "indexOfPath"]
        fn index_of_path(&self, path: &QString) -> i32;

        /// The full path of `row`, or an empty string. Selection ranges are
        /// computed in QML from row indices, which only the model can turn
        /// back into paths.
        #[qinvokable]
        #[cxx_name = "pathAt"]
        fn path_at(&self, row: i32) -> QString;

        /// Selects `row` and nothing else. An out-of-range row clears the
        /// selection, which is what a click on a row that vanished under the
        /// pointer has to mean.
        #[qinvokable]
        #[cxx_name = "selectOnly"]
        fn select_only(self: Pin<&mut DirectoryListModel>, row: i32);

        /// Flips `row` in the selection, the Ctrl-click case.
        #[qinvokable]
        #[cxx_name = "toggleAt"]
        fn toggle_at(self: Pin<&mut DirectoryListModel>, row: i32);

        /// Replaces the selection with `[first, last]`, clipped to the rows
        /// that exist: a Shift-click and a marquee both reach past the end on
        /// purpose.
        #[qinvokable]
        #[cxx_name = "selectRange"]
        fn select_range(self: Pin<&mut DirectoryListModel>, first: i32, last: i32);

        #[qinvokable]
        #[cxx_name = "selectAll"]
        fn select_all_rows(self: Pin<&mut DirectoryListModel>);

        #[qinvokable]
        #[cxx_name = "clearSelection"]
        fn clear_selection(self: Pin<&mut DirectoryListModel>);

        /// Replaces the selection with the rows whose path is in `paths`. Used
        /// by history, where the entry was captured before the listing it
        /// belongs to has been read back: a path whose row has not streamed in
        /// yet is held and flagged when it arrives.
        #[qinvokable]
        #[cxx_name = "setSelectedPaths"]
        fn set_selected_paths(self: Pin<&mut DirectoryListModel>, paths: &QStringList);

        /// The full paths of the selected rows, in listing order.
        #[qinvokable]
        #[cxx_name = "selectedPaths"]
        fn selected_paths(&self) -> QStringList;

        /// Starts a rubber-band drag. `keep` is the Ctrl case: the band then
        /// extends the selection instead of replacing it.
        #[qinvokable]
        #[cxx_name = "beginMarquee"]
        fn begin_marquee(self: Pin<&mut DirectoryListModel>, keep: bool);

        /// Sets the selection to the marquee base plus the cells of the band.
        /// The band is a rectangle of `columns` columns; the detail list passes
        /// one column, so its band is a row range. A negative row clears the
        /// band back to the base.
        #[qinvokable]
        #[cxx_name = "marqueeCells"]
        fn marquee_cells(
            self: Pin<&mut DirectoryListModel>,
            first_row: i32,
            first_column: i32,
            last_row: i32,
            last_column: i32,
            columns: i32,
        );

        #[qinvokable]
        #[cxx_name = "endMarquee"]
        fn end_marquee(self: Pin<&mut DirectoryListModel>);

        /// The first row at or after `start` (wrapping once) whose name begins
        /// with `needle`, or -1. `needle` must already be lowercase: the
        /// type-ahead folds the buffer once, not once per row.
        #[qinvokable]
        #[cxx_name = "indexOfNamePrefix"]
        fn index_of_name_prefix(&self, needle: &QString, start: i32) -> i32;

        /// Whether `row` is a directory, for Enter/activation.
        #[qinvokable]
        #[cxx_name = "isDirectoryAt"]
        fn is_directory_at(&self, row: i32) -> bool;

        /// Re-reads this directory's Git state, branches included. The branch
        /// panel calls it when it opens so a checkout made in a terminal shows
        /// up even when no file in this directory changed.
        #[qinvokable]
        #[cxx_name = "refreshGit"]
        fn refresh_git(self: Pin<&mut DirectoryListModel>);

        #[qinvokable]
        #[cxx_name = "gitCheckoutBranch"]
        fn git_checkout_branch(self: Pin<&mut DirectoryListModel>, name: &QString, remote: bool);

        #[qinvokable]
        #[cxx_name = "gitCreateBranch"]
        fn git_create_branch(self: Pin<&mut DirectoryListModel>, name: &QString, checkout: bool);

        #[qinvokable]
        #[cxx_name = "gitFetch"]
        fn git_fetch(self: Pin<&mut DirectoryListModel>);

        #[qinvokable]
        #[cxx_name = "gitPull"]
        fn git_pull(self: Pin<&mut DirectoryListModel>);

        #[qinvokable]
        #[cxx_name = "gitPush"]
        fn git_push(self: Pin<&mut DirectoryListModel>);

        #[qinvokable]
        #[cxx_name = "gitSync"]
        fn git_sync(self: Pin<&mut DirectoryListModel>);

        /// A branch command succeeded. `operation` is `checkout`, `create`,
        /// `fetch`, `pull`, `push` or `sync`; the create form uses it to close
        /// only after its own command landed.
        #[qsignal]
        #[cxx_name = "gitOperationFinished"]
        fn git_operation_finished(
            self: Pin<&mut DirectoryListModel>,
            operation: &QString,
            message: &QString,
        );

        #[qsignal]
        #[cxx_name = "gitOperationFailed"]
        fn git_operation_failed(
            self: Pin<&mut DirectoryListModel>,
            operation: &QString,
            message: &QString,
        );

        /// Stops the stream and drops every row. A closed tab calls this
        /// before it is released, so a 120k-row directory is not kept alive by
        /// a model nobody shows.
        #[qinvokable]
        fn dispose(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_insert_rows"]
        unsafe fn beginInsertRows(
            self: Pin<&mut DirectoryListModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_insert_rows"]
        unsafe fn endInsertRows(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_remove_rows"]
        unsafe fn beginRemoveRows(
            self: Pin<&mut DirectoryListModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );

        #[inherit]
        #[rust_name = "end_remove_rows"]
        unsafe fn endRemoveRows(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut DirectoryListModel>);

        #[inherit]
        #[rust_name = "model_index"]
        unsafe fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        /// Repaints the Git column without rebuilding the rows, so a status
        /// refresh landing after the listing keeps the scroll position and the
        /// current row.
        #[inherit]
        #[rust_name = "data_changed"]
        unsafe fn dataChanged(
            self: Pin<&mut DirectoryListModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    impl cxx_qt::Threading for DirectoryListModel {}
}

/// `PartialEq` is what lets a refresh keep the rows it already has: the diff
/// compares old and new entries field by field, so an unchanged row costs no
/// model signal at all. `selected` is deliberately outside that comparison —
/// it is view state, and a click must not make the diff treat the whole
/// directory as changed.
#[derive(Clone)]
struct StoredRow {
    name: String,
    /// Kept only for the rare row that is not under the listing base. Rows that
    /// are (the local reader's normal shape) rebuild their path from the base,
    /// which is what keeps a 120k-row model cheap.
    path: Option<String>,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    created_at: Option<u64>,
    /// `created_at` is the ctime fallback, so the column can say so.
    created_fallback: bool,
    hidden: bool,
    read_only: bool,
    selected: bool,
    /// The group this row falls into under the current grouping, computed once
    /// per grouping change so the comparator and the section header agree.
    /// Empty when grouping is off.
    group_label: String,
    group_rank: i64,
    /// Tag ids on this row, read from the tag index in a batch after the
    /// listing lands. Empty until then.
    tag_ids: Vec<String>,
    /// Tag names joined in definition order, kept for the tag sort.
    tag_sort: String,
}

impl PartialEq for StoredRow {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.path == other.path
            && self.kind == other.kind
            && self.size == other.size
            && self.modified_at == other.modified_at
            && self.created_at == other.created_at
            && self.created_fallback == other.created_fallback
            && self.hidden == other.hidden
            && self.read_only == other.read_only
    }
}

impl SelectableRow for StoredRow {
    fn is_selected(&self) -> bool {
        self.selected
    }

    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    fn selection_key(&self) -> &str {
        // The same identity `row_for_path` compares: rows under the base are
        // addressed by name, rows that spell their own path by that path.
        match &self.path {
            Some(path) => path.as_str(),
            None => self.name.as_str(),
        }
    }
}

impl StoredRow {
    fn from_entry(entry: &DirectoryEntry, base: &str) -> Self {
        let path = if entry.path.starts_with(base) {
            None
        } else {
            Some(entry.path.clone())
        };
        Self {
            name: entry.name.clone(),
            path,
            kind: entry.kind,
            size: entry.size,
            modified_at: entry.modified_at,
            created_at: entry.created_at,
            created_fallback: entry.created_fallback,
            hidden: entry.hidden,
            read_only: entry.read_only,
            selected: false,
            group_label: String::new(),
            group_rank: 0,
            tag_ids: Vec::new(),
            tag_sort: String::new(),
        }
    }

    fn from_listing(row: ListingRow) -> Self {
        Self {
            name: row.name,
            path: row.path,
            kind: row.kind,
            size: row.size,
            modified_at: row.modified_at,
            created_at: row.created_at,
            created_fallback: row.created_fallback,
            hidden: row.hidden,
            read_only: row.read_only,
            selected: false,
            group_label: String::new(),
            group_rank: 0,
            tag_ids: Vec::new(),
            tag_sort: String::new(),
        }
    }

    fn full_path(&self, base: &str) -> String {
        match &self.path {
            Some(path) => path.clone(),
            None => format!("{base}{}", self.name),
        }
    }
}

/// The ordering the explorer shows. Mirrors the Tauri shell's
/// `ExplorerSortKey`/`ExplorerSortOrder` pair (see
/// `src/features/explorer/entry-order.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortKey {
    Name,
    Modified,
    Created,
    Type,
    Size,
    Path,
    Tag,
}

impl SortKey {
    fn parse(value: &str) -> Self {
        match value {
            "modified" => Self::Modified,
            "created" => Self::Created,
            "type" => Self::Type,
            "size" => Self::Size,
            "path" => Self::Path,
            "tag" => Self::Tag,
            _ => Self::Name,
        }
    }
}

/// Where folders sit relative to files at the top of every group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FolderOrder {
    #[default]
    FoldersFirst,
    FilesFirst,
    Mixed,
}

impl FolderOrder {
    fn parse(value: &str) -> Self {
        match value {
            "filesFirst" => Self::FilesFirst,
            "mixed" => Self::Mixed,
            _ => Self::FoldersFirst,
        }
    }

    fn rank(self, kind: EntryKind) -> u8 {
        let folder = u8::from(kind == EntryKind::Directory);
        match self {
            Self::FoldersFirst => folder ^ 1,
            Self::FilesFirst => folder,
            Self::Mixed => 0,
        }
    }
}

/// What the listing is sectioned by. The label is computed per row (see
/// `assign_groups`), so the comparator stays a pure function of precomputed
/// values and the worker sort needs no tag or timestamp lookups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GroupKey {
    #[default]
    None,
    Name,
    Modified,
    Created,
    Size,
    Type,
    Tag,
}

impl GroupKey {
    fn parse(value: &str) -> Self {
        match value {
            "name" => Self::Name,
            "modified" => Self::Modified,
            "created" => Self::Created,
            "size" => Self::Size,
            "type" => Self::Type,
            "tag" => Self::Tag,
            _ => Self::None,
        }
    }
}

/// How precisely a date group names its bucket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GroupDate {
    Year,
    #[default]
    Month,
    Day,
}

impl GroupDate {
    fn parse(value: &str) -> Self {
        match value {
            "year" => Self::Year,
            "day" => Self::Day,
            _ => Self::Month,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Grouping {
    key: GroupKey,
    date: GroupDate,
}

impl Grouping {
    fn active(self) -> bool {
        self.key != GroupKey::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    fn parse(value: &str) -> Self {
        if value == "desc" {
            Self::Desc
        } else {
            Self::Asc
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewOrder {
    key: SortKey,
    order: SortOrder,
    folders: FolderOrder,
    grouping: Grouping,
}

impl Default for ViewOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            order: SortOrder::Asc,
            folders: FolderOrder::FoldersFirst,
            grouping: Grouping::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KindFilter {
    All,
    Folders,
    Files,
    Images,
}

impl KindFilter {
    fn parse(value: &str) -> Self {
        match value {
            "folders" => Self::Folders,
            "files" => Self::Files,
            "images" => Self::Images,
            _ => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModifiedFilter {
    Any,
    Today,
    Week,
    Month,
}

impl ModifiedFilter {
    fn parse(value: &str) -> Self {
        match value {
            "today" => Self::Today,
            "week" => Self::Week,
            "month" => Self::Month,
            _ => Self::Any,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeFilter {
    Any,
    Small,
    Medium,
    Large,
}

impl SizeFilter {
    fn parse(value: &str) -> Self {
        match value {
            "small" => Self::Small,
            "medium" => Self::Medium,
            "large" => Self::Large,
            _ => Self::Any,
        }
    }
}

/// Which rows a listing shows. Kept as a value so a batch can be filtered
/// against one snapshot of the preferences rather than re-reading the model
/// settings per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowFilter {
    show_hidden: bool,
    kind: KindFilter,
    modified: ModifiedFilter,
    size: SizeFilter,
}

impl Default for RowFilter {
    fn default() -> Self {
        Self {
            show_hidden: true,
            kind: KindFilter::All,
            modified: ModifiedFilter::Any,
            size: SizeFilter::Any,
        }
    }
}

impl RowFilter {
    fn keeps(&self, row: &StoredRow, modified_cutoff: u64) -> bool {
        if !self.show_hidden && row.hidden {
            return false;
        }

        match self.kind {
            KindFilter::All => {}
            KindFilter::Folders => {
                if row.kind != EntryKind::Directory {
                    return false;
                }
            }
            KindFilter::Files => {
                if row.kind != EntryKind::File {
                    return false;
                }
            }
            KindFilter::Images => {
                if row.kind != EntryKind::File || category_of(row.kind, &row.name) != "image" {
                    return false;
                }
            }
        }

        if modified_cutoff > 0 && row.modified_at.unwrap_or(0) < modified_cutoff {
            return false;
        }

        if let Some((low, high)) = self.size_range()
            && row.kind == EntryKind::File
        {
            let size = row.size.unwrap_or(0);
            if size < low || size >= high {
                return false;
            }
        }

        true
    }

    fn size_range(&self) -> Option<(u64, u64)> {
        const MIB: u64 = 1024 * 1024;
        match self.size {
            SizeFilter::Any => None,
            SizeFilter::Small => Some((0, MIB)),
            SizeFilter::Medium => Some((MIB, 100 * MIB)),
            SizeFilter::Large => Some((100 * MIB, u64::MAX)),
        }
    }

    /// Milliseconds before which an entry is outside the modified bucket.
    /// Computed once per batch; `None` maps to `0`, which disables the test.
    fn modified_cutoff(&self) -> u64 {
        let now = chrono::Local::now();
        let millis = match self.modified {
            ModifiedFilter::Any => 0,
            ModifiedFilter::Today => now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|midnight| {
                    use chrono::TimeZone;
                    chrono::Local.from_local_datetime(&midnight).earliest()
                })
                .map(|midnight| midnight.timestamp_millis().max(0) as u64)
                .unwrap_or(0),
            ModifiedFilter::Week => {
                (now - chrono::Duration::days(7)).timestamp_millis().max(0) as u64
            }
            ModifiedFilter::Month => {
                (now - chrono::Duration::days(30)).timestamp_millis().max(0) as u64
            }
        };
        millis
    }
}

/// The fields the ordering rules read. Implemented by [`StoredRow`] and by the
/// snapshot the completion sort hands to its worker, so both use one copy of
/// the comparator.
trait SortFields {
    fn sort_kind(&self) -> EntryKind;
    fn sort_name(&self) -> &str;
    fn sort_size(&self) -> Option<u64>;
    fn sort_modified_at(&self) -> Option<u64>;
    fn sort_created_at(&self) -> Option<u64>;
    /// The row's path suffix. Every row of one listing hangs off the same
    /// base, so comparing these is the full-path order without building a
    /// string per comparison; a patched row from another directory sorts by
    /// its own spelling, which is the only case where the base would matter.
    fn sort_path(&self) -> &str;
    fn sort_tag(&self) -> &str;
    fn sort_group(&self) -> (i64, &str);
}

impl SortFields for StoredRow {
    fn sort_kind(&self) -> EntryKind {
        self.kind
    }

    fn sort_name(&self) -> &str {
        &self.name
    }

    fn sort_size(&self) -> Option<u64> {
        self.size
    }

    fn sort_modified_at(&self) -> Option<u64> {
        self.modified_at
    }

    fn sort_created_at(&self) -> Option<u64> {
        self.created_at
    }

    fn sort_path(&self) -> &str {
        self.path.as_deref().unwrap_or(&self.name)
    }

    fn sort_tag(&self) -> &str {
        &self.tag_sort
    }

    fn sort_group(&self) -> (i64, &str) {
        (self.group_rank, &self.group_label)
    }
}

/// Orders two rows for display.
///
/// The rules reproduce the Tauri shell's comparator: folders lead in both
/// directions when `foldersFirst` is on, the active key is scaled by the
/// direction, and the name breaks ties ascending so equal sizes/timestamps
/// still have one order.
fn compare_rows<T: SortFields>(left: &T, right: &T, order: &ViewOrder) -> Ordering {
    if order.grouping.active() {
        let (left_rank, left_label) = left.sort_group();
        let (right_rank, right_label) = right.sort_group();
        let groups = left_rank
            .cmp(&right_rank)
            .then_with(|| natural_cmp(left_label, right_label));
        let groups = match order.order {
            SortOrder::Asc => groups,
            SortOrder::Desc => groups.reverse(),
        };
        if groups != Ordering::Equal {
            return groups;
        }
    }

    // Untagged rows sort after tagged ones in both directions: "no tag" is
    // not a value that belongs at either end of the tag order.
    if order.key == SortKey::Tag {
        let left_empty = left.sort_tag().is_empty();
        let right_empty = right.sort_tag().is_empty();
        if left_empty != right_empty {
            return if left_empty {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
    }

    let folders = order
        .folders
        .rank(left.sort_kind())
        .cmp(&order.folders.rank(right.sort_kind()));
    if folders != Ordering::Equal {
        return folders;
    }

    let primary = match order.key {
        SortKey::Name => natural_cmp(left.sort_name(), right.sort_name()),
        SortKey::Modified => left
            .sort_modified_at()
            .unwrap_or(0)
            .cmp(&right.sort_modified_at().unwrap_or(0)),
        SortKey::Created => left
            .sort_created_at()
            .unwrap_or(0)
            .cmp(&right.sort_created_at().unwrap_or(0)),
        SortKey::Size => left
            .sort_size()
            .unwrap_or(0)
            .cmp(&right.sort_size().unwrap_or(0)),
        SortKey::Path => natural_cmp(left.sort_path(), right.sort_path()),
        SortKey::Tag => natural_cmp(left.sort_tag(), right.sort_tag()),
        SortKey::Type => {
            let left_category = category_of(left.sort_kind(), left.sort_name());
            let right_category = category_of(right.sort_kind(), right.sort_name());
            category_rank(left_category)
                .cmp(&category_rank(right_category))
                .then_with(|| {
                    natural_cmp(
                        extension_of(left.sort_name()),
                        extension_of(right.sort_name()),
                    )
                })
        }
    };
    let primary = match order.order {
        SortOrder::Asc => primary,
        SortOrder::Desc => primary.reverse(),
    };
    if primary != Ordering::Equal {
        return primary;
    }

    natural_cmp(left.sort_name(), right.sort_name())
}

fn sort_rows(rows: &mut [StoredRow], order: &ViewOrder) {
    rows.sort_by(|left, right| compare_rows(left, right, order));
}

/// One row reduced to the fields the comparator reads. The completion sort
/// hands these to a worker so the GUI thread keeps its rows (and paints them)
/// while the ordering is computed.
struct SortEntry {
    name: String,
    path: Option<String>,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    created_at: Option<u64>,
    tag_sort: String,
    group_label: String,
    group_rank: i64,
}

impl SortFields for SortEntry {
    fn sort_kind(&self) -> EntryKind {
        self.kind
    }

    fn sort_name(&self) -> &str {
        &self.name
    }

    fn sort_size(&self) -> Option<u64> {
        self.size
    }

    fn sort_modified_at(&self) -> Option<u64> {
        self.modified_at
    }

    fn sort_created_at(&self) -> Option<u64> {
        self.created_at
    }

    fn sort_path(&self) -> &str {
        self.path.as_deref().unwrap_or(&self.name)
    }

    fn sort_tag(&self) -> &str {
        &self.tag_sort
    }

    fn sort_group(&self) -> (i64, &str) {
        (self.group_rank, &self.group_label)
    }
}

fn sort_snapshot(rows: &[StoredRow]) -> Vec<SortEntry> {
    rows.iter()
        .map(|row| SortEntry {
            name: row.name.clone(),
            path: row.path.clone(),
            kind: row.kind,
            size: row.size,
            modified_at: row.modified_at,
            created_at: row.created_at,
            tag_sort: row.tag_sort.clone(),
            group_label: row.group_label.clone(),
            group_rank: row.group_rank,
        })
        .collect()
}

/// Indices into `entries`, in display order. The index comparison is the
/// stable-sort tie breaker: [`sort_rows`] keeps equal names in arrival order,
/// and an unstable sort over indices has to reproduce that explicitly.
fn sort_permutation(entries: &[SortEntry], order: &ViewOrder) -> Vec<u32> {
    let mut perm: Vec<u32> = (0..entries.len() as u32).collect();
    // The index tie breaker makes the order total, so an unstable sort over
    // the 4-byte indices produces exactly what the stable row sort does.
    perm.sort_unstable_by(|&left, &right| {
        compare_rows(&entries[left as usize], &entries[right as usize], order)
            .then_with(|| left.cmp(&right))
    });
    perm
}

/// Identifies one worker sort: the model state it was started for and the
/// request that owns it. The request token lets a newer interactive sort
/// supersede an older one without waiting for a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SortTicket {
    generation: u64,
    rows: usize,
    order: ViewOrder,
    request: u64,
}

/// Whether `ticket`'s permutation still describes the model. A load changes
/// the generation, a newer sort changes the request (or the order), a row edit
/// changes the row count, and a malformed permutation is refused before it can
/// lose rows.
fn worker_sort_is_current(
    closed: bool,
    current_generation: u64,
    current_rows: usize,
    current_order: ViewOrder,
    current_request: u64,
    ticket: SortTicket,
    perm: &[u32],
) -> bool {
    !closed
        && current_generation == ticket.generation
        && current_rows == ticket.rows
        && current_order == ticket.order
        && current_request == ticket.request
        && is_permutation(perm, ticket.rows)
}

fn is_permutation(perm: &[u32], len: usize) -> bool {
    if perm.len() != len {
        return false;
    }
    let mut seen = vec![false; len];
    for &index in perm {
        let index = index as usize;
        if index >= len || seen[index] {
            return false;
        }
        seen[index] = true;
    }
    true
}

/// Reorders `rows` so that `perm[position]` is the old index of the row that
/// lands there. Returns `None` and leaves the input alone when `perm` is not a
/// permutation, so a bad worker result cannot lose rows.
fn apply_permutation(rows: Vec<StoredRow>, perm: &[u32]) -> Option<Vec<StoredRow>> {
    if !is_permutation(perm, rows.len()) {
        return None;
    }
    let mut slots: Vec<Option<StoredRow>> = rows.into_iter().map(Some).collect();
    Some(
        perm.iter()
            .map(|&index| slots[index as usize].take().expect("checked permutation"))
            .collect(),
    )
}

fn rows_are_sorted(rows: &[StoredRow], order: &ViewOrder) -> bool {
    rows.windows(2)
        .all(|pair| compare_rows(&pair[0], &pair[1], order) != Ordering::Greater)
}

/// Filters a group by its id. The UI maps these ids to localized labels; the
/// model keeps them language-neutral so a section header is translated once,
/// not per row.
const GROUP_TAG_LESS: i64 = i64::MAX;

/// Computes the section label and ordering rank for every row.
///
/// Runs once per grouping change (and once per streamed batch, so the rows
/// arriving under an active grouping already carry their header). The
/// comparator then reads two plain values, which is what lets the worker sort
/// stay allocation-free.
fn assign_groups(rows: &mut [StoredRow], grouping: Grouping) {
    if !grouping.active() {
        for row in rows {
            row.group_label.clear();
            row.group_rank = 0;
        }
        return;
    }

    let definitions = if grouping.key == GroupKey::Tag {
        dae_core::fs::tags::definitions()
    } else {
        Vec::new()
    };

    for row in rows {
        let (rank, label) = group_of(row, grouping, &definitions);
        row.group_rank = rank;
        row.group_label = label;
    }
}

fn group_of(
    row: &StoredRow,
    grouping: Grouping,
    definitions: &[dae_core::fs::tags::TagDefinition],
) -> (i64, String) {
    match grouping.key {
        GroupKey::None => (0, String::new()),
        GroupKey::Name => {
            let label = row
                .name
                .chars()
                .next()
                .map(|character| {
                    let mut folded = character.to_uppercase();
                    folded.next().unwrap_or('#').to_string()
                })
                // Digits get the `#` bucket: a numeric first character is
                // not a letter group, which is what file managers have always
                // done.
                .filter(|label| label.chars().next().is_some_and(char::is_alphabetic))
                .unwrap_or_else(|| "#".to_owned());
            (0, label)
        }
        GroupKey::Modified => date_group(row.modified_at, grouping.date),
        // A fallback value is still the best bucket the file system has; the
        // header names the grouping, and the created column carries the
        // fallback note.
        GroupKey::Created => date_group(row.created_at, grouping.date),
        GroupKey::Size => {
            if row.kind == EntryKind::Directory {
                return (3, "—".to_owned());
            }
            const MIB: u64 = 1024 * 1024;
            let size = row.size.unwrap_or(0);
            if size < MIB {
                (0, "< 1 MB".to_owned())
            } else if size < 100 * MIB {
                (1, "1–100 MB".to_owned())
            } else {
                (2, "> 100 MB".to_owned())
            }
        }
        GroupKey::Type => {
            let category = category_of(row.kind, &row.name);
            (i64::from(category_rank(category)), category.to_owned())
        }
        GroupKey::Tag => {
            if row.tag_ids.is_empty() {
                return (GROUP_TAG_LESS, String::new());
            }
            let mut ordered: Vec<(usize, &str)> = row
                .tag_ids
                .iter()
                .filter_map(|id| {
                    definitions
                        .iter()
                        .position(|definition| definition.id == *id)
                        .map(|position| (position, definitions[position].name.as_str()))
                })
                .collect();
            ordered.sort_unstable_by_key(|(position, _)| *position);
            let label = ordered
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>()
                .join(", ");
            let rank = row
                .tag_ids
                .iter()
                .filter_map(|id| {
                    definitions
                        .iter()
                        .position(|definition| definition.id == *id)
                })
                .min()
                .map(|position| position as i64)
                .unwrap_or(GROUP_TAG_LESS - 1);
            (rank, label)
        }
    }
}

/// Writes one row's tag ids and the derived sort key from `definitions`.
fn set_row_tags(
    row: &mut StoredRow,
    ids: &[String],
    definitions: &[dae_core::fs::tags::TagDefinition],
) {
    row.tag_ids = ids.to_vec();
    row.tag_sort = definitions
        .iter()
        .filter(|definition| ids.contains(&definition.id))
        .map(|definition| definition.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
}

/// Stamps the tags already read onto a batch, without building any row's path:
/// a row under `base` is matched by name, and only a row that spells its own
/// path pays that lookup. A refresh keeps the cache, so a per-row `full_path`
/// here would charge a 120,000-entry re-listing one allocation per entry.
fn decorate_tags(
    rows: &mut [StoredRow],
    cache: &HashMap<String, Vec<String>>,
    definitions: &[dae_core::fs::tags::TagDefinition],
    base: &str,
) {
    let by_name: HashMap<&str, &[String]> = cache
        .iter()
        .filter_map(|(path, ids)| path.strip_prefix(base).map(|name| (name, ids.as_slice())))
        .collect();
    for row in rows.iter_mut() {
        let ids = match &row.path {
            Some(path) => cache.get(path).map(Vec::as_slice),
            None => by_name.get(row.name.as_str()).copied(),
        };
        if let Some(ids) = ids {
            set_row_tags(row, ids, definitions);
        }
    }
}

/// A date bucket's rank and label. The rank is `YYYYMMDD`-shaped so
/// lexicographic and numeric order agree; a missing timestamp sorts last.
fn date_group(timestamp: Option<u64>, granularity: GroupDate) -> (i64, String) {
    use chrono::TimeZone;
    let Some(timestamp) = timestamp else {
        return (GROUP_TAG_LESS, "—".to_owned());
    };
    let Some(moment) = chrono::Local
        .timestamp_millis_opt(timestamp as i64)
        .single()
    else {
        return (GROUP_TAG_LESS, "—".to_owned());
    };
    use chrono::Datelike;
    let year = moment.year();
    let month = moment.month();
    let day = moment.day();
    match granularity {
        GroupDate::Year => (i64::from(year) * 10_000, format!("{year}")),
        GroupDate::Month => (
            i64::from(year) * 10_000 + i64::from(month) * 100,
            format!("{year}-{month:02}"),
        ),
        GroupDate::Day => (
            i64::from(year) * 10_000 + i64::from(month) * 100 + i64::from(day),
            format!("{year}-{month:02}-{day:02}"),
        ),
    }
}

/// Case-insensitive natural ordering: `file2` sorts before `file10`.
///
/// `Intl.Collator` with `{numeric: true, sensitivity: "base"}` also folds
/// accents; Rust has no built-in collator here, so this covers the parts that
/// matter for file names (case, digits) and leaves diacritics compared by code
/// point.
///
/// This runs inside `sort_by` on the GUI thread, so it never allocates: the
/// lowercase form is walked from the original strings (see [`Folded`]) and a
/// digit run is compared where it sits instead of being collected into a
/// `String`. Sorting 120,000 names is 4.3× faster than the allocating version,
/// which is what the model's completion re-sort needed (see
/// `docs/decisions/listing-shell-costs.md` §2).
fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left = Folded::new(left);
    let mut right = Folded::new(right);

    loop {
        // Digits are ASCII, so the folded stream carries them unchanged; a run
        // on both sides is compared as two slices of the original strings.
        if left.on_digit() && right.on_digit() {
            let ordering = left.compare_digits(&mut right);
            if ordering != Ordering::Equal {
                return ordering;
            }
            continue;
        }

        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                let ordering = left_char.cmp(&right_char);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

/// How many characters one source character may expand to when folded.
///
/// Unicode's SpecialCasing has a single multi-character lowercase mapping
/// (U+0130 → `i` + U+0307), so two would do; three leaves room for a case
/// mapping added later, and the tests assert the bound over every `char`.
const FOLDED_PENDING: usize = 3;

/// A `&str` walked one **lowercase** character at a time.
///
/// `char::to_lowercase` may yield more than one character; the extras wait in
/// `pending` until the next step asks for them, which keeps the stream equal to
/// `text.chars().flat_map(char::to_lowercase)` without building it. `at` is the
/// byte offset of the next character to fold, which is what a digit run needs in
/// order to compare in place.
struct Folded<'a> {
    text: &'a str,
    at: usize,
    pending: [char; FOLDED_PENDING],
    pending_len: usize,
    pending_at: usize,
}

impl<'a> Folded<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            at: 0,
            pending: ['\0'; FOLDED_PENDING],
            pending_len: 0,
            pending_at: 0,
        }
    }

    /// The next character of the lowercase stream.
    fn next(&mut self) -> Option<char> {
        if self.pending_at < self.pending_len {
            self.pending_at += 1;
            return Some(self.pending[self.pending_at - 1]);
        }

        let character = self.text[self.at..].chars().next()?;
        self.at += character.len_utf8();
        // Every ASCII character folds to exactly one character, and most names
        // are ASCII, so this is the path the sort spends its time in.
        if character.is_ascii() {
            return Some(character.to_ascii_lowercase());
        }

        let mut len = 0;
        for folded in character.to_lowercase() {
            if len < FOLDED_PENDING {
                self.pending[len] = folded;
            }
            len += 1;
        }
        debug_assert!(
            len <= FOLDED_PENDING,
            "U+{:04X} expands to {len} lowercase characters",
            character as u32
        );
        self.pending_len = len.min(FOLDED_PENDING);
        self.pending_at = if self.pending_len == 0 { 0 } else { 1 };
        Some(self.pending[0])
    }

    /// Whether the next character is an ASCII digit and no folded expansion is
    /// still waiting — the state in which a run can be compared in place.
    fn on_digit(&self) -> bool {
        self.pending_at == self.pending_len
            && self
                .text
                .as_bytes()
                .get(self.at)
                .is_some_and(u8::is_ascii_digit)
    }

    /// Compares this side's digit run against `other`'s and advances both past
    /// it: numeric value first (length after leading zeros, then the digits),
    /// then fewer leading zeros, the way the collator breaks `file1` vs `file01`.
    fn compare_digits(&mut self, other: &mut Self) -> Ordering {
        let (left_start, left_end) = digit_run(self.text, self.at);
        let (right_start, right_end) = digit_run(other.text, other.at);
        let left = &self.text[left_start..left_end];
        let right = &other.text[right_start..right_end];
        let ordering = left
            .len()
            .cmp(&right.len())
            .then_with(|| left.cmp(right))
            .then_with(|| (left_end - self.at).cmp(&(right_end - other.at)));
        self.at = left_end;
        other.at = right_end;
        ordering
    }
}

/// The bounds of the digit run at `at`, with leading zeros excluded from the
/// start. `0` is a digit too, so a run of zeros comes back empty — its length
/// still matters for the tie-break in [`Folded::compare_digits`].
fn digit_run(text: &str, at: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut end = at;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    let mut start = at;
    while start < end && bytes[start] == b'0' {
        start += 1;
    }
    (start, end)
}

pub(crate) fn extension_of(name: &str) -> &str {
    name.rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("")
}

/// The coarse kind the built-in glyph set paints.
pub(crate) fn category_of(kind: EntryKind, name: &str) -> &'static str {
    match kind {
        EntryKind::Directory => "folder",
        EntryKind::Symlink => "link",
        EntryKind::Other => "other",
        EntryKind::File => category_for_extension(extension_of(name)),
    }
}

fn category_for_extension(extension: &str) -> &'static str {
    const IMAGE: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "heic", "avif",
    ];
    const VIDEO: &[&str] = &[
        "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg",
    ];
    const AUDIO: &[&str] = &["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus"];
    const ARCHIVE: &[&str] = &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst", "iso"];
    const CODE: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "qml", "py", "go", "java", "c", "h", "cpp",
        "hpp", "cc", "cs", "rb", "php", "swift", "kt", "kts", "lua", "sh", "bash", "zsh", "ps1",
        "bat", "cmd", "sql", "html", "htm", "css", "scss", "less", "json", "yaml", "yml", "toml",
        "xml",
    ];
    const TEXT: &[&str] = &[
        "txt", "md", "markdown", "log", "csv", "tsv", "ini", "conf", "cfg",
    ];
    const DOCUMENT: &[&str] = &[
        "doc", "docx", "xls", "xlsx", "ppt", "pptx", "rtf", "odt", "ods",
    ];
    const FONT: &[&str] = &["ttf", "otf", "woff", "woff2"];
    const EXECUTABLE: &[&str] = &["exe", "msi", "appref-ms"];

    let contains = |set: &[&str]| {
        set.iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    };
    if contains(IMAGE) {
        "image"
    } else if contains(VIDEO) {
        "video"
    } else if contains(AUDIO) {
        "audio"
    } else if contains(ARCHIVE) {
        "archive"
    } else if contains(CODE) {
        "code"
    } else if contains(TEXT) {
        "text"
    } else if contains(DOCUMENT) {
        "document"
    } else if contains(FONT) {
        "font"
    } else if contains(EXECUTABLE) {
        "executable"
    } else {
        "file"
    }
}

fn category_rank(category: &str) -> u8 {
    match category {
        "folder" => 0,
        "link" => 1,
        "image" => 2,
        "video" => 3,
        "audio" => 4,
        "document" => 5,
        "text" => 6,
        "code" => 7,
        "archive" => 8,
        "font" => 9,
        "executable" => 10,
        "other" => 12,
        _ => 11,
    }
}

/// Rows collected while the directory already on screen is re-read. The old
/// rows stay visible until this is complete, so the view never shows an empty
/// model between the two listings.
#[derive(Default)]
struct RefreshStage {
    rows: Vec<StoredRow>,
    base: String,
    path: String,
}

/// A Git command the branch panel or status chip can start. Kept as one enum
/// so the busy flag, failure relay and post-success refresh live in one place.
enum GitOperation {
    Checkout { name: String, remote: bool },
    Create { name: String, checkout: bool },
    Fetch,
    Pull,
    Push,
    Sync,
}

impl GitOperation {
    fn kind(&self) -> &'static str {
        match self {
            Self::Checkout { .. } => "checkout",
            Self::Create { .. } => "create",
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push => "push",
            Self::Sync => "sync",
        }
    }

    fn success_message(&self) -> String {
        match self {
            Self::Checkout { name, .. } => format!("Switched to branch {name}"),
            Self::Create { name, .. } => format!("Created branch {name}"),
            Self::Fetch => "Fetched remote updates".to_owned(),
            Self::Pull => "Pulled remote updates".to_owned(),
            Self::Push => "Pushed to remote".to_owned(),
            Self::Sync => "Synced with remote".to_owned(),
        }
    }
}

pub struct DirectoryListModelRust {
    loaded: i32,
    images: i32,
    files: i32,
    /// Rows currently selected, published as the `selectedCount` property so
    /// the status bar never recounts the selection.
    selected_count: i32,
    finished: bool,
    status: QString,
    path: QString,
    /// The ancestors of `path`, as the breadcrumb bar reads them. Both lists
    /// come from the same `Vec<Breadcrumb>`, so they always pair up index for
    /// index.
    breadcrumb_names: QStringList,
    breadcrumb_paths: QStringList,
    rows: Vec<StoredRow>,
    base: String,
    /// `Some` while a same-directory refresh runs; the rows above are the
    /// previous listing until the stage is swapped in.
    refresh: Option<RefreshStage>,
    stream_id: Option<String>,
    generation: u64,
    /// Rows came from a network backend. Their `name` is the only part that
    /// carries a file extension, so the icon URL is built from it rather than
    /// from the full path. Exposed to QML so local-only actions can be gated.
    remote: bool,
    /// This model's identity in the core watcher registry. One id for the
    /// model's whole life: navigation re-targets the observation rather than
    /// minting a new one, so a slow arm cannot leave a stale watcher behind.
    watcher_id: String,
    /// A change arrived while a read was in flight. Re-reading now would cancel
    /// that read, so the reload waits for it to finish instead.
    reload_pending: bool,
    /// The last read failed, so the rows on screen are not a listing of `path`.
    /// A watcher patch must not add to or edit them; only a read can recover.
    load_failed: bool,
    /// `dispose` ran and released the watcher; a reload already queued on the
    /// Qt thread must not revive a closed model.
    closed: bool,
    /// A sort is running on a worker. Patches are refused while it is: they
    /// plan insertions against the row order, and the rows are still in their
    /// old order until the permutation lands.
    sorting: bool,
    /// Identifies the newest worker sort. A reply carrying an older value is
    /// stale even when the generation and row count still match, which is how
    /// an interactive re-sort can be superseded without a load.
    sort_request: u64,
    order: ViewOrder,
    filter: RowFilter,
    /// Which rows are selected. The flags live on the rows; this is the index
    /// that keeps a click from scanning the whole listing.
    selection: Selection,
    /// The worktree root and the branch facts the status bar paints. Written
    /// together from one branch listing; the status-only refresh leaves the
    /// branch lists alone so a 1000-branch repository is not re-enumerated on
    /// every directory reload.
    git_root: QString,
    git_branch: QString,
    git_detached: bool,
    git_has_upstream: bool,
    git_ahead: i32,
    git_behind: i32,
    git_busy: bool,
    git_remotes: QStringList,
    git_local_branches: QStringList,
    git_remote_branches: QStringList,
    git_current_branch: QString,
    /// Direct children with a working-tree change, keyed by name. Kept beside
    /// the rows rather than inside `StoredRow`: a status refresh must not make
    /// the row diff treat the whole directory as changed.
    git_entries: HashMap<String, GitEntryStatusKind>,
    /// Tag ids per full path, filled in batches after the listing lands.
    /// Keyed by path rather than stored on the row so a refresh can carry the
    /// ids over when a row's metadata changed but its path did not.
    tags_cache: HashMap<String, Vec<String>>,
    /// Drops a tag read that lands after a load or a newer read.
    tag_generation: u64,
    /// Every row's tags have been read, so a tag sort does not have to start
    /// another pass.
    tags_complete: bool,
    /// The tag definitions, read once per load and after an edit in the tag
    /// manager. `data()` runs per visible cell, so it must not open
    /// `tags.toml`.
    tag_definitions: Vec<dae_core::fs::tags::TagDefinition>,
    /// The shown directory is entirely untracked; every child gets the `U`
    /// badge without an entry per file.
    git_directory_untracked: bool,
    /// Drops a Git read that lands after the user navigated away or a newer
    /// request replaced it.
    git_generation: u64,
    /// Byte totals for directories the user asked to measure, keyed by full
    /// path. Folders are never walked during enumeration; this map only holds
    /// the totals a "calculate size" pass returned.
    folder_sizes: HashMap<String, u64>,
}

impl Default for DirectoryListModelRust {
    fn default() -> Self {
        Self {
            loaded: 0,
            images: 0,
            files: 0,
            selected_count: 0,
            finished: false,
            status: QString::default(),
            path: QString::default(),
            breadcrumb_names: QStringList::default(),
            breadcrumb_paths: QStringList::default(),
            rows: Vec::new(),
            base: String::new(),
            refresh: None,
            stream_id: None,
            generation: 0,
            remote: false,
            watcher_id: next_watcher_id(),
            reload_pending: false,
            load_failed: false,
            closed: false,
            sorting: false,
            sort_request: 0,
            order: ViewOrder::default(),
            filter: RowFilter::default(),
            selection: Selection::default(),
            git_root: QString::default(),
            git_branch: QString::default(),
            git_detached: false,
            git_has_upstream: false,
            git_ahead: 0,
            git_behind: 0,
            git_busy: false,
            git_remotes: QStringList::default(),
            git_local_branches: QStringList::default(),
            git_remote_branches: QStringList::default(),
            git_current_branch: QString::default(),
            git_entries: HashMap::new(),
            tags_cache: HashMap::new(),
            tag_generation: 0,
            tags_complete: false,
            tag_definitions: Vec::new(),
            git_directory_untracked: false,
            folder_sizes: HashMap::new(),
            git_generation: 0,
        }
    }
}

impl Drop for DirectoryListModelRust {
    fn drop(&mut self) {
        // `dispose` normally released this already; a model the shell let go
        // without disposing must not leave an OS watcher running.
        watch::unwatch_directory(self.watcher_id.clone());
    }
}

impl qobject::DirectoryListModel {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        // A list model has one level: any valid parent has no children.
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
            ROLE_PATH => QVariant::from(&QString::from(row.full_path(&self.base))),
            ROLE_RELATIVE => QVariant::from(&QString::from(row.name.as_str())),
            // A folder's byte total is not known without walking it, so the
            // detail view shows an em dash instead of a made-up "0 B".
            ROLE_SIZE => {
                let text = if row.kind == EntryKind::Directory {
                    self.folder_sizes
                        .get(&row.full_path(&self.base))
                        .map(|bytes| crate::format::size(*bytes))
                        .unwrap_or_else(|| "—".to_owned())
                } else {
                    crate::format::size(row.size.unwrap_or(0))
                };
                QVariant::from(&QString::from(text))
            }
            ROLE_MODIFIED => QVariant::from(&QString::from(crate::format::timestamp(
                row.modified_at.unwrap_or(0),
            ))),
            ROLE_IS_DIR => QVariant::from(&(row.kind == EntryKind::Directory)),
            ROLE_HIDDEN => QVariant::from(&row.hidden),
            ROLE_READ_ONLY => QVariant::from(&row.read_only),
            ROLE_SELECTED => QVariant::from(&row.selected),
            ROLE_ICON => {
                // The desktop icon lookup cannot answer for a remote
                // directory (a bare name is not a local path), so leave the
                // URL empty and let the delegate paint its fallback square
                // instead of asking the provider for an icon it cannot make.
                if self.remote && row.kind == EntryKind::Directory {
                    return QVariant::from(&QString::default());
                }
                let path = if self.remote {
                    row.name.clone()
                } else {
                    row.full_path(&self.base)
                };
                let version = icon_version(row);
                QVariant::from(&QString::from(
                    crate::icons::source_for(&path, row.kind == EntryKind::Directory, &version)
                        .as_str(),
                ))
            }
            ROLE_CATEGORY => QVariant::from(&QString::from(category_of(row.kind, &row.name))),
            ROLE_EXTENSION => QVariant::from(&QString::from(extension_of(&row.name))),
            ROLE_THUMBNAIL => {
                let path = row.full_path(&self.base);
                QVariant::from(&QString::from(thumbnail_url(
                    self.remote,
                    &path,
                    &row.name,
                    row,
                )))
            }
            ROLE_GIT => QVariant::from(&QString::from(git_badge(
                &self.git_entries,
                self.git_directory_untracked,
                &row.name,
            ))),
            ROLE_CREATED => QVariant::from(&QString::from(
                if row.created_at.is_some() {
                    crate::format::timestamp(row.created_at.unwrap_or(0))
                } else {
                    "—".to_owned()
                }
                .as_str(),
            )),
            ROLE_CREATED_FALLBACK => QVariant::from(&row.created_fallback),
            ROLE_TAGS => QVariant::from(&QString::from(self.tag_names(&row.tag_ids).as_str())),
            ROLE_TAG_IDS => QVariant::from(&QString::from(row.tag_ids.join(",").as_str())),
            ROLE_TAG_COLORS => {
                QVariant::from(&QString::from(self.tag_colors(&row.tag_ids).as_str()))
            }
            ROLE_GROUP => QVariant::from(&QString::from(row.group_label.as_str())),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_NAME, QByteArray::from("fileName"));
        roles.insert(ROLE_PATH, QByteArray::from("filePath"));
        roles.insert(ROLE_RELATIVE, QByteArray::from("relativePath"));
        roles.insert(ROLE_SIZE, QByteArray::from("sizeText"));
        roles.insert(ROLE_MODIFIED, QByteArray::from("modifiedText"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles.insert(ROLE_HIDDEN, QByteArray::from("hidden"));
        roles.insert(ROLE_READ_ONLY, QByteArray::from("readOnly"));
        roles.insert(ROLE_ICON, QByteArray::from("iconSource"));
        roles.insert(ROLE_CATEGORY, QByteArray::from("category"));
        roles.insert(ROLE_EXTENSION, QByteArray::from("extension"));
        roles.insert(ROLE_THUMBNAIL, QByteArray::from("thumbnailSource"));
        roles.insert(ROLE_GIT, QByteArray::from("gitStatus"));
        roles.insert(ROLE_SELECTED, QByteArray::from("selected"));
        roles.insert(ROLE_CREATED, QByteArray::from("createdText"));
        roles.insert(ROLE_CREATED_FALLBACK, QByteArray::from("createdFallback"));
        roles.insert(ROLE_TAGS, QByteArray::from("tagNames"));
        roles.insert(ROLE_TAG_IDS, QByteArray::from("tagIds"));
        roles.insert(ROLE_TAG_COLORS, QByteArray::from("tagColors"));
        roles.insert(ROLE_GROUP, QByteArray::from("groupLabel"));
        roles
    }

    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let requested = path.to_string();
        if requested.trim().is_empty() {
            self.as_mut().set_status(QString::from("No path"));
            return;
        }

        // Reloading the directory already on screen keeps its rows until the
        // replacement has arrived; the reset is deferred to the swap so the
        // user never sees an empty list. Navigating elsewhere still clears at
        // once, or the old rows would appear under the new path.
        let refreshing = !self.path.as_slice().is_empty() && self.path.to_string() == requested;
        if !refreshing {
            // A view can only remember where it was scrolled before the reset
            // below; afterwards the rows — and the position with them — are gone.
            self.as_mut().about_to_reload();
        }

        let generation = self.generation.wrapping_add(1);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.sorting = false;
            // A reply from a sort started before this load describes rows that
            // are about to be replaced, even if the row count happens to match.
            rust.sort_request = rust.sort_request.wrapping_add(1);
            // Same rule for tag reads, plus a definition re-read: the tag
            // manager may have changed a name since the last directory.
            rust.tag_generation = rust.tag_generation.wrapping_add(1);
            rust.tags_complete = false;
            rust.tag_definitions = dae_core::fs::tags::definitions();
            if !refreshing {
                rust.tags_cache.clear();
            }
        }
        if let Some(stream_id) = self.as_mut().rust_mut().stream_id.take() {
            listing::cancel_directory_listing(stream_id);
        }

        // A scheme prefix sends this load through the vfs backend instead of
        // the local streaming reader. Remote backends list in one piece.
        let scheme = vfs::scheme_of(&requested);
        let remote = matches!(scheme, Ok(scheme) if scheme != Scheme::Local);

        if refreshing {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = generation;
            rust.refresh = Some(RefreshStage::default());
            rust.reload_pending = false;
            rust.closed = false;
        } else {
            unsafe {
                self.as_mut().begin_reset_model();
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.generation = generation;
                    rust.rows.clear();
                    rust.base.clear();
                    rust.folder_sizes.clear();
                    // A selection belongs to the rows it was made on. Staying
                    // inside the same directory (the refresh branch above)
                    // keeps it; navigating drops it before the new rows can
                    // inherit flags for names that happen to match.
                    rust.selection.reset();
                    rust.refresh = None;
                    rust.reload_pending = false;
                    rust.closed = false;
                }
                self.as_mut().end_reset_model();
            }
        }
        self.as_mut().set_remote(remote);
        self.as_mut().rust_mut().load_failed = false;

        self.as_mut().set_finished(false);
        if !refreshing {
            self.as_mut().set_loaded(0);
            self.as_mut().publish_selection_count();
            self.as_mut().set_path(QString::from(requested.as_str()));
            self.as_mut().set_status(QString::from("Loading…"));
        }

        // Navigation asks for branches too, because the status bar's
        // ahead/behind numbers come from them. A same-directory reload only
        // refreshes the working-tree scan: enumerating 1000 branches on every
        // watcher trip would cost more than the listing itself.
        self.as_mut().begin_git_load(!refreshing);

        // An unknown scheme is a failure of this path, not of the previous
        // directory: the model has already adopted `requested` above.
        if let Err(error) = scheme {
            self.as_mut().rust_mut().refresh = None;
            self.as_mut().rust_mut().load_failed = true;
            let message = crate::errors::friendly_error(&error.to_string());
            self.as_mut().set_status(QString::from(message.as_str()));
            self.as_mut().set_finished(true);
            return;
        }

        let stream_id = next_stream_id();
        self.as_mut().rust_mut().stream_id = Some(stream_id.clone());

        let thread = self.as_mut().qt_thread();
        let target = Arc::new(QtBatchTarget::new(thread.clone(), generation));
        let watch_host: Arc<dyn Host> = Arc::new(ListingHost::new(thread));
        let worker = Arc::clone(&target);
        let requested_path = requested.clone();
        let watcher_id = self.watcher_id.clone();

        std::thread::spawn(move || {
            if remote {
                match vfs::resolve(&requested_path) {
                    Ok(backend) => {
                        // Armed before the read, like the local arm: a change
                        // landing during enumeration is either in the view or
                        // reported as an event.
                        watch::arm_polling_watcher(
                            Arc::clone(&watch_host),
                            &requested_path,
                            backend.clone(),
                            &watcher_id,
                        );
                        match backend.read_dir(&requested_path) {
                            Ok(view) => {
                                let base = entry_codec::base_of(&view.path);
                                let rows = view
                                    .entries
                                    .iter()
                                    .map(|entry| StoredRow::from_entry(entry, &base))
                                    .collect();
                                worker.deliver_head(base, view.path, view.breadcrumbs, rows, true);
                            }
                            Err(error) => {
                                worker.fail(crate::errors::friendly_error(&error.to_string()))
                            }
                        }
                    }
                    Err(error) => worker.fail(crate::errors::friendly_error(&error.to_string())),
                }
                return;
            }

            match listing::open_streamed_listing(
                watch_host,
                PathBuf::from(&requested_path),
                stream_id,
                Some(watcher_id),
                worker.clone(),
            ) {
                Ok(view) => {
                    let done = view.stream_id.is_none();
                    let base = entry_codec::base_of(&view.path);
                    let rows = view
                        .entries
                        .iter()
                        .map(|entry| StoredRow::from_entry(entry, &base))
                        .collect();
                    worker.deliver_head(base, view.path, view.breadcrumbs, rows, done);
                }
                Err(error) => worker.fail(crate::errors::friendly_error(&error.to_string())),
            }
        });
    }

    fn reload(mut self: Pin<&mut Self>) {
        let path = self.path.clone();
        if !path.as_slice().is_empty() {
            self.as_mut().load(&path);
        }
    }

    /// Starts the Git read that belongs to the load above. Navigation clears
    /// the previous repository's badges and branch facts synchronously, so a
    /// slow read can never paint them onto the new directory.
    fn begin_git_load(mut self: Pin<&mut Self>, navigating: bool) {
        let path = self.path.to_string();
        let local = !path.is_empty() && matches!(vfs::scheme_of(&path), Ok(Scheme::Local));
        let generation = self.git_generation.wrapping_add(1);
        self.as_mut().rust_mut().git_generation = generation;
        if navigating {
            self.as_mut().clear_git_state();
        }
        if !local || self.closed {
            return;
        }
        self.as_mut().spawn_git_read(generation, navigating);
    }

    fn refresh_git(mut self: Pin<&mut Self>) {
        if self.git_busy || self.closed {
            return;
        }
        let path = self.path.to_string();
        let local = !path.is_empty() && matches!(vfs::scheme_of(&path), Ok(Scheme::Local));
        let generation = self.git_generation.wrapping_add(1);
        self.as_mut().rust_mut().git_generation = generation;
        if !local {
            self.as_mut().clear_git_state();
            return;
        }
        self.as_mut().spawn_git_read(generation, true);
    }

    fn spawn_git_read(mut self: Pin<&mut Self>, generation: u64, with_branches: bool) {
        let path = self.path.to_string();
        let thread = self.as_mut().qt_thread();
        crate::runtime::spawn(async move {
            let status = git::get_git_status(path.clone()).await;
            let branches = if with_branches && matches!(&status, Ok(Some(_))) {
                git::list_git_branches(path).await.ok().flatten()
            } else {
                None
            };
            let _ = thread.queue(move |mut model| {
                model
                    .as_mut()
                    .adopt_git(generation, with_branches, status, branches)
            });
        });
    }

    fn adopt_git(
        mut self: Pin<&mut Self>,
        generation: u64,
        with_branches: bool,
        status: Result<Option<GitDirectoryStatus>, dae_core::error::FileSystemError>,
        branches: Option<GitBranches>,
    ) {
        if self.closed || self.git_generation != generation {
            return;
        }
        // A failed read leaves the last good state in place; only a
        // successful "not a worktree" answer clears the chip.
        let Ok(status) = status else {
            return;
        };
        let Some(status) = status else {
            self.as_mut().clear_git_state();
            return;
        };

        let GitDirectoryStatus {
            root,
            branch,
            entries,
            directory_untracked,
        } = status;
        let branch_changed = self.git_branch.to_string() != branch;
        let entries: HashMap<String, GitEntryStatusKind> = entries
            .into_iter()
            .map(|entry| (entry.name, entry.kind))
            .collect();
        let badges_changed =
            self.git_entries != entries || self.git_directory_untracked != directory_untracked;

        self.as_mut().set_git_root(QString::from(root.as_str()));
        self.as_mut().set_git_branch(QString::from(branch.as_str()));
        {
            let mut rust = self.as_mut().rust_mut();
            rust.git_entries = entries;
            rust.git_directory_untracked = directory_untracked;
        }
        if badges_changed {
            self.as_mut().emit_git_badges_changed();
        }

        if with_branches {
            if let Some(branches) = branches {
                self.as_mut().adopt_git_branches(branches);
            }
        } else if branch_changed {
            // HEAD moved outside this shell (a terminal checkout). The status
            // scan already knows the new name; fetch the branch lists so the
            // panel and ahead/behind agree with it.
            self.as_mut().refresh_git();
        }
    }

    fn adopt_git_branches(mut self: Pin<&mut Self>, branches: GitBranches) {
        let mut local = QStringList::default();
        let mut current = QString::default();
        for branch in &branches.local {
            if branch.is_current {
                current = QString::from(branch.name.as_str());
            }
            local.append(QString::from(branch.name.as_str()));
        }
        let mut remote = QStringList::default();
        for branch in &branches.remote {
            remote.append(QString::from(branch.name.as_str()));
        }
        let mut remotes = QStringList::default();
        for name in &branches.remotes {
            remotes.append(QString::from(name.as_str()));
        }

        self.as_mut()
            .set_git_root(QString::from(branches.root.as_str()));
        self.as_mut()
            .set_git_branch(QString::from(branches.branch.as_str()));
        self.as_mut().set_git_detached(branches.detached);
        self.as_mut().set_git_has_upstream(branches.has_upstream);
        self.as_mut()
            .set_git_ahead(branches.ahead.min(i32::MAX as u32) as i32);
        self.as_mut()
            .set_git_behind(branches.behind.min(i32::MAX as u32) as i32);
        self.as_mut().set_git_remotes(remotes);
        self.as_mut().set_git_local_branches(local);
        self.as_mut().set_git_remote_branches(remote);
        self.as_mut().set_git_current_branch(current);
    }

    /// Drops the badge map and every branch property. The qproperty setters
    /// are skipped when the value is already empty, because each write emits a
    /// change signal and would rebind the status bar for nothing.
    fn clear_git_state(mut self: Pin<&mut Self>) {
        let had_badges = !self.git_entries.is_empty() || self.git_directory_untracked;
        {
            let mut rust = self.as_mut().rust_mut();
            rust.git_entries.clear();
            rust.git_directory_untracked = false;
        }
        if !self.git_root.as_slice().is_empty() {
            self.as_mut().set_git_root(QString::default());
        }
        if !self.git_branch.as_slice().is_empty() {
            self.as_mut().set_git_branch(QString::default());
        }
        if self.git_detached {
            self.as_mut().set_git_detached(false);
        }
        if self.git_has_upstream {
            self.as_mut().set_git_has_upstream(false);
        }
        if self.git_ahead != 0 {
            self.as_mut().set_git_ahead(0);
        }
        if self.git_behind != 0 {
            self.as_mut().set_git_behind(0);
        }
        if self.git_remotes.len() != 0 {
            self.as_mut().set_git_remotes(QStringList::default());
        }
        if self.git_local_branches.len() != 0 {
            self.as_mut().set_git_local_branches(QStringList::default());
        }
        if self.git_remote_branches.len() != 0 {
            self.as_mut()
                .set_git_remote_branches(QStringList::default());
        }
        if !self.git_current_branch.as_slice().is_empty() {
            self.as_mut().set_git_current_branch(QString::default());
        }
        if had_badges {
            self.as_mut().emit_git_badges_changed();
        }
    }

    /// Repaints only the Git role for every row. The whole range in one signal
    /// keeps this O(1) in the model; the view only re-reads its visible
    /// delegates.
    fn emit_git_badges_changed(mut self: Pin<&mut Self>) {
        let count = self.rows.len();
        if count == 0 {
            return;
        }
        unsafe {
            let parent = QModelIndex::default();
            let top = self.as_mut().model_index(0, 0, &parent);
            let bottom = self.as_mut().model_index(count as i32 - 1, 0, &parent);
            let mut roles = QList::<i32>::default();
            roles.append(ROLE_GIT);
            self.as_mut().data_changed(&top, &bottom, &roles);
        }
    }

    fn git_checkout_branch(mut self: Pin<&mut Self>, name: &QString, remote: bool) {
        let name = name.to_string();
        if name.trim().is_empty() {
            self.as_mut()
                .fail_git_operation("checkout", "Branch name cannot be empty");
            return;
        }
        self.as_mut()
            .start_git_operation(GitOperation::Checkout { name, remote });
    }

    fn git_create_branch(mut self: Pin<&mut Self>, name: &QString, checkout: bool) {
        let name = name.to_string().trim().to_owned();
        if name.is_empty() {
            self.as_mut()
                .fail_git_operation("create", "Branch name cannot be empty");
            return;
        }
        self.as_mut()
            .start_git_operation(GitOperation::Create { name, checkout });
    }

    fn git_fetch(mut self: Pin<&mut Self>) {
        self.as_mut().start_git_operation(GitOperation::Fetch);
    }

    fn git_pull(mut self: Pin<&mut Self>) {
        self.as_mut().start_git_operation(GitOperation::Pull);
    }

    fn git_push(mut self: Pin<&mut Self>) {
        self.as_mut().start_git_operation(GitOperation::Push);
    }

    fn git_sync(mut self: Pin<&mut Self>) {
        self.as_mut().start_git_operation(GitOperation::Sync);
    }

    fn start_git_operation(mut self: Pin<&mut Self>, operation: GitOperation) {
        if self.git_busy || self.closed {
            return;
        }
        let path = self.path.to_string();
        if path.is_empty() || !matches!(vfs::scheme_of(&path), Ok(Scheme::Local)) {
            let kind = operation.kind();
            self.as_mut()
                .fail_git_operation(kind, "Git operations are not supported in this directory");
            return;
        }

        let kind = operation.kind().to_owned();
        let success = operation.success_message();
        self.as_mut().set_git_busy(true);
        let thread = self.as_mut().qt_thread();
        crate::runtime::spawn(async move {
            let result = match operation {
                GitOperation::Checkout { name, remote } => {
                    git::git_checkout_branch(path, name, remote).await
                }
                GitOperation::Create { name, checkout } => {
                    git::git_create_branch(path, name, checkout).await
                }
                GitOperation::Fetch => git::git_fetch(path).await,
                GitOperation::Pull => git::git_pull(path).await,
                GitOperation::Push => git::git_push(path).await,
                GitOperation::Sync => git::git_sync(path).await,
            };
            let _ = thread
                .queue(move |mut model| model.as_mut().finish_git_operation(kind, success, result));
        });
    }

    fn finish_git_operation(
        mut self: Pin<&mut Self>,
        kind: String,
        success: String,
        result: Result<(), dae_core::error::FileSystemError>,
    ) {
        if self.closed {
            return;
        }
        self.as_mut().set_git_busy(false);
        match result {
            Ok(()) => {
                self.as_mut().git_operation_finished(
                    &QString::from(kind.as_str()),
                    &QString::from(success.as_str()),
                );
                self.as_mut().refresh_git();
            }
            Err(error) => {
                let message = crate::errors::friendly_error(&error.to_string());
                self.as_mut().git_operation_failed(
                    &QString::from(kind.as_str()),
                    &QString::from(message.as_str()),
                );
            }
        }
    }

    fn fail_git_operation(mut self: Pin<&mut Self>, kind: &str, message: &str) {
        self.as_mut()
            .git_operation_failed(&QString::from(kind), &QString::from(message));
    }

    fn set_order(
        mut self: Pin<&mut Self>,
        sort_key: &QString,
        sort_order: &QString,
        folder_order: &QString,
    ) {
        let mut next = self.order;
        next.key = SortKey::parse(&sort_key.to_string());
        next.order = SortOrder::parse(&sort_order.to_string());
        next.folders = FolderOrder::parse(&folder_order.to_string());
        self.as_mut().apply_order(next);
    }

    fn set_group(mut self: Pin<&mut Self>, group_key: &QString, group_date: &QString) {
        let mut next = self.order;
        next.grouping = Grouping {
            key: GroupKey::parse(&group_key.to_string()),
            date: GroupDate::parse(&group_date.to_string()),
        };
        self.as_mut().apply_order(next);
    }

    /// Adopts `next`, recomputes the section labels it implies and re-sorts.
    /// A tag order or tag grouping needs every row's tags first; that read is
    /// started here and re-applies the sort when it lands.
    fn apply_order(mut self: Pin<&mut Self>, next: ViewOrder) {
        if self.order == next {
            return;
        }
        let needs_tags = next.key == SortKey::Tag || next.grouping.key == GroupKey::Tag;
        {
            let mut rust = self.as_mut().rust_mut();
            rust.order = next;
            assign_groups(&mut rust.rows, next.grouping);
        }
        if needs_tags && !self.tags_complete {
            self.as_mut().start_full_tag_load();
        }
        if self.rows.len() < 2 || rows_are_sorted(&self.rows, &next) {
            return;
        }
        if self.rows.len() < WORKER_SORT_MIN_ROWS {
            self.as_mut().resort_rows();
        } else {
            self.as_mut().start_worker_sort();
        }
    }

    fn set_directory_size(mut self: Pin<&mut Self>, path: &QString, bytes: f64) {
        let path = path.to_string();
        if path.is_empty() || !bytes.is_finite() || bytes < 0.0 {
            return;
        }
        let bytes = bytes.round() as u64;
        if self
            .folder_sizes
            .get(&path)
            .is_some_and(|stored| *stored == bytes)
        {
            return;
        }
        self.as_mut()
            .rust_mut()
            .folder_sizes
            .insert(path.clone(), bytes);

        let Some(index) = self
            .rows
            .iter()
            .position(|row| row.full_path(&self.base) == path)
        else {
            return;
        };
        unsafe {
            let parent = QModelIndex::default();
            let cell = self.as_mut().model_index(index as i32, 0, &parent);
            let mut roles = QList::<i32>::default();
            roles.append(ROLE_SIZE);
            self.as_mut().data_changed(&cell, &cell, &roles);
        }
    }

    fn request_tags(mut self: Pin<&mut Self>, first: i32, last: i32) {
        let Ok(first) = usize::try_from(first) else {
            return;
        };
        let last = usize::try_from(last)
            .unwrap_or(usize::MAX)
            .min(self.rows.len().saturating_sub(1));
        if first > last || first >= self.rows.len() {
            return;
        }
        let mut paths: Vec<String> = Vec::with_capacity(last - first + 1);
        for row in &self.rows[first..=last] {
            let path = row.full_path(&self.base);
            if !self.tags_cache.contains_key(&path) && !paths.contains(&path) {
                paths.push(path);
            }
        }
        if !paths.is_empty() {
            self.as_mut().start_tag_load(paths, false);
        }
    }

    fn refresh_tags(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        if paths.is_empty() {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            // An edit invalidates any read that started before it, even one
            // for a different path: the definitions may have changed with it.
            rust.tag_generation = rust.tag_generation.wrapping_add(1);
            for path in &paths {
                rust.tags_cache.remove(path);
            }
        }
        // A tag edit can change a sort or grouping that reads the tags, so a
        // refresh also re-runs the order once the new ids are in.
        self.as_mut().start_tag_load(paths, false);
    }

    /// Reads `paths`' tags on a blocking thread. Results carry the load's
    /// generation and the tag epoch; a load that started before an edit (or
    /// before a navigation) is dropped instead of repainting stale ids.
    fn start_tag_load(mut self: Pin<&mut Self>, paths: Vec<String>, full: bool) {
        let generation = self.generation;
        let epoch = self.tag_generation;
        let thread = self.as_mut().qt_thread();
        crate::runtime::spawn(async move {
            let read = tokio::task::spawn_blocking(move || dae_core::fs::tags::tags_for(&paths));
            if let Ok(entries) = read.await {
                let _ = thread.queue(move |mut model| {
                    model.as_mut().adopt_tags(generation, epoch, full, entries)
                });
            }
        });
    }

    fn start_full_tag_load(mut self: Pin<&mut Self>) {
        if self.rows.is_empty() || self.tags_complete {
            return;
        }
        let base = self.base.to_string();
        let mut paths: Vec<String> = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let path = row.full_path(&base);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        self.as_mut().start_tag_load(paths, true);
    }

    fn adopt_tags(
        mut self: Pin<&mut Self>,
        generation: u64,
        epoch: u64,
        full: bool,
        entries: Vec<dae_core::fs::tags::EntryTags>,
    ) {
        if self.generation != generation || self.tag_generation != epoch {
            return;
        }
        let base = self.base.to_string();
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_path: HashMap<String, Vec<String>> = HashMap::new();
        for entry in entries {
            self.as_mut()
                .rust_mut()
                .tags_cache
                .insert(entry.path.clone(), entry.tag_ids.clone());
            match entry.path.strip_prefix(base.as_str()) {
                Some(name) if !name.contains(['/', '\\']) => {
                    by_name.insert(name.to_owned(), entry.tag_ids);
                }
                _ => {
                    by_path.insert(entry.path, entry.tag_ids);
                }
            }
        }
        let definitions = self.tag_definitions.clone();
        {
            let DirectoryListModelRust {
                rows, tags_cache, ..
            } = &mut *self.as_mut().rust_mut();
            for row in rows.iter_mut() {
                let ids = match by_name.get(row.name.as_str()) {
                    Some(ids) => Some(ids),
                    None => {
                        let path = row.full_path(&base);
                        by_path.get(&path).or_else(|| tags_cache.get(&path))
                    }
                };
                if let Some(ids) = ids
                    && row.tag_ids != *ids
                {
                    set_row_tags(row, ids, &definitions);
                }
            }
        }
        if full {
            self.as_mut().rust_mut().tags_complete = true;
        }
        self.as_mut().emit_tag_roles();
        if self.order.key == SortKey::Tag || self.order.grouping.key == GroupKey::Tag {
            let grouping = self.order.grouping;
            {
                let mut rust = self.as_mut().rust_mut();
                assign_groups(&mut rust.rows, grouping);
            }
            self.as_mut().resort_if_needed();
        }
    }

    /// Repaints the tag and section roles for every row in one signal. The
    /// view only re-reads its visible delegates, so the range costs nothing
    /// per row in QML.
    fn emit_tag_roles(mut self: Pin<&mut Self>) {
        let count = self.rows.len();
        if count == 0 {
            return;
        }
        unsafe {
            let parent = QModelIndex::default();
            let top = self.as_mut().model_index(0, 0, &parent);
            let bottom = self.as_mut().model_index(count as i32 - 1, 0, &parent);
            let mut roles = QList::<i32>::default();
            roles.append(ROLE_TAGS);
            roles.append(ROLE_TAG_IDS);
            roles.append(ROLE_TAG_COLORS);
            roles.append(ROLE_GROUP);
            self.as_mut().data_changed(&top, &bottom, &roles);
        }
    }

    fn resort_if_needed(mut self: Pin<&mut Self>) {
        if self.rows.len() < 2 || rows_are_sorted(&self.rows, &self.order) {
            return;
        }
        if self.rows.len() < WORKER_SORT_MIN_ROWS {
            self.as_mut().resort_rows();
        } else {
            self.as_mut().start_worker_sort();
        }
    }

    fn refresh_tag_definitions(mut self: Pin<&mut Self>) {
        let definitions = dae_core::fs::tags::definitions();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.tag_definitions = definitions;
            // A renamed tag changes the sort key and the section labels.
            let order = rust.order;
            let base = rust.base.clone();
            let definitions = rust.tag_definitions.clone();
            let mut cache = rust.tags_cache.clone();
            for row in rust.rows.iter_mut() {
                if let Some(ids) = cache.remove(&row.full_path(&base)) {
                    set_row_tags(row, &ids, &definitions);
                }
            }
            assign_groups(&mut rust.rows, order.grouping);
        }
        self.as_mut().emit_tag_roles();
        self.as_mut().resort_if_needed();
    }

    fn set_filters(
        mut self: Pin<&mut Self>,
        show_hidden: bool,
        kind: &QString,
        modified: &QString,
        size: &QString,
    ) {
        let next = RowFilter {
            show_hidden,
            kind: KindFilter::parse(&kind.to_string()),
            modified: ModifiedFilter::parse(&modified.to_string()),
            size: SizeFilter::parse(&size.to_string()),
        };
        if self.filter == next {
            return;
        }
        self.as_mut().rust_mut().filter = next;
        if !self.path.as_slice().is_empty() {
            self.as_mut().reload();
        }
    }

    /// Re-sorts the rows already in the model. The viewport is told to
    /// remember its position first: a reset drops both the rows and the
    /// position, and `FileList` restores the latter once the rows are back.
    fn resort_rows(mut self: Pin<&mut Self>) {
        let order = self.order;
        // A sync re-sort supersedes any worker sort still in flight: its
        // permutation no longer describes the rows this call is about to sort.
        {
            let mut rust = self.as_mut().rust_mut();
            rust.sorting = false;
            rust.sort_request = rust.sort_request.wrapping_add(1);
        }
        let mut rows = std::mem::take(&mut self.as_mut().rust_mut().rows);
        self.as_mut().about_to_reload();
        // The two halves of the stall a QML trace cannot see: the comparator
        // and `begin/endResetModel` (plus the QML `aboutToReload` handler that
        // saves the scroll position). `DAE_LOG=debug` prints it next to the
        // watcher lines.
        let sort_started = std::time::Instant::now();
        sort_rows(&mut rows, &order);
        let sort_ms = sort_started.elapsed().as_secs_f64() * 1e3;
        let total = rows.len() as i32;
        let reset_started = std::time::Instant::now();
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let DirectoryListModelRust {
                    rows: stored,
                    selection,
                    ..
                } = &mut *self.as_mut().rust_mut();
                *stored = rows;
                // The flags travelled with their rows; the indices that point
                // at them did not.
                selection.invalidate();
            }
            self.as_mut().end_reset_model();
        }
        let reset_ms = reset_started.elapsed().as_secs_f64() * 1e3;
        log::debug!(
            target: "dae::listing",
            "re-sort: {} rows, sort {:.1} ms, reset {:.1} ms",
            total,
            sort_ms,
            reset_ms
        );
        self.as_mut().set_loaded(total);
    }

    /// Applies the visibility filter to a batch that is about to be appended.
    /// A refresh stage is left unfiltered: it is filtered once, at the swap,
    /// so a filter that changed during the read is honoured too.
    fn keep_visible(&self, rows: Vec<StoredRow>) -> Vec<StoredRow> {
        if self.filter == RowFilter::default() {
            return rows;
        }
        let filter = self.filter;
        let cutoff = filter.modified_cutoff();
        rows.into_iter()
            .filter(|row| filter.keeps(row, cutoff))
            .collect()
    }

    /// Counts the two things the adaptive rule asks about. One pass at
    /// completion, never per batch.
    fn publish_counts(mut self: Pin<&mut Self>) {
        let mut images = 0;
        let mut files = 0;
        for row in &self.rows {
            match row.kind {
                EntryKind::File => {
                    files += 1;
                    if category_of(row.kind, &row.name) == "image" {
                        images += 1;
                    }
                }
                EntryKind::Directory => {}
                _ => {}
            }
        }
        if self.images != images {
            self.as_mut().set_images(images);
        }
        if self.files != files {
            self.as_mut().set_files(files);
        }
    }

    fn index_of_path(&self, path: &QString) -> i32 {
        let path = path.to_string();
        row_for_path(&self.rows, &self.base, &path)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.full_path(&self.base).as_str()))
            .unwrap_or_default()
    }

    /// Republishes the selected count and repaints the spans whose flags moved.
    /// One `dataChanged` per contiguous run, carrying only the selected role:
    /// the delegate re-reads one property, and rows whose flag did not move are
    /// not touched at all.
    fn publish_selection(mut self: Pin<&mut Self>, runs: Vec<Run>) {
        self.as_mut().publish_selection_count();
        if runs.is_empty() {
            return;
        }
        unsafe {
            let parent = QModelIndex::default();
            let mut roles = QList::<i32>::default();
            roles.append(ROLE_SELECTED);
            for (first, last) in runs {
                let top = self.as_mut().model_index(first as i32, 0, &parent);
                let bottom = self.as_mut().model_index(last as i32, 0, &parent);
                self.as_mut().data_changed(&top, &bottom, &roles);
            }
        }
    }

    fn publish_selection_count(mut self: Pin<&mut Self>) {
        let count = self.selection.count();
        if self.selected_count != count {
            self.as_mut().set_selected_count(count);
        }
    }

    fn select_only(mut self: Pin<&mut Self>, row: i32) {
        // A negative row addresses nothing, and `usize::MAX` is past the last
        // row either way: both clear, which is what a click on a row that is
        // no longer there has to mean.
        let row = usize::try_from(row).unwrap_or(usize::MAX);
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_only(rows, row)
        };
        self.as_mut().publish_selection(runs);
    }

    fn toggle_at(mut self: Pin<&mut Self>, row: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.toggle(rows, row)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_range(mut self: Pin<&mut Self>, first: i32, last: i32) {
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_range(rows, first, last)
        };
        self.as_mut().publish_selection(runs);
    }

    fn select_all_rows(mut self: Pin<&mut Self>) {
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.select_all(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn clear_selection(mut self: Pin<&mut Self>) {
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.clear(rows)
        };
        self.as_mut().publish_selection(runs);
    }

    fn set_selected_paths(mut self: Pin<&mut Self>, paths: &QStringList) {
        let base = self.base.to_string();
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.restore(rows, &paths, &base)
        };
        self.as_mut().publish_selection(runs);
    }

    fn selected_paths(&self) -> QStringList {
        let mut paths = QStringList::default();
        for row in self.rows.iter().filter(|row| row.selected) {
            paths.append(QString::from(row.full_path(&self.base).as_str()));
        }
        paths
    }

    fn begin_marquee(mut self: Pin<&mut Self>, keep: bool) {
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.begin_marquee(rows, keep)
        };
        self.as_mut().publish_selection(runs);
    }

    fn marquee_cells(
        mut self: Pin<&mut Self>,
        first_row: i32,
        first_column: i32,
        last_row: i32,
        last_column: i32,
        columns: i32,
    ) {
        let band = match (usize::try_from(first_row), usize::try_from(last_row)) {
            (Ok(first_row), Ok(last_row))
                if last_row >= first_row
                    && columns > 0
                    && first_column >= 0
                    && last_column >= first_column =>
            {
                Some(Band {
                    first_row,
                    last_row,
                    first_column: first_column as usize,
                    last_column: last_column as usize,
                    columns: columns as usize,
                })
            }
            _ => None,
        };
        let runs = {
            let DirectoryListModelRust {
                rows, selection, ..
            } = &mut *self.as_mut().rust_mut();
            selection.marquee_cells(rows, band)
        };
        self.as_mut().publish_selection(runs);
    }

    fn end_marquee(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().selection.end_marquee();
    }

    fn index_of_name_prefix(&self, needle: &QString, start: i32) -> i32 {
        let needle = needle.to_string();
        let start = usize::try_from(start).unwrap_or(0);
        find_name_prefix(&self.rows, start, &needle)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn is_directory_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|entry| entry.kind == EntryKind::Directory)
            .unwrap_or(false)
    }

    /// The visible names of `ids`, in definition order. Unknown ids are
    /// dropped: a definition the user deleted must not keep painting.
    fn tag_names(&self, ids: &[String]) -> String {
        self.tag_definitions
            .iter()
            .filter(|definition| ids.contains(&definition.id))
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn tag_colors(&self, ids: &[String]) -> String {
        self.tag_definitions
            .iter()
            .filter(|definition| ids.contains(&definition.id))
            .map(|definition| definition.color.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn dispose(mut self: Pin<&mut Self>) {
        watch::unwatch_directory(self.watcher_id.clone());
        self.as_mut().rust_mut().closed = true;
        self.as_mut().rust_mut().reload_pending = false;
        self.as_mut().rust_mut().refresh = None;
        if let Some(stream_id) = self.as_mut().rust_mut().stream_id.take() {
            listing::cancel_directory_listing(stream_id);
        }
        // A batch already queued on the Qt thread carries the old generation
        // and is dropped instead of appending to a model its window let go.
        let generation = self.generation.wrapping_add(1);
        let git_generation = self.git_generation.wrapping_add(1);
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let mut rust = self.as_mut().rust_mut();
                rust.generation = generation;
                rust.git_generation = git_generation;
                rust.rows.clear();
                rust.base.clear();
                rust.selection.reset();
                rust.tags_cache.clear();
                rust.tags_complete = false;
                rust.tag_generation = rust.tag_generation.wrapping_add(1);
                rust.git_entries.clear();
                rust.git_directory_untracked = false;
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_remote(false);
        self.as_mut().set_loaded(0);
        self.as_mut().publish_selection_count();
        self.as_mut().set_status(QString::from("Closed"));
        self.as_mut().set_finished(true);
        self.as_mut().set_images(0);
        self.as_mut().set_files(0);
        self.as_mut().set_git_busy(false);
        self.as_mut().clear_git_state();
    }

    /// Publishes the ancestors of the shown directory to the breadcrumb bar.
    ///
    /// Skipped when nothing changed: a refresh hands back the ancestors of the
    /// directory already on screen, and rewriting both lists would rebind every
    /// crumb in the bar for no reason.
    fn adopt_breadcrumbs(mut self: Pin<&mut Self>, breadcrumbs: Vec<Breadcrumb>) {
        let count = breadcrumbs.len();
        let unchanged = self.breadcrumb_names.len() == count as isize
            && self
                .breadcrumb_names
                .iter()
                .zip(breadcrumbs.iter())
                .all(|(name, crumb)| name.to_string() == crumb.name)
            && self
                .breadcrumb_paths
                .iter()
                .zip(breadcrumbs.iter())
                .all(|(path, crumb)| path.to_string() == crumb.path);
        if unchanged {
            return;
        }

        let mut names = QStringList::default();
        let mut paths = QStringList::default();
        for crumb in &breadcrumbs {
            names.append(QString::from(crumb.name.as_str()));
            paths.append(QString::from(crumb.path.as_str()));
        }

        self.as_mut().set_breadcrumb_names(names);
        self.as_mut().set_breadcrumb_paths(paths);
    }

    fn append_head(
        mut self: Pin<&mut Self>,
        generation: u64,
        base: String,
        path: String,
        breadcrumbs: Vec<Breadcrumb>,
        rows: Vec<StoredRow>,
        done: bool,
    ) {
        if self.generation != generation {
            return;
        }

        // Set before the rows: the bar names the directory the rows belong to,
        // and it is what the user sees first while a large listing streams.
        self.as_mut().adopt_breadcrumbs(breadcrumbs);

        if self.refresh.is_some() {
            if let Some(stage) = self.as_mut().rust_mut().refresh.as_mut() {
                stage.base = base;
                stage.path = path;
                stage.rows.extend(rows);
            }
            if done {
                self.as_mut().finish_refresh(generation);
            }
            return;
        }

        self.as_mut().set_path(QString::from(path.as_str()));
        self.as_mut().rust_mut().base = base;
        self.as_mut().append_rows(generation, rows, done);
    }

    /// Applies the current grouping and any tags already read to a batch that
    /// is about to be inserted. Batch-local so an active section header is
    /// right before the completion sort, not only after it.
    fn decorate_rows(&self, rows: &mut [StoredRow]) {
        let grouping = self.order.grouping;
        if grouping.active() {
            assign_groups(rows, grouping);
        }
        if !self.tags_cache.is_empty() {
            decorate_tags(
                rows,
                &self.tags_cache,
                &self.tag_definitions,
                &self.base.to_string(),
            );
            if grouping.active() {
                assign_groups(rows, grouping);
            }
        }
    }

    fn append_rows(mut self: Pin<&mut Self>, generation: u64, rows: Vec<StoredRow>, done: bool) {
        if self.generation != generation {
            return;
        }

        if self.refresh.is_some() {
            if let Some(stage) = self.as_mut().rust_mut().refresh.as_mut() {
                stage.rows.extend(rows);
            }
            if done {
                self.as_mut().finish_refresh(generation);
            }
            return;
        }

        let mut rows = self.as_mut().keep_visible(rows);
        self.as_mut().decorate_rows(&mut rows);
        let added = rows.len() as i32;
        let first = self.loaded;
        if added > 0 {
            let last = first + added - 1;
            unsafe {
                self.as_mut()
                    .begin_insert_rows(&QModelIndex::default(), first, last);
                {
                    let DirectoryListModelRust {
                        rows: stored,
                        base,
                        selection,
                        ..
                    } = &mut *self.as_mut().rust_mut();
                    // A history entry restored before its listing was read has
                    // been waiting for exactly these rows; the head batch is
                    // what tells the selection which base their paths are
                    // spelled from.
                    selection.resolve_pending(base);
                    selection.rows_inserted(&mut rows);
                    stored.extend(rows);
                }
                // First batch only, and before the view sees the insertion: the
                // data moment, not the layout that follows it. See
                // `crates/dae-qt/src/startup.rs`.
                if first == 0 {
                    crate::window::startup_mark_once("qml-first-model");
                }
                self.as_mut().end_insert_rows();
            }
            self.as_mut().set_loaded(first + added);
            self.as_mut().publish_selection_count();
        }

        if done {
            // Streamed chunks arrive in enumeration order, so each one is
            // sorted on its own but the sequence as a whole is not. Sorting
            // once here is what makes the finished listing obey the chosen
            // order without paying a reset per batch. `aboutToReload` inside
            // `resortRows` lets the view keep its scroll position.
            self.as_mut().sort_at_completion();
            // The stream is over: a restored path that never matched a row is
            // a file that was renamed or deleted since the entry was captured.
            self.as_mut().rust_mut().selection.settle();
            let pending = self.reload_pending;
            self.as_mut().rust_mut().stream_id = None;
            if pending {
                self.as_mut().rust_mut().reload_pending = false;
            }
            let total = self.rows.len() as i32;
            self.as_mut().publish_counts();
            // Status before `finished`: QML reads the status in
            // `onFinishedChanged`, which is emitted synchronously.
            self.as_mut().set_status(QString::from(format!(
                "{total} item{}",
                if total == 1 { "" } else { "s" }
            )));
            self.as_mut().set_finished(true);
            // A change arrived during this read, so the rows just shown may
            // already be stale. Re-read now that the stream is over instead of
            // cancelling it mid-flight.
            if pending {
                self.as_mut().reload();
            }
        }
    }

    fn sort_at_completion(mut self: Pin<&mut Self>) {
        if self.rows.len() < 2 || rows_are_sorted(&self.rows, &self.order) {
            return;
        }
        // A worker and a queued reply cost more than the sort for a directory
        // that fits in a few batches; only the large listings are offloaded.
        if self.rows.len() < WORKER_SORT_MIN_ROWS {
            self.as_mut().resort_rows();
        } else {
            self.as_mut().start_worker_sort();
        }
    }

    /// Snapshots the comparator's inputs and sorts an index permutation on a
    /// worker. The rows stay in the model and keep painting in their old order
    /// until `apply_worker_sort` lands on the Qt thread; a load, order change,
    /// row-count change or newer sort in between drops the result.
    fn start_worker_sort(mut self: Pin<&mut Self>) {
        let ticket = SortTicket {
            generation: self.generation,
            rows: self.rows.len(),
            order: self.order,
            request: self.sort_request.wrapping_add(1),
        };
        let snapshot_started = std::time::Instant::now();
        let entries = sort_snapshot(&self.rows);
        log::debug!(
            target: "dae::listing",
            "sort: {} rows, snapshot {:.1} ms",
            ticket.rows,
            snapshot_started.elapsed().as_secs_f64() * 1e3
        );
        {
            let mut rust = self.as_mut().rust_mut();
            rust.sort_request = ticket.request;
            rust.sorting = true;
        }
        let thread = self.as_mut().qt_thread();
        std::thread::spawn(move || {
            let sort_started = std::time::Instant::now();
            let perm = sort_permutation(&entries, &ticket.order);
            let sort_ms = sort_started.elapsed().as_secs_f64() * 1e3;
            let _ = thread
                .queue(move |mut model| model.as_mut().apply_worker_sort(ticket, perm, sort_ms));
        });
    }

    /// Applies a worker's permutation with the same one-reset shape as the
    /// synchronous path, plus `sortApplied` so the view can restore the scroll
    /// position the reset drops.
    fn apply_worker_sort(
        mut self: Pin<&mut Self>,
        ticket: SortTicket,
        perm: Vec<u32>,
        sort_ms: f64,
    ) {
        // Only the newest request may clear the flag: a superseded sort has
        // nothing to say about the rows the newer one is ordering.
        if self.sort_request == ticket.request {
            self.as_mut().rust_mut().sorting = false;
        }
        if !worker_sort_is_current(
            self.closed,
            self.generation,
            self.rows.len(),
            self.order,
            self.sort_request,
            ticket,
            &perm,
        ) {
            return;
        }
        let expected = ticket.rows;
        let permute_started = std::time::Instant::now();
        let rows = std::mem::take(&mut self.as_mut().rust_mut().rows);
        // `is_permutation` above is the same check `apply_permutation` makes;
        // reaching its failure branch would lose every row.
        let rows = apply_permutation(rows, &perm).expect("validated permutation");
        let permute_ms = permute_started.elapsed().as_secs_f64() * 1e3;
        self.as_mut().about_to_reload();
        let reset_started = std::time::Instant::now();
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let DirectoryListModelRust {
                    rows: stored,
                    selection,
                    ..
                } = &mut *self.as_mut().rust_mut();
                *stored = rows;
                selection.invalidate();
            }
            self.as_mut().end_reset_model();
        }
        let reset_ms = reset_started.elapsed().as_secs_f64() * 1e3;
        log::debug!(
            target: "dae::listing",
            "re-sort: {} rows, sort {:.1} ms, permute {:.1} ms, reset {:.1} ms",
            expected,
            sort_ms,
            permute_ms,
            reset_ms
        );
        self.as_mut().sort_applied();
    }

    /// Applies a completed refresh. The new rows are already in hand, so the
    /// model can edit itself in place: the diff keeps every unchanged row and
    /// only inserts, removes or replaces the ones that differ. A full reset is
    /// reserved for a refresh that also changes the base or path, where the
    /// rows would address different files.
    fn finish_refresh(mut self: Pin<&mut Self>, generation: u64) {
        if self.generation != generation {
            self.as_mut().rust_mut().refresh = None;
            return;
        }
        // The staged rows replace what a worker may still be permuting; its
        // indices describe the old rows even when the count happens to match.
        {
            let mut rust = self.as_mut().rust_mut();
            rust.sorting = false;
            rust.sort_request = rust.sort_request.wrapping_add(1);
        }
        let Some(stage) = self.as_mut().rust_mut().refresh.take() else {
            return;
        };
        let RefreshStage { rows, base, path } = stage;
        // The stage holds the raw listing: filter and order it the same way a
        // first read would, so a refresh neither resurrects a hidden row nor
        // changes the order under the user.
        let mut fresh = self.as_mut().keep_visible(rows);
        sort_rows(&mut fresh, &self.order);

        // The rows below are rebuilt, so the selection is captured by key: a
        // row that only moved keeps its flag, and one that really went away
        // leaves the count with it.
        let keep: std::collections::HashSet<String> = if self.selection.count() > 0 {
            self.selection
                .selected_keys(&self.rows)
                .into_iter()
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        let pending = self.reload_pending;
        let total = fresh.len() as i32;
        let path_changed = self.path.to_string() != path;
        let base_changed = self.base != base;

        self.as_mut().rust_mut().stream_id = None;
        self.as_mut().rust_mut().reload_pending = false;

        if path_changed || base_changed {
            if path_changed {
                self.as_mut().set_path(QString::from(path.as_str()));
            }
            self.as_mut().about_to_reload();
            unsafe {
                self.as_mut().begin_reset_model();
                {
                    let DirectoryListModelRust {
                        rows: stored,
                        base: stored_base,
                        selection,
                        ..
                    } = &mut *self.as_mut().rust_mut();
                    *stored = fresh;
                    *stored_base = base;
                    *selection = Selection::default();
                    selection.flag(stored, &keep);
                }
                self.as_mut().end_reset_model();
            }
            self.as_mut().set_loaded(total);
            self.as_mut().publish_selection_count();
        } else {
            self.as_mut().apply_refreshed_rows(fresh, &keep);
        }

        self.as_mut().publish_counts();
        self.as_mut().set_status(QString::from(format!(
            "{total} item{}",
            if total == 1 { "" } else { "s" }
        )));
        self.as_mut().set_finished(true);
        // A change arrived during this read; re-read now that the stream is
        // over instead of having shown a mix of the two listings.
        if pending {
            self.as_mut().reload();
        }
    }

    /// Turns `fresh` into the smallest set of row operations that reaches it
    /// from the rows on screen. Unchanged rows keep their delegates, so a
    /// create/delete does not rebuild the whole view. `keep` holds the selected
    /// rows' keys: the ones that were re-inserted get their flags back, and the
    /// ones that are gone leave the count with them.
    fn apply_refreshed_rows(
        mut self: Pin<&mut Self>,
        mut fresh: Vec<StoredRow>,
        keep: &std::collections::HashSet<String>,
    ) {
        let (prefix, removed, added) = row_edit(&self.rows, &fresh);
        let mut middle: Vec<StoredRow> = fresh.drain(prefix..prefix + added).collect();

        unsafe {
            if removed > 0 {
                self.as_mut().begin_remove_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + removed - 1) as i32,
                );
                {
                    let DirectoryListModelRust {
                        rows, selection, ..
                    } = &mut *self.as_mut().rust_mut();
                    selection.rows_removed(&rows[prefix..prefix + removed]);
                    rows.drain(prefix..prefix + removed);
                }
                self.as_mut().end_remove_rows();
            }
            if added > 0 {
                self.as_mut().begin_insert_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + added - 1) as i32,
                );
                {
                    let DirectoryListModelRust {
                        rows, selection, ..
                    } = &mut *self.as_mut().rust_mut();
                    selection.flag(&mut middle, keep);
                    rows.splice(prefix..prefix, middle);
                }
                self.as_mut().end_insert_rows();
            }
        }

        let total = self.rows.len() as i32;
        self.as_mut().set_loaded(total);
        self.as_mut().publish_selection_count();
    }

    /// A watcher reported a change in `change`. The path is re-checked because an
    /// arm that lost a navigation can still have an event in flight.
    fn reload_after_change(mut self: Pin<&mut Self>, change: Change) {
        if self.closed {
            return;
        }
        if self.stream_id.is_some() {
            // The displayed path may still be the requested spelling while the
            // head is in flight; the reload waits for it to adopt the
            // canonical one instead of being dropped by the comparison below.
            self.as_mut().rust_mut().reload_pending = true;
            return;
        }
        if self.path.to_string() != change.path {
            return;
        }
        match change.names {
            Some(names) if self.can_patch(&names) => {
                log::debug!(
                    target: "dae::listing",
                    "watcher: patching {} child(ren) of {}",
                    names.len(),
                    change.path
                );
                self.as_mut().patch_rows(names);
            }
            _ => {
                log::debug!(target: "dae::listing", "watcher: re-reading {}", change.path);
                self.as_mut().reload();
            }
        }
    }

    /// Whether a burst can be applied to the rows on screen instead of re-reading
    /// the directory. See [`plan_patch`] for what a patch is allowed to do.
    fn can_patch(&self, names: &[String]) -> bool {
        !self.remote
            && self.finished
            && !self.sorting
            && !self.load_failed
            && self.refresh.is_none()
            && !names.is_empty()
            && names.len() <= PATCH_LIMIT
            // Only the plain name order is stable under a metadata change: any
            // other key or an active grouping would have to move the patched
            // row (and re-sort the listing).
            && self.order.key == SortKey::Name
            && !self.order.grouping.active()
            // A non-default filter dropped rows at insert time, so a row that
            // now passes it cannot be recovered without a re-read.
            && self.filter == RowFilter::default()
    }

    /// Reads the changed children off the GUI thread; `apply_patch` lands on the
    /// Qt thread carrying the generation the read started under.
    fn patch_rows(mut self: Pin<&mut Self>, names: Vec<String>) {
        let directory = self.path.to_string();
        let generation = self.generation;
        let thread = self.as_mut().qt_thread();
        crate::runtime::spawn(async move {
            let requested = names.clone();
            let result = dae_core::fs::service::read_directory_changes(directory, requested).await;
            let _ = thread
                .queue(move |mut model| model.as_mut().apply_patch(generation, names, result));
        });
    }

    fn apply_patch(
        mut self: Pin<&mut Self>,
        generation: u64,
        names: Vec<String>,
        result: Result<Vec<Option<DirectoryEntry>>, dae_core::fs::error::FileSystemError>,
    ) {
        if self.closed || self.generation != generation {
            return;
        }
        // A read started while the patch was being resolved (the user
        // navigated, or the directory was refreshed). It delivers the rows
        // itself, and re-reading now would cancel it and start the same walk
        // again; ask for one more read after it instead.
        if self.stream_id.is_some() {
            self.as_mut().rust_mut().reload_pending = true;
            return;
        }
        // The state the patch was resolved against moved on: the last read
        // failed, or the order or the filter no longer matches the rows.
        if self.refresh.is_some()
            || !self.finished
            || self.load_failed
            || self.order.key != SortKey::Name
            || self.filter != RowFilter::default()
        {
            log::debug!(
                target: "dae::listing",
                "watcher: state moved on, re-reading {}",
                self.path
            );
            self.as_mut().reload();
            return;
        }
        let Ok(entries) = result else {
            // A stat that failed (the directory went away, a permission change)
            // is answered better by a read, which also reports the failure.
            log::debug!(
                target: "dae::listing",
                "watcher: patch read failed, re-reading {}",
                self.path
            );
            self.as_mut().reload();
            return;
        };
        let Some(patch) = plan_patch(&self.rows, &self.base, &names, &entries) else {
            // A child changed kind, so its place in the folder grouping is no
            // longer the one the rows have. Let the reader sort that out.
            log::debug!(
                target: "dae::listing",
                "watcher: a child changed kind or spelling, re-reading {}",
                self.path
            );
            self.as_mut().reload();
            return;
        };
        if patch.is_empty() {
            return;
        }
        self.as_mut().apply_row_patch(patch);
    }

    /// Applies a resolved patch with the smallest signals it can: one remove or
    /// insert per changed row, and one `dataChanged` span per run of
    /// replacements. No reset, so the scroll position and the delegates survive.
    fn apply_row_patch(mut self: Pin<&mut Self>, patch: RowPatch) {
        let RowPatch {
            mut removals,
            replacements,
            insertions,
        } = patch;
        let (removed, replaced, inserted) = (removals.len(), replacements.len(), insertions.len());
        removals.sort_unstable();

        for &index in removals.iter().rev() {
            unsafe {
                self.as_mut().begin_remove_rows(
                    &QModelIndex::default(),
                    index as i32,
                    index as i32,
                );
                {
                    let DirectoryListModelRust {
                        rows, selection, ..
                    } = &mut *self.as_mut().rust_mut();
                    selection.rows_removed(&rows[index..index + 1]);
                    rows.remove(index);
                }
                self.as_mut().end_remove_rows();
            }
        }

        let mut changed: Vec<(usize, StoredRow)> = replacements
            .into_iter()
            .map(|(index, row)| (index_after_removals(index, &removals), row))
            .collect();
        changed.sort_unstable_by_key(|(index, _)| *index);

        let mut runs: Vec<(i32, i32)> = Vec::new();
        for (index, _) in &changed {
            match runs.last_mut() {
                Some((_, last)) if *last as usize + 1 == *index => *last = *index as i32,
                _ => runs.push((*index as i32, *index as i32)),
            }
        }
        for (index, mut row) in changed {
            let rust = &mut *self.as_mut().rust_mut();
            // The same file with new metadata: the selection is view state and
            // survives the replacement, and so do the tags already read.
            row.selected = rust.rows[index].selected;
            row.tag_ids = rust.rows[index].tag_ids.clone();
            row.tag_sort = rust.rows[index].tag_sort.clone();
            row.group_label = rust.rows[index].group_label.clone();
            row.group_rank = rust.rows[index].group_rank;
            rust.rows[index] = row;
        }
        let parent = QModelIndex::default();
        for (first, last) in runs {
            unsafe {
                let top = self.as_mut().model_index(first, 0, &parent);
                let bottom = self.as_mut().model_index(last, 0, &parent);
                self.as_mut()
                    .data_changed(&top, &bottom, &QList::<i32>::default());
            }
        }

        let order = self.order;
        let mut insertions = insertions;
        insertions.sort_by(|left, right| compare_rows(left, right, &order));
        for row in insertions {
            let at = self.rows.partition_point(|existing| {
                compare_rows(existing, &row, &order) != Ordering::Greater
            });
            unsafe {
                self.as_mut()
                    .begin_insert_rows(&QModelIndex::default(), at as i32, at as i32);
                self.as_mut().rust_mut().rows.insert(at, row);
                self.as_mut().end_insert_rows();
            }
        }

        let total = self.rows.len() as i32;
        self.as_mut().set_loaded(total);
        // Every insert above moved the rows after it, and a removal dropped
        // one outright: the cached indices no longer address the selected rows.
        self.as_mut().rust_mut().selection.invalidate();
        self.as_mut().set_status(QString::from(format!(
            "{total} item{}",
            if total == 1 { "" } else { "s" }
        )));
        log::debug!(
            target: "dae::listing",
            "watcher: {} rows now ({} removed, {} replaced, {} inserted)",
            total,
            removed,
            replaced,
            inserted
        );
        // The patched rows can change what the Git column says for them (a saved
        // file turns modified, a new one untracked), so re-read the status. This
        // is the same read a reload triggers, without the branch enumeration.
        self.as_mut().begin_git_load(false);
    }

    fn mark_failed(mut self: Pin<&mut Self>, generation: u64, message: String) {
        if self.generation != generation {
            return;
        }
        self.as_mut().rust_mut().stream_id = None;
        self.as_mut().rust_mut().reload_pending = false;
        self.as_mut().rust_mut().load_failed = true;
        // A failed refresh keeps the rows already on screen; a failed
        // navigation had already cleared them.
        self.as_mut().rust_mut().refresh = None;
        self.as_mut().set_status(QString::from(message));
        self.as_mut().set_finished(true);
    }
}

/// One coalesced watcher burst for the directory a model displays.
///
/// The core reports the children the OS event named, so a rename or a save can
/// be applied to the rows on screen instead of re-reading a directory of tens
/// of thousands of entries. `names` is `None` when the event could not be
/// pinned to children — the core's documented "re-read, something differs"
/// answer — and the only safe response then is a full reload.
struct Change {
    path: String,
    names: Option<Vec<String>>,
}

impl Change {
    fn from_event(path: String, names: Vec<String>) -> Self {
        // An empty list is the core's "something differs, re-read" signal, not
        // "nothing changed".
        if names.is_empty() {
            return Self { path, names: None };
        }
        // One event can name the same child twice (a rename reports the old and
        // the new path); a patch plans row edits by name, so duplicates would
        // edit the same row twice.
        let mut names = names;
        names.sort_unstable();
        names.dedup();
        Self {
            path,
            names: Some(names),
        }
    }

    /// Folds a later event of the same burst into this one. Names are unioned
    /// because every one of them has to be patched; one unattributable event
    /// makes the whole burst a re-read.
    fn merge(&mut self, next: Change) {
        if self.path != next.path {
            // The burst spans a navigation: only the newest directory has rows
            // on screen, and the older names address rows that are gone.
            *self = next;
            return;
        }
        match (&mut self.names, next.names) {
            (Some(mine), Some(mut theirs)) => {
                mine.append(&mut theirs);
                mine.sort_unstable();
                mine.dedup();
            }
            _ => self.names = None,
        }
    }
}

/// How many children one burst may patch before the re-read wins. Every patched
/// name is resolved against the rows with a linear scan, so this keeps the work
/// bounded at roughly `names × rows` comparisons; a big copy or extraction is
/// cheaper to answer with the streamed reader than with a row edit per name.
const PATCH_LIMIT: usize = 64;

/// Listings below this size are re-sorted in place. The snapshot clone, the
/// worker and the queued reply cost more than the sort itself at that size, and
/// the reset that follows would be the whole stall anyway.
const WORKER_SORT_MIN_ROWS: usize = 2_048;

/// Forwards the directory watcher's events into the model's Qt thread.
///
/// The core arms a watcher for the directory this listing displays (see
/// `dae_core::fs::listing::open_streamed_listing`), and the watcher reports
/// changes here. A burst — a copy, an extraction — is coalesced: the reload is
/// delivered once the burst has been quiet for [`CHANGE_DEBOUNCE`], so a
/// directory being written into is not reset once per file.
struct ListingHost {
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
    /// Created on the first event, so a load whose watcher was already armed
    /// (the steady-state refresh) costs nothing.
    changes: Mutex<Option<mpsc::Sender<Change>>>,
}

impl ListingHost {
    fn new(thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>) -> Self {
        Self {
            thread,
            changes: Mutex::new(None),
        }
    }
}

impl Host for ListingHost {
    fn emit(&self, event: Event) {
        let Event::DirectoryChanged { path, names } = event else {
            return;
        };

        let mut changes = self.changes.lock().expect("listing host lock poisoned");
        let sender = changes.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel();
            let thread = self.thread.clone();
            std::thread::spawn(move || forward_changes(receiver, thread));
            sender
        });
        let _ = sender.send(Change::from_event(path, names));
    }
}

/// How long a watcher burst has to go quiet before its reload is delivered.
/// Matches the web shell's debounce, and stays below the interval at which a
/// person notices the list is stale.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);

fn forward_changes(
    changes: mpsc::Receiver<Change>,
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
) {
    let mut pending = match changes.recv() {
        Ok(change) => change,
        Err(_) => return,
    };

    loop {
        match changes.recv_timeout(CHANGE_DEBOUNCE) {
            Ok(change) => pending.merge(change),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let change = pending;
                let _ = thread.queue(move |mut model| model.as_mut().reload_after_change(change));
                pending = match changes.recv() {
                    Ok(change) => change,
                    Err(_) => return,
                };
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The shell end of `dae_core::fs::listing::BatchTarget`.
///
/// `open_streamed_listing` returns the head directly and streams the rest, so
/// the head must be inserted before the first streamed batch. A batch that
/// races the head waits in `pending` while `head_ready` is false; the release
/// and the normal path both queue under the same lock, which keeps delivery
/// order equal to the reader's order.
struct QtBatchTarget {
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
    generation: u64,
    state: Mutex<TargetState>,
}

#[derive(Default)]
struct TargetState {
    head_ready: bool,
    pending: Vec<Batch>,
}

impl QtBatchTarget {
    fn new(thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>, generation: u64) -> Self {
        Self {
            thread,
            generation,
            state: Mutex::new(TargetState::default()),
        }
    }

    fn queue_rows(&self, rows: Vec<StoredRow>, done: bool) -> bool {
        let generation = self.generation;
        self.thread
            .queue(move |mut model| model.as_mut().append_rows(generation, rows, done))
            .is_ok()
    }

    fn queue_batch(&self, batch: Batch) -> bool {
        let done = batch.done;
        let rows = batch
            .rows
            .into_iter()
            .map(StoredRow::from_listing)
            .collect();
        self.queue_rows(rows, done)
    }

    fn deliver_head(
        &self,
        base: String,
        path: String,
        breadcrumbs: Vec<Breadcrumb>,
        rows: Vec<StoredRow>,
        done: bool,
    ) {
        let mut state = self.state.lock().expect("listing target poisoned");
        let generation = self.generation;
        let _ = self.thread.queue(move |mut model| {
            model
                .as_mut()
                .append_head(generation, base, path, breadcrumbs, rows, done)
        });
        state.head_ready = true;
        for batch in state.pending.drain(..) {
            self.queue_batch(batch);
        }
    }

    fn fail(&self, message: String) {
        let generation = self.generation;
        let _ = self
            .thread
            .queue(move |mut model| model.as_mut().mark_failed(generation, message));
    }
}

impl BatchTarget for QtBatchTarget {
    fn deliver(&self, batch: Batch) -> bool {
        let mut state = self.state.lock().expect("listing target poisoned");
        if !state.head_ready {
            state.pending.push(batch);
            return true;
        }
        self.queue_batch(batch)
    }
}

fn next_stream_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("qt-{}", NEXT.fetch_add(1, AtomicOrdering::Relaxed))
}

/// A distinct registry key per model, so two tabs showing the same directory
/// each keep their own observation (one arm must not steal the other's).
fn next_watcher_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("qt-view-{}", NEXT.fetch_add(1, AtomicOrdering::Relaxed))
}

/// The smallest single block of rows that has to change to turn `old` into
/// `new`: everything before the common prefix and after the common suffix is
/// kept. Returns `(prefix, removed, added)`.
///
/// A create or delete therefore costs one insert or one remove — the rest of
/// the view keeps its delegates — instead of the whole-model reset that made
/// the list flicker. A change spread across the listing widens the block, but
/// it is still a legal edit: the rows in it are removed and re-inserted.
fn row_edit(old: &[StoredRow], new: &[StoredRow]) -> (usize, usize, usize) {
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(old, new)| old == new)
        .count();
    // The suffix must not overlap the prefix when the lists share a tail that
    // is longer than what is left after the prefix.
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(old, new)| old == new)
        .count();

    (
        prefix,
        old.len() - prefix - suffix,
        new.len() - prefix - suffix,
    )
}

/// The row whose full path is `path`, or `None`.
///
/// The normal row shape is `(base, name)`, so the comparison is a borrowed
/// `strip_prefix` instead of building `base + name` for every row: a lookup on
/// a 120,000-row listing used to allocate 120,000 `String`s, and it sits at the
/// end of every model reset and on every Shift-click anchor.
fn row_for_path(rows: &[StoredRow], base: &str, path: &str) -> Option<usize> {
    rows.iter().position(|row| match &row.path {
        // A row outside the listing base carries its whole spelling.
        Some(full) => full == path,
        None => path.strip_prefix(base) == Some(row.name.as_str()),
    })
}

/// The first row at or after `start`, wrapping once past the end, whose name
/// begins with `needle`, or `None`.
///
/// This is the type-ahead scan. It runs over the rows in Rust so a miss — the
/// common case — costs no cxx-qt call, no `QString` and no JS `toLowerCase` per
/// row. `needle` must already be lowercase; [`folded_starts_with`] folds the
/// row names as it compares.
fn find_name_prefix(rows: &[StoredRow], start: usize, needle: &str) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let start = start % rows.len();
    (start..rows.len())
        .chain(0..start)
        .find(|&index| folded_starts_with(&rows[index].name, needle))
}

/// The row edits a watcher patch resolves to.
struct RowPatch {
    /// Indices of rows that no longer exist, ascending.
    removals: Vec<usize>,
    /// Rows whose metadata changed, with their index in the current rows.
    replacements: Vec<(usize, StoredRow)>,
    /// Children that were not on screen yet, in no particular order.
    insertions: Vec<StoredRow>,
}

impl RowPatch {
    fn is_empty(&self) -> bool {
        self.removals.is_empty() && self.replacements.is_empty() && self.insertions.is_empty()
    }
}

/// Resolves a watcher patch into row edits, or `None` when it cannot be applied
/// in place.
///
/// `names` is the request and `entries` its answer, index for index; `None` in
/// `entries` is a child that no longer exists. The caller has restricted this to
/// the name order, so a metadata change keeps its row where it is. Two cases
/// still need the reader: a child that changed *kind* (folders lead, so its
/// place in the listing moved) and a name that differs from a row only by case.
/// The second one is not a rename on every platform — a case-insensitive
/// filesystem can report a spelling the listing did not, and matching it would
/// duplicate the row where the names are distinct, while missing it would add a
/// second row where they are not.
fn plan_patch(
    rows: &[StoredRow],
    base: &str,
    names: &[String],
    entries: &[Option<DirectoryEntry>],
) -> Option<RowPatch> {
    debug_assert_eq!(names.len(), entries.len());
    let mut patch = RowPatch {
        removals: Vec::new(),
        replacements: Vec::new(),
        insertions: Vec::new(),
    };
    for (name, entry) in names.iter().zip(entries) {
        let existing = rows
            .iter()
            .position(|row| row.name.as_str() == name.as_str());
        match (existing, entry) {
            (Some(index), None) => patch.removals.push(index),
            (Some(index), Some(entry)) => {
                let row = StoredRow::from_entry(entry, base);
                if row.kind != rows[index].kind {
                    return None;
                }
                if row != rows[index] {
                    patch.replacements.push((index, row));
                }
            }
            (None, entry) => {
                if rows.iter().any(|row| folded_eq(&row.name, name)) {
                    return None;
                }
                if let Some(entry) = entry {
                    patch.insertions.push(StoredRow::from_entry(entry, base));
                }
            }
        }
    }
    Some(patch)
}

/// The index a row has once the removals before it have been applied.
fn index_after_removals(index: usize, removals: &[usize]) -> usize {
    index - removals.partition_point(|&removed| removed < index)
}

/// Whether two names are equal after case folding. A plain `==` miss is not
/// enough to call a child new: the watcher may spell the name differently from
/// the listing and still mean the same file.
fn folded_eq(left: &str, right: &str) -> bool {
    let mut left = Folded::new(left);
    let mut right = Folded::new(right);
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(left_char), Some(right_char)) if left_char == right_char => {}
            _ => return false,
        }
    }
}

/// Whether `name`, folded, begins with `needle` — which must already be folded
/// (`"istanbul"`, not `"Istanbul"`). The row name is folded as it is compared,
/// so a scan over a listing builds no lowercased copies.
fn folded_starts_with(name: &str, needle: &str) -> bool {
    let mut name = Folded::new(name);
    for expected in needle.chars() {
        match name.next() {
            Some(actual) if actual == expected => {}
            _ => return false,
        }
    }
    true
}

/// The coarse Git state QML paints for one direct child.
///
/// The core keys entries by the spelling Git reported. On a case-insensitive
/// filesystem the directory listing can disagree about case, so fall back to
/// an ASCII-insensitive lookup before treating the row as clean.
fn git_badge(
    entries: &HashMap<String, GitEntryStatusKind>,
    directory_untracked: bool,
    name: &str,
) -> &'static str {
    if directory_untracked {
        return "untracked";
    }
    let kind = entries.get(name).copied().or_else(|| {
        entries
            .iter()
            .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
            .map(|(_, kind)| *kind)
    });
    match kind {
        Some(GitEntryStatusKind::Added) => "added",
        Some(GitEntryStatusKind::Modified) => "modified",
        Some(GitEntryStatusKind::Untracked) => "untracked",
        None => "",
    }
}

/// The listing's `mtime-size`, used to version a per-file icon URL. Type icons
/// are shared and deliberately not versioned.
fn icon_version(row: &StoredRow) -> String {
    format!("{}-{}", row.modified_at.unwrap_or(0), row.size.unwrap_or(0))
}

/// The same stamp built from loose values, for a model whose rows are not
/// `StoredRow` (search results keep their own struct).
pub(crate) fn icon_version_of(modified_at: Option<u64>, size: Option<u64>) -> String {
    format!("{}-{}", modified_at.unwrap_or(0), size.unwrap_or(0))
}

/// The thumbnail URL for a local file, or empty when the desktop cannot draw
/// one. Directories and files without an extension have no first frame.
pub(crate) fn thumbnail_for(path: &str, name: &str, kind: EntryKind, version: &str) -> String {
    if kind != EntryKind::File {
        return String::new();
    }
    if !(dae_core::fs::preview::is_thumbnail_path(path)
        || dae_core::fs::thumbnails::is_supported_path(path))
    {
        return String::new();
    }
    if extension_of(name).is_empty() {
        return String::new();
    }
    crate::icons::thumbnail_source(path, version)
}

/// The thumbnail URL for a row, or empty when the desktop cannot draw one.
///
/// Network entries and directories have no first frame; the extension check is
/// the same one the preview pane uses, so a row's thumbnail slot and the
/// preview's image slot agree about what is renderable.
fn thumbnail_url(remote: bool, path: &str, name: &str, row: &StoredRow) -> String {
    if remote {
        return String::new();
    }
    thumbnail_for(path, name, row.kind, &icon_version(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> DirectoryEntry {
        DirectoryEntry {
            name: path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
            path: path.to_owned(),
            kind: EntryKind::File,
            modified_at: Some(1_000),
            created_at: Some(900),
            created_fallback: false,
            size: Some(2_048),
            hidden: true,
            read_only: true,
        }
    }

    #[test]
    fn rows_under_the_base_keep_only_the_name() {
        let row = StoredRow::from_entry(&entry("K:/dae/\u{6587}\u{4ef6}.txt"), "K:/dae/");
        assert!(row.path.is_none());
        assert_eq!(row.full_path("K:/dae/"), "K:/dae/\u{6587}\u{4ef6}.txt");
    }

    #[test]
    fn rows_outside_the_base_keep_their_spelling() {
        let row = StoredRow::from_entry(&entry("K:/elsewhere/file.txt"), "K:/dae/");
        assert_eq!(row.path.as_deref(), Some("K:/elsewhere/file.txt"));
    }

    #[test]
    fn git_badges_map_core_kinds_and_ignore_file_name_case() {
        let mut entries = HashMap::new();
        entries.insert("Readme.md".to_owned(), GitEntryStatusKind::Modified);
        entries.insert("logo.svg".to_owned(), GitEntryStatusKind::Added);
        assert_eq!(git_badge(&entries, false, "Readme.md"), "modified");
        assert_eq!(git_badge(&entries, false, "readme.md"), "modified");
        assert_eq!(git_badge(&entries, false, "logo.svg"), "added");
        assert_eq!(git_badge(&entries, false, "clean.txt"), "");
        assert_eq!(git_badge(&entries, true, "clean.txt"), "untracked");
    }

    fn rows(names: &[&str]) -> Vec<StoredRow> {
        names
            .iter()
            .map(|name| StoredRow::from_entry(&entry(&format!("K:/dae/{name}")), "K:/dae/"))
            .collect()
    }

    #[test]
    fn an_unchanged_listing_edits_nothing() {
        let old = rows(&["a.txt", "b.txt"]);
        assert_eq!(row_edit(&old, &old.clone()), (2, 0, 0));
    }

    #[test]
    fn a_new_entry_is_one_insert_at_its_sorted_position() {
        let old = rows(&["a.txt", "b.txt"]);
        let new = rows(&["a.txt", "b.txt", "c.txt"]);
        assert_eq!(row_edit(&old, &new), (2, 0, 1));

        let middle = rows(&["a.txt", "ab.txt", "b.txt"]);
        assert_eq!(row_edit(&old, &middle), (1, 0, 1));
    }

    #[test]
    fn a_deleted_entry_is_one_remove() {
        let old = rows(&["a.txt", "b.txt", "c.txt"]);
        let new = rows(&["a.txt", "c.txt"]);
        assert_eq!(row_edit(&old, &new), (1, 1, 0));
    }

    #[test]
    fn a_changed_entry_is_one_replace() {
        let old = rows(&["a.txt", "b.txt"]);
        let mut new = old.clone();
        new[1].size = Some(4_096);
        assert_eq!(row_edit(&old, &new), (1, 1, 1));
    }

    #[test]
    fn a_tail_shared_with_a_shorter_list_does_not_overlap_the_prefix() {
        let old = rows(&["a.txt", "b.txt"]);
        let new = rows(&["a.txt"]);
        assert_eq!(row_edit(&old, &new), (1, 1, 0));
    }

    fn stored(name: &str, kind: EntryKind, size: u64, modified: u64, hidden: bool) -> StoredRow {
        StoredRow {
            name: name.to_owned(),
            path: None,
            kind,
            size: (kind == EntryKind::File).then_some(size),
            modified_at: Some(modified),
            created_at: Some(modified.saturating_sub(1)),
            created_fallback: false,
            hidden,
            read_only: false,
            selected: false,
            group_label: String::new(),
            group_rank: 0,
            tag_ids: Vec::new(),
            tag_sort: String::new(),
        }
    }

    fn names(rows: &[StoredRow]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn natural_order_puts_file2_before_file10_and_ignores_case() {
        let mut values = ["file10", "File2", "file1", "folder"];
        values.sort_by(|left, right| natural_cmp(left, right));
        assert_eq!(values, ["file1", "File2", "file10", "folder"]);
        assert_eq!(natural_cmp("File", "file"), Ordering::Equal);
        assert_eq!(natural_cmp("file01", "file1"), Ordering::Greater);
    }

    /// The comparator as it was before it was made allocation-free. Kept as the
    /// oracle: the fast one has to answer every pair the same way, including the
    /// leading-zero tie-break, the digit-run tie-break and the folding of
    /// characters whose lowercase is more than one character.
    fn natural_cmp_allocating(left: &str, right: &str) -> Ordering {
        let mut left_chars = left.chars().flat_map(char::to_lowercase).peekable();
        let mut right_chars = right.chars().flat_map(char::to_lowercase).peekable();

        loop {
            match (left_chars.next(), right_chars.next()) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(left_char), Some(right_char)) => {
                    if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                        let left_run = allocating_digit_run(left_char, &mut left_chars);
                        let right_run = allocating_digit_run(right_char, &mut right_chars);
                        let left_trimmed = left_run.trim_start_matches('0');
                        let right_trimmed = right_run.trim_start_matches('0');
                        let numbers = left_trimmed
                            .len()
                            .cmp(&right_trimmed.len())
                            .then_with(|| left_trimmed.cmp(right_trimmed));
                        if numbers != Ordering::Equal {
                            return numbers;
                        }
                        let zeros = left_run.len().cmp(&right_run.len());
                        if zeros != Ordering::Equal {
                            return zeros;
                        }
                    } else {
                        let ordering = left_char.cmp(&right_char);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                }
            }
        }
    }

    fn allocating_digit_run<I: Iterator<Item = char>>(
        first: char,
        chars: &mut std::iter::Peekable<I>,
    ) -> String {
        let mut run = String::new();
        run.push(first);
        while let Some(&next) = chars.peek() {
            if next.is_ascii_digit() {
                run.push(next);
                chars.next();
            } else {
                break;
            }
        }
        run
    }

    /// Names whose ordering the two comparators must agree on: separators, case,
    /// digit runs with and without leading zeros, and characters whose lowercase
    /// is not the same character (CJK keeps its code point, `İ` expands to two).
    fn oracle_names() -> Vec<&'static str> {
        let mut names = vec![
            "", " ", ".", "..", "...", "a", "A", "a b", "a-b", "a_b", "1", "01", "001", "0", "00",
            "9", "10", "010",
        ];
        names.extend([
            "file1", "file01", "file001", "file2", "file10", "File2", "FILE10",
        ]);
        names.extend(["file1.txt", "file01.txt", "file10.txt", "file2.txt"]);
        names.extend([
            "IMG_1.png",
            "IMG_01.png",
            "IMG_0001.png",
            "img_1.PNG",
            "IMG_2.png",
        ]);
        names.extend([
            "IMG_10.png",
            "a1b2c3",
            "a01b2c03",
            "a1b20c3",
            "x0y",
            "x00y",
            "x000y",
        ]);
        names.extend(["文档1.txt", "文档01.txt", "文档2.txt", "文档10.txt", "文档"]);
        names.extend(["éclair", "Éclair", "ÉCLAIR", "eclair"]);
        names.extend(["İstanbul", "istanbul", "Istanbul", "İ", "i"]);
        names.extend(["ẞ", "ß", "SS", "ss"]);
        names.extend(["🎉", "🎉1", "1🎉", "🎉01"]);
        names
    }

    #[test]
    fn the_folded_cursor_holds_every_lowercase_expansion() {
        let mut widest = 0;
        for code in 0..=0x10FFFFu32 {
            let Some(character) = char::from_u32(code) else {
                continue;
            };
            widest = widest.max(character.to_lowercase().count());
        }
        assert!(
            widest <= FOLDED_PENDING,
            "a lowercase expansion needs {widest} chars, FOLDED_PENDING is {FOLDED_PENDING}"
        );
    }

    #[test]
    fn natural_order_agrees_with_the_allocating_comparator() {
        let names = oracle_names();
        for left in &names {
            for right in &names {
                assert_eq!(
                    natural_cmp(left, right),
                    natural_cmp_allocating(left, right),
                    "comparator disagreed on {left:?} vs {right:?}"
                );
            }
        }

        // The same corpus, sorted both ways: a stable sort over agreeing
        // comparators must produce the same sequence.
        let mut fast = names.clone();
        fast.sort_by(|left, right| natural_cmp(left, right));
        let mut slow = names.clone();
        slow.sort_by(|left, right| natural_cmp_allocating(left, right));
        assert_eq!(fast, slow);
    }

    /// Names drawn from an alphabet that mixes digits, separators, CJK,
    /// accented, multi-character-folding and supplementary-plane characters,
    /// so the generated sort exercises the digit cursor and the fold buffer.
    fn generated_names(count: usize) -> Vec<String> {
        let alphabet: Vec<char> = "abAB01_ -.文档Ääéßİẞ🎉".chars().collect();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let len = (state % 7 + 1) as usize;
                let mut name = String::new();
                for _ in 0..len {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    name.push(alphabet[(state >> 33) as usize % alphabet.len()]);
                }
                name
            })
            .collect()
    }

    #[test]
    fn natural_order_agrees_with_the_allocating_comparator_on_generated_names() {
        let names = generated_names(4_000);
        for left in &names[..200] {
            for right in &names[..200] {
                assert_eq!(
                    natural_cmp(left, right),
                    natural_cmp_allocating(left, right),
                    "comparator disagreed on {left:?} vs {right:?}"
                );
            }
        }

        let mut fast = names.clone();
        fast.sort_by(|left, right| natural_cmp(left, right));
        let mut slow = names.clone();
        slow.sort_by(|left, right| natural_cmp_allocating(left, right));
        assert_eq!(fast, slow);
    }

    #[test]
    fn the_completion_permutation_matches_the_stable_sort() {
        let mut names = generated_names(2_000);
        // Case-only ties are where a stable sort and an index permutation can
        // disagree; the tie breaker has to keep arrival order.
        names.extend([
            "README".to_owned(),
            "readme".to_owned(),
            "ReadMe".to_owned(),
        ]);
        let entries: Vec<StoredRow> = names
            .iter()
            .map(|name| StoredRow::from_entry(&entry(&format!("K:/dae/{name}")), "K:/dae/"))
            .collect();
        let order = ViewOrder::default();

        let mut sorted = entries.clone();
        sort_rows(&mut sorted, &order);

        let snapshot = sort_snapshot(&entries);
        let perm = sort_permutation(&snapshot, &order);
        assert!(is_permutation(&perm, entries.len()));
        let applied = apply_permutation(entries, &perm).expect("a permutation");

        assert_eq!(
            applied
                .iter()
                .map(|row| row.name.clone())
                .collect::<Vec<_>>(),
            sorted
                .iter()
                .map(|row| row.name.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_permutation_that_does_not_cover_every_row_is_rejected() {
        let rows = rows(&["a", "b", "c"]);
        assert!(apply_permutation(rows.clone(), &[0, 1]).is_none());
        assert!(apply_permutation(rows.clone(), &[0, 0, 2]).is_none());
        assert!(apply_permutation(rows, &[0, 1, 3]).is_none());
    }

    #[test]
    fn a_superseded_sort_reply_is_dropped() {
        let order = ViewOrder::default();
        let ticket = SortTicket {
            generation: 7,
            rows: 3,
            order,
            request: 4,
        };
        let perm = [2, 0, 1];
        assert!(worker_sort_is_current(false, 7, 3, order, 4, ticket, &perm));
        // A newer click bumps the request while generation and rows stay put.
        assert!(!worker_sort_is_current(
            false, 7, 3, order, 5, ticket, &perm
        ));
        assert!(!worker_sort_is_current(
            false, 8, 3, order, 4, ticket, &perm
        ));
        assert!(!worker_sort_is_current(
            false, 7, 2, order, 4, ticket, &perm
        ));
        let another = ViewOrder {
            key: SortKey::Size,
            ..order
        };
        assert!(!worker_sort_is_current(
            false, 7, 3, another, 4, ticket, &perm
        ));
        assert!(!worker_sort_is_current(true, 7, 3, order, 4, ticket, &perm));
        assert!(!worker_sort_is_current(
            false,
            7,
            3,
            order,
            4,
            ticket,
            &[0, 0, 2]
        ));
    }

    /// Where the model's completion re-sort spends its time, on a corpus shaped
    /// like a large folder's names. Ignored by default; run it with
    /// `--release` so the numbers are comparable to
    /// `docs/decisions/listing-shell-costs.md` §2.2:
    ///
    /// ```text
    /// cargo test -p dae-qt --lib --release -- --ignored --nocapture reports_where_the_completion_sort_spends_its_time
    /// ```
    #[test]
    #[ignore = "prints the completion sort's cost; run with --release --ignored --nocapture"]
    fn reports_where_the_completion_sort_spends_its_time() {
        for count in [35_803usize, 120_000] {
            let names: Vec<String> = (0..count)
                .map(|index| format!("IMG_{}{}.png", index % 997, index))
                .collect();
            let mut folding = 0.0;
            let mut allocating = 0.0;
            for _ in 0..5 {
                let mut fast = names.clone();
                let start = std::time::Instant::now();
                fast.sort_by(|left, right| natural_cmp(left, right));
                folding += start.elapsed().as_secs_f64() * 1000.0;

                let mut slow = names.clone();
                let start = std::time::Instant::now();
                slow.sort_by(|left, right| natural_cmp_allocating(left, right));
                allocating += start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(fast, slow, "the two orders must agree");
            }
            println!(
                "{count:>7} names: allocating {:>7.1} ms, folded {:>7.1} ms",
                allocating / 5.0,
                folding / 5.0
            );
        }
    }

    #[test]
    fn a_path_lookup_compares_borrowed_spellings() {
        let under_base = rows(&["a.txt", "b.txt"]);
        assert_eq!(
            row_for_path(&under_base, "K:/dae/", "K:/dae/b.txt"),
            Some(1)
        );
        assert_eq!(
            row_for_path(&under_base, "K:/dae/", "K:/dae/a.txt"),
            Some(0)
        );
        assert_eq!(row_for_path(&under_base, "K:/dae/", "K:/dae/c.txt"), None);
        assert_eq!(row_for_path(&under_base, "K:/dae/", "K:/dae/b.txt "), None);

        let outside = vec![StoredRow::from_entry(
            &entry("K:/elsewhere/file.txt"),
            "K:/dae/",
        )];
        assert_eq!(
            row_for_path(&outside, "K:/dae/", "K:/elsewhere/file.txt"),
            Some(0)
        );
        assert_eq!(row_for_path(&outside, "K:/dae/", "K:/dae/file.txt"), None);
    }

    #[test]
    fn a_patch_replaces_a_changed_row_and_inserts_a_new_one() {
        let base = "K:/dae/";
        let rows = rows(&["a.txt", "b.txt", "c.txt"]);
        let mut changed = entry("K:/dae/b.txt");
        changed.size = Some(4_096);
        let names = [
            "a.txt".to_owned(),
            "b.txt".to_owned(),
            "gone.txt".to_owned(),
            "d.txt".to_owned(),
        ];
        let entries = [
            Some(entry("K:/dae/a.txt")),
            Some(changed),
            None,
            Some(entry("K:/dae/d.txt")),
        ];

        let patch = plan_patch(&rows, base, &names, &entries).expect("the patch applies in place");
        assert!(patch.removals.is_empty(), "an unknown name is not a row");
        assert_eq!(patch.replacements.len(), 1);
        assert_eq!(patch.replacements[0].0, 1);
        assert_eq!(patch.replacements[0].1.size, Some(4_096));
        assert_eq!(patch.insertions.len(), 1);
        assert_eq!(patch.insertions[0].name, "d.txt");
    }

    #[test]
    fn a_patch_removes_a_row_whose_child_is_gone() {
        let rows = rows(&["a.txt", "b.txt"]);
        let patch = plan_patch(&rows, "K:/dae/", &["b.txt".to_owned()], &[None])
            .expect("the patch applies in place");
        assert_eq!(patch.removals, vec![1]);
        assert!(patch.replacements.is_empty());
        assert!(patch.insertions.is_empty());
    }

    #[test]
    fn a_child_that_changed_kind_needs_a_read() {
        let rows = rows(&["a.txt"]);
        let mut directory = entry("K:/dae/a.txt");
        directory.kind = EntryKind::Directory;
        let names = ["a.txt".to_owned()];
        assert!(plan_patch(&rows, "K:/dae/", &names, &[Some(directory)]).is_none());
    }

    #[test]
    fn a_spelling_that_differs_only_by_case_needs_a_read() {
        let rows = rows(&["readme.md"]);
        let names = ["README.md".to_owned()];
        let entries = [Some(entry("K:/dae/README.md"))];
        assert!(plan_patch(&rows, "K:/dae/", &names, &entries).is_none());
    }

    #[test]
    fn replacements_keep_their_index_past_the_removals_before_them() {
        let removals = [1usize, 4, 9];
        assert_eq!(index_after_removals(0, &removals), 0);
        assert_eq!(index_after_removals(2, &removals), 1);
        assert_eq!(index_after_removals(5, &removals), 3);
        assert_eq!(index_after_removals(10, &removals), 7);
    }

    #[test]
    fn type_ahead_finds_the_first_prefix_from_the_cursor_and_wraps() {
        let rows = rows(&["apple.txt", "banana.txt", "berry.txt", "cherry.txt"]);
        assert_eq!(find_name_prefix(&rows, 0, "b"), Some(1));
        assert_eq!(find_name_prefix(&rows, 1, "b"), Some(1));
        assert_eq!(find_name_prefix(&rows, 2, "b"), Some(2));
        assert_eq!(
            find_name_prefix(&rows, 3, "b"),
            Some(1),
            "a start past the last match wraps once"
        );
        assert_eq!(find_name_prefix(&rows, 0, "be"), Some(2));
        assert_eq!(find_name_prefix(&rows, 0, "zzz"), None);
        assert_eq!(find_name_prefix(&[], 0, "a"), None);
    }

    #[test]
    fn a_prefix_match_ignores_case_and_folds_like_the_comparator() {
        let rows = rows(&["İstanbul.txt", "École.txt"]);
        assert_eq!(find_name_prefix(&rows, 0, "i"), Some(0));
        assert_eq!(
            find_name_prefix(&rows, 0, "istan"),
            None,
            "the folded name is i + U+0307 + stan, exactly like JS toLowerCase"
        );
        assert_eq!(find_name_prefix(&rows, 0, "i\u{307}stan"), Some(0));
        assert_eq!(find_name_prefix(&rows, 0, "école"), Some(1));
        assert!(
            !folded_starts_with("École", "ECOLE"),
            "the caller folds the needle; an unfolded one must not match"
        );
    }

    #[test]
    fn folders_lead_in_both_directions() {
        let mut rows = vec![
            stored("b.txt", EntryKind::File, 10, 1, false),
            stored("a", EntryKind::Directory, 0, 9, false),
            stored("c.txt", EntryKind::File, 20, 2, false),
        ];
        let ascending = ViewOrder::default();
        sort_rows(&mut rows, &ascending);
        assert_eq!(names(&rows), ["a", "b.txt", "c.txt"]);

        let descending = ViewOrder {
            order: SortOrder::Desc,
            ..ascending
        };
        sort_rows(&mut rows, &descending);
        assert_eq!(
            names(&rows),
            ["a", "c.txt", "b.txt"],
            "a folder stays ahead even when the key is reversed"
        );
    }

    #[test]
    fn files_first_is_the_mirror_of_folders_first() {
        let mut rows = vec![
            stored("a", EntryKind::Directory, 0, 1, false),
            stored("b.txt", EntryKind::File, 1, 2, false),
        ];
        sort_rows(
            &mut rows,
            &ViewOrder {
                folders: FolderOrder::FilesFirst,
                ..ViewOrder::default()
            },
        );
        assert_eq!(names(&rows), ["b.txt", "a"]);

        sort_rows(
            &mut rows,
            &ViewOrder {
                folders: FolderOrder::Mixed,
                ..ViewOrder::default()
            },
        );
        assert_eq!(names(&rows), ["a", "b.txt"], "mixed keeps the name order");
    }

    #[test]
    fn created_time_is_its_own_sort_key() {
        let mut rows = vec![
            stored("old.txt", EntryKind::File, 1, 1, false),
            stored("new.txt", EntryKind::File, 1, 1, false),
        ];
        // `stored` derives the birth time from the modification time, so give
        // the second row a newer one explicitly.
        rows[1].created_at = Some(9_000);
        sort_rows(
            &mut rows,
            &ViewOrder {
                key: SortKey::Created,
                order: SortOrder::Desc,
                ..ViewOrder::default()
            },
        );
        assert_eq!(names(&rows), ["new.txt", "old.txt"]);
    }

    #[test]
    fn tag_sort_reads_the_precomputed_key_and_puts_untagged_last() {
        let mut tagged = stored("b.txt", EntryKind::File, 1, 1, false);
        tagged.tag_ids = vec!["work".to_owned()];
        tagged.tag_sort = "Work".to_owned();
        let untagged = stored("a.txt", EntryKind::File, 1, 1, false);
        let mut rows = vec![untagged, tagged];

        sort_rows(
            &mut rows,
            &ViewOrder {
                key: SortKey::Tag,
                order: SortOrder::Desc,
                ..ViewOrder::default()
            },
        );
        assert_eq!(names(&rows), ["b.txt", "a.txt"]);

        sort_rows(
            &mut rows,
            &ViewOrder {
                key: SortKey::Tag,
                order: SortOrder::Asc,
                ..ViewOrder::default()
            },
        );
        assert_eq!(names(&rows), ["b.txt", "a.txt"], "untagged stays last");
    }

    #[test]
    fn name_grouping_uses_the_first_letter_and_puts_non_letters_under_hash() {
        let mut rows = vec![
            stored("beta.txt", EntryKind::File, 1, 1, false),
            stored("alpha.txt", EntryKind::File, 1, 1, false),
            stored("1.txt", EntryKind::File, 1, 1, false),
        ];
        assign_groups(
            &mut rows,
            Grouping {
                key: GroupKey::Name,
                date: GroupDate::Month,
            },
        );
        assert_eq!(rows[0].group_label, "B");
        assert_eq!(rows[1].group_label, "A");
        assert_eq!(rows[2].group_label, "#");

        sort_rows(
            &mut rows,
            &ViewOrder {
                grouping: Grouping {
                    key: GroupKey::Name,
                    date: GroupDate::Month,
                },
                ..ViewOrder::default()
            },
        );
        assert_eq!(
            names(&rows),
            ["1.txt", "alpha.txt", "beta.txt"],
            "the hash bucket leads because it sorts before letters"
        );
    }

    #[test]
    fn date_groups_rank_chronologically_whatever_the_label_locale() {
        let mut older = stored("old.txt", EntryKind::File, 1, 1_000, false);
        older.created_at = Some(1_000);
        let mut newer = stored("new.txt", EntryKind::File, 1, 9_000_000_000_000, false);
        newer.created_at = Some(9_000_000_000_000);
        let mut rows = vec![older, newer];
        assign_groups(
            &mut rows,
            Grouping {
                key: GroupKey::Created,
                date: GroupDate::Year,
            },
        );
        assert!(rows[0].group_rank < rows[1].group_rank);
        assert!(!rows[0].group_label.is_empty());
        assert_ne!(rows[0].group_label, rows[1].group_label);
    }

    #[test]
    fn tag_groups_order_by_definition_and_untagged_last() {
        use dae_core::fs::tags::TagDefinition;
        let definitions = vec![
            TagDefinition {
                id: "work".into(),
                name: "Work".into(),
                color: "#111111".into(),
            },
            TagDefinition {
                id: "home".into(),
                name: "Home".into(),
                color: "#222222".into(),
            },
        ];
        let mut tagged = stored("a.txt", EntryKind::File, 1, 1, false);
        tagged.tag_ids = vec!["home".to_owned(), "work".to_owned()];
        let untagged = stored("b.txt", EntryKind::File, 1, 1, false);
        let grouping = Grouping {
            key: GroupKey::Tag,
            date: GroupDate::Month,
        };

        let (tagged_rank, tagged_label) = group_of(&tagged, grouping, &definitions);
        let (untagged_rank, untagged_label) = group_of(&untagged, grouping, &definitions);
        assert_eq!(tagged_label, "Work, Home", "definition order, not id order");
        assert!(tagged_rank < untagged_rank, "untagged sorts last");
        assert_eq!(untagged_label, "");
    }

    #[test]
    fn decorate_tags_matches_by_name_and_by_stored_path() {
        use dae_core::fs::tags::TagDefinition;
        let definitions = vec![TagDefinition {
            id: "work".into(),
            name: "Work".into(),
            color: "#111111".into(),
        }];
        let relative = stored("a.txt", EntryKind::File, 1, 1, false);
        let mut outside = stored("b.txt", EntryKind::File, 1, 1, false);
        outside.path = Some("D:/other/b.txt".to_owned());
        let untagged = stored("c.txt", EntryKind::File, 1, 1, false);
        let mut rows = vec![relative, outside, untagged];
        let cache = HashMap::from([
            ("C:/dir/a.txt".to_owned(), vec!["work".to_owned()]),
            ("D:/other/b.txt".to_owned(), vec!["work".to_owned()]),
        ]);

        decorate_tags(&mut rows, &cache, &definitions, "C:/dir/");

        assert_eq!(rows[0].tag_ids, ["work"], "rows under the base match by name");
        assert_eq!(rows[0].tag_sort, "Work");
        assert_eq!(rows[1].tag_ids, ["work"], "rows with a path match on it");
        assert!(rows[2].tag_ids.is_empty());
    }

    #[test]
    fn folders_interleave_when_the_preference_is_off() {
        let mut rows = vec![
            stored("b.txt", EntryKind::File, 10, 1, false),
            stored("a", EntryKind::Directory, 0, 9, false),
        ];
        let order = ViewOrder {
            folders: FolderOrder::Mixed,
            ..ViewOrder::default()
        };
        sort_rows(&mut rows, &order);
        assert_eq!(names(&rows), ["a", "b.txt"]);
    }

    #[test]
    fn size_and_timestamp_orders_are_directional_and_name_broken() {
        let mut rows = vec![
            stored("small.txt", EntryKind::File, 10, 30, false),
            stored("large.txt", EntryKind::File, 30, 10, false),
            stored("middle.txt", EntryKind::File, 20, 20, false),
        ];
        let by_size = ViewOrder {
            key: SortKey::Size,
            order: SortOrder::Desc,
            ..ViewOrder::default()
        };
        sort_rows(&mut rows, &by_size);
        assert_eq!(names(&rows), ["large.txt", "middle.txt", "small.txt"]);

        let by_modified = ViewOrder {
            key: SortKey::Modified,
            ..by_size
        };
        sort_rows(&mut rows, &by_modified);
        assert_eq!(names(&rows), ["small.txt", "middle.txt", "large.txt"]);
    }

    #[test]
    fn type_order_groups_categories_and_orders_by_extension() {
        let mut rows = vec![
            stored("main.rs", EntryKind::File, 1, 1, false),
            stored("photo.png", EntryKind::File, 1, 1, false),
            stored("archive.zip", EntryKind::File, 1, 1, false),
            stored("notes.txt", EntryKind::File, 1, 1, false),
        ];
        let order = ViewOrder {
            key: SortKey::Type,
            ..ViewOrder::default()
        };
        sort_rows(&mut rows, &order);
        assert_eq!(
            names(&rows),
            ["photo.png", "notes.txt", "main.rs", "archive.zip"]
        );
    }

    #[test]
    fn hidden_kind_modified_and_size_filters_are_applied() {
        let now = 1_800_000_000_000u64;
        let rows = vec![
            stored("hidden.txt", EntryKind::File, 10, now, true),
            stored("photo.png", EntryKind::File, 2 * 1024 * 1024, now, false),
            stored("folder", EntryKind::Directory, 0, now, false),
            stored("old.txt", EntryKind::File, 10, now - 40 * 86_400_000, false),
            stored("huge.iso", EntryKind::File, 200 * 1024 * 1024, now, false),
        ];

        let without_hidden = RowFilter {
            show_hidden: false,
            ..RowFilter::default()
        };
        let kept: Vec<&str> = rows
            .iter()
            .filter(|row| without_hidden.keeps(row, 0))
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(kept, ["photo.png", "folder", "old.txt", "huge.iso"]);

        let folders = RowFilter {
            kind: KindFilter::Folders,
            ..RowFilter::default()
        };
        assert_eq!(
            names(
                &rows
                    .iter()
                    .filter(|row| folders.keeps(row, 0))
                    .cloned()
                    .collect::<Vec<_>>()
            ),
            ["folder"]
        );

        let images = RowFilter {
            kind: KindFilter::Images,
            ..RowFilter::default()
        };
        assert_eq!(
            names(
                &rows
                    .iter()
                    .filter(|row| images.keeps(row, 0))
                    .cloned()
                    .collect::<Vec<_>>()
            ),
            ["photo.png"]
        );

        let small = RowFilter {
            size: SizeFilter::Small,
            ..RowFilter::default()
        };
        let kept: Vec<&str> = rows
            .iter()
            .filter(|row| small.keeps(row, 0))
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(
            kept,
            ["hidden.txt", "folder", "old.txt"],
            "a directory is not sized out of the listing"
        );

        let this_month = RowFilter {
            modified: ModifiedFilter::Month,
            ..RowFilter::default()
        };
        let kept: Vec<&str> = rows
            .iter()
            .filter(|row| this_month.keeps(row, now - 30 * 86_400_000))
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(kept, ["hidden.txt", "photo.png", "folder", "huge.iso"]);
    }

    #[test]
    fn categories_cover_the_files_the_glyphs_paint() {
        assert_eq!(category_of(EntryKind::Directory, "docs"), "folder");
        assert_eq!(category_of(EntryKind::File, "photo.PNG"), "image");
        assert_eq!(category_of(EntryKind::File, "main.rs"), "code");
        assert_eq!(category_of(EntryKind::File, "Button.qml"), "code");
        assert_eq!(category_of(EntryKind::File, "song.flac"), "audio");
        assert_eq!(category_of(EntryKind::File, "bundle.7z"), "archive");
        assert_eq!(category_of(EntryKind::File, "README"), "file");
        assert_eq!(extension_of("README"), "");
        assert_eq!(extension_of("a.tar.gz"), "gz");
    }

    /// M1 acceptance: the tag work must stay out of the 120,000-entry listing's
    /// hot path, and the listing's p95 must stay within 5% of the same pipeline
    /// without it. Two hot-path variants are measured because they are the ones
    /// the model actually runs: the default listing (cold tag cache, where
    /// `decorate_rows` is a no-op) and a re-listing with a warm cache, where
    /// `decorate_rows` builds every row's path, looks it up and stamps a hit.
    /// Grouping is opt-in (`GroupKey::None` by default), so its cost is reported
    /// without the listing budget; the tag read of the visible rows happens
    /// after the listing on the blocking pool and is measured on its own.
    ///
    /// Runs the real code paths — `open_canonical_listing` + growing batches,
    /// `StoredRow::from_listing`, `decorate_tags`, `assign_groups`, the worker
    /// sort trio — not a mirror.
    ///
    /// Fixture: `bench/listing-pipeline prepare <dir> 120000`, then
    /// `DAE_LISTING_BENCH_DIR=<dir> cargo test -p dae-qt --lib --release -- \
    ///   --ignored --nocapture reports_the_tag_group_listing_regression`
    ///
    /// The test binary runs on the system heap (no mimalloc), so absolute
    /// numbers differ from the app; every arm shares the heap, which is what
    /// the regression ratio needs.
    #[test]
    #[ignore = "needs DAE_LISTING_BENCH_DIR pointing at a 120,000-entry fixture"]
    fn reports_the_tag_group_listing_regression() {
        use std::collections::HashMap;
        use std::time::{Duration, Instant};

        let Ok(dir) = std::env::var("DAE_LISTING_BENCH_DIR") else {
            eprintln!("DAE_LISTING_BENCH_DIR not set; skipping");
            return;
        };
        let path = std::path::PathBuf::from(&dir);

        // The enumeration half of `open_streamed_listing`: the head batch,
        // then the reader's doubling batches up to MAX_BATCH_SIZE.
        fn enumerate(path: &std::path::Path) -> (Vec<StoredRow>, String, Duration) {
            let started = Instant::now();
            let listing =
                dae_core::fs::local::open_canonical_listing(path.to_path_buf(), 512)
                    .expect("fixture listing");
            // The model builds rows against `base_of(view.path)`, the string
            // that makes `base + name` the iterator's own path.
            let base = dae_core::fs::entry_codec::base_of(&listing.view.path);
            let mut rows: Vec<StoredRow> = listing
                .view
                .entries
                .iter()
                .cloned()
                .map(|entry| StoredRow::from_entry(&entry, &base))
                .collect();
            if let Some(mut cursor) = listing.cursor {
                let mut batch = 512usize;
                loop {
                    let (chunk, done) = cursor.take_rows(batch);
                    rows.extend(chunk.into_iter().map(StoredRow::from_listing));
                    if done {
                        break;
                    }
                    batch = (batch * 2).min(8192);
                }
            }
            (rows, base, started.elapsed())
        }

        fn worker_sort(rows: Vec<StoredRow>, order: &ViewOrder) -> Vec<StoredRow> {
            let entries = sort_snapshot(&rows);
            let perm = sort_permutation(&entries, order);
            apply_permutation(rows, &perm).expect("permutation")
        }

        let (probe, base, _) = enumerate(&path);
        assert!(
            probe.len() >= 100_000,
            "fixture has {} entries; prepare one with 120,000",
            probe.len()
        );
        let visible_count = 512.min(probe.len());
        let visible_paths: Vec<String> = probe[..visible_count]
            .iter()
            .map(|row| row.full_path(&base))
            .collect();
        drop(probe);

        // A re-listing after the first screen has painted: the model's cache
        // holds those rows' tags, and every row in the new listing pays a path
        // build plus a lookup.
        let definitions = dae_core::fs::tags::definitions();
        let stamp = definitions
            .first()
            .map(|definition| definition.id.clone())
            .unwrap_or_else(|| "smoke".to_owned());
        let cache: HashMap<String, Vec<String>> = visible_paths
            .iter()
            .map(|path| (path.clone(), vec![stamp.clone()]))
            .collect();

        let plain = ViewOrder::default();
        let by_name = ViewOrder {
            grouping: Grouping {
                key: GroupKey::Name,
                date: GroupDate::Month,
            },
            ..ViewOrder::default()
        };
        let by_month = ViewOrder {
            grouping: Grouping {
                key: GroupKey::Modified,
                date: GroupDate::Month,
            },
            ..ViewOrder::default()
        };

        let mut samples: [Vec<Duration>; 4] = Default::default();
        let mut walk_samples: Vec<Duration> = Vec::new();
        let mut tag_samples: Vec<Duration> = Vec::new();
        let passes = 12;
        for pass in 0..passes + 3 {
            let measured = pass >= 3;
            // Rotate the arm order so a warming cache cannot favor one arm.
            for slot in 0..4 {
                let arm = (slot + pass) % 4;
                let (mut rows, base_now, walk) = enumerate(&path);
                let started = Instant::now();
                match arm {
                    // The listing as it ships: no grouping, no cache hits.
                    0 => rows = worker_sort(rows, &plain),
                    // A re-listing with the first screen's tags cached.
                    1 => {
                        // `decorate_rows` runs this exact pass per batch.
                        decorate_tags(&mut rows, &cache, &definitions, &base_now);
                        rows = worker_sort(rows, &plain);
                    }
                    2 => {
                        assign_groups(&mut rows, by_name.grouping);
                        rows = worker_sort(rows, &by_name);
                    }
                    _ => {
                        assign_groups(&mut rows, by_month.grouping);
                        rows = worker_sort(rows, &by_month);
                    }
                }
                let total = walk + started.elapsed();
                if measured {
                    samples[arm].push(total);
                    if arm == 0 {
                        walk_samples.push(walk);
                    }
                }
                drop(rows);
            }
            if measured {
                // The post-listing tag phase: visible rows only, exactly what
                // `requestTags` asks for once the listing lands.
                let started = Instant::now();
                let tags = dae_core::fs::tags::tags_for(&visible_paths);
                tag_samples.push(started.elapsed());
                assert_eq!(tags.len(), visible_paths.len());
            }
        }

        fn median(values: &mut [Duration]) -> Duration {
            values.sort();
            values[values.len() / 2]
        }
        fn p95(values: &mut [Duration]) -> Duration {
            values.sort();
            values[(values.len() * 95).div_ceil(100) - 1]
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;

        let labels = [
            "listing (baseline)",
            "listing + warm tags",
            "name groups (opt-in)",
            "month groups (opt-in)",
        ];
        println!("\n{} entries, {passes} measured passes:", probe_len(&path));
        for arm in 0..4 {
            println!(
                "  {:<22} median {:7.2} ms   p95 {:7.2} ms",
                labels[arm],
                ms(median(&mut samples[arm])),
                ms(p95(&mut samples[arm])),
            );
        }
        println!(
            "  {:<22} median {:7.2} ms (walk + row build, shared by every arm)",
            "enumeration",
            ms(median(&mut walk_samples))
        );
        println!(
            "  tags, {visible_count} visible rows after listing: median {:.2} ms, p95 {:.2} ms",
            ms(median(&mut tag_samples)),
            ms(p95(&mut tag_samples)),
        );
        let baseline = p95(&mut samples[0]);
        let tagged = p95(&mut samples[1]);
        let regression = (tagged.as_secs_f64() / baseline.as_secs_f64() - 1.0) * 100.0;
        println!("  warm-tag p95 regression: {regression:+.2}% (budget +5%)");
        assert!(
            tagged.as_secs_f64() <= baseline.as_secs_f64() * 1.05,
            "warm-cache tag decoration regressed p95 by {regression:.2}%"
        );
        // Grouping is a view the user opts into; its numbers are printed above
        // so a change that makes it pathological is visible, without applying
        // the listing budget to a deliberately heavier order.

        fn probe_len(path: &std::path::Path) -> usize {
            std::fs::read_dir(path).map(|it| it.count()).unwrap_or(0)
        }
    }
}
