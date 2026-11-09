//! Running one file operation with administrator rights.
//!
//! A file manager hits `Access Denied` on paths the user legitimately owns but
//! the process does not: `Program Files`, `System32`, another profile's
//! `Documents`. The alternative to "there is nothing I can do" is to hand that
//! one operation to an elevated copy of this binary:
//!
//! 1. The shell writes a **job** — the sources, the destination, the conflict
//!    policy — into `{config}/elevated/`.
//! 2. It launches *itself* with `--elevated-job <path>`, which is a `runas`
//!    launch on Windows, `pkexec` on Linux, and `osascript … with administrator
//!    privileges` on macOS.
//! 3. [`run_from_args`] answers in that child *before* Qt is initialised: it
//!    performs the transfer with the same engine, writes a result file, and
//!    exits.
//!
//! Nothing else in the app is elevated. The window, the listings, the terminal
//! and the index stay at the user's own token, which is the whole point: an
//! elevated *app* would leave elevated handles and tokens lying around, and one
//! elevated *operation* leaves nothing behind but the bytes it moved.
//!
//! # Why the result is a file
//!
//! The child has no channel back to the parent's event loop — that is what
//! elevation costs. A result file next to the job is the smallest thing that
//! survives a crash of either process, and the parent polls it by waiting on the
//! child process, so a result that is missing means the child died.

use super::transfer::{self, TransferSource};
use super::types::{TransferItem, TransferPair};
use super::vfs;
use crate::error::FileSystemError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The flag the elevated child is launched with.
pub const JOB_FLAG: &str = "--elevated-job";

/// One operation, exactly as the shell asked for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevatedJob {
    /// `"copy"`, `"move"` or `"delete"`.
    pub kind: String,
    pub sources: Vec<TransferItem>,
    /// The target directory for a copy or a move; `None` for a delete.
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Runs the job, launches the elevated child, and waits for it.
///
/// Blocking: the caller runs it on a worker thread and shows an indeterminate
/// progress row, because an elevated process cannot report progress back into
/// this one's event loop.
pub fn run(job: ElevatedJob) -> Result<(), FileSystemError> {
    let directory = job_directory()?;
    fs::create_dir_all(&directory)?;

    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0)
    );
    let job_path = directory.join(format!("{id}.json"));
    let result_path = directory.join(format!("{id}.result.json"));

    let contents = serde_json::to_vec_pretty(&job)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    fs::write(&job_path, contents)?;

    let outcome = launch(&job_path).and_then(|()| read_result(&result_path));

    // The job names paths the user selected; it has no reason to stay on disk.
    let _ = fs::remove_file(&job_path);
    let _ = fs::remove_file(&result_path);

    outcome
}

/// Where jobs and results live. Private to this user, which is also the check
/// [`run_from_args`] makes before it trusts a job it was handed.
pub fn job_directory() -> Result<PathBuf, FileSystemError> {
    Ok(crate::paths::config_dir()?.join("elevated"))
}

/// Answers `Some(exit_code)` when this process was started *as* an elevated
/// child, and `None` for every ordinary launch.
///
/// Called from `main` before Qt exists: an elevated child never creates a
/// window, never takes the single-instance lock, and never loads QML.
pub fn run_from_args() -> Option<i32> {
    let mut arguments = std::env::args().skip(1);
    let mut job_path: Option<PathBuf> = None;

    while let Some(argument) = arguments.next() {
        if argument == JOB_FLAG {
            job_path = arguments.next().map(PathBuf::from);
            break;
        }
        if let Some(rest) = argument.strip_prefix(&format!("{JOB_FLAG}=")) {
            job_path = Some(PathBuf::from(rest));
            break;
        }
    }

    let job_path = job_path?;

    let result_path = job_path.with_extension("result.json");
    let outcome = execute_job_file(&job_path);

    let payload = match &outcome {
        Ok(()) => JobResult {
            ok: true,
            error: None,
        },
        Err(error) => JobResult {
            ok: false,
            error: Some(error.to_string()),
        },
    };

    if let Ok(contents) = serde_json::to_vec_pretty(&payload) {
        let _ = fs::write(&result_path, contents);
    }

    Some(match outcome {
        Ok(()) => 0,
        Err(_) => 1,
    })
}

fn execute_job_file(path: &Path) -> Result<(), FileSystemError> {
    // An elevated process must not be talked into running an arbitrary job
    // file: only the directory this user's own shell writes to is accepted.
    if !is_own_job(path) {
        return Err(FileSystemError::InvalidInput(format!(
            "Refusing a job outside the dae directory: {}",
            path.display()
        )));
    }

    let contents = fs::read(path)?;
    let job: ElevatedJob = serde_json::from_slice(&contents)
        .map_err(|error| FileSystemError::InvalidInput(error.to_string()))?;

    execute(&job)
}

/// True when `path` sits directly inside [`job_directory`].
fn is_own_job(path: &Path) -> bool {
    let Ok(directory) = job_directory() else {
        return false;
    };

    match (path.parent(), directory.canonicalize()) {
        (Some(parent), Ok(directory)) => parent.canonicalize().is_ok_and(|parent| parent == directory),
        _ => false,
    }
}

/// The transfer itself, with no window and no progress.
fn execute(job: &ElevatedJob) -> Result<(), FileSystemError> {
    let sources = job
        .sources
        .iter()
        .map(|item| {
            Ok(TransferSource {
                path: item.path.clone(),
                backend: vfs::resolve(&item.path)?,
                on_conflict: item.on_conflict,
            })
        })
        .collect::<Result<Vec<_>, FileSystemError>>()?;

    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "The elevated job carries no sources".to_owned(),
        ));
    }

    let progress = SilentProgress;
    let mut journal: Vec<TransferPair> = Vec::new();

    match job.kind.as_str() {
        "delete" => transfer::delete_entries(sources, &progress),
        "copy" | "move" => {
            let destination = job.destination.clone().ok_or_else(|| {
                FileSystemError::InvalidInput("The elevated job has no destination".to_owned())
            })?;
            let backend = vfs::resolve(&destination)?;

            if job.kind == "move" {
                transfer::move_entries(sources, &destination, &backend, &progress, &mut journal)
            } else {
                transfer::copy_entries(sources, &destination, &backend, &progress, &mut journal)
            }
        }
        other => Err(FileSystemError::InvalidInput(format!(
            "Unknown elevated job kind: {other}"
        ))),
    }
}

/// A reporter for a process with no UI to report to.
///
/// Every method is empty on purpose: the transfer engine only needs the seam to
/// exist, and `control()` is left at its `None` default because an elevated
/// child is not pausable from the window that launched it.
struct SilentProgress;

impl super::progress::FileOperationProgressReporterTrait for SilentProgress {
    fn start(&self, _total: u64) {}
    fn begin_entry(&self, _path: &Path) {}
    fn advance_by(&self, _units: u64, _path: &Path) {}
    fn finish(&self) {}
}

fn read_result(path: &Path) -> Result<(), FileSystemError> {
    let contents = fs::read(path).map_err(|error| {
        // The child exited without writing a result: either the user dismissed
        // the elevation prompt or the child died before it could report.
        FileSystemError::Internal(format!("The elevated operation did not report back: {error}"))
    })?;

    let result: JobResult = serde_json::from_slice(&contents)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    match (result.ok, result.error) {
        (true, _) => Ok(()),
        (false, Some(error)) => Err(FileSystemError::Internal(error)),
        (false, None) => Err(FileSystemError::Internal(
            "The elevated operation failed".to_owned(),
        )),
    }
}

#[cfg(windows)]
fn launch(job_path: &Path) -> Result<(), FileSystemError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let executable = std::env::current_exe()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let mut file: Vec<u16> = executable.as_os_str().encode_wide().collect();
    file.push(0);

    let parameters = format!("{JOB_FLAG} \"{}\"", job_path.display());
    let mut parameters: Vec<u16> = parameters.encode_utf16().collect();
    parameters.push(0);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: windows::core::w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    // SAFETY: `info` is fully initialised, and both string buffers outlive the
    // call; `ShellExecuteExW` copies what it needs before returning.
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        // SAFETY: reading the last error of this thread cannot fail.
        let code = unsafe { GetLastError() };
        if code == ERROR_CANCELLED {
            return Err(FileSystemError::PermissionDenied(
                "The elevation prompt was dismissed".to_owned(),
            ));
        }
        return Err(FileSystemError::Internal(error.to_string()));
    }

    if !info.hProcess.is_invalid() {
        // SAFETY: the handle came from `ShellExecuteExW` and is closed below.
        unsafe {
            let _ = WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn launch(job_path: &Path) -> Result<(), FileSystemError> {
    let executable = std::env::current_exe()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    // `pkexec` is the polkit CLI every desktop ships; without it there is no
    // supported way to raise one process's rights from inside a session.
    let status = std::process::Command::new("pkexec")
        .arg(&executable)
        .arg(JOB_FLAG)
        .arg(job_path)
        .status()
        .map_err(|error| {
            FileSystemError::Unsupported(format!(
                "Elevation needs polkit's pkexec, which could not be started: {error}"
            ))
        })?;

    status_to_result(status)
}

#[cfg(target_os = "macos")]
fn launch(job_path: &Path) -> Result<(), FileSystemError> {
    let executable = std::env::current_exe()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    // `quoted form of` is AppleScript's shell quoting; building the command by
    // hand would let a path with a quote break out of it.
    let script = format!(
        "do shell script (quoted form of \"{}\") & \" {JOB_FLAG} \" & (quoted form of \"{}\") with administrator privileges",
        executable.display(),
        job_path.display()
    );

    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|error| FileSystemError::Unsupported(error.to_string()))?;

    status_to_result(status).map_err(|error| match error {
        FileSystemError::Internal(message) if message.contains("(-128)") => {
            FileSystemError::PermissionDenied("The elevation prompt was dismissed".to_owned())
        }
        other => other,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn status_to_result(status: std::process::ExitStatus) -> Result<(), FileSystemError> {
    if status.success() {
        return Ok(());
    }

    // The child writes its own error into the result file; this covers the
    // cases where it never got that far (a dismissed prompt, a missing binary).
    Err(FileSystemError::Internal(format!(
        "The elevated process exited with {status}"
    )))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn launch(_job_path: &Path) -> Result<(), FileSystemError> {
    Err(FileSystemError::Unsupported(
        "Elevation is not implemented on this platform".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::types::ConflictAction;

    #[test]
    fn a_job_round_trips_through_json() {
        let job = ElevatedJob {
            kind: "copy".to_owned(),
            sources: vec![TransferItem {
                path: r"C:\Windows\notepad.exe".to_owned(),
                on_conflict: ConflictAction::Replace,
            }],
            destination: Some(r"C:\temp".to_owned()),
        };

        let encoded = serde_json::to_string(&job).expect("encode the job");
        let decoded: ElevatedJob = serde_json::from_str(&encoded).expect("decode the job");

        assert_eq!(decoded.kind, "copy");
        assert_eq!(decoded.sources.len(), 1);
        assert_eq!(decoded.sources[0].on_conflict, ConflictAction::Replace);
        assert_eq!(decoded.destination.as_deref(), Some(r"C:\temp"));
    }

    #[test]
    fn a_job_outside_the_dae_directory_is_refused() {
        let outside = std::env::temp_dir().join("dae-elevated-outsider.json");
        assert!(!is_own_job(&outside));
    }

    #[test]
    fn an_unknown_kind_is_rejected_before_any_io() {
        let job = ElevatedJob {
            kind: "rename".to_owned(),
            sources: vec![TransferItem {
                path: "/tmp/whatever".to_owned(),
                on_conflict: ConflictAction::Fail,
            }],
            destination: None,
        };

        let outcome = execute(&job);
        assert!(
            matches!(outcome, Err(FileSystemError::InvalidInput(_))),
            "expected an invalid-input rejection, got {outcome:?}"
        );
    }

    #[test]
    fn a_missing_result_file_reports_that_the_child_never_answered() {
        let missing = std::env::temp_dir().join("dae-elevated-no-result.json");
        let outcome = read_result(&missing);
        assert!(matches!(outcome, Err(FileSystemError::Internal(_))));
    }

    #[test]
    fn a_failed_result_carries_the_childs_message() {
        let path = std::env::temp_dir().join(format!(
            "dae-elevated-result-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            br#"{"ok":false,"error":"Access is denied"}"#,
        )
        .expect("write the result");

        let outcome = read_result(&path);
        let _ = fs::remove_file(&path);

        assert_eq!(
            outcome.err().map(|error| error.to_string()),
            Some(
                FileSystemError::Internal("Access is denied".to_owned())
                    .to_string()
            )
        );
    }
}
