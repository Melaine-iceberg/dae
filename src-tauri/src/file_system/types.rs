use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

// Backend-agnostic data types exchanged with the frontend. Every storage
// backend produces these, so they must stay free of protocol-specific types.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Breadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    /// The absolute, canonical path that was read.
    pub path: String,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum NewEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntry {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub kind: EntryKind,
    pub modified_at: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub entries: Vec<SearchEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchMatch {
    pub line_number: u64,
    pub line_text: String,
    /// Zero-based character ranges of the matches within `line_text`.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchFile {
    pub path: String,
    pub relative_path: String,
    pub matches: Vec<ContentSearchMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchResponse {
    pub files: Vec<ContentSearchFile>,
    pub truncated: bool,
}

/// Backend-neutral metadata used by the generic transfer engine.
#[derive(Debug, Clone)]
pub struct EntryStat {
    pub kind: EntryKind,
    pub size: u64,
}

pub fn entry_kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::Symlink => 1,
        EntryKind::File => 2,
        EntryKind::Other => 3,
    }
}

pub fn entry_sort_key(entry: &DirectoryEntry) -> (u8, String, String) {
    (
        entry_kind_rank(&entry.kind),
        entry.name.to_lowercase(),
        entry.name.clone(),
    )
}

pub fn path_to_string(path: &Path) -> String {
    normalize_path_for_display(&path.to_string_lossy())
}

/// Derives a display name from the last path segment, handling both `/` and
/// `\\` separators so it works for local, UNC, and `smb://` style paths.
/// Trailing separators are ignored (`C:\` becomes `C:`); only when nothing
/// remains (e.g. `/`) does it fall back to the full path.
pub fn display_name_from_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let separator_index = trimmed
        .rfind(['/', '\\'])
        .map(|index| index + 1)
        .unwrap_or(0);
    let name = &trimmed[separator_index..];

    if name.is_empty() {
        path.to_string()
    } else {
        name.to_string()
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn normalize_path_for_display(path: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }

        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    }

    #[cfg(not(windows))]
    path.to_owned()
}
