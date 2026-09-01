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
    /// OS hidden marker (DOS attribute on Windows, dot prefix on Unix-like
    /// systems). Views dim such entries and hide them entirely when the
    /// show-hidden-files preference is off.
    pub hidden: bool,
    /// Read-only marker. Directories report the raw flag too; the UI only
    /// badges files because the DOS READONLY bit on folders is vestigial.
    pub read_only: bool,
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
    /// See [`DirectoryEntry::hidden`]; search rows reuse the same badges.
    pub hidden: bool,
    /// See [`DirectoryEntry::read_only`].
    pub read_only: bool,
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

/// Cross-platform file properties for the properties dialog.
///
/// The common fields every backend can report live here; the
/// platform-specific layer (POSIX mode bits, Windows DOS attributes) rides
/// in `platform` as a tagged union so the frontend renders exactly the
/// section the current backend supports.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileProperties {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Milliseconds since the Unix epoch; `None` when the file system
    /// cannot report the timestamp (e.g. birth time on Linux).
    pub created_at: Option<u64>,
    pub modified_at: Option<u64>,
    pub accessed_at: Option<u64>,
    /// Destination of a symbolic link, when `kind` is `Symlink`.
    pub target: Option<String>,
    pub platform: PlatformProperties,
}

/// The platform-specific section of [`FileProperties`].
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformProperties {
    /// POSIX mode bits and ownership (Linux, macOS).
    Unix(UnixProperties),
    /// DOS attribute flags (Windows local disks).
    Windows(WindowsProperties),
    /// Backends without a permission model (SMB today): view-only.
    Basic,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnixProperties {
    /// Permission bits including the file-type mask (e.g. `0o100644`).
    /// Keep only the low 12 bits (`mode & 0o7777`) when editing.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// Resolved account names; `None` when the account no longer exists.
    pub user_name: Option<String>,
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WindowsProperties {
    pub read_only: bool,
    pub hidden: bool,
    pub archive: bool,
    pub system: bool,
}

/// A field-set of property edits. `None` means "leave unchanged", so one
/// command can chmod, chown, or toggle DOS attributes independently.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PropertyChanges {
    /// POSIX `chmod`: the low 12 bits (setuid/setgid/sticky included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// POSIX `chown`: account names or numeric ids; each side optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<OwnerChange>,
    /// Windows `SetFileAttributes`: read-only flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// Windows `SetFileAttributes`: hidden flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Windows `SetFileAttributes`: archive flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<bool>,
    /// Windows `SetFileAttributes`: system flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<bool>,
}

/// Summary of a recursive property update over a directory tree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecursivePropertyUpdateOutcome {
    /// Entries whose properties were applied successfully (no-ops included).
    pub updated: u64,
    /// Entries that could not be updated; the walk continued past them.
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnerChange {
    /// User name or numeric uid; `None` keeps the current owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Group name or numeric gid; `None` keeps the current group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl FileProperties {
    /// Builds the view-only variant from the backend-neutral [`EntryStat`],
    /// used by backends without a permission model.
    pub fn basic(path: &str, stat: EntryStat) -> Self {
        Self {
            path: path.to_owned(),
            name: display_name_from_path(path),
            kind: stat.kind,
            size: Some(stat.size),
            created_at: None,
            modified_at: stat.modified_at,
            accessed_at: None,
            target: None,
            platform: PlatformProperties::Basic,
        }
    }
}

/// Backend-neutral metadata used by the generic transfer engine.
#[derive(Debug, Clone)]
pub struct EntryStat {
    pub kind: EntryKind,
    pub size: u64,
    /// Milliseconds since the Unix epoch; `None` when the backend cannot
    /// report it. Surfaced by the conflict dialog to compare both sides.
    pub modified_at: Option<u64>,
}

/// How a transfer resolves an existing entry at the destination.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    /// Fails with `AlreadyExists` (the pre-dialog behavior).
    #[default]
    Fail,
    /// Leaves the destination untouched and transfers nothing for this source.
    Skip,
    /// Deletes the existing destination entry, then transfers.
    Replace,
    /// Transfers under a generated "副本" name, keeping both entries.
    KeepBoth,
}

/// One source entry paired with its conflict resolution.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferItem {
    pub path: String,
    pub on_conflict: ConflictAction,
}

/// One requested rename in a batch: the entry's current full path paired
/// with the bare new name it should receive (never a path).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameRequest {
    pub path: String,
    pub new_name: String,
}

/// One executed (source, destination) pair from a copy or move batch, as
/// reported by the transfer engines' journal. Destinations reflect what
/// actually landed on disk, "副本" auto-renames included. Internal to the
/// backend: the undo/redo history is built from these pairs.
#[derive(Debug, Clone)]
pub struct TransferPair {
    pub source: String,
    pub destination: String,
}

/// A name collision detected before a transfer runs, with both sides'
/// metadata so the conflict dialog can compare them.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferConflict {
    pub source_path: String,
    pub target_path: String,
    pub name: String,
    pub source_kind: EntryKind,
    pub source_size: Option<u64>,
    pub source_modified_at: Option<u64>,
    pub target_kind: EntryKind,
    pub target_size: Option<u64>,
    pub target_modified_at: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn removes_windows_verbatim_path_prefixes() {
        assert_eq!(
            normalize_path_for_display(r"\\?\C:\Users\test"),
            r"C:\Users\test"
        );
        assert_eq!(
            normalize_path_for_display(r"\\?\UNC\server\share\folder"),
            r"\\server\share\folder"
        );
    }

    #[test]
    fn places_directories_before_files_case_insensitively() {
        let mut entries = [
            DirectoryEntry {
                name: "zeta.txt".into(),
                path: "zeta.txt".into(),
                kind: EntryKind::File,
                modified_at: None,
                size: None,
                hidden: false,
                read_only: false,
            },
            DirectoryEntry {
                name: "alpha".into(),
                path: "alpha".into(),
                kind: EntryKind::Directory,
                modified_at: None,
                size: None,
                hidden: false,
                read_only: false,
            },
            DirectoryEntry {
                name: "Beta".into(),
                path: "Beta".into(),
                kind: EntryKind::Directory,
                modified_at: None,
                size: None,
                hidden: false,
                read_only: false,
            },
        ];

        entries.sort_by_cached_key(entry_sort_key);

        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "Beta");
        assert_eq!(entries[2].name, "zeta.txt");
    }

    #[test]
    fn display_name_from_path_handles_separators_and_roots() {
        assert_eq!(display_name_from_path(r"C:\Users\alice\docs"), "docs");
        assert_eq!(display_name_from_path("/home/alice/pictures/"), "pictures");
        assert_eq!(display_name_from_path(r"\wsl$\Ubuntu"), "Ubuntu");
        assert_eq!(display_name_from_path("smb://nas.local/media"), "media");
        // Trailing separators are ignored, so drive roots keep their letter and
        // connection roots show their host.
        assert_eq!(display_name_from_path(r"C:\"), "C:");
        assert_eq!(display_name_from_path("smb://nas.local"), "nas.local");
        assert_eq!(display_name_from_path("/"), "/");
    }
}
