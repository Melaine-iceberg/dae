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
