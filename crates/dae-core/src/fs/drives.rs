//! Drive tools: eject, mount, unmount, and the system entry points for format
//! and cleanup.
//!
//! The rule from `docs/files-parity.md` §5.7 is that dae never formats a
//! volume itself. Formatting is a destructive, privileged, per-file-system
//! operation; the platform already ships a dialog for it, and three `mkfs`
//! wrappers would be three chances to get it wrong. This module therefore
//! exposes two kinds of function:
//!
//! * **Actions we perform** — eject, unmount, mount — through the platform's
//!   own tooling (`IOCTL_STORAGE_EJECT_MEDIA`, `udisksctl`, `diskutil`).
//! * **System entry points** — the format dialog and Storage Sense — where the
//!   only work is starting the right program.
//!
//! Every function returns the concrete failure rather than a silent no-op:
//! the menu hides what a platform cannot do, and what it can do but failed at
//! has to explain itself.
//!
//! The public functions stay one definition each; the platform bodies are
//! `*_native` items compiled on exactly one target, so an unsupported action
//! still exists as a callable that returns [`FileSystemError::Unsupported`].

use std::process::Command;

use crate::error::FileSystemError;

/// What the drive menu may offer for the running platform. The menu hides the
/// rest instead of showing a row that cannot work here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveCapabilities {
    pub eject: bool,
    pub unmount: bool,
    pub mount: bool,
    pub format: bool,
    pub cleanup: bool,
    /// A read-only encryption status (`BitLocker`, FileVault, LUKS).
    pub encryption: bool,
    /// Whether a previous-versions entry exists to open.
    pub previous_versions: bool,
}

pub fn capabilities() -> DriveCapabilities {
    if cfg!(windows) {
        DriveCapabilities {
            eject: true,
            // Windows mounts and unmounts volumes itself; there is no
            // user-facing unmount step to add.
            unmount: false,
            mount: false,
            format: true,
            cleanup: true,
            encryption: true,
            previous_versions: true,
        }
    } else if cfg!(target_os = "macos") {
        DriveCapabilities {
            eject: true,
            unmount: true,
            mount: true,
            format: true,
            cleanup: false,
            encryption: true,
            previous_versions: true,
        }
    } else {
        DriveCapabilities {
            eject: true,
            unmount: true,
            mount: true,
            // gnome-disks is the entry point; there is no portable dialog.
            format: which("gnome-disks").is_some(),
            cleanup: false,
            encryption: true,
            previous_versions: false,
        }
    }
}

/// Unmounts and then ejects a removable volume.
pub fn eject(mount_point: &str) -> Result<(), FileSystemError> {
    eject_native(mount_point)
}

/// Unmounts without ejecting the medium.
pub fn unmount(mount_point: &str) -> Result<(), FileSystemError> {
    unmount_native(mount_point)
}

/// Mounts a volume that is present but not mounted.
pub fn mount(mount_point: &str) -> Result<(), FileSystemError> {
    mount_native(mount_point)
}

/// Starts the platform's own format dialog. The drive letter is only
/// meaningful on Windows; the Unix entry points are not tied to a volume.
pub fn open_format_tool(mount_point: &str) -> Result<(), FileSystemError> {
    format_native(mount_point)
}

/// Opens the platform's cleanup/storage settings, where cleanup tools live.
/// Only Windows has a URL for it.
pub fn open_cleanup_tool() -> Result<(), FileSystemError> {
    cleanup_native()
}

/// A read-only sentence about disk encryption for `mount_point`, so the drive
/// menu can show it without offering to change it. Errors carry the tool's own
/// message: "manage-bde is not running as administrator" is more useful than
/// "unknown".
pub fn encryption_status(mount_point: &str) -> Result<String, FileSystemError> {
    encryption_native(mount_point)
}

/// Opens the platform's previous-versions UI. v1 shows the system entry only:
/// reading a VSS snapshot or an APFS local snapshot inside the app would need a
/// second file-system implementation for a rarely used recovery path.
pub fn open_previous_versions(mount_point: &str) -> Result<(), FileSystemError> {
    previous_versions_native(mount_point)
}

// ---------------------------------------------------------------- helpers

fn tool_error(tool: &str, error: FileSystemError) -> FileSystemError {
    match error {
        FileSystemError::Internal(message) | FileSystemError::InvalidInput(message) => {
            FileSystemError::Internal(format!("{tool} failed: {message}"))
        }
        other => other,
    }
}

/// Runs a system tool and returns its stdout. A missing program and a non-zero
/// exit are both errors with the tool's own text attached.
fn run(program: &str, args: &[&str]) -> Result<String, FileSystemError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| FileSystemError::Internal(format!("{program}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(FileSystemError::Internal(if detail.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            detail
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `PATH` lookup, used only to decide which tool to call. Windows reaches the
/// format dialog through the `windows` crate instead of a tool, so the helper
/// is only exercised by the tests there.
#[cfg_attr(windows, allow(dead_code))]
fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn unsupported(what: &str) -> FileSystemError {
    FileSystemError::Unsupported(format!("{what} is not supported on this platform"))
}

// ------------------------------------------------------------- Windows

#[cfg(windows)]
fn eject_native(mount_point: &str) -> Result<(), FileSystemError> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::{
        FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA,
    };
    use windows::core::HSTRING;

    let drive = mount_point.trim_end_matches(['\\', '/']);
    if !is_drive_letter(drive) {
        return Err(FileSystemError::InvalidInput(
            "Not a drive letter".to_owned(),
        ));
    }
    // `\\.\D:` addresses the volume rather than a directory on it.
    let path = HSTRING::from(format!("\\\\.\\{drive}"));
    let handle = unsafe {
        CreateFileW(
            &path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(|error| FileSystemError::Internal(format!("Could not open {drive}: {error}")))?;

    let ioctl = |code: u32| unsafe { DeviceIoControl(handle, code, None, 0, None, 0, None, None) };

    // Lock and dismount are best-effort: a failure here is usually "already
    // locked by another process", and the eject ioctl reports that better.
    let _ = ioctl(FSCTL_LOCK_VOLUME);
    let _ = ioctl(FSCTL_DISMOUNT_VOLUME);
    let result = ioctl(IOCTL_STORAGE_EJECT_MEDIA);
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.map_err(|error| FileSystemError::Internal(format!("Could not eject {drive}: {error}")))
}

#[cfg(windows)]
fn unmount_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Unmounting"))
}

#[cfg(windows)]
fn mount_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Mounting"))
}

#[cfg(windows)]
fn format_native(mount_point: &str) -> Result<(), FileSystemError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        SHFMT_CANCEL, SHFMT_ERROR, SHFMT_ID_DEFAULT, SHFMT_NOFORMAT, SHFMT_OPT_NONE, SHFormatDrive,
    };

    let drive = mount_point.trim_end_matches(['\\', '/']);
    if !is_drive_letter(drive) {
        return Err(FileSystemError::InvalidInput(
            "Not a drive letter".to_owned(),
        ));
    }
    let letter = drive.chars().next().unwrap_or('C');
    // SHFormatDrive's `drive` argument is 0 for A:, 1 for B:, … and it shows
    // the shell's own dialog, so the destructive part stays the user's click.
    let result = unsafe {
        SHFormatDrive(
            HWND(std::ptr::null_mut()),
            u32::from(letter.to_ascii_uppercase()) - u32::from('A'),
            SHFMT_ID_DEFAULT,
            SHFMT_OPT_NONE,
        )
    };
    if result == u32::MAX
        || result == SHFMT_ERROR.0
        || result == SHFMT_CANCEL.0
        || result == SHFMT_NOFORMAT.0
    {
        return Err(FileSystemError::Internal(
            "The format dialog reported an error".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn cleanup_native() -> Result<(), FileSystemError> {
    open_windows_uri("ms-settings:storagesense")
}

#[cfg(windows)]
fn encryption_native(mount_point: &str) -> Result<String, FileSystemError> {
    let drive = mount_point.trim_end_matches(['\\', '/']).to_owned();
    if !is_drive_letter(&drive) {
        return Err(FileSystemError::InvalidInput(
            "Not a drive letter".to_owned(),
        ));
    }
    run("manage-bde", &["-status", &drive, "-summary"])
        .map(|output| summarize_bde(&output))
        .map_err(|error| tool_error("manage-bde", error))
}

#[cfg(windows)]
fn previous_versions_native(mount_point: &str) -> Result<(), FileSystemError> {
    // The Properties sheet's Previous Versions tab is owned by the shell.
    run("rundll32", &["shell32.dll,Control_RunDLL", mount_point])
        .map(|_| ())
        .map_err(|error| tool_error("rundll32", error))
}

#[cfg(windows)]
fn open_windows_uri(uri: &str) -> Result<(), FileSystemError> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;

    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(uri),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value above 32 on success.
    if result.0 as isize <= 32 {
        return Err(FileSystemError::Internal(format!("Could not open {uri}")));
    }
    Ok(())
}

/// Exactly `X:`: the form the volume path and `SHFormatDrive` both take.
#[cfg(windows)]
fn is_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(windows)]
fn summarize_bde(output: &str) -> String {
    // `manage-bde -summary` prints "Conversion Status: Fully Decrypted" (or
    // "Fully Encrypted", "Encryption in Progress", …). Keep that line and the
    // protection status; the rest is dialogue about the volume.
    let mut lines = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Conversion Status")
            || trimmed.starts_with("Protection Status")
            || trimmed.starts_with("Percentage Encrypted")
        {
            lines.push(trimmed.to_owned());
        }
    }
    if lines.is_empty() {
        output.trim().to_owned()
    } else {
        lines.join("\n")
    }
}

// --------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn eject_native(mount_point: &str) -> Result<(), FileSystemError> {
    unmount_native(mount_point)?;
    let device = mount_device(mount_point).unwrap_or_else(|| mount_point.to_owned());
    if which("eject").is_some() {
        return run("eject", &[&device])
            .map(|_| ())
            .map_err(|error| tool_error("eject", error));
    }
    if which("gio").is_some() {
        return run("gio", &["mount", "-e", &device])
            .map(|_| ())
            .map_err(|error| tool_error("gio mount -e", error));
    }
    Err(FileSystemError::Unsupported(
        "No eject tool is installed (eject or gio)".to_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn unmount_native(mount_point: &str) -> Result<(), FileSystemError> {
    let device = mount_device(mount_point).ok_or_else(|| {
        FileSystemError::InvalidInput(format!("Could not find the device behind {mount_point}"))
    })?;
    if which("udisksctl").is_some() {
        return run("udisksctl", &["unmount", "-b", &device])
            .map(|_| ())
            .map_err(|error| tool_error("udisksctl unmount", error));
    }
    run("umount", &[mount_point])
        .map(|_| ())
        .map_err(|error| tool_error("umount", error))
}

#[cfg(target_os = "linux")]
fn mount_native(mount_point: &str) -> Result<(), FileSystemError> {
    let device = mount_device(mount_point).ok_or_else(|| {
        FileSystemError::InvalidInput(format!("Could not find the device behind {mount_point}"))
    })?;
    if which("udisksctl").is_some() {
        return run("udisksctl", &["mount", "-b", &device])
            .map(|_| ())
            .map_err(|error| tool_error("udisksctl mount", error));
    }
    run("mount", &[mount_point])
        .map(|_| ())
        .map_err(|error| tool_error("mount", error))
}

#[cfg(target_os = "linux")]
fn format_native(_mount_point: &str) -> Result<(), FileSystemError> {
    if which("gnome-disks").is_some() {
        return run("gnome-disks", &[])
            .map(|_| ())
            .map_err(|error| tool_error("gnome-disks", error));
    }
    Err(FileSystemError::Unsupported(
        "gnome-disks is not installed".to_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn cleanup_native() -> Result<(), FileSystemError> {
    Err(unsupported("Storage cleanup"))
}

#[cfg(target_os = "linux")]
fn encryption_native(mount_point: &str) -> Result<String, FileSystemError> {
    let device = mount_device(mount_point).unwrap_or_else(|| mount_point.to_owned());
    run("cryptsetup", &["status", &device])
        .map(|output| {
            output
                .lines()
                .find(|line| line.trim_start().starts_with("type:"))
                .map(|line| line.trim().to_owned())
                .unwrap_or_else(|| "Encrypted volume".to_owned())
        })
        .map_err(|error| tool_error("cryptsetup", error))
}

#[cfg(target_os = "linux")]
fn previous_versions_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Previous versions"))
}

#[cfg(target_os = "linux")]
fn mount_device(mount_point: &str) -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let wanted = mount_point.trim_end_matches('/');
    mounts.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let device = fields.next()?;
        let target = fields.next()?;
        (target == wanted).then(|| device.to_owned())
    })
}

// --------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn eject_native(mount_point: &str) -> Result<(), FileSystemError> {
    run("diskutil", &["eject", mount_point])
        .map(|_| ())
        .map_err(|error| tool_error("diskutil eject", error))
}

#[cfg(target_os = "macos")]
fn unmount_native(mount_point: &str) -> Result<(), FileSystemError> {
    run("diskutil", &["unmount", mount_point])
        .map(|_| ())
        .map_err(|error| tool_error("diskutil unmount", error))
}

#[cfg(target_os = "macos")]
fn mount_native(mount_point: &str) -> Result<(), FileSystemError> {
    let device = mount_device(mount_point).unwrap_or_else(|| mount_point.to_owned());
    run("diskutil", &["mount", &device])
        .map(|_| ())
        .map_err(|error| tool_error("diskutil mount", error))
}

#[cfg(target_os = "macos")]
fn format_native(_mount_point: &str) -> Result<(), FileSystemError> {
    run("open", &["-a", "Disk Utility"])
        .map(|_| ())
        .map_err(|error| tool_error("Disk Utility", error))
}

#[cfg(target_os = "macos")]
fn cleanup_native() -> Result<(), FileSystemError> {
    Err(unsupported("Storage cleanup"))
}

#[cfg(target_os = "macos")]
fn encryption_native(_mount_point: &str) -> Result<String, FileSystemError> {
    run("/usr/bin/fdesetup", &["status"])
        .map(|output| output.trim().to_owned())
        .map_err(|error| tool_error("fdesetup", error))
}

#[cfg(target_os = "macos")]
fn previous_versions_native(_mount_point: &str) -> Result<(), FileSystemError> {
    run("open", &["-a", "Time Machine"])
        .map(|_| ())
        .map_err(|error| tool_error("Time Machine", error))
}

#[cfg(target_os = "macos")]
fn mount_device(mount_point: &str) -> Option<String> {
    // `df -P` prints the device, size, used, available, capacity and mount
    // point on one line per file system.
    let output = run("/bin/df", &["-P", mount_point]).ok()?;
    output
        .lines()
        .nth(1)?
        .split_whitespace()
        .next()
        .map(|device| device.to_owned())
}

// ------------------------------------------------- every other platform

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn eject_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Eject"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn unmount_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Unmount"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn mount_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Mount"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn format_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Formatting"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn cleanup_native() -> Result<(), FileSystemError> {
    Err(unsupported("Storage cleanup"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn encryption_native(_mount_point: &str) -> Result<String, FileSystemError> {
    Err(unsupported("Disk encryption status"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn previous_versions_native(_mount_point: &str) -> Result<(), FileSystemError> {
    Err(unsupported("Previous versions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_table_matches_the_platform() {
        let capabilities = capabilities();
        assert!(capabilities.eject);
        if cfg!(windows) {
            // Windows does not expose unmount/mount as user actions.
            assert!(!capabilities.unmount && !capabilities.mount);
            assert!(capabilities.format && capabilities.cleanup);
        }
        if cfg!(target_os = "linux") {
            assert!(!capabilities.cleanup);
            assert!(!capabilities.previous_versions);
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_non_drive_path_is_rejected_before_starting_a_process() {
        let error = eject("C:/Users").expect_err("not a volume");
        assert!(matches!(error, FileSystemError::InvalidInput(_)));
    }

    #[test]
    fn tools_that_exist_are_found_by_which() {
        let program = if cfg!(windows) { "cmd.exe" } else { "sh" };
        assert!(which(program).is_some(), "{program} should be on PATH");
        assert!(which("dae-definitely-not-a-tool").is_none());
    }
}
