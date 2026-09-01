//! Platform-aware file properties for the properties dialog.
//!
//! The permission model differs per OS: POSIX systems expose mode bits and
//! ownership (`chmod`/`chown`), Windows exposes DOS attribute flags
//! (`SetFileAttributesW`). Both are folded into the cross-platform
//! [`FileProperties`] model declared in `types.rs`.

use super::directory::entry_kind;
use crate::file_system::error::FileSystemError;
use crate::file_system::progress::FileOperationProgressReporterTrait;
use crate::file_system::types::{
    EntryKind, FileProperties, PlatformProperties, PropertyChanges,
    RecursivePropertyUpdateOutcome, display_name_from_path, path_to_string,
};
use std::fs;
use std::path::Path;

#[cfg(unix)]
use crate::file_system::types::UnixProperties;
#[cfg(windows)]
use crate::file_system::types::WindowsProperties;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAGS_AND_ATTRIBUTES, SetFileAttributesW,
};

pub fn read_properties(path: &Path) -> Result<FileProperties, FileSystemError> {
    let metadata = fs::symlink_metadata(path)?;
    let kind = entry_kind(metadata.file_type());
    let target = (kind == EntryKind::Symlink)
        .then(|| fs::read_link(path).ok())
        .flatten()
        .map(|target| target.to_string_lossy().into_owned());

    // std's chmod/chown follow symlinks, so symlinks report the target's
    // metadata for size, timestamps, and permissions; a dangling link falls
    // back to the link itself.
    let detail = if kind == EntryKind::Symlink {
        fs::metadata(path).unwrap_or(metadata)
    } else {
        metadata
    };

    Ok(FileProperties {
        path: path_to_string(path),
        name: display_name_from_path(&path.to_string_lossy()),
        kind,
        size: matches!(kind, EntryKind::File | EntryKind::Symlink).then(|| detail.len()),
        created_at: timestamp_millis(detail.created()),
        modified_at: timestamp_millis(detail.modified()),
        accessed_at: timestamp_millis(detail.accessed()),
        target,
        platform: platform_properties(&detail),
    })
}

pub fn update_properties(path: &Path, changes: &PropertyChanges) -> Result<(), FileSystemError> {
    #[cfg(unix)]
    {
        update_unix(path, changes)?;
    }

    #[cfg(windows)]
    {
        update_windows(path, changes)?;
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, changes);
        return Err(FileSystemError::Unsupported(
            "Properties editing is not available on this platform".into(),
        ));
    }

    Ok(())
}

/// Applies `changes` to `root` and everything beneath it ("apply to enclosed
/// items"). Symlinks are skipped — chmod/chown and `SetFileAttributesW` all
/// follow links, so touching them would modify targets outside the tree.
/// Entries that cannot be updated are counted as failures and the walk moves
/// on; the result distinguishes the two so the UI can report partial success.
pub fn apply_properties_recursive(
    root: &Path,
    changes: &PropertyChanges,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<RecursivePropertyUpdateOutcome, FileSystemError> {
    let plan = PropertyPlan::new(changes)?;
    let mut outcome = RecursivePropertyUpdateOutcome { updated: 0, failed: 0 };
    if plan.is_empty() {
        return Ok(outcome);
    }

    let total = count_walkable_entries(root)?;
    progress.start(total);

    process_entry(root, &plan, progress, &mut outcome);

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        // Entries under an unreadable directory were never included in
        // `total`, so there is nothing to advance for them either.
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    outcome.failed += 1;
                    progress.advance(&path);
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }

            process_entry(&path, &plan, progress, &mut outcome);
            if file_type.is_dir() {
                stack.push(path);
            }
        }
    }

    if outcome.updated == 0 && outcome.failed > 0 {
        return Err(FileSystemError::Io(format!(
            "Failed to update properties on {} items",
            outcome.failed
        )));
    }

    Ok(outcome)
}

/// Counts the entries a recursive walk reaches: `root` itself plus every
/// non-symlink beneath it. Unreadable directories stop the descent but their
/// unreadable-ness is only discovered when applying, mirroring that walk.
fn count_walkable_entries(root: &Path) -> Result<u64, FileSystemError> {
    let mut total = 1u64;
    let mut is_root = true;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if is_root => return Err(error.into()),
            Err(_) => continue,
        };
        is_root = false;

        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    total += 1;
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }

            total += 1;
            if file_type.is_dir() {
                stack.push(entry.path());
            }
        }
    }

    Ok(total)
}

fn process_entry(
    path: &Path,
    plan: &PropertyPlan,
    progress: &dyn FileOperationProgressReporterTrait,
    outcome: &mut RecursivePropertyUpdateOutcome,
) {
    progress.begin_entry(path);
    match plan.apply(path) {
        Ok(()) => outcome.updated += 1,
        Err(_) => outcome.failed += 1,
    }
    progress.advance(path);
}

/// The resolved platform operations from a [`PropertyChanges`], applied
/// verbatim to each target entry. Owner names resolve once up front so a
/// recursive walk does not repeat NSS lookups per entry.
#[cfg(unix)]
struct PropertyPlan {
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
}

#[cfg(unix)]
impl PropertyPlan {
    fn new(changes: &PropertyChanges) -> Result<Self, FileSystemError> {
        let (uid, gid) = match &changes.owner {
            Some(owner) => (
                owner
                    .user
                    .as_deref()
                    .map(|user| resolve_account_id(user, true))
                    .transpose()?,
                owner
                    .group
                    .as_deref()
                    .map(|group| resolve_account_id(group, false))
                    .transpose()?,
            ),
            None => (None, None),
        };

        Ok(Self {
            mode: changes.mode,
            uid,
            gid,
        })
    }

    fn is_empty(&self) -> bool {
        self.mode.is_none() && self.uid.is_none() && self.gid.is_none()
    }

    fn apply(&self, path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        if let Some(mode) = self.mode {
            // Preserve the file-type bits; replace only the 12 permission bits.
            let permissions = fs::symlink_metadata(path)?.permissions();
            let full_mode = (permissions.mode() & !0o7777) | (mode & 0o7777);
            fs::set_permissions(path, fs::Permissions::from_mode(full_mode))?;
        }

        if self.uid.is_some() || self.gid.is_some() {
            std::os::unix::fs::chown(path, self.uid, self.gid)?;
        }

        Ok(())
    }
}

#[cfg(windows)]
struct PropertyPlan {
    set_mask: u32,
    clear_mask: u32,
}

#[cfg(windows)]
impl PropertyPlan {
    fn new(changes: &PropertyChanges) -> Result<Self, FileSystemError> {
        let mut plan = Self {
            set_mask: 0,
            clear_mask: 0,
        };

        for (value, flag) in [
            (changes.read_only, FILE_ATTRIBUTE_READONLY.0),
            (changes.hidden, FILE_ATTRIBUTE_HIDDEN.0),
            (changes.archive, FILE_ATTRIBUTE_ARCHIVE.0),
            (changes.system, FILE_ATTRIBUTE_SYSTEM.0),
        ] {
            match value {
                Some(true) => plan.set_mask |= flag,
                Some(false) => plan.clear_mask |= flag,
                None => {}
            }
        }

        Ok(plan)
    }

    fn is_empty(&self) -> bool {
        self.set_mask == 0 && self.clear_mask == 0
    }

    fn apply_to_attributes(&self, attributes: u32) -> u32 {
        (attributes & !self.clear_mask) | self.set_mask
    }

    fn apply(&self, path: &Path) -> std::io::Result<()> {
        use std::os::windows::fs::MetadataExt;

        let attributes = fs::symlink_metadata(path)?.file_attributes();
        let flags = self.apply_to_attributes(attributes);
        if flags == attributes {
            return Ok(());
        }

        set_file_attributes(path, flags)
    }
}

#[cfg(not(any(unix, windows)))]
struct PropertyPlan;

#[cfg(not(any(unix, windows)))]
impl PropertyPlan {
    fn new(_changes: &PropertyChanges) -> Result<Self, FileSystemError> {
        Err(FileSystemError::Unsupported(
            "Properties editing is not available on this platform".into(),
        ))
    }

    fn is_empty(&self) -> bool {
        true
    }

    fn apply(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

fn timestamp_millis(time: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    time.ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

#[cfg(unix)]
fn platform_properties(detail: &fs::Metadata) -> PlatformProperties {
    use std::os::unix::fs::MetadataExt;

    let uid = detail.uid();
    let gid = detail.gid();

    PlatformProperties::Unix(UnixProperties {
        mode: detail.mode(),
        uid,
        gid,
        user_name: accounts::user_name_by_uid(uid),
        group_name: accounts::group_name_by_gid(gid),
    })
}

#[cfg(windows)]
fn platform_properties(detail: &fs::Metadata) -> PlatformProperties {
    use std::os::windows::fs::MetadataExt;

    let attributes = detail.file_attributes();

    PlatformProperties::Windows(WindowsProperties {
        read_only: attributes & FILE_ATTRIBUTE_READONLY.0 != 0,
        hidden: attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
        archive: attributes & FILE_ATTRIBUTE_ARCHIVE.0 != 0,
        system: attributes & FILE_ATTRIBUTE_SYSTEM.0 != 0,
    })
}

#[cfg(unix)]
fn update_unix(path: &Path, changes: &PropertyChanges) -> Result<(), FileSystemError> {
    let plan = PropertyPlan::new(changes)?;
    if plan.is_empty() {
        return Ok(());
    }
    plan.apply(path).map_err(FileSystemError::from)
}

/// Resolves an owner field that accepts either a numeric id or an account
/// name, so the UI can offer a free-form input.
#[cfg(unix)]
fn resolve_account_id(value: &str, is_user: bool) -> Result<u32, FileSystemError> {
    if let Ok(id) = value.parse::<u32>() {
        return Ok(id);
    }

    let resolved = if is_user {
        accounts::uid_by_name(value)
    } else {
        accounts::gid_by_name(value)
    };

    resolved.ok_or_else(|| {
        let kind = if is_user { "user" } else { "group" };
        FileSystemError::InvalidInput(format!("Unknown {kind} account: {value}"))
    })
}

#[cfg(windows)]
fn update_windows(path: &Path, changes: &PropertyChanges) -> Result<(), FileSystemError> {
    use std::os::windows::fs::MetadataExt;

    let plan = PropertyPlan::new(changes)?;
    if plan.is_empty() {
        return Ok(());
    }

    let attributes = fs::symlink_metadata(path)?.file_attributes();
    let flags = plan.apply_to_attributes(attributes);
    if flags == attributes {
        return Ok(());
    }

    set_file_attributes(path, flags).map_err(FileSystemError::from)
}

/// Raw Win32 attribute write; unlike `fs::set_permissions` it can toggle the
/// hidden/archive/system flags, not just read-only.
#[cfg(windows)]
fn set_file_attributes(path: &Path, flags: u32) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // SAFETY: `wide_path` is nul-terminated and stays alive for the call.
    unsafe {
        SetFileAttributesW(PCWSTR(wide_path.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(flags)).map_err(
            |error| {
                // HRESULT 0x80070005 wraps Win32 ERROR_ACCESS_DENIED.
                let kind = if error.code().0 == -2147024891 {
                    std::io::ErrorKind::PermissionDenied
                } else {
                    std::io::ErrorKind::Other
                };
                std::io::Error::new(kind, error.to_string())
            },
        )
    }
}

/// NSS-aware account lookups. Names resolve through whatever the system is
/// configured with (`/etc/passwd`, SSSD, macOS Open Directory, ...), so this
/// stays correct on domain-joined machines where parsing local files fails.
#[cfg(unix)]
mod accounts {
    use std::ffi::{CStr, CString};

    const INITIAL_BUFFER_SIZE: usize = 1024;

    fn cstr_to_string(pointer: *const libc::c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }

        // SAFETY: libc guarantees the pointer outlives the call and points
        // to a valid nul-terminated string.
        Some(unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned())
    }

    pub fn user_name_by_uid(uid: u32) -> Option<String> {
        let mut buffer = vec![0 as libc::c_char; INITIAL_BUFFER_SIZE];

        loop {
            // SAFETY: all pointers reference live, correctly-sized storage.
            let (code, result) = unsafe {
                let mut passwd: libc::passwd = std::mem::zeroed();
                let mut result: *mut libc::passwd = std::ptr::null_mut();
                let code = libc::getpwuid_r(
                    uid,
                    &mut passwd,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                );
                (code, result)
            };

            match code {
                0 => return cstr_to_string(result.as_ref()?.pw_name),
                libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
                _ => return None,
            }
        }
    }

    pub fn uid_by_name(name: &str) -> Option<u32> {
        let name = CString::new(name).ok()?;
        let mut buffer = vec![0 as libc::c_char; INITIAL_BUFFER_SIZE];

        loop {
            // SAFETY: `name` is nul-terminated; other pointers reference
            // live, correctly-sized storage.
            let (code, result) = unsafe {
                let mut passwd: libc::passwd = std::mem::zeroed();
                let mut result: *mut libc::passwd = std::ptr::null_mut();
                let code = libc::getpwnam_r(
                    name.as_ptr(),
                    &mut passwd,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                );
                (code, result)
            };

            match code {
                0 => return result.as_ref().map(|passwd| passwd.pw_uid),
                libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
                _ => return None,
            }
        }
    }

    pub fn group_name_by_gid(gid: u32) -> Option<String> {
        let mut buffer = vec![0 as libc::c_char; INITIAL_BUFFER_SIZE];

        loop {
            // SAFETY: all pointers reference live, correctly-sized storage.
            let (code, result) = unsafe {
                let mut group: libc::group = std::mem::zeroed();
                let mut result: *mut libc::group = std::ptr::null_mut();
                let code = libc::getgrgid_r(
                    gid,
                    &mut group,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                );
                (code, result)
            };

            match code {
                0 => return cstr_to_string(result.as_ref()?.gr_name),
                libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
                _ => return None,
            }
        }
    }

    pub fn gid_by_name(name: &str) -> Option<u32> {
        let name = CString::new(name).ok()?;
        let mut buffer = vec![0 as libc::c_char; INITIAL_BUFFER_SIZE];

        loop {
            // SAFETY: `name` is nul-terminated; other pointers reference
            // live, correctly-sized storage.
            let (code, result) = unsafe {
                let mut group: libc::group = std::mem::zeroed();
                let mut result: *mut libc::group = std::ptr::null_mut();
                let code = libc::getgrnam_r(
                    name.as_ptr(),
                    &mut group,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                );
                (code, result)
            };

            match code {
                0 => return result.as_ref().map(|group| group.gr_gid),
                libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::test_support::TestProgress;
    use std::sync::atomic::Ordering as AtomicOrdering;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dae-properties-{tag}-{}", std::process::id()))
    }

    #[cfg(windows)]
    fn windows_flags(properties: &FileProperties) -> &WindowsProperties {
        match &properties.platform {
            PlatformProperties::Windows(windows) => windows,
            other => panic!("expected Windows platform properties, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn toggles_archive_and_system_attributes() {
        let directory = temp_dir("attribute-toggle");
        let file = directory.join("flagged.txt");
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(&file, "flagged").expect("create test file");

        update_properties(
            &file,
            &PropertyChanges {
                archive: Some(true),
                system: Some(true),
                ..Default::default()
            },
        )
        .expect("set archive and system");
        let flagged = read_properties(&file).expect("read flagged properties");
        assert!(windows_flags(&flagged).archive);
        assert!(windows_flags(&flagged).system);

        update_properties(
            &file,
            &PropertyChanges {
                archive: Some(false),
                system: Some(false),
                ..Default::default()
            },
        )
        .expect("clear archive and system");
        let cleared = read_properties(&file).expect("read cleared properties");
        assert!(!windows_flags(&cleared).archive);
        assert!(!windows_flags(&cleared).system);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(windows)]
    #[test]
    fn applies_attributes_recursively() {
        let directory = temp_dir("recursive-attributes");
        let nested = directory.join("nested");
        let top_file = directory.join("top.txt");
        let nested_file = nested.join("deep.txt");
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(&top_file, "top").expect("create top file");
        fs::write(&nested_file, "deep").expect("create nested file");

        let progress = TestProgress::new();
        let outcome = apply_properties_recursive(
            &directory,
            &PropertyChanges {
                hidden: Some(true),
                ..Default::default()
            },
            &progress,
        )
        .expect("recursive hidden apply");

        // Root + top file + nested directory + nested file.
        assert_eq!(outcome.updated, 4);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            progress.completed.load(AtomicOrdering::Relaxed),
            progress.total.load(AtomicOrdering::Relaxed)
        );

        for path in [&directory, &top_file, &nested, &nested_file] {
            let properties = read_properties(path).expect("read properties");
            assert!(windows_flags(&properties).hidden, "{path:?} stayed visible");
        }

        let cleared = apply_properties_recursive(
            &directory,
            &PropertyChanges {
                hidden: Some(false),
                ..Default::default()
            },
            &TestProgress::new(),
        )
        .expect("recursive hidden clear");
        assert_eq!(cleared.failed, 0);
        for path in [&directory, &top_file, &nested, &nested_file] {
            let properties = read_properties(path).expect("read properties");
            assert!(!windows_flags(&properties).hidden, "{path:?} stayed hidden");
        }

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn applies_mode_recursively() {
        use std::os::unix::fs::MetadataExt;

        let directory = temp_dir("recursive-mode");
        let nested = directory.join("nested");
        let top_file = directory.join("top.txt");
        let nested_file = nested.join("deep.txt");
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(&top_file, "top").expect("create top file");
        fs::write(&nested_file, "deep").expect("create nested file");

        let progress = TestProgress::new();
        let outcome = apply_properties_recursive(
            &directory,
            &PropertyChanges {
                mode: Some(0o640),
                ..Default::default()
            },
            &progress,
        )
        .expect("recursive mode apply");

        assert_eq!(outcome.updated, 4);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            progress.completed.load(AtomicOrdering::Relaxed),
            progress.total.load(AtomicOrdering::Relaxed)
        );

        for path in [&directory, &top_file, &nested, &nested_file] {
            let mode = fs::metadata(path).expect("read metadata").mode() & 0o7777;
            assert_eq!(mode, 0o640, "{path:?} kept mode {mode:o}");
        }

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    /// The walk must not follow symlinks: chmod/chown and SetFileAttributesW
    /// both dereference links, so applying through one would rewrite whatever
    /// the link points at — potentially outside the tree.
    #[cfg(unix)]
    #[test]
    fn recursive_apply_skips_symlinked_entries() {
        use std::os::unix::fs::MetadataExt;

        let outside_dir = temp_dir("recursive-symlink-outside");
        let directory = temp_dir("recursive-symlink-tree");
        let outside_file = outside_dir.join("outside.txt");
        fs::create_dir_all(&outside_dir).expect("create outside directory");
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(&outside_file, "outside").expect("create outside file");
        std::os::unix::fs::symlink(&outside_file, directory.join("link.txt"))
            .expect("create symlink");

        let outside_mode_before = fs::metadata(&outside_file).expect("read outside metadata");

        let outcome = apply_properties_recursive(
            &directory,
            &PropertyChanges {
                mode: Some(0o600),
                ..Default::default()
            },
            &TestProgress::new(),
        )
        .expect("recursive mode apply");

        // Only the root directory itself; the symlink is skipped entirely.
        assert_eq!(outcome.updated, 1);
        assert_eq!(outcome.failed, 0);

        let outside_mode_after = fs::metadata(&outside_file).expect("read outside metadata");
        assert_eq!(
            outside_mode_before.mode() & 0o7777,
            outside_mode_after.mode() & 0o7777,
            "the symlink target outside the tree was modified"
        );

        fs::remove_dir_all(directory).expect("remove test directory");
        fs::remove_dir_all(outside_dir).expect("remove outside directory");
    }
}
