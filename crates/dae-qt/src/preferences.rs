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

const VIEW_MODES: &[&str] = &["list", "grid", "column"];
const DENSITIES: &[&str] = &["compact", "comfortable", "spacious"];
const SORT_KEYS: &[&str] = &["name", "modified", "type", "size"];
const SORT_ORDERS: &[&str] = &["asc", "desc"];
const ICON_STYLES: &[&str] = &["themed", "system"];
const KIND_FILTERS: &[&str] = &["all", "folders", "files", "images"];
const MODIFIED_FILTERS: &[&str] = &["any", "today", "week", "month"];
const SIZE_FILTERS: &[&str] = &["any", "small", "medium", "large"];

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
        #[qproperty(bool, folders_first, cxx_name = "foldersFirst")]
        #[qproperty(QString, icon_style, cxx_name = "iconStyle")]
        #[qproperty(bool, row_thumbnails, cxx_name = "rowThumbnails")]
        #[qproperty(bool, show_hidden_files, cxx_name = "showHiddenFiles")]
        #[qproperty(QString, kind_filter, cxx_name = "kindFilter")]
        #[qproperty(QString, modified_filter, cxx_name = "modifiedFilter")]
        #[qproperty(QString, size_filter, cxx_name = "sizeFilter")]
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

        /// Kinds, modified-time buckets and size buckets back to "all/any/any".
        #[qinvokable]
        #[cxx_name = "resetFilters"]
        fn reset_filters(self: Pin<&mut ExplorerPrefs>);
    }
}

use std::pin::Pin;

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredPrefs {
    view_mode: String,
    density: String,
    sort_key: String,
    sort_order: String,
    folders_first: bool,
    icon_style: String,
    row_thumbnails: bool,
    show_hidden_files: bool,
    kind_filter: String,
    modified_filter: String,
    size_filter: String,
    split_enabled: bool,
    reduce_motion: bool,
    preview_width: i32,
    sidebar_width: i32,
    terminal_shell: String,
}

impl Default for StoredPrefs {
    fn default() -> Self {
        Self {
            view_mode: "list".to_owned(),
            density: "comfortable".to_owned(),
            sort_key: "name".to_owned(),
            sort_order: "asc".to_owned(),
            folders_first: true,
            // The shell draws as one object: arc, type and state all in the same
            // ink, with colour reserved for the signal and the git badges. A
            // column of full-colour shell icons broke that on the first screen.
            // The desktop's own icons stay one menu click away.
            icon_style: "themed".to_owned(),
            row_thumbnails: false,
            // The app has always listed hidden entries; the toggle is for
            // people who want the classic hidden-by-default reading.
            show_hidden_files: true,
            kind_filter: "all".to_owned(),
            modified_filter: "any".to_owned(),
            size_filter: "any".to_owned(),
            split_enabled: false,
            reduce_motion: false,
            preview_width: 300,
            sidebar_width: 216,
            terminal_shell: String::new(),
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
        let boolean = |key: &str, target: &mut bool| {
            match fields.get(key).copied() {
                Some("true") => *target = true,
                Some("false") => *target = false,
                _ => {}
            }
        };

        string("viewMode", VIEW_MODES, &mut self.view_mode);
        string("density", DENSITIES, &mut self.density);
        string("sortKey", SORT_KEYS, &mut self.sort_key);
        string("sortOrder", SORT_ORDERS, &mut self.sort_order);
        boolean("foldersFirst", &mut self.folders_first);
        string("iconStyle", ICON_STYLES, &mut self.icon_style);
        boolean("rowThumbnails", &mut self.row_thumbnails);
        boolean("showHiddenFiles", &mut self.show_hidden_files);
        string("kindFilter", KIND_FILTERS, &mut self.kind_filter);
        string("modifiedFilter", MODIFIED_FILTERS, &mut self.modified_filter);
        string("sizeFilter", SIZE_FILTERS, &mut self.size_filter);
        boolean("splitEnabled", &mut self.split_enabled);
        boolean("reduceMotion", &mut self.reduce_motion);
        number("previewWidth", &mut self.preview_width);
        number("sidebarWidth", &mut self.sidebar_width);
        // Free text, unlike the enumerated preferences above: any executable
        // name or path is valid, and an empty value means "the platform guess".
        if let Some(value) = fields.get("terminalShell") {
            self.terminal_shell = (*value).to_owned();
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
        write(&mut out, "foldersFirst", bool_text(self.folders_first));
        write(&mut out, "iconStyle", &self.icon_style);
        write(&mut out, "rowThumbnails", bool_text(self.row_thumbnails));
        write(
            &mut out,
            "showHiddenFiles",
            bool_text(self.show_hidden_files),
        );
        write(&mut out, "kindFilter", &self.kind_filter);
        write(&mut out, "modifiedFilter", &self.modified_filter);
        write(&mut out, "sizeFilter", &self.size_filter);
        write(&mut out, "splitEnabled", bool_text(self.split_enabled));
        write(&mut out, "reduceMotion", bool_text(self.reduce_motion));
        write(&mut out, "previewWidth", &self.preview_width.to_string());
        write(&mut out, "sidebarWidth", &self.sidebar_width.to_string());
        write(&mut out, "terminalShell", &self.terminal_shell);
        out
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
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
        eprintln!("dae: could not create the preferences directory: {error}");
        return;
    }
    if let Err(error) = std::fs::write(&path, prefs.document()) {
        eprintln!("dae: could not save explorer preferences: {error}");
    }
}

pub struct ExplorerPrefsRust {
    view_mode: QString,
    density: QString,
    sort_key: QString,
    sort_order: QString,
    folders_first: bool,
    icon_style: QString,
    row_thumbnails: bool,
    show_hidden_files: bool,
    kind_filter: QString,
    modified_filter: QString,
    size_filter: QString,
    split_enabled: bool,
    reduce_motion: bool,
    preview_width: i32,
    sidebar_width: i32,
    terminal_shell: QString,
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
            folders_first: prefs.folders_first,
            icon_style: QString::from(prefs.icon_style.as_str()),
            row_thumbnails: prefs.row_thumbnails,
            show_hidden_files: prefs.show_hidden_files,
            kind_filter: QString::from(prefs.kind_filter.as_str()),
            modified_filter: QString::from(prefs.modified_filter.as_str()),
            size_filter: QString::from(prefs.size_filter.as_str()),
            split_enabled: prefs.split_enabled,
            reduce_motion: prefs.reduce_motion,
            preview_width: prefs.preview_width,
            sidebar_width: prefs.sidebar_width,
            terminal_shell: QString::from(prefs.terminal_shell.as_str()),
        }
    }

    fn stored(&self) -> StoredPrefs {
        StoredPrefs {
            view_mode: self.view_mode.to_string(),
            density: self.density.to_string(),
            sort_key: self.sort_key.to_string(),
            sort_order: self.sort_order.to_string(),
            folders_first: self.folders_first,
            icon_style: self.icon_style.to_string(),
            row_thumbnails: self.row_thumbnails,
            show_hidden_files: self.show_hidden_files,
            kind_filter: self.kind_filter.to_string(),
            modified_filter: self.modified_filter.to_string(),
            size_filter: self.size_filter.to_string(),
            split_enabled: self.split_enabled,
            reduce_motion: self.reduce_motion,
            preview_width: self.preview_width,
            sidebar_width: self.sidebar_width,
            terminal_shell: self.terminal_shell.to_string(),
        }
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

        let allowed: &[&str] = match key.as_str() {
            "viewMode" => VIEW_MODES,
            "density" => DENSITIES,
            "sortKey" => SORT_KEYS,
            "sortOrder" => SORT_ORDERS,
            "iconStyle" => ICON_STYLES,
            "kindFilter" => KIND_FILTERS,
            "modifiedFilter" => MODIFIED_FILTERS,
            "sizeFilter" => SIZE_FILTERS,
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
            _ => false,
        };

        if changed {
            save(&self.stored());
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
            _ => false,
        };

        if changed {
            save(&self.stored());
        }
    }

    fn set_bool(mut self: Pin<&mut Self>, key: &QString, value: bool) {
        let key = key.to_string();
        let changed = match key.as_str() {
            "foldersFirst" => {
                if self.folders_first == value {
                    return;
                }
                self.as_mut().set_folders_first(value);
                true
            }
            "rowThumbnails" => {
                if self.row_thumbnails == value {
                    return;
                }
                self.as_mut().set_row_thumbnails(value);
                true
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
            _ => false,
        };
        if changed {
            save(&self.stored());
        }
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
        assert!(prefs.folders_first);
        assert!(prefs.show_hidden_files);
    }

    #[test]
    fn a_hand_edited_document_round_trips() {
        let mut prefs = StoredPrefs::default();
        prefs.apply_document(
            "viewMode=grid\nsortKey=size\nsortOrder=desc\nfoldersFirst=false\n\
             showHiddenFiles=false\nkindFilter=images\nsizeFilter=small\n",
        );
        assert_eq!(prefs.view_mode, "grid");
        assert_eq!(prefs.sort_key, "size");
        assert_eq!(prefs.sort_order, "desc");
        assert!(!prefs.folders_first);
        assert!(!prefs.show_hidden_files);
        assert_eq!(prefs.kind_filter, "images");
        assert_eq!(prefs.size_filter, "small");

        let reparsed = {
            let mut next = StoredPrefs::default();
            next.apply_document(&prefs.document());
            next
        };
        assert_eq!(reparsed, prefs);
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
                "viewMode=carousel\nsortKey=colour\nfoldersFirst=maybe\n\
                 nonsense=1\nshowHiddenFiles=true\n",
            );
            prefs
        };
        assert_eq!(prefs.view_mode, "list", "an invalid enum keeps the default");
        assert_eq!(prefs.sort_key, "name");
        assert!(prefs.folders_first, "an invalid bool keeps the default");
        assert!(prefs.show_hidden_files);
    }
}
