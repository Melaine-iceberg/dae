//! Direct reader for the Windows recycle bin.
//!
//! The `trash` crate lists the recycle bin through the shell: enumerating it
//! costs ~0.3 ms per entry, and the per-entry size lookup `list_trash` used to
//! layer on top re-creates a shell item and opens its property store for
//! another ~1.6 ms — 32 s for a 17,900-entry bin, all of it inside the blocking
//! command the trash view waits on before it can paint anything.
//!
//! `$Recycle.Bin\<SID>\` holds one `$I<id>` file per deleted item, whose fixed
//! header carries the payload size, the deletion time, and the absolute path
//! the item had before it was deleted, next to a `$R<id>` payload holding the
//! data itself. Reading those directories is a plain filesystem walk: measured
//! at 0.84 s for the same 17,945 entries, with byte-identical ids and identical
//! name / original-location / deletion-time fields, so entries found here still
//! round-trip through the shell-backed restore and purge commands.
//!
//! Restoring and purging deliberately stay on the `trash` crate: those are
//! user-initiated, confirmed, and progress-reported, so the shell's cost is
//! paid once per operation instead of on every visit to the view.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
use windows::core::PCWSTR;

use super::TrashEntry;
use super::super::types::path_to_string;

/// `$I<id>` header: format version, payload size, then the deletion time.
const METADATA_HEADER_LEN: usize = 24;

/// Version 2 writes the original path's length (UTF-16 code units, terminator
/// included) between the header and the path. Version 1 has no such field.
const METADATA_PATH_LENGTH_LEN: usize = 4;

/// `GetDriveTypeW` results worth walking: fixed disks and removable media.
/// Remote drives answer to a server instead of a local `$Recycle.Bin`, and
/// optical drives are usually empty, so both would cost a failed round trip
/// on every visit. See the `DRIVE_*` return values of `GetDriveTypeW`.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// Every entry in every local recycle bin, newest deletion first, unsorted
/// aside from the caller's sort.
///
/// Failures are per volume and per entry: an unreadable bin or a `$I` file
/// that cannot be decoded is skipped so one bad item cannot hide the rest of
/// the trash — the same best-effort contract the shell-backed listing had. A
/// bin that exists but cannot be opened at all is reported instead of being
/// presented as an empty trash.
pub(super) fn scan() -> Result<Vec<TrashEntry>, super::super::error::FileSystemError> {
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

        // Each SID directory is one user's bin. Enumerating all of them matches
        // what the shell's recycle-bin folder reports, measured field-for-field
        // against `trash::os_limited::list()`.
        for sid_dir in sid_dirs.flatten() {
            if !sid_dir.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            collect_bin(&sid_dir.path(), &mut entries);
        }
    }

    if bin_found && !bin_opened {
        return Err(super::super::error::FileSystemError::Internal(
            "fs.trash_unavailable".into(),
        ));
    }
    Ok(entries)
}

/// Collects one `$Recycle.Bin\<SID>` directory into `out`.
fn collect_bin(sid_dir: &Path, out: &mut Vec<TrashEntry>) {
    let Ok(entries) = fs::read_dir(sid_dir) else {
        return;
    };

    // A deleted item is a `$I<id>` metadata file plus a `$R<id>` payload that
    // share the id, and the ids only pair up once both halves are known, so
    // the whole directory is read before anything is decoded. `desktop.ini`
    // and any other stray file is not an item and is ignored.
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
        // Without its payload there is nothing left to restore or purge, and
        // no id the shell would recognise.
        let Some(payload) = payloads.get(&id) else {
            continue;
        };

        out.push(TrashEntry {
            // The shell hands back the payload's own path as the trash id, so
            // ids built the same way still match in `take_trash_items`. It is
            // deliberately not run through `path_to_string`: the shell-side id
            // is not normalized either.
            id: payload.path.to_string_lossy().into_owned(),
            name: metadata.name,
            original_parent: metadata.original_parent,
            time_deleted: metadata.time_deleted,
            is_directory: payload.is_directory,
            size_bytes: (!payload.is_directory).then_some(metadata.size_bytes),
        });
    }
}

/// A `$R<id>` payload: where it sits and whether it is a directory.
struct Payload {
    path: PathBuf,
    is_directory: bool,
}

/// What a `$I<id>` file says about the item it was split off from.
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
        .chunks_exact(2)
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
        // An entry with no parent is still listed, showing an unknown original
        // location — the same shape the shell-backed listing produced.
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

/// Drive roots worth scanning, from the logical-drive bitmap. The bitmap is
/// read without touching any device, so an unreachable mapped drive cannot
/// stall the scan.
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

/// Case-insensitive child lookup that keeps the directory's on-disk casing, so
/// ids built from the result compare equal to the shell's.
fn find_child(parent: &Path, wanted: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(wanted) {
            return Some(entry.path());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Unix seconds as a FILETIME.
    fn filetime(unix_seconds: u64) -> u64 {
        116_444_736_000_000_000 + unix_seconds * 10_000_000
    }

    /// The bytes of a version 2 `$I` file.
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
        // Version 1 writes the path straight after the header, with no length.
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

    /// The restore, purge, and empty commands resolve their ids through the
    /// shell, so every id the direct scan reports has to be one the shell knows.
    /// Ignored by default because it walks the entire bin through the shell,
    /// which takes tens of seconds on a large one.
    #[test]
    #[ignore = "enumerates the whole recycle bin through the shell"]
    fn agrees_with_the_shell_listing() {
        let direct = scan().expect("direct scan");
        let shell = trash::os_limited::list().expect("shell listing");

        let known: HashSet<String> = shell
            .iter()
            .map(|item| item.id.to_string_lossy().into_owned())
            .collect();
        for entry in &direct {
            assert!(
                known.contains(&entry.id),
                "id the shell would not resolve: {}",
                entry.id
            );
        }

        println!("direct={} shell={}", direct.len(), shell.len());
        // Coverage rather than equality: the two walks cannot be atomic, so a
        // concurrent delete can shift the bin between them.
        assert!(
            direct.len() * 10 >= shell.len() * 9,
            "direct scan covered {} of {} entries",
            direct.len(),
            shell.len()
        );
    }
}
