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
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::entry_codec::{self, ListingRow};
use dae_core::fs::listing::{self, Batch, BatchTarget};
use dae_core::fs::vfs::{self, Scheme};
use dae_core::fs::watch;
use dae_core::host::{Event, Host};
use dae_core::types::{DirectoryEntry, EntryKind};

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
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, loaded)]
        #[qproperty(bool, finished)]
        #[qproperty(QString, status)]
        #[qproperty(QString, path)]
        /// True when this model was loaded through a network backend. The
        /// shell reads it to keep local-only actions off `smb://`, `sftp://`
        /// and `cloud://` paths instead of letting them fail on click.
        #[qproperty(bool, remote)]
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
        /// returns immediately; batches that arrive later are appended in
        /// stream order and the completed listing is sorted once more.
        #[qinvokable]
        #[cxx_name = "setOrder"]
        fn set_order(
            self: Pin<&mut DirectoryListModel>,
            sort_key: &QString,
            sort_order: &QString,
            folders_first: bool,
        );

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

        /// Whether `row` is a directory, for Enter/activation.
        #[qinvokable]
        #[cxx_name = "isDirectoryAt"]
        fn is_directory_at(&self, row: i32) -> bool;

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
    }

    impl cxx_qt::Threading for DirectoryListModel {}
}

/// `PartialEq` is what lets a refresh keep the rows it already has: the diff
/// compares old and new entries field by field, so an unchanged row costs no
/// model signal at all.
#[derive(Clone, PartialEq)]
struct StoredRow {
    name: String,
    /// Kept only for the rare row that is not under the listing base. Rows that
    /// are (the local reader's normal shape) rebuild their path from the base,
    /// which is what keeps a 120k-row model cheap.
    path: Option<String>,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    hidden: bool,
    read_only: bool,
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
            hidden: entry.hidden,
            read_only: entry.read_only,
        }
    }

    fn from_listing(row: ListingRow) -> Self {
        Self {
            name: row.name,
            path: row.path,
            kind: row.kind,
            size: row.size,
            modified_at: row.modified_at,
            hidden: row.hidden,
            read_only: row.read_only,
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
    Type,
    Size,
}

impl SortKey {
    fn parse(value: &str) -> Self {
        match value {
            "modified" => Self::Modified,
            "type" => Self::Type,
            "size" => Self::Size,
            _ => Self::Name,
        }
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
    folders_first: bool,
}

impl Default for ViewOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            order: SortOrder::Asc,
            folders_first: true,
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
                    chrono::Local
                        .from_local_datetime(&midnight)
                        .earliest()
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

/// Orders two rows for display.
///
/// The rules reproduce the Tauri shell's comparator: folders lead in both
/// directions when `foldersFirst` is on, the active key is scaled by the
/// direction, and the name breaks ties ascending so equal sizes/timestamps
/// still have one order.
fn compare_rows(left: &StoredRow, right: &StoredRow, order: &ViewOrder) -> Ordering {
    if order.folders_first {
        let left_folder = u8::from(left.kind == EntryKind::Directory);
        let right_folder = u8::from(right.kind == EntryKind::Directory);
        let folders = right_folder.cmp(&left_folder);
        if folders != Ordering::Equal {
            return folders;
        }
    }

    let primary = match order.key {
        SortKey::Name => natural_cmp(&left.name, &right.name),
        SortKey::Modified => left
            .modified_at
            .unwrap_or(0)
            .cmp(&right.modified_at.unwrap_or(0)),
        SortKey::Size => left.size.unwrap_or(0).cmp(&right.size.unwrap_or(0)),
        SortKey::Type => {
            let left_category = category_of(left.kind, &left.name);
            let right_category = category_of(right.kind, &right.name);
            category_rank(left_category)
                .cmp(&category_rank(right_category))
                .then_with(|| {
                    natural_cmp(
                        extension_of(&left.name),
                        extension_of(&right.name),
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

    natural_cmp(&left.name, &right.name)
}

fn sort_rows(rows: &mut [StoredRow], order: &ViewOrder) {
    rows.sort_by(|left, right| compare_rows(left, right, order));
}

fn rows_are_sorted(rows: &[StoredRow], order: &ViewOrder) -> bool {
    rows.windows(2)
        .all(|pair| compare_rows(&pair[0], &pair[1], order) != Ordering::Greater)
}

/// Case-insensitive natural ordering: `file2` sorts before `file10`.
///
/// `Intl.Collator` with `{numeric: true, sensitivity: "base"}` also folds
/// accents; Rust has no built-in collator here, so this covers the parts that
/// matter for file names (case, digits) and leaves diacritics compared by code
/// point.
fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().flat_map(char::to_lowercase).peekable();
    let mut right_chars = right.chars().flat_map(char::to_lowercase).peekable();

    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_run = digit_run(left_char, &mut left_chars);
                    let right_run = digit_run(right_char, &mut right_chars);
                    let left_trimmed = left_run.trim_start_matches('0');
                    let right_trimmed = right_run.trim_start_matches('0');
                    let numbers = left_trimmed
                        .len()
                        .cmp(&right_trimmed.len())
                        .then_with(|| left_trimmed.cmp(right_trimmed));
                    if numbers != Ordering::Equal {
                        return numbers;
                    }
                    // Numerically equal ("file01" vs "file1"): fewer leading
                    // zeros first, like the collator's tie-break followed by
                    // the exact name comparison.
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

fn digit_run<I: Iterator<Item = char>>(
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

fn extension_of(name: &str) -> &str {
    name.rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("")
}

/// The coarse kind the built-in glyph set paints.
fn category_of(kind: EntryKind, name: &str) -> &'static str {
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
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "c", "h", "cpp",
        "hpp", "cc", "cs", "rb", "php", "swift", "kt", "kts", "lua", "sh", "bash", "zsh",
        "ps1", "bat", "cmd", "sql", "html", "htm", "css", "scss", "less", "json", "yaml",
        "yml", "toml", "xml",
    ];
    const TEXT: &[&str] = &["txt", "md", "markdown", "log", "csv", "tsv", "ini", "conf", "cfg"];
    const DOCUMENT: &[&str] = &["doc", "docx", "xls", "xlsx", "ppt", "pptx", "rtf", "odt", "ods"];
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

pub struct DirectoryListModelRust {
    loaded: i32,
    finished: bool,
    status: QString,
    path: QString,
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
    /// `dispose` ran and released the watcher; a reload already queued on the
    /// Qt thread must not revive a closed model.
    closed: bool,
    order: ViewOrder,
    filter: RowFilter,
}

impl Default for DirectoryListModelRust {
    fn default() -> Self {
        Self {
            loaded: 0,
            finished: false,
            status: QString::default(),
            path: QString::default(),
            rows: Vec::new(),
            base: String::new(),
            refresh: None,
            stream_id: None,
            generation: 0,
            remote: false,
            watcher_id: next_watcher_id(),
            reload_pending: false,
            closed: false,
            order: ViewOrder::default(),
            filter: RowFilter::default(),
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
            ROLE_SIZE => QVariant::from(&QString::from(crate::format::size(row.size.unwrap_or(0)))),
            ROLE_MODIFIED => QVariant::from(&QString::from(crate::format::timestamp(
                row.modified_at.unwrap_or(0),
            ))),
            ROLE_IS_DIR => QVariant::from(&(row.kind == EntryKind::Directory)),
            ROLE_HIDDEN => QVariant::from(&row.hidden),
            ROLE_READ_ONLY => QVariant::from(&row.read_only),
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
                QVariant::from(&QString::from(thumbnail_url(self.remote, &path, &row.name, row)))
            }
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
        roles
    }

    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let requested = path.to_string();
        if requested.trim().is_empty() {
            self.as_mut().set_status(QString::from("无路径"));
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
                    rust.refresh = None;
                    rust.reload_pending = false;
                    rust.closed = false;
                }
                self.as_mut().end_reset_model();
            }
        }
        self.as_mut().set_remote(remote);

        self.as_mut().set_finished(false);
        if !refreshing {
            self.as_mut().set_loaded(0);
            self.as_mut().set_path(QString::from(requested.as_str()));
            self.as_mut().set_status(QString::from("读取中…"));
        }

        // An unknown scheme is a failure of this path, not of the previous
        // directory: the model has already adopted `requested` above.
        if let Err(error) = scheme {
            self.as_mut().rust_mut().refresh = None;
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
                                worker.deliver_head(base, view.path, rows, true);
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
                    worker.deliver_head(base, view.path, rows, done);
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

    fn set_order(
        mut self: Pin<&mut Self>,
        sort_key: &QString,
        sort_order: &QString,
        folders_first: bool,
    ) {
        let next = ViewOrder {
            key: SortKey::parse(&sort_key.to_string()),
            order: SortOrder::parse(&sort_order.to_string()),
            folders_first,
        };
        if self.order == next {
            return;
        }
        self.as_mut().rust_mut().order = next;
        if self.rows.len() < 2 || rows_are_sorted(&self.rows, &next) {
            return;
        }
        self.as_mut().resort_rows();
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
        let mut rows = std::mem::take(&mut self.as_mut().rust_mut().rows);
        self.as_mut().about_to_reload();
        sort_rows(&mut rows, &order);
        let total = rows.len() as i32;
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
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

    fn index_of_path(&self, path: &QString) -> i32 {
        let path = path.to_string();
        self.rows
            .iter()
            .position(|row| row.full_path(&self.base) == path)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn path_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|entry| QString::from(entry.full_path(&self.base).as_str()))
            .unwrap_or_default()
    }

    fn is_directory_at(&self, row: i32) -> bool {
        self.rows
            .get(row as usize)
            .map(|entry| entry.kind == EntryKind::Directory)
            .unwrap_or(false)
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
        unsafe {
            self.as_mut().begin_reset_model();
            {
                let mut rust = self.as_mut().rust_mut();
                rust.generation = generation;
                rust.rows.clear();
                rust.base.clear();
            }
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_remote(false);
        self.as_mut().set_loaded(0);
        self.as_mut().set_status(QString::from("已关闭"));
        self.as_mut().set_finished(true);
    }

    fn append_head(
        mut self: Pin<&mut Self>,
        generation: u64,
        base: String,
        path: String,
        rows: Vec<StoredRow>,
        done: bool,
    ) {
        if self.generation != generation {
            return;
        }

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

        let rows = self.as_mut().keep_visible(rows);
        let added = rows.len() as i32;
        let first = self.loaded;
        if added > 0 {
            let last = first + added - 1;
            unsafe {
                self.as_mut()
                    .begin_insert_rows(&QModelIndex::default(), first, last);
                self.as_mut().rust_mut().rows.extend(rows);
                self.as_mut().end_insert_rows();
            }
            self.as_mut().set_loaded(first + added);
        }

        if done {
            // Streamed chunks arrive in enumeration order, so each one is
            // sorted on its own but the sequence as a whole is not. Sorting
            // once here is what makes the finished listing obey the chosen
            // order without paying a reset per batch. `aboutToReload` inside
            // `resortRows` lets the view keep its scroll position.
            self.as_mut().sort_at_completion();
            let pending = self.reload_pending;
            self.as_mut().rust_mut().stream_id = None;
            if pending {
                self.as_mut().rust_mut().reload_pending = false;
            }
            let total = self.rows.len() as i32;
            // Status before `finished`: QML reads the status in
            // `onFinishedChanged`, which is emitted synchronously.
            self.as_mut()
                .set_status(QString::from(format!("{total} 项")));
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
        self.as_mut().resort_rows();
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
        let Some(stage) = self.as_mut().rust_mut().refresh.take() else {
            return;
        };
        let RefreshStage { rows, base, path } = stage;
        // The stage holds the raw listing: filter and order it the same way a
        // first read would, so a refresh neither resurrects a hidden row nor
        // changes the order under the user.
        let mut fresh = self.as_mut().keep_visible(rows);
        sort_rows(&mut fresh, &self.order);

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
                    let mut rust = self.as_mut().rust_mut();
                    rust.rows = fresh;
                    rust.base = base;
                }
                self.as_mut().end_reset_model();
            }
            self.as_mut().set_loaded(total);
        } else {
            self.as_mut().apply_refreshed_rows(fresh);
        }

        self.as_mut()
            .set_status(QString::from(format!("{total} 项")));
        self.as_mut().set_finished(true);
        // A change arrived during this read; re-read now that the stream is
        // over instead of having shown a mix of the two listings.
        if pending {
            self.as_mut().reload();
        }
    }

    /// Turns `fresh` into the smallest set of row operations that reaches it
    /// from the rows on screen. Unchanged rows keep their delegates, so a
    /// create/delete does not rebuild the whole view.
    fn apply_refreshed_rows(mut self: Pin<&mut Self>, mut fresh: Vec<StoredRow>) {
        let (prefix, removed, added) = row_edit(&self.rows, &fresh);
        let middle: Vec<StoredRow> = fresh.drain(prefix..prefix + added).collect();

        unsafe {
            if removed > 0 {
                self.as_mut().begin_remove_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + removed - 1) as i32,
                );
                self.as_mut()
                    .rust_mut()
                    .rows
                    .drain(prefix..prefix + removed);
                self.as_mut().end_remove_rows();
            }
            if added > 0 {
                self.as_mut().begin_insert_rows(
                    &QModelIndex::default(),
                    prefix as i32,
                    (prefix + added - 1) as i32,
                );
                self.as_mut().rust_mut().rows.splice(prefix..prefix, middle);
                self.as_mut().end_insert_rows();
            }
        }

        let total = self.rows.len() as i32;
        self.as_mut().set_loaded(total);
    }

    /// A watcher reported a change in `changed`. `changed` is re-checked
    /// because an arm that lost a navigation can still have an event in flight.
    fn reload_after_change(mut self: Pin<&mut Self>, changed: &str) {
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
        if self.path.to_string() != changed {
            return;
        }
        self.as_mut().reload();
    }

    fn mark_failed(mut self: Pin<&mut Self>, generation: u64, message: String) {
        if self.generation != generation {
            return;
        }
        self.as_mut().rust_mut().stream_id = None;
        self.as_mut().rust_mut().reload_pending = false;
        // A failed refresh keeps the rows already on screen; a failed
        // navigation had already cleared them.
        self.as_mut().rust_mut().refresh = None;
        self.as_mut().set_status(QString::from(message));
        self.as_mut().set_finished(true);
    }
}

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
    changes: Mutex<Option<mpsc::Sender<String>>>,
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
        let Event::DirectoryChanged { path, .. } = event else {
            return;
        };

        let mut changes = self.changes.lock().expect("listing host lock poisoned");
        let sender = changes.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel();
            let thread = self.thread.clone();
            std::thread::spawn(move || forward_changes(receiver, thread));
            sender
        });
        let _ = sender.send(path);
    }
}

/// How long a watcher burst has to go quiet before its reload is delivered.
/// Matches the web shell's debounce, and stays below the interval at which a
/// person notices the list is stale.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);

fn forward_changes(
    changes: mpsc::Receiver<String>,
    thread: cxx_qt::CxxQtThread<qobject::DirectoryListModel>,
) {
    let mut pending = match changes.recv() {
        Ok(path) => path,
        Err(_) => return,
    };

    loop {
        match changes.recv_timeout(CHANGE_DEBOUNCE) {
            Ok(path) => pending = path,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let changed = pending;
                let _ = thread.queue(move |mut model| model.as_mut().reload_after_change(&changed));
                pending = match changes.recv() {
                    Ok(path) => path,
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

    fn deliver_head(&self, base: String, path: String, rows: Vec<StoredRow>, done: bool) {
        let mut state = self.state.lock().expect("listing target poisoned");
        let generation = self.generation;
        let _ = self.thread.queue(move |mut model| {
            model
                .as_mut()
                .append_head(generation, base, path, rows, done)
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

/// The listing's `mtime-size`, used to version a per-file icon URL. Type icons
/// are shared and deliberately not versioned.
fn icon_version(row: &StoredRow) -> String {
    format!("{}-{}", row.modified_at.unwrap_or(0), row.size.unwrap_or(0))
}

/// The thumbnail URL for a row, or empty when the desktop cannot draw one.
///
/// Network entries and directories have no first frame; the extension check is
/// the same one the preview pane uses, so a row's thumbnail slot and the
/// preview's image slot agree about what is renderable.
fn thumbnail_url(remote: bool, path: &str, name: &str, row: &StoredRow) -> String {
    if remote || row.kind != EntryKind::File {
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
    crate::icons::thumbnail_source(path, &icon_version(row))
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
            hidden,
            read_only: false,
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
    fn folders_interleave_when_the_preference_is_off() {
        let mut rows = vec![
            stored("b.txt", EntryKind::File, 10, 1, false),
            stored("a", EntryKind::Directory, 0, 9, false),
        ];
        let order = ViewOrder {
            folders_first: false,
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
            folders_first: true,
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
        assert_eq!(category_of(EntryKind::File, "song.flac"), "audio");
        assert_eq!(category_of(EntryKind::File, "bundle.7z"), "archive");
        assert_eq!(category_of(EntryKind::File, "README"), "file");
        assert_eq!(extension_of("README"), "");
        assert_eq!(extension_of("a.tar.gz"), "gz");
    }
}
