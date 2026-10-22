//! Windows recycle bin scan.
//!
//! `$Recycle.Bin\<SID>\` holds one `$I<id>` metadata file per deleted item,
//! whose fixed header carries the payload size, the deletion time, and the
//! absolute path the item had before deletion, next to a `$R<id>` payload
//! holding the data. Reading those directories is a plain filesystem walk and
//! yields ids the shell-backed restore/purge commands still resolve.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::types::path_to_string;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
use windows::core::PCWSTR;

use super::TrashEntry;

/// `$I<id>` header: format version, payload size, then the deletion time.
const METADATA_HEADER_LEN: usize = 24;

/// Version 2 writes the original path's length (UTF-16 code units, terminator
/// included) between the header and the path. Version 1 has no such field.
const METADATA_PATH_LENGTH_LEN: usize = 4;

/// `GetDriveTypeW` results worth walking: fixed disks and removable media.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// Every entry in every local recycle bin, unsorted.
///
/// Failures are per volume and per entry so one bad item cannot hide the rest
/// of the trash. A bin that exists but cannot be opened at all is reported
/// instead of being presented as an empty trash.
pub(super) fn scan() -> Result<Vec<TrashEntry>, FileSystemError> {
    let mut entries = Vec::new();
    let mut bin_found = false;
    let mut bin_opened = false;

    for volume in local_volumes() {
        let Some(bin_root) = find_child(&volume, "$Recycle.Bin") else {
            continue;
        };
        bin_found = true;
        let Ok(sid_dirs) = fs::read_dir(&bin_root) else {
            continue;
        };
        bin_opened = true;

        for sid_dir in sid_dirs.flatten() {
            if !sid_dir.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            collect_bin(&sid_dir.path(), &mut entries);
        }
    }

    if bin_found && !bin_opened {
        return Err(FileSystemError::Internal("fs.trash_unavailable".into()));
    }
    Ok(entries)
}

/// Collects one `$Recycle.Bin\<SID>` directory into `out`.
fn collect_bin(sid_dir: &Path, out: &mut Vec<TrashEntry>) {
    let Ok(entries) = fs::read_dir(sid_dir) else {
        return;
    };

    // A deleted item is a `$I<id>` metadata file plus a `$R<id>` payload that
    // share the id, so the whole directory is read before anything is decoded.
    let mut payloads: HashMap<String, Payload> = HashMap::new();
    let mut metadata_files: Vec<(String, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let rendered = file_name.to_string_lossy();
        let Some(tag) = rendered.get(..2) else {
            continue;
        };
        let id = rendered[2..].to_ascii_lowercase();

        if tag.eq_ignore_ascii_case("$I") {
            metadata_files.push((id, entry.path()));
        } else if tag.eq_ignore_ascii_case("$R") {
            payloads.insert(
                id,
                Payload {
                    path: sid_dir.join(&file_name),
                    is_directory: entry.file_type().is_ok_and(|kind| kind.is_dir()),
                },
            );
        }
    }

    for (id, metadata_path) in metadata_files {
        let Ok(bytes) = fs::read(&metadata_path) else {
            continue;
        };
        let Some(metadata) = decode_metadata(&bytes) else {
            continue;
        };
        let Some(payload) = payloads.get(&id) else {
            continue;
        };

        out.push(TrashEntry {
            // The shell hands back the payload's own path as the trash id, so
            // ids built the same way still match in `take_trash_items`.
            id: payload.path.to_string_lossy().into_owned(),
            name: metadata.name,
            original_parent: metadata.original_parent,
            time_deleted: metadata.time_deleted,
            is_directory: payload.is_directory,
            size_bytes: (!payload.is_directory).then_some(metadata.size_bytes),
        });
    }
}

struct Payload {
    path: PathBuf,
    is_directory: bool,
}

struct Metadata {
    size_bytes: u64,
    time_deleted: i64,
    name: String,
    original_parent: String,
}

/// Decodes a `$I<id>` file: payload size, deletion time, and the absolute path
/// the item had before it was deleted.
fn decode_metadata(bytes: &[u8]) -> Option<Metadata> {
    let header = bytes.get(..METADATA_HEADER_LEN)?;
    let version = u64::from_le_bytes(header[0..8].try_into().ok()?);
    let size_bytes = u64::from_le_bytes(header[8..16].try_into().ok()?);
    let deleted = u64::from_le_bytes(header[16..24].try_into().ok()?);

    let length_prefix = if version >= 2 {
        METADATA_PATH_LENGTH_LEN
    } else {
        0
    };
    let path_bytes = bytes.get(METADATA_HEADER_LEN + length_prefix..)?;
    let units: Vec<u16> = path_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    if units.is_empty() {
        return None;
    }

    // The path is UTF-16 and NTFS allows unpaired surrogates, so it is kept as
    // an `OsString` instead of going through `String`.
    let original = PathBuf::from(OsString::from_wide(&units));

    Some(Metadata {
        size_bytes,
        time_deleted: filetime_to_unix(deleted),
        name: original.file_name()?.to_string_lossy().into_owned(),
        original_parent: original.parent().map(path_to_string).unwrap_or_default(),
    })
}

/// Windows FILETIME (100 ns ticks since 1601-01-01) to unix seconds.
fn filetime_to_unix(filetime: u64) -> i64 {
    const EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    const TICKS_PER_SECOND: u64 = 10_000_000;

    if filetime < EPOCH_AS_FILETIME {
        return 0;
    }
    ((filetime - EPOCH_AS_FILETIME) / TICKS_PER_SECOND) as i64
}

/// Drive roots worth scanning, from the logical-drive bitmap.
fn local_volumes() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    let mut volumes = Vec::new();

    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let root = format!("{}:\\", char::from(b'A' + index as u8));
        let mut wide: Vec<u16> = root.encode_utf16().collect();
        wide.push(0);

        let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
        if kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE {
            volumes.push(PathBuf::from(root));
        }
    }
    volumes
}

/// Case-insensitive child lookup that keeps the directory's on-disk casing.
fn find_child(parent: &Path, wanted: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(wanted)
        {
            return Some(entry.path());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filetime(unix_seconds: u64) -> u64 {
        116_444_736_000_000_000 + unix_seconds * 10_000_000
    }

    fn version_2_file(size_bytes: u64, deleted: u64, original: &str) -> Vec<u8> {
        let units: Vec<u16> = original.encode_utf16().chain(std::iter::once(0)).collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u64.to_le_bytes());
        bytes.extend_from_slice(&size_bytes.to_le_bytes());
        bytes.extend_from_slice(&deleted.to_le_bytes());
        bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_a_version_2_metadata_file() {
        let bytes = version_2_file(320, filetime(1_600_000_000), r"K:\code\dae\dist\razor.svg");
        let metadata = decode_metadata(&bytes).expect("decoded");
        assert_eq!(metadata.size_bytes, 320);
        assert_eq!(metadata.time_deleted, 1_600_000_000);
        assert_eq!(metadata.name, "razor.svg");
        assert_eq!(metadata.original_parent, r"K:\code\dae\dist");
    }

    #[test]
    fn decodes_a_version_1_metadata_file() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        bytes.extend_from_slice(&filetime(42).to_le_bytes());
        for unit in r"C:\tmp\notes.txt".encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }

        let metadata = decode_metadata(&bytes).expect("decoded");
        assert_eq!(metadata.size_bytes, 7);
        assert_eq!(metadata.time_deleted, 42);
        assert_eq!(metadata.name, "notes.txt");
        assert_eq!(metadata.original_parent, r"C:\tmp");
    }

    #[test]
    fn rejects_metadata_files_without_a_path() {
        assert!(decode_metadata(&[]).is_none());
        assert!(decode_metadata(&[0u8; METADATA_HEADER_LEN]).is_none());

        let mut truncated = version_2_file(1, 0, r"C:\a.txt");
        truncated.truncate(METADATA_HEADER_LEN + METADATA_PATH_LENGTH_LEN);
        assert!(decode_metadata(&truncated).is_none());
    }

    #[test]
    fn converts_filetime_to_unix_seconds() {
        assert_eq!(filetime_to_unix(0), 0);
        assert_eq!(filetime_to_unix(filetime(0)), 0);
        assert_eq!(filetime_to_unix(filetime(1_600_000_000)), 1_600_000_000);
    }
}
