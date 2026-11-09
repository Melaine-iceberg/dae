//! The explorer's display preferences: how a listing is arranged.
//!
//! The Tauri shell kept these in `localStorage` (see
//! `src/features/explorer/preferences.ts`); the Qt shell has no web storage, so
//! the same choices live in a small document next to the other dae profile
//! files. Only this shell reads it — the React shell's atoms stay its own —
//! which is why the file is named after the shell rather than the app.
//!
//! One process-wide singleton, because two windows must agree: sorting one
//! listing by size is a statement about the user's preference, not about that
//! window. `DirectoryListModel` owns the actual ordering and filtering; this
//! object is only the source of truth QML binds to and writes back.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cxx_qt_lib::QString;

const FILE_NAME: &str = "explorer-qt.ini";
/// Per-directory layouts live beside the global document rather than in it:
/// the global file stays a readable list of defaults, and the map that grows
/// to the LRU cap is one TOML table of tables.
const VIEWS_FILE_NAME: &str = "views-qt.toml";
/// Entries kept before the oldest is evicted. 512 covers a year of daily
/// folders without an unbounded document.
const MAX_STORED_VIEWS: usize = 512;
/// Queries kept in the search field's history.
const SEARCH_HISTORY_LIMIT: usize = 20;
/// Named searches kept on disk. Well past what the sidebar section shows;
/// the list scrolls, and the cap only stops an unbounded document.
const SAVED_SEARCH_LIMIT: usize = 40;
/// Column ids the list view knows. A stored id outside this set is ignored,
/// so removing a column from a future build cannot leave a hole.
const COLUMN_IDS: &[&str] = &["type", "size", "modified", "created", "tags"];

const VIEW_MODES: &[&str] = &["list", "grid", "column"];
const DENSITIES: &[&str] = &["compact", "comfortable", "spacious"];
const SORT_KEYS: &[&str] = &["name", "modified", "created", "type", "size", "path", "tag"];
const FOLDER_ORDERS: &[&str] = &["foldersFirst", "filesFirst", "mixed"];
const GROUP_KEYS: &[&str] = &["", "name", "modified", "created", "size", "type", "tag"];
const GROUP_DATES: &[&str] = &["year", "month", "day"];
const SORT_ORDERS: &[&str] = &["asc", "desc"];
const ICON_STYLES: &[&str] = &["themed", "system"];
const KIND_FILTERS: &[&str] = &["all", "folders", "files", "images"];
const MODIFIED_FILTERS: &[&str] = &["any", "today", "week", "month"];
const SIZE_FILTERS: &[&str] = &["any", "small", "medium", "large"];
/// `binary` prints KiB/MiB (the 1024 divisor); `decimal` prints KB/MB.
const SIZE_UNITS: &[&str] = &["binary", "decimal"];

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// QML usage: read `ExplorerPrefs.sortKey`, call
        /// `ExplorerPrefs.setString("sortKey", "size")`.
        ///
        /// `setString`/`setBool` are keyed rather than one method per field so
        /// adding a preference costs a match arm instead of a bridge method.
        /// Unknown keys and unknown values are ignored: QML is the only caller
        /// and a typo must not corrupt the document on disk.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, view_mode, cxx_name = "viewMode")]
        #[qproperty(QString, density)]
        #[qproperty(QString, sort_key, cxx_name = "sortKey")]
        #[qproperty(QString, sort_order, cxx_name = "sortOrder")]
        /// `foldersFirst`, `filesFirst` or `mixed`. Replaces the old boolean;
        /// a stored `foldersFirst=false` migrates to `mixed`.
        #[qproperty(QString, folder_order, cxx_name = "folderOrder")]
        /// The section key (`""` for none) and, for the two date keys, the
        /// bucket precision.
        #[qproperty(QString, group_key, cxx_name = "groupKey")]
        #[qproperty(QString, group_date, cxx_name = "groupDate")]
        /// Grid tile / list thumbnail size, 24–96 dp.
        #[qproperty(i32, icon_size, cxx_name = "iconSize")]
        /// Whether names are painted with their extension. Display only: it
        /// never changes what a rename, a search or a sort sees.
        #[qproperty(bool, show_extensions, cxx_name = "showExtensions")]
        /// The visible details columns, comma-separated.
        #[qproperty(QString, columns)]
        /// Their widths, comma-separated, in the same order as `columns`.
        #[qproperty(QString, column_widths, cxx_name = "columnWidths")]
        /// Layout follows the directory when off, and one global layout when
        /// on.
        #[qproperty(bool, sync_layout, cxx_name = "syncLayout")]
        /// Pick a starting layout for a directory that has none, from what it
        /// contains.
        #[qproperty(bool, adaptive_layout, cxx_name = "adaptiveLayout")]
        /// The last 20 name searches, newest first, as JSON. The search field
        /// shows them when it is focused and empty; `rememberSearch` writes
        /// them.
        #[qproperty(QString, search_history, cxx_name = "searchHistory")]
        /// Named searches, newest first, as JSON: the query plus the mode,
        /// scope and filters it ran with. Written by `saveSearch`, read by the
        /// sidebar section and the search bar's "保存" chip.
        #[qproperty(QString, saved_searches, cxx_name = "savedSearches")]
        /// The directory whose stored layout is on screen. Write it with
        /// `setString("currentPath", …)`; the setter also restores that
        /// directory's layout when `syncLayout` is off.
        #[qproperty(QString, current_path, cxx_name = "currentPath")]
        #[qproperty(QString, icon_style, cxx_name = "iconStyle")]
        #[qproperty(bool, row_thumbnails, cxx_name = "rowThumbnails")]
        #[qproperty(bool, show_hidden_files, cxx_name = "showHiddenFiles")]
        #[qproperty(QString, kind_filter, cxx_name = "kindFilter")]
        #[qproperty(QString, modified_filter, cxx_name = "modifiedFilter")]
        #[qproperty(QString, size_filter, cxx_name = "sizeFilter")]
        /// `binary` (KiB/MiB) or `decimal` (KB/MB). Read by the formatter for
        /// every row, so it is a display preference rather than a layout key.
        #[qproperty(QString, size_units, cxx_name = "sizeUnits")]
        /// Reopen the previous session's windows and tabs on launch.
        #[qproperty(bool, restore_session, cxx_name = "restoreSession")]
        /// Landing-page sections to hide, comma-separated. A free-text value:
        /// the model drops names it does not know.
        #[qproperty(QString, hidden_home_sections, cxx_name = "hiddenHomeSections")]
        #[qproperty(bool, split_enabled, cxx_name = "splitEnabled")]
        /// Off by default: the shell animates, and this is the single switch the
        /// spec calls for. Qt 6 has no system-wide reduced-motion query, so the
        /// preference has to live here and QML has to ask before every Behavior.
        #[qproperty(bool, reduce_motion, cxx_name = "reduceMotion")]
        /// Panel widths, in dp. A drag handle writes them; the window reads
        /// them back on the next launch so a widened preview stays widened.
        #[qproperty(i32, preview_width, cxx_name = "previewWidth")]
        #[qproperty(i32, sidebar_width, cxx_name = "sidebarWidth")]
        /// The executable a new terminal starts. Empty means the shell picks
        /// the platform default (pwsh/PowerShell, `$SHELL`, …).
        #[qproperty(QString, terminal_shell, cxx_name = "terminalShell")]
        /// Toolbar action ids, comma-separated, in display order. The leading
        /// group sits before the address bar, the trailing group after it.
        /// QML normalizes unknown or duplicated ids against its own model.
        #[qproperty(QString, toolbar_leading, cxx_name = "toolbarLeading")]
        #[qproperty(QString, toolbar_trailing, cxx_name = "toolbarTrailing")]
        /// An image path painted behind the listing. Empty means none.
        #[qproperty(QString, background_image, cxx_name = "backgroundImage")]
        /// How much of the theme canvas covers the image, 0–90: higher is
        /// calmer and keeps text contrast; the listing never becomes fully
        /// transparent.
        #[qproperty(i32, background_dim, cxx_name = "backgroundDim")]
        type ExplorerPrefs = super::ExplorerPrefsRust;

        #[qinvokable]
        #[cxx_name = "setString"]
        fn set_string(self: Pin<&mut ExplorerPrefs>, key: &QString, value: &QString);

        #[qinvokable]
        #[cxx_name = "setBool"]
        fn set_bool(self: Pin<&mut ExplorerPrefs>, key: &QString, value: bool);

        /// Widths are clamped on the way in, not by the caller: a preference
        /// file edited by hand should not be able to make a panel invisible.
        #[qinvokable]
        #[cxx_name = "setNumber"]
        fn set_number(self: Pin<&mut ExplorerPrefs>, key: &QString, value: i32);

        /// `grid`, `column` or `list` for a directory with no stored layout,
        /// from its contents. Returns the mode it applied, or an empty string
        /// when the user has a stored layout or adaptive layout is off.
        #[qinvokable]
        #[cxx_name = "applyAdaptiveView"]
        fn apply_adaptive_view(
            self: Pin<&mut ExplorerPrefs>,
            path: &QString,
            images: i32,
            files: i32,
            depth: i32,
        ) -> QString;

        /// Records a query at the front of the history, de-duplicated and
        /// capped at 20.
        #[qinvokable]
        #[cxx_name = "rememberSearch"]
        fn remember_search(self: Pin<&mut ExplorerPrefs>, query: &QString);

        #[qinvokable]
        #[cxx_name = "clearSearchHistory"]
        fn clear_search_history(self: Pin<&mut ExplorerPrefs>);

        /// Stores `spec` (a `SavedSearch` as JSON) under `name`. A name that
        /// already exists replaces that entry instead of adding a second one
        /// with the same label. False when the name or the query is empty, or
        /// when the spec is not valid JSON.
        #[qinvokable]
        #[cxx_name = "saveSearch"]
        fn save_search(
            self: Pin<&mut ExplorerPrefs>,
            name: &QString,
            spec: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "deleteSavedSearch"]
        fn delete_saved_search(self: Pin<&mut ExplorerPrefs>, id: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "renameSavedSearch"]
        fn rename_saved_search(
            self: Pin<&mut ExplorerPrefs>,
            id: &QString,
            name: &QString,
        ) -> bool;

        /// Drops a directory's stored layout, returning it to the defaults.
        #[qinvokable]
        #[cxx_name = "forgetView"]
        fn forget_view(self: Pin<&mut ExplorerPrefs>, path: &QString);

        /// Kinds, modified-time buckets and size buckets back to "all/any/any".
        #[qinvokable]
        #[cxx_name = "resetFilters"]
        fn reset_filters(self: Pin<&mut ExplorerPrefs>);
    }
}

use std::pin::Pin;

/// One named search: the query and everything the bar needs to re-run it
/// exactly as it ran. Every field but `id`, `name` and `query` is optional in
/// the document, so a hand edit that drops one falls back to the default.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SavedSearch {
    id: String,
    name: String,
    query: String,
    /// `name` or `content`.
    mode: String,
    /// `current`, `favorites`, `recent` or `volumes`.
    scope: String,
    /// The content-search type chip (`all`, `doc`, …).
    type_id: String,
    /// The listing's kind/modified/size buckets, which the name search reuses.
    kind: String,
    modified: String,
    size: String,
    regex: bool,
    case_sensitive: bool,
    /// The content-search glob list.
    filter: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredPrefs {
    view_mode: String,
    density: String,
    sort_key: String,
    sort_order: String,
    folder_order: String,
    group_key: String,
    group_date: String,
    icon_style: String,
    icon_size: i32,
    row_thumbnails: bool,
    show_extensions: bool,
    columns: String,
    column_widths: String,
    show_hidden_files: bool,
    kind_filter: String,
    modified_filter: String,
    size_filter: String,
    size_units: String,
    restore_session: bool,
    hidden_home_sections: String,
    split_enabled: bool,
    reduce_motion: bool,
    sync_layout: bool,
    adaptive_layout: bool,
    preview_width: i32,
    sidebar_width: i32,
    terminal_shell: String,
    toolbar_leading: String,
    toolbar_trailing: String,
    background_image: String,
    background_dim: i32,
    search_history: Vec<String>,
    saved_searches: Vec<SavedSearch>,
}

impl Default for StoredPrefs {
    fn default() -> Self {
        Self {
            view_mode: "list".to_owned(),
            density: "comfortable".to_owned(),
            sort_key: "name".to_owned(),
            sort_order: "asc".to_owned(),
            folder_order: "foldersFirst".to_owned(),
            group_key: String::new(),
            group_date: "month".to_owned(),
            // The shell draws as one object: arc, type and state all in the same
            // ink, with colour reserved for the signal and the git badges. A
            // column of full-colour shell icons broke that on the first screen.
            // The desktop's own icons stay one menu click away.
            icon_style: "themed".to_owned(),
            icon_size: 38,
            row_thumbnails: false,
            show_extensions: true,
            columns: "type,size,modified".to_owned(),
            column_widths: "108,92,142,142,120".to_owned(),
            // The app has always listed hidden entries; the toggle is for
            // people who want the classic hidden-by-default reading.
            show_hidden_files: true,
            kind_filter: "all".to_owned(),
            modified_filter: "any".to_owned(),
            size_filter: "any".to_owned(),
            size_units: "binary".to_owned(),
            restore_session: true,
            hidden_home_sections: String::new(),
            split_enabled: false,
            reduce_motion: false,
            sync_layout: true,
            adaptive_layout: false,
            preview_width: 300,
            sidebar_width: 216,
            terminal_shell: String::new(),
            toolbar_leading: "back,forward,up".to_owned(),
            toolbar_trailing: "search,trash,view,more".to_owned(),
            background_image: String::new(),
            background_dim: 70,
            search_history: Vec::new(),
            saved_searches: Vec::new(),
        }
    }
}

impl StoredPrefs {
    fn read(path: &PathBuf) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut prefs = Self::default();
        prefs.apply_document(&contents);
        prefs
    }

    fn apply_document(&mut self, contents: &str) {
        let fields: BTreeMap<&str, &str> = contents
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                line.split_once('=')
                    .map(|(key, value)| (key.trim(), value.trim()))
            })
            .collect();

        let string = |key: &str, allowed: &[&str], target: &mut String| {
            if let Some(value) = fields.get(key)
                && allowed.contains(value)
            {
                *target = (*value).to_owned();
            }
        };
        let number = |key: &str, target: &mut i32| {
            if let Some(value) = fields.get(key)
                && let Ok(parsed) = value.parse::<i32>()
            {
                *target = parsed;
            }
        };
        let boolean = |key: &str, target: &mut bool| match fields.get(key).copied() {
            Some("true") => *target = true,
            Some("false") => *target = false,
            _ => {}
        };

        string("viewMode", VIEW_MODES, &mut self.view_mode);
        string("density", DENSITIES, &mut self.density);
        string("sortKey", SORT_KEYS, &mut self.sort_key);
        string("sortOrder", SORT_ORDERS, &mut self.sort_order);
        string("folderOrder", FOLDER_ORDERS, &mut self.folder_order);
        string("groupKey", GROUP_KEYS, &mut self.group_key);
        string("groupDate", GROUP_DATES, &mut self.group_date);
        // migrated: the old boolean only carried "files first" as `false`,
        // and only when the enum key is absent (a first run after upgrading).
        if !fields.contains_key("folderOrder") && fields.get("foldersFirst") == Some(&"false") {
            self.folder_order = "mixed".to_owned();
        }
        string("iconStyle", ICON_STYLES, &mut self.icon_style);
        number("iconSize", &mut self.icon_size);
        boolean("rowThumbnails", &mut self.row_thumbnails);
        boolean("showExtensions", &mut self.show_extensions);
        if let Some(value) = fields.get("columns") {
            self.columns = sanitize_columns(value);
        }
        if let Some(value) = fields.get("columnWidths") {
            self.column_widths = sanitize_widths(value);
        }
        boolean("showHiddenFiles", &mut self.show_hidden_files);
        string("kindFilter", KIND_FILTERS, &mut self.kind_filter);
        string(
            "modifiedFilter",
            MODIFIED_FILTERS,
            &mut self.modified_filter,
        );
        string("sizeFilter", SIZE_FILTERS, &mut self.size_filter);
        string("sizeUnits", SIZE_UNITS, &mut self.size_units);
        boolean("restoreSession", &mut self.restore_session);
        if let Some(value) = fields.get("hiddenHomeSections") {
            self.hidden_home_sections = (*value).to_owned();
        }
        boolean("splitEnabled", &mut self.split_enabled);
        boolean("reduceMotion", &mut self.reduce_motion);
        boolean("syncLayout", &mut self.sync_layout);
        boolean("adaptiveLayout", &mut self.adaptive_layout);
        number("previewWidth", &mut self.preview_width);
        number("sidebarWidth", &mut self.sidebar_width);
        // Free text, unlike the enumerated preferences above: any executable
        // name or path is valid, and an empty value means "the platform guess".
        if let Some(value) = fields.get("terminalShell") {
            self.terminal_shell = (*value).to_owned();
        }
        // Free text: the shell's own model normalizes ids, and an image path
        // is whatever the file dialog handed over.
        if let Some(value) = fields.get("toolbarLeading") {
            self.toolbar_leading = (*value).to_owned();
        }
        if let Some(value) = fields.get("toolbarTrailing") {
            self.toolbar_trailing = (*value).to_owned();
        }
        if let Some(value) = fields.get("backgroundImage") {
            self.background_image = (*value).to_owned();
        }
        if let Some(value) = fields.get("backgroundDim")
            && let Ok(parsed) = value.parse::<i32>()
        {
            self.background_dim = parsed.clamp(0, 90);
        }
        if let Some(value) = fields.get("searchHistory") {
            self.search_history = serde_json::from_str(value).unwrap_or_default();
            self.search_history.truncate(SEARCH_HISTORY_LIMIT);
        }
        if let Some(value) = fields.get("savedSearches") {
            let mut saved: Vec<SavedSearch> = serde_json::from_str(value).unwrap_or_default();
            // An entry without a name or a query can never be shown or run, so
            // it is dropped on the way in instead of being filtered by every
            // reader.
            saved.retain(|entry| !entry.query.trim().is_empty() && !entry.name.trim().is_empty());
            saved.truncate(SAVED_SEARCH_LIMIT);
            self.saved_searches = saved;
        }
    }

    fn document(&self) -> String {
        let mut out = String::new();
        out.push_str("# dae Qt shell explorer preferences\n");
        let write = |out: &mut String, key: &str, value: &str| {
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push('\n');
        };
        write(&mut out, "viewMode", &self.view_mode);
        write(&mut out, "density", &self.density);
        write(&mut out, "sortKey", &self.sort_key);
        write(&mut out, "sortOrder", &self.sort_order);
        write(&mut out, "folderOrder", &self.folder_order);
        write(&mut out, "groupKey", &self.group_key);
        write(&mut out, "groupDate", &self.group_date);
        write(&mut out, "iconStyle", &self.icon_style);
        write(&mut out, "iconSize", &self.icon_size.to_string());
        write(&mut out, "rowThumbnails", bool_text(self.row_thumbnails));
        write(&mut out, "showExtensions", bool_text(self.show_extensions));
        write(&mut out, "columns", &self.columns);
        write(&mut out, "columnWidths", &self.column_widths);
        write(
            &mut out,
            "showHiddenFiles",
            bool_text(self.show_hidden_files),
        );
        write(&mut out, "kindFilter", &self.kind_filter);
        write(&mut out, "modifiedFilter", &self.modified_filter);
        write(&mut out, "sizeFilter", &self.size_filter);
        write(&mut out, "sizeUnits", &self.size_units);
        write(&mut out, "restoreSession", bool_text(self.restore_session));
        write(&mut out, "hiddenHomeSections", &self.hidden_home_sections);
        write(&mut out, "splitEnabled", bool_text(self.split_enabled));
        write(&mut out, "reduceMotion", bool_text(self.reduce_motion));
        write(&mut out, "syncLayout", bool_text(self.sync_layout));
        write(&mut out, "adaptiveLayout", bool_text(self.adaptive_layout));
        write(&mut out, "previewWidth", &self.preview_width.to_string());
        write(&mut out, "sidebarWidth", &self.sidebar_width.to_string());
        write(&mut out, "terminalShell", &self.terminal_shell);
        write(&mut out, "toolbarLeading", &self.toolbar_leading);
        write(&mut out, "toolbarTrailing", &self.toolbar_trailing);
        write(&mut out, "backgroundImage", &self.background_image);
        write(&mut out, "backgroundDim", &self.background_dim.to_string());
        // One line of JSON: the INI reader is line-based, so a newline-
        // separated list would end the value at the first entry.
        if let Ok(encoded) = serde_json::to_string(&self.search_history) {
            write(&mut out, "searchHistory", &encoded);
        }
        if let Ok(encoded) = serde_json::to_string(&self.saved_searches) {
            write(&mut out, "savedSearches", &encoded);
        }
        out
    }

    /// Applies the directory layout in `view` over the defaults.
    fn apply_view(&mut self, view: &StoredView) {
        let apply_string = |value: &Option<String>, allowed: &[&str], target: &mut String| {
            if let Some(value) = value
                && allowed.contains(&value.as_str())
            {
                *target = value.clone();
            }
        };
        apply_string(&view.view_mode, VIEW_MODES, &mut self.view_mode);
        apply_string(&view.density, DENSITIES, &mut self.density);
        apply_string(&view.sort_key, SORT_KEYS, &mut self.sort_key);
        apply_string(&view.sort_order, SORT_ORDERS, &mut self.sort_order);
        apply_string(&view.folder_order, FOLDER_ORDERS, &mut self.folder_order);
        apply_string(&view.group_key, GROUP_KEYS, &mut self.group_key);
        apply_string(&view.group_date, GROUP_DATES, &mut self.group_date);
        if let Some(size) = view.icon_size {
            self.icon_size = size;
        }
        if let Some(show) = view.show_extensions {
            self.show_extensions = show;
        }
        if let Some(columns) = &view.columns {
            self.columns = sanitize_columns(columns);
        }
        if let Some(widths) = &view.column_widths {
            self.column_widths = sanitize_widths(widths);
        }
    }

    fn view(&self, last_used: u64) -> StoredView {
        StoredView {
            view_mode: Some(self.view_mode.clone()),
            density: Some(self.density.clone()),
            sort_key: Some(self.sort_key.clone()),
            sort_order: Some(self.sort_order.clone()),
            folder_order: Some(self.folder_order.clone()),
            group_key: Some(self.group_key.clone()),
            group_date: Some(self.group_date.clone()),
            icon_size: Some(self.icon_size),
            show_extensions: Some(self.show_extensions),
            columns: Some(self.columns.clone()),
            column_widths: Some(self.column_widths.clone()),
            last_used: Some(last_used),
        }
    }
}

/// One directory's layout. Every field is optional so an older document (or a
/// hand edit that drops a key) still loads and the missing field falls back to
/// the global default.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredView {
    view_mode: Option<String>,
    density: Option<String>,
    sort_key: Option<String>,
    sort_order: Option<String>,
    folder_order: Option<String>,
    group_key: Option<String>,
    group_date: Option<String>,
    icon_size: Option<i32>,
    show_extensions: Option<bool>,
    columns: Option<String>,
    column_widths: Option<String>,
    last_used: Option<u64>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ViewsFile {
    #[serde(default)]
    views: BTreeMap<String, StoredView>,
}

/// The key a directory is stored under: trailing separators trimmed,
/// case-folded on the platforms whose file systems fold.
fn view_key(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let trimmed = if trimmed.is_empty() { path } else { trimmed };
    if cfg!(any(windows, target_os = "macos")) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_owned()
    }
}

fn views_path() -> Option<PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|dir| dir.join(VIEWS_FILE_NAME))
}

fn read_views() -> ViewsFile {
    let Some(path) = views_path() else {
        return ViewsFile::default();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return ViewsFile::default();
    };
    toml::from_str(&contents).unwrap_or_else(|error| {
        log::warn!(target: "dae::preferences", "views-qt.toml is not readable: {error}");
        ViewsFile::default()
    })
}

fn write_views(views: &mut ViewsFile) {
    // LRU cap: the oldest `lastUsed` leaves first. A missing stamp is the
    // oldest thing in the document.
    if views.views.len() > MAX_STORED_VIEWS {
        let mut keys: Vec<(String, u64)> = views
            .views
            .iter()
            .map(|(key, view)| (key.clone(), view.last_used.unwrap_or(0)))
            .collect();
        keys.sort_unstable_by_key(|(_, stamp)| *stamp);
        let excess = views.views.len() - MAX_STORED_VIEWS;
        for (key, _) in keys.into_iter().take(excess) {
            views.views.remove(&key);
        }
    }
    let Some(path) = views_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log::warn!(target: "dae::preferences", "could not create the views directory: {error}");
        return;
    }
    let Ok(contents) = toml::to_string_pretty(views) else {
        return;
    };
    if let Err(error) = dae_core::fs::atomic::write_atomic(&path, contents.as_bytes()) {
        log::warn!(target: "dae::preferences", "could not save per-directory views: {error}");
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Keeps only known column ids, in the canonical order, with no duplicates.
fn sanitize_columns(value: &str) -> String {
    let requested: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|id| COLUMN_IDS.contains(id))
        .collect();
    COLUMN_IDS
        .iter()
        .filter(|id| requested.contains(id))
        .copied()
        .collect::<Vec<_>>()
        .join(",")
}

/// Keeps one clamped width per known column, padding a short list with the
/// defaults. The list is keyed by [`COLUMN_IDS`] order, not by which columns
/// are visible, so hiding a column does not shift the widths of the rest.
fn sanitize_widths(value: &str) -> String {
    const DEFAULTS: [i32; 5] = [108, 92, 142, 142, 120];
    let mut widths: Vec<String> = value
        .split(',')
        .map(|part| part.trim())
        .filter_map(|part| part.parse::<i32>().ok())
        .map(|width| width.clamp(60, 420).to_string())
        .collect();
    for default in DEFAULTS.iter().skip(widths.len()) {
        widths.push(default.to_string());
    }
    widths.truncate(DEFAULTS.len());
    widths.join(",")
}

fn size_units_of(value: &str) -> crate::format::SizeUnits {
    if value == "decimal" {
        crate::format::SizeUnits::Decimal
    } else {
        crate::format::SizeUnits::Binary
    }
}

/// Applies the display preferences other modules read directly. `format::size`
/// has no QML object to consult, and it runs on the first row of the first
/// listing — before the `ExplorerPrefs` singleton exists — so the value is read
/// from the document once during startup.
pub fn apply_process_wide_preferences() {
    let stored = preferences_path()
        .map(|path| StoredPrefs::read(&path))
        .unwrap_or_default();
    crate::format::set_size_units(size_units_of(&stored.size_units));
}

fn preferences_path() -> Option<PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|dir| dir.join(FILE_NAME))
}

fn save(prefs: &StoredPrefs) {
    let Some(path) = preferences_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log::warn!(target: "dae::preferences", "could not create the preferences directory: {error}");
        return;
    }
    if let Err(error) = std::fs::write(&path, prefs.document()) {
        log::warn!(target: "dae::preferences", "could not save explorer preferences: {error}");
    }
}

pub struct ExplorerPrefsRust {
    view_mode: QString,
    density: QString,
    sort_key: QString,
    sort_order: QString,
    folder_order: QString,
    group_key: QString,
    group_date: QString,
    icon_style: QString,
    icon_size: i32,
    row_thumbnails: bool,
    show_extensions: bool,
    columns: QString,
    column_widths: QString,
    show_hidden_files: bool,
    kind_filter: QString,
    modified_filter: QString,
    size_filter: QString,
    size_units: QString,
    restore_session: bool,
    hidden_home_sections: QString,
    split_enabled: bool,
    reduce_motion: bool,
    sync_layout: bool,
    adaptive_layout: bool,
    preview_width: i32,
    sidebar_width: i32,
    terminal_shell: QString,
    toolbar_leading: QString,
    toolbar_trailing: QString,
    background_image: QString,
    background_dim: i32,
    search_history: QString,
    saved_searches: QString,
    current_path: QString,
}

impl Default for ExplorerPrefsRust {
    fn default() -> Self {
        let stored = preferences_path()
            .map(|path| StoredPrefs::read(&path))
            .unwrap_or_default();
        Self::from_stored(&stored)
    }
}

impl ExplorerPrefsRust {
    fn from_stored(prefs: &StoredPrefs) -> Self {
        Self {
            view_mode: QString::from(prefs.view_mode.as_str()),
            density: QString::from(prefs.density.as_str()),
            sort_key: QString::from(prefs.sort_key.as_str()),
            sort_order: QString::from(prefs.sort_order.as_str()),
            folder_order: QString::from(prefs.folder_order.as_str()),
            group_key: QString::from(prefs.group_key.as_str()),
            group_date: QString::from(prefs.group_date.as_str()),
            icon_style: QString::from(prefs.icon_style.as_str()),
            icon_size: prefs.icon_size,
            row_thumbnails: prefs.row_thumbnails,
            show_extensions: prefs.show_extensions,
            columns: QString::from(prefs.columns.as_str()),
            column_widths: QString::from(prefs.column_widths.as_str()),
            show_hidden_files: prefs.show_hidden_files,
            kind_filter: QString::from(prefs.kind_filter.as_str()),
            modified_filter: QString::from(prefs.modified_filter.as_str()),
            size_filter: QString::from(prefs.size_filter.as_str()),
            size_units: QString::from(prefs.size_units.as_str()),
            restore_session: prefs.restore_session,
            hidden_home_sections: QString::from(prefs.hidden_home_sections.as_str()),
            split_enabled: prefs.split_enabled,
            reduce_motion: prefs.reduce_motion,
            sync_layout: prefs.sync_layout,
            adaptive_layout: prefs.adaptive_layout,
            preview_width: prefs.preview_width,
            sidebar_width: prefs.sidebar_width,
            terminal_shell: QString::from(prefs.terminal_shell.as_str()),
            toolbar_leading: QString::from(prefs.toolbar_leading.as_str()),
            toolbar_trailing: QString::from(prefs.toolbar_trailing.as_str()),
            background_image: QString::from(prefs.background_image.as_str()),
            background_dim: prefs.background_dim,
            search_history: QString::from(
                serde_json::to_string(&prefs.search_history)
                    .unwrap_or_else(|_| "[]".to_owned())
                    .as_str(),
            ),
            saved_searches: QString::from(
                serde_json::to_string(&prefs.saved_searches)
                    .unwrap_or_else(|_| "[]".to_owned())
                    .as_str(),
            ),
            current_path: QString::default(),
        }
    }

    fn stored(&self) -> StoredPrefs {
        StoredPrefs {
            view_mode: self.view_mode.to_string(),
            density: self.density.to_string(),
            sort_key: self.sort_key.to_string(),
            sort_order: self.sort_order.to_string(),
            folder_order: self.folder_order.to_string(),
            group_key: self.group_key.to_string(),
            group_date: self.group_date.to_string(),
            icon_style: self.icon_style.to_string(),
            icon_size: self.icon_size,
            row_thumbnails: self.row_thumbnails,
            show_extensions: self.show_extensions,
            columns: self.columns.to_string(),
            column_widths: self.column_widths.to_string(),
            show_hidden_files: self.show_hidden_files,
            kind_filter: self.kind_filter.to_string(),
            modified_filter: self.modified_filter.to_string(),
            size_filter: self.size_filter.to_string(),
            size_units: self.size_units.to_string(),
            restore_session: self.restore_session,
            hidden_home_sections: self.hidden_home_sections.to_string(),
            split_enabled: self.split_enabled,
            reduce_motion: self.reduce_motion,
            sync_layout: self.sync_layout,
            adaptive_layout: self.adaptive_layout,
            preview_width: self.preview_width,
            sidebar_width: self.sidebar_width,
            terminal_shell: self.terminal_shell.to_string(),
            toolbar_leading: self.toolbar_leading.to_string(),
            toolbar_trailing: self.toolbar_trailing.to_string(),
            background_image: self.background_image.to_string(),
            background_dim: self.background_dim,
            search_history: serde_json::from_str(&self.search_history.to_string())
                .unwrap_or_default(),
            saved_searches: serde_json::from_str(&self.saved_searches.to_string())
                .unwrap_or_default(),
        }
    }

    /// Writes the current layout to the document it belongs to: the global
    /// preferences while `syncLayout` is on (or no directory is known), the
    /// current directory's entry otherwise.
    fn persist_layout(&self) {
        if self.sync_layout || self.current_path.as_slice().is_empty() {
            save(&self.stored());
            return;
        }
        let key = view_key(&self.current_path.to_string());
        let mut views = read_views();
        views.views.insert(key, self.stored().view(now_millis()));
        write_views(&mut views);
    }
}

/// The adaptive rule, kept pure and boring: images dominate a folder of at
/// least eight files → grid; three levels deep → columns; otherwise a list.
/// Wrong guesses are one menu click away, which is why the rule does not try
/// to be clever.
fn suggest_view(images: i32, files: i32, depth: i32) -> &'static str {
    if files >= 8 && i64::from(images) * 5 >= i64::from(files) * 3 {
        "grid"
    } else if depth >= 3 {
        "column"
    } else {
        "list"
    }
}

fn decode_saved_searches(json: &str) -> Vec<SavedSearch> {
    let mut entries: Vec<SavedSearch> = serde_json::from_str(json).unwrap_or_default();
    entries.truncate(SAVED_SEARCH_LIMIT);
    entries
}

fn encode_saved_searches(entries: &[SavedSearch]) -> QString {
    QString::from(
        serde_json::to_string(entries)
            .unwrap_or_else(|_| "[]".to_owned())
            .as_str(),
    )
}

/// Ids are compared, never shown, so a timestamp plus a suffix for the
/// same-millisecond case is enough.
fn next_saved_search_id(entries: &[SavedSearch]) -> String {
    let base = format!("s{}", now_millis());
    let mut candidate = base.clone();
    let mut suffix = 2;
    while entries.iter().any(|entry| entry.id == candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

impl qobject::ExplorerPrefs {
    fn restore_view(mut self: Pin<&mut Self>, path: &str) {
        if self.sync_layout {
            return;
        }
        let mut stored = preferences_path()
            .map(|path| StoredPrefs::read(&path))
            .unwrap_or_default();
        if let Some(view) = read_views().views.get(&view_key(path)) {
            stored.apply_view(view);
        }

        self.as_mut()
            .set_view_mode(QString::from(stored.view_mode.as_str()));
        self.as_mut()
            .set_density(QString::from(stored.density.as_str()));
        self.as_mut()
            .set_sort_key(QString::from(stored.sort_key.as_str()));
        self.as_mut()
            .set_sort_order(QString::from(stored.sort_order.as_str()));
        self.as_mut()
            .set_folder_order(QString::from(stored.folder_order.as_str()));
        self.as_mut()
            .set_group_key(QString::from(stored.group_key.as_str()));
        self.as_mut()
            .set_group_date(QString::from(stored.group_date.as_str()));
        self.as_mut().set_icon_size(stored.icon_size);
        self.as_mut().set_show_extensions(stored.show_extensions);
        self.as_mut()
            .set_columns(QString::from(stored.columns.as_str()));
        self.as_mut()
            .set_column_widths(QString::from(stored.column_widths.as_str()));
    }

    fn apply_adaptive_view(
        mut self: Pin<&mut Self>,
        path: &QString,
        images: i32,
        files: i32,
        depth: i32,
    ) -> QString {
        let path = path.to_string();
        if self.sync_layout || !self.adaptive_layout || path.trim().is_empty() {
            return QString::default();
        }
        if read_views().views.contains_key(&view_key(&path)) {
            return QString::default();
        }
        let mode = suggest_view(images, files, depth);
        self.as_mut().set_current_path(QString::from(path.as_str()));
        self.as_mut().set_view_mode(QString::from(mode));
        self.as_mut().persist_layout();
        QString::from(mode)
    }

    fn forget_view(mut self: Pin<&mut Self>, path: &QString) {
        let key = view_key(&path.to_string());
        let mut views = read_views();
        if views.views.remove(&key).is_some() {
            write_views(&mut views);
        }
        let path = path.to_string();
        self.as_mut().restore_view(&path);
    }
}

impl qobject::ExplorerPrefs {
    fn set_string(mut self: Pin<&mut Self>, key: &QString, value: &QString) {
        let key = key.to_string();
        let value = value.to_string();

        // The one free-text preference: an executable name or path. Empty
        // clears it so the platform default is used again.
        if key == "terminalShell" {
            let next = QString::from(value.trim());
            if self.terminal_shell != next {
                self.as_mut().set_terminal_shell(next);
                save(&self.stored());
            }
            return;
        }

        // The directory whose layout is on screen. Writing it is how the shell
        // asks for that directory's stored layout; it is not persisted itself.
        if key == "currentPath" {
            let next = QString::from(value.as_str());
            if self.current_path == next {
                return;
            }
            self.as_mut().set_current_path(next);
            self.as_mut().restore_view(&value);
            return;
        }

        if key == "hiddenHomeSections" {
            let next = QString::from(value.trim());
            if self.hidden_home_sections != next {
                self.as_mut().set_hidden_home_sections(next);
                save(&self.stored());
            }
            return;
        }

        // Free text, like `terminalShell`: QML owns the toolbar id model and
        // the image path comes from a file dialog. An empty image clears it.
        if key == "toolbarLeading" || key == "toolbarTrailing" || key == "backgroundImage" {
            let next = QString::from(value.trim());
            let changed = match key.as_str() {
                "toolbarLeading" => {
                    if self.toolbar_leading == next {
                        return;
                    }
                    self.as_mut().set_toolbar_leading(next);
                    true
                }
                "toolbarTrailing" => {
                    if self.toolbar_trailing == next {
                        return;
                    }
                    self.as_mut().set_toolbar_trailing(next);
                    true
                }
                _ => {
                    if self.background_image == next {
                        return;
                    }
                    self.as_mut().set_background_image(next);
                    true
                }
            };
            if changed {
                save(&self.stored());
            }
            return;
        }

        if key == "columns" {
            let next = QString::from(sanitize_columns(&value).as_str());
            if self.columns != next {
                self.as_mut().set_columns(next);
                self.as_mut().persist_layout();
            }
            return;
        }

        if key == "columnWidths" {
            let next = QString::from(sanitize_widths(&value).as_str());
            if self.column_widths != next {
                self.as_mut().set_column_widths(next);
                self.as_mut().persist_layout();
            }
            return;
        }

        let allowed: &[&str] = match key.as_str() {
            "viewMode" => VIEW_MODES,
            "density" => DENSITIES,
            "sortKey" => SORT_KEYS,
            "sortOrder" => SORT_ORDERS,
            "folderOrder" => FOLDER_ORDERS,
            "groupKey" => GROUP_KEYS,
            "groupDate" => GROUP_DATES,
            "iconStyle" => ICON_STYLES,
            "kindFilter" => KIND_FILTERS,
            "modifiedFilter" => MODIFIED_FILTERS,
            "sizeFilter" => SIZE_FILTERS,
            "sizeUnits" => SIZE_UNITS,
            _ => return,
        };
        if !allowed.contains(&value.as_str()) {
            return;
        }

        let changed = match key.as_str() {
            "viewMode" => {
                let next = QString::from(value.as_str());
                if self.view_mode == next {
                    return;
                }
                self.as_mut().set_view_mode(next);
                true
            }
            "density" => {
                let next = QString::from(value.as_str());
                if self.density == next {
                    return;
                }
                self.as_mut().set_density(next);
                true
            }
            "sortKey" => {
                let next = QString::from(value.as_str());
                if self.sort_key == next {
                    return;
                }
                self.as_mut().set_sort_key(next);
                true
            }
            "sortOrder" => {
                let next = QString::from(value.as_str());
                if self.sort_order == next {
                    return;
                }
                self.as_mut().set_sort_order(next);
                true
            }
            "folderOrder" => {
                let next = QString::from(value.as_str());
                if self.folder_order == next {
                    return;
                }
                self.as_mut().set_folder_order(next);
                true
            }
            "groupKey" => {
                let next = QString::from(value.as_str());
                if self.group_key == next {
                    return;
                }
                self.as_mut().set_group_key(next);
                true
            }
            "groupDate" => {
                let next = QString::from(value.as_str());
                if self.group_date == next {
                    return;
                }
                self.as_mut().set_group_date(next);
                true
            }
            "iconStyle" => {
                let next = QString::from(value.as_str());
                if self.icon_style == next {
                    return;
                }
                self.as_mut().set_icon_style(next);
                true
            }
            "kindFilter" => {
                let next = QString::from(value.as_str());
                if self.kind_filter == next {
                    return;
                }
                self.as_mut().set_kind_filter(next);
                true
            }
            "modifiedFilter" => {
                let next = QString::from(value.as_str());
                if self.modified_filter == next {
                    return;
                }
                self.as_mut().set_modified_filter(next);
                true
            }
            "sizeFilter" => {
                let next = QString::from(value.as_str());
                if self.size_filter == next {
                    return;
                }
                self.as_mut().set_size_filter(next);
                true
            }
            "sizeUnits" => {
                let next = QString::from(value.as_str());
                if self.size_units == next {
                    return;
                }
                self.as_mut().set_size_units(next);
                // Every subsequent `format::size` call follows the new unit.
                crate::format::set_size_units(size_units_of(&value));
                true
            }
            _ => false,
        };

        if changed {
            // Filters and the icon style are statements about the whole shell;
            // the layout keys follow the current directory when layouts are
            // per-directory.
            let layout_key = matches!(
                key.as_str(),
                "viewMode"
                    | "density"
                    | "sortKey"
                    | "sortOrder"
                    | "folderOrder"
                    | "groupKey"
                    | "groupDate"
            );
            if layout_key {
                self.as_mut().persist_layout();
            } else {
                save(&self.stored());
            }
        }
    }

    fn set_number(mut self: Pin<&mut Self>, key: &QString, value: i32) {
        let key = key.to_string();
        let changed = match key.as_str() {
            "previewWidth" => {
                // 260–420, the range the grip and its arrow keys share.
                let next = value.clamp(260, 420);
                if self.preview_width == next {
                    return;
                }
                self.as_mut().set_preview_width(next);
                true
            }
            "sidebarWidth" => {
                let next = value.clamp(180, 280);
                if self.sidebar_width == next {
                    return;
                }
                self.as_mut().set_sidebar_width(next);
                true
            }
            // The grid's slider and the list's thumbnails share one value.
            "iconSize" => {
                let next = value.clamp(24, 96);
                if self.icon_size == next {
                    return;
                }
                self.as_mut().set_icon_size(next);
                self.as_mut().persist_layout();
                return;
            }
            // The veil over the background image: 0 shows the picture, 90 is
            // nearly the plain canvas. Never 100, so "set" and "not set"
            // cannot look identical.
            "backgroundDim" => {
                let next = value.clamp(0, 90);
                if self.background_dim == next {
                    return;
                }
                self.as_mut().set_background_dim(next);
                true
            }
            _ => false,
        };

        if changed {
            save(&self.stored());
        }
    }

    fn set_bool(mut self: Pin<&mut Self>, key: &QString, value: bool) {
        let key = key.to_string();
        let changed = match key.as_str() {
            "rowThumbnails" => {
                if self.row_thumbnails == value {
                    return;
                }
                self.as_mut().set_row_thumbnails(value);
                true
            }
            "showExtensions" => {
                if self.show_extensions == value {
                    return;
                }
                self.as_mut().set_show_extensions(value);
                self.as_mut().persist_layout();
                return;
            }
            "showHiddenFiles" => {
                if self.show_hidden_files == value {
                    return;
                }
                self.as_mut().set_show_hidden_files(value);
                true
            }
            "reduceMotion" => {
                if self.reduce_motion == value {
                    return;
                }
                self.as_mut().set_reduce_motion(value);
                true
            }
            "splitEnabled" => {
                if self.split_enabled == value {
                    return;
                }
                self.as_mut().set_split_enabled(value);
                true
            }
            "syncLayout" => {
                if self.sync_layout == value {
                    return;
                }
                self.as_mut().set_sync_layout(value);
                // Turning sync on makes the global document the truth again;
                // turning it off starts the current directory from what is on
                // screen in the global defaults.
                if value {
                    save(&self.stored());
                } else if !self.current_path.as_slice().is_empty() {
                    // Start the directory from the layout on screen, so the
                    // next folder does not inherit a per-directory edit from
                    // the moment before the switch.
                    self.as_mut().persist_layout();
                }
                return;
            }
            "adaptiveLayout" => {
                if self.adaptive_layout == value {
                    return;
                }
                self.as_mut().set_adaptive_layout(value);
                true
            }
            "restoreSession" => {
                if self.restore_session == value {
                    return;
                }
                self.as_mut().set_restore_session(value);
                true
            }
            _ => false,
        };
        if changed {
            save(&self.stored());
        }
    }

    fn remember_search(mut self: Pin<&mut Self>, query: &QString) {
        let query = query.to_string().trim().to_owned();
        if query.is_empty() {
            return;
        }
        let mut entries: Vec<String> =
            serde_json::from_str(&self.search_history.to_string()).unwrap_or_default();
        entries.retain(|entry| entry != &query);
        entries.insert(0, query);
        entries.truncate(SEARCH_HISTORY_LIMIT);
        self.as_mut().set_search_history(QString::from(
            serde_json::to_string(&entries)
                .unwrap_or_else(|_| "[]".to_owned())
                .as_str(),
        ));
        save(&self.stored());
    }

    fn clear_search_history(mut self: Pin<&mut Self>) {
        if self.search_history.to_string() == "[]" {
            return;
        }
        self.as_mut().set_search_history(QString::from("[]"));
        save(&self.stored());
    }

    fn save_search(mut self: Pin<&mut Self>, name: &QString, spec: &QString) -> bool {
        let name = name.to_string().trim().to_owned();
        if name.is_empty() {
            return false;
        }
        let Ok(mut draft) = serde_json::from_str::<SavedSearch>(&spec.to_string()) else {
            return false;
        };
        if draft.query.trim().is_empty() {
            return false;
        }
        draft.query = draft.query.trim().to_owned();
        draft.name = name;

        let mut entries = decode_saved_searches(&self.saved_searches.to_string());
        match entries
            .iter()
            .position(|entry| entry.name.eq_ignore_ascii_case(&draft.name))
        {
            Some(index) => {
                // Keep the entry's identity: a sidebar row already drawn for
                // this name must keep pointing at the same search.
                if draft.id.is_empty() {
                    draft.id = entries[index].id.clone();
                }
                entries[index] = draft;
            }
            None => {
                if draft.id.is_empty() {
                    draft.id = next_saved_search_id(&entries);
                }
                entries.insert(0, draft);
                entries.truncate(SAVED_SEARCH_LIMIT);
            }
        }
        self.as_mut()
            .set_saved_searches(encode_saved_searches(&entries));
        save(&self.stored());
        true
    }

    fn delete_saved_search(mut self: Pin<&mut Self>, id: &QString) -> bool {
        let id = id.to_string();
        if id.is_empty() {
            return false;
        }
        let mut entries = decode_saved_searches(&self.saved_searches.to_string());
        let before = entries.len();
        entries.retain(|entry| entry.id != id);
        if entries.len() == before {
            return false;
        }
        self.as_mut()
            .set_saved_searches(encode_saved_searches(&entries));
        save(&self.stored());
        true
    }

    fn rename_saved_search(mut self: Pin<&mut Self>, id: &QString, name: &QString) -> bool {
        let id = id.to_string();
        let name = name.to_string().trim().to_owned();
        if id.is_empty() || name.is_empty() {
            return false;
        }
        let mut entries = decode_saved_searches(&self.saved_searches.to_string());
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        entry.name = name;
        self.as_mut()
            .set_saved_searches(encode_saved_searches(&entries));
        save(&self.stored());
        true
    }

    fn reset_filters(mut self: Pin<&mut Self>) {
        let mut changed = false;
        if self.kind_filter.to_string() != "all" {
            self.as_mut().set_kind_filter(QString::from("all"));
            changed = true;
        }
        if self.modified_filter.to_string() != "any" {
            self.as_mut().set_modified_filter(QString::from("any"));
            changed = true;
        }
        if self.size_filter.to_string() != "any" {
            self.as_mut().set_size_filter(QString::from("any"));
            changed = true;
        }
        if changed {
            save(&self.stored());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_document_reads_the_defaults() {
        let prefs = StoredPrefs::read(&PathBuf::from("definitely/missing.ini"));
        assert_eq!(prefs, StoredPrefs::default());
        assert_eq!(prefs.view_mode, "list");
        assert_eq!(prefs.folder_order, "foldersFirst");
        assert!(prefs.show_hidden_files);
        assert!(prefs.show_extensions);
        assert!(prefs.sync_layout);
        assert_eq!(prefs.icon_size, 38);
    }

    #[test]
    fn a_hand_edited_document_round_trips() {
        let mut prefs = StoredPrefs::default();
        prefs.apply_document(
            "viewMode=grid\nsortKey=size\nsortOrder=desc\nfolderOrder=mixed\n\
             groupKey=modified\ngroupDate=day\niconSize=40\nshowExtensions=false\n\
             showHiddenFiles=false\nkindFilter=images\nsizeFilter=small\n\
             columns=modified,tags\ncolumnWidths=140,200\n",
        );
        assert_eq!(prefs.view_mode, "grid");
        assert_eq!(prefs.sort_key, "size");
        assert_eq!(prefs.sort_order, "desc");
        assert_eq!(prefs.folder_order, "mixed");
        assert_eq!(prefs.group_key, "modified");
        assert_eq!(prefs.group_date, "day");
        assert_eq!(prefs.icon_size, 40);
        assert!(!prefs.show_extensions);
        assert!(!prefs.show_hidden_files);
        assert_eq!(prefs.kind_filter, "images");
        assert_eq!(prefs.size_filter, "small");
        assert_eq!(prefs.columns, "modified,tags", "canonical order");
        assert_eq!(prefs.column_widths, "140,200,142,142,120");

        let reparsed = {
            let mut next = StoredPrefs::default();
            next.apply_document(&prefs.document());
            next
        };
        assert_eq!(reparsed, prefs);
    }

    #[test]
    fn the_old_folder_boolean_migrates_to_the_enum() {
        let mut prefs = StoredPrefs::default();
        prefs.apply_document("foldersFirst=false\n");
        assert_eq!(prefs.folder_order, "mixed");

        // An explicit enum key wins over the stale boolean beside it.
        let mut prefs = StoredPrefs::default();
        prefs.apply_document("folderOrder=filesFirst\nfoldersFirst=false\n");
        assert_eq!(prefs.folder_order, "filesFirst");
    }

    #[test]
    fn the_configured_shell_round_trips() {
        let mut prefs = StoredPrefs::default();
        assert_eq!(prefs.terminal_shell, "", "empty means the platform default");

        prefs.apply_document("terminalShell=C:\\Program Files\\PowerShell\\7\\pwsh.exe\n");
        assert_eq!(
            prefs.terminal_shell,
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
        );

        let mut reparsed = StoredPrefs::default();
        reparsed.apply_document(&prefs.document());
        assert_eq!(reparsed.terminal_shell, prefs.terminal_shell);
    }

    #[test]
    fn unknown_keys_and_values_are_ignored() {
        let prefs = {
            let mut prefs = StoredPrefs::default();
            prefs.apply_document(
                "viewMode=carousel\nsortKey=colour\nfolderOrder=sideways\n\
                 groupKey=nonsense\niconSize=-5\nnonsense=1\nshowHiddenFiles=true\n",
            );
            prefs
        };
        assert_eq!(prefs.view_mode, "list", "an invalid enum keeps the default");
        assert_eq!(prefs.sort_key, "name");
        assert_eq!(prefs.folder_order, "foldersFirst");
        assert_eq!(prefs.group_key, "");
        assert_eq!(prefs.icon_size, -5, "normalisation clamps on the setters");
        assert!(prefs.show_hidden_files);
    }

    #[test]
    fn a_directory_view_overlays_only_its_own_keys() {
        let mut prefs = StoredPrefs::default();
        let view = StoredView {
            view_mode: Some("grid".to_owned()),
            sort_key: Some("created".to_owned()),
            group_key: Some("created".to_owned()),
            group_date: Some("year".to_owned()),
            icon_size: Some(48),
            show_extensions: Some(false),
            ..StoredView::default()
        };
        prefs.apply_view(&view);
        assert_eq!(prefs.view_mode, "grid");
        assert_eq!(prefs.sort_key, "created");
        assert_eq!(prefs.group_key, "created");
        assert_eq!(prefs.group_date, "year");
        assert_eq!(prefs.icon_size, 48);
        assert!(!prefs.show_extensions);
        assert_eq!(prefs.density, "comfortable", "absent keys keep the default");
        assert_eq!(prefs.sort_order, "asc");
    }

    #[test]
    fn a_stored_view_round_trips_through_toml() {
        let mut views = ViewsFile::default();
        views.views.insert(
            "c:/code/dae".to_owned(),
            StoredPrefs::default().view(1_700_000_000_000),
        );
        let document = toml::to_string_pretty(&views).expect("serialize");
        let parsed: ViewsFile = toml::from_str(&document).expect("parse");
        assert_eq!(parsed.views.len(), 1);
        let view = parsed.views.values().next().unwrap();
        assert_eq!(view.view_mode.as_deref(), Some("list"));
        assert_eq!(view.last_used, Some(1_700_000_000_000));
    }

    #[test]
    fn view_keys_fold_case_on_case_insensitive_platforms_and_trim_separators() {
        let key = view_key("C:/Code/Dae/");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(key, "c:/code/dae");
        } else {
            assert_eq!(key, "C:/Code/Dae");
        }
        assert_eq!(view_key("/a/b///"), "/a/b");
        assert_eq!(view_key("/"), "/");
    }

    #[test]
    fn column_lists_are_canonical_and_widths_are_clamped() {
        assert_eq!(sanitize_columns("tags,size,tags,bogus"), "size,tags");
        assert_eq!(sanitize_columns(""), "");
        assert_eq!(sanitize_widths("10,900,120"), "60,420,120,142,120");
        assert_eq!(sanitize_widths("nonsense"), "108,92,142,142,120");
    }

    #[test]
    fn the_adaptive_rule_is_explainable() {
        assert_eq!(suggest_view(10, 12, 1), "grid", "images dominate");
        assert_eq!(suggest_view(1, 12, 1), "list");
        assert_eq!(suggest_view(0, 2, 4), "column", "deep and sparse");
        assert_eq!(suggest_view(0, 2, 1), "list");
    }

    #[test]
    fn saved_searches_round_trip_through_the_document() {
        let mut prefs = StoredPrefs::default();
        prefs.apply_document(
            "savedSearches=[{\"id\":\"s1\",\"name\":\"Large videos\",\"query\":\"*.mp4\",\
             \"mode\":\"name\",\"scope\":\"volumes\",\"typeId\":\"media\",\"kind\":\"videos\",\
             \"modified\":\"month\",\"size\":\"large\",\"regex\":false,\
             \"caseSensitive\":false,\"filter\":\"*.mp4\"}]\n",
        );
        assert_eq!(prefs.saved_searches.len(), 1);
        assert_eq!(prefs.saved_searches[0].name, "Large videos");
        assert_eq!(prefs.saved_searches[0].type_id, "media");
        assert_eq!(prefs.saved_searches[0].scope, "volumes");

        let reparsed = {
            let mut next = StoredPrefs::default();
            next.apply_document(&prefs.document());
            next
        };
        assert_eq!(reparsed, prefs);
    }

    #[test]
    fn a_saved_search_without_a_query_or_name_is_dropped() {
        let mut prefs = StoredPrefs::default();
        prefs.apply_document(
            "savedSearches=[{\"id\":\"a\",\"name\":\"Empty\",\"query\":\"  \"},\
             {\"id\":\"b\",\"name\":\"\",\"query\":\"rs\"},\
             {\"id\":\"c\",\"name\":\"Kept\",\"query\":\"rs\"}]\n",
        );
        assert_eq!(prefs.saved_searches.len(), 1);
        assert_eq!(prefs.saved_searches[0].id, "c");
    }

    #[test]
    fn a_saved_search_spec_from_qml_defaults_the_missing_fields() {
        let search: SavedSearch = serde_json::from_str(
            "{\"query\":\"report\",\"mode\":\"content\",\"typeId\":\"doc\"}",
        )
        .expect("the spec is what the search bar can always send");
        assert_eq!(search.query, "report");
        assert_eq!(search.mode, "content");
        assert_eq!(search.type_id, "doc");
        assert!(search.scope.is_empty());
        assert!(!search.regex);
    }
}
