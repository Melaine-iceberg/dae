//! Platform-aware file properties for the properties dialog.
//!
//! The permission model differs per OS: POSIX systems expose mode bits and
//! ownership (`chmod`/`chown`), Windows exposes DOS attribute flags
//! (`SetFileAttributesW`). Both are folded into the cross-platform
//! [`FileProperties`] model declared in `types.rs`.

use super::directory::entry_kind;
use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    EntryKind, FileProperties, PlatformProperties, PropertyChanges, display_name_from_path,
    path_to_string,
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
    use std::os::unix::fs::PermissionsExt;

    if let Some(mode) = changes.mode {
        // Preserve the file-type bits; replace only the 12 permission bits.
        let permissions = fs::metadata(path)?.permissions();
        let full_mode = (permissions.mode() & !0o7777) | (mode & 0o7777);
        fs::set_permissions(path, fs::Permissions::from_mode(full_mode))?;
    }

    if let Some(owner) = &changes.owner {
        let uid = match &owner.user {
            Some(user) => Some(resolve_account_id(user, true)?),
            None => None,
        };
        let gid = match &owner.group {
            Some(group) => Some(resolve_account_id(group, false)?),
            None => None,
        };

        if uid.is_some() || gid.is_some() {
            std::os::unix::fs::chown(path, uid, gid)?;
        }
    }

    Ok(())
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
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;

    if changes.read_only.is_none() && changes.hidden.is_none() {
        return Ok(());
    }

    let attributes = fs::symlink_metadata(path)?.file_attributes();
    let mut flags = attributes;

    if let Some(read_only) = changes.read_only {
        flags = if read_only {
            flags | FILE_ATTRIBUTE_READONLY.0
        } else {
            flags & !FILE_ATTRIBUTE_READONLY.0
        };
    }
    if let Some(hidden) = changes.hidden {
        flags = if hidden {
            flags | FILE_ATTRIBUTE_HIDDEN.0
        } else {
            flags & !FILE_ATTRIBUTE_HIDDEN.0
        };
    }

    if flags == attributes {
        return Ok(());
    }

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // SAFETY: `wide_path` is nul-terminated and stays alive for the call.
    unsafe {
        SetFileAttributesW(
            PCWSTR(wide_path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(flags),
        )
        .map_err(|error| FileSystemError::Io(error.to_string()))?;
    }

    Ok(())
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
