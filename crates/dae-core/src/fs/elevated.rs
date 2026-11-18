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
//!    runs the same execution function the window's own transfers use, writes
//!    a result file — including the journal of what landed, so the parent can
//!    record the undo the child's own stack could never reach — and exits.
//!
//! Nothing else in the app is elevated. The window, the listings, the terminal
//! and the index stay at the user's own token, which is the whole point: an
//! elevated *app* would leave elevated handles and tokens lying around, and one
//! elevated *operation* leaves nothing behind but the bytes it moved.
//!
//! # Why the result is a file
//!
//! The child has no channel back to the parent's event loop — that is what
//! elevation costs. Files next to the job are the smallest thing that survives
//! a crash of either process: a result file the parent reads once the child
//! exits, a progress file the child writes while it runs, and pause/cancel
//! flag files the parent writes and the child watches. A result that is
//! missing means the child died.

use super::progress::{
    FileOperationKind, FileOperationPhase, FileOperationProgressReporterTrait,
    emit_file_operation_progress, emit_preparing,
};
use super::transfer::{self, TransferSource};
use super::transfer_control::{ControlRegistration, TransferControl};
use super::types::{TransferItem, TransferPair};
use super::vfs;
use crate::error::FileSystemError;
use crate::host::Host;
use crate::types::{DirectoryView, path_to_string};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The flag the elevated child is launched with.
pub const JOB_FLAG: &str = "--elevated-job";

/// One operation, exactly as the shell asked for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevatedJob {
    /// `"copy"`, `"move"`, `"delete"` or `"list"`.
    pub kind: String,
    pub sources: Vec<TransferItem>,
    /// The target directory for a copy or a move; `None` for a delete or list.
    pub destination: Option<String>,
    /// Bytes per second the child should obey, inherited from the shell's
    /// process-wide limit. Without it the child would chunk (and therefore
    /// answer pause/cancel per entry) only when it happened to have a limit of
    /// its own — which a separate process never does.
    #[serde(default)]
    pub rate_limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobResult {
    ok: bool,
    /// The child was cancelled through the flag file, not broken: the shell
    /// renders a neutral "cancelled" row instead of an error.
    #[serde(default)]
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The one-shot directory snapshot a `"list"` job answers with. A transfer
    /// job leaves it `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<DirectoryView>,
    /// The entries a transfer landed, kept even on partial failure: the
    /// parent records its undo from these, since the child's own undo stack
    /// dies with it.
    #[serde(default)]
    journal: Vec<TransferPair>,
}

/// What the child reports back: the operation's outcome, the listing a
/// `"list"` job produced, and the transfer journal. The outcome is carried
/// separately from the journal so a half-finished batch still hands over the
/// entries that did land.
pub struct JobReport {
    pub outcome: Result<(), FileSystemError>,
    pub view: Option<DirectoryView>,
    pub journal: Vec<TransferPair>,
}

/// Runs the transfer in the elevated child, reporting progress and answering
/// pause/cancel while it runs.
///
/// Blocking, like every transfer: the caller runs it on a worker thread. The
/// child cannot reach this process's event loop, so progress crosses as a file
/// the child writes and this one polls, and control crosses as flag files in
/// the other direction — see [`poll_child`].
pub fn run(
    job: ElevatedJob,
    host: &dyn Host,
    operation_id: &str,
) -> Result<JobReport, FileSystemError> {
    let kind = match job.kind.as_str() {
        "move" => FileOperationKind::Move,
        "delete" => FileOperationKind::Delete,
        _ => FileOperationKind::Copy,
    };
    let (job_path, result_path) = write_job(&job)?;
    emit_preparing(host, operation_id, kind);

    // The shell's pause/cancel buttons address this control by operation id,
    // exactly as for an in-process transfer. The registration is dropped when
    // this function returns, so a late button press cannot reach a dead job.
    let (_registration, control) = ControlRegistration::new(operation_id);

    let report = launch(&job_path).and_then(|waiter| {
        poll_child(waiter, &job_path, &control, host, operation_id, kind)?;
        read_result(&result_path)
    });

    cleanup(&job_path, &result_path);
    let report = report?;
    if report.view.is_some() {
        return Err(FileSystemError::InvalidInput(
            "The elevated listing returned a directory view to a transfer".to_owned(),
        ));
    }
    Ok(report)
}

/// Reads one local directory with the elevated child's token.
///
/// This is the read-only counterpart of [`run`]: the window and every other
/// listing stay at the user's own token, and only this snapshot is produced by
/// the elevated process. It exists for the permission-denied empty state, where
/// "retry as administrator" is the only action that can answer.
pub fn run_list(path: &Path) -> Result<DirectoryView, FileSystemError> {
    let job = ElevatedJob {
        kind: "list".to_owned(),
        sources: vec![TransferItem {
            path: path.to_string_lossy().into_owned(),
            on_conflict: Default::default(),
        }],
        destination: None,
        rate_limit: None,
    };

    match run_job(job)?.view {
        Some(view) => Ok(view),
        None => Err(FileSystemError::Internal(
            "The elevated listing did not report a view".to_owned(),
        )),
    }
}

/// The blocking, progress-free path, kept for [`run_list`]: a one-shot
/// listing has nothing to pause and finishes in a blink.
fn run_job(job: ElevatedJob) -> Result<JobReport, FileSystemError> {
    let (job_path, result_path) = write_job(&job)?;
    let outcome = launch(&job_path).and_then(|waiter| {
        wait_blocking(waiter)?;
        read_result(&result_path)
    });
    cleanup(&job_path, &result_path);
    outcome
}

fn write_job(job: &ElevatedJob) -> Result<(PathBuf, PathBuf), FileSystemError> {
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

    let contents = serde_json::to_vec_pretty(job)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    fs::write(&job_path, contents)?;
    Ok((job_path, result_path))
}

/// The job names paths the user selected; none of the files either process
/// produced has a reason to stay on disk.
fn cleanup(job_path: &Path, result_path: &Path) {
    let _ = fs::remove_file(job_path);
    let _ = fs::remove_file(result_path);
    let _ = fs::remove_file(job_path.with_extension("progress.json"));
    let _ = fs::remove_file(job_path.with_extension("cancel"));
    let _ = fs::remove_file(job_path.with_extension("pause"));
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
    let report = execute_job_file(&job_path).unwrap_or_else(|error| JobReport {
        outcome: Err(error),
        view: None,
        journal: Vec::new(),
    });

    let payload = JobResult {
        ok: report.outcome.is_ok(),
        cancelled: matches!(report.outcome, Err(FileSystemError::Cancelled(_))),
        error: report.outcome.err().map(|error| error.to_string()),
        view: report.view,
        journal: report.journal,
    };

    if let Ok(contents) = serde_json::to_vec_pretty(&payload) {
        let _ = fs::write(&result_path, contents);
    }

    Some(if payload.ok { 0 } else { 1 })
}

fn execute_job_file(path: &Path) -> Result<JobReport, FileSystemError> {
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

    if job.kind == "list" {
        let source = job.sources.first().ok_or_else(|| {
            FileSystemError::InvalidInput("The elevated listing has no path".to_owned())
        })?;
        return Ok(match super::local::read_directory_sync(PathBuf::from(&source.path)) {
            Ok(view) => JobReport {
                outcome: Ok(()),
                view: Some(view),
                journal: Vec::new(),
            },
            Err(error) => JobReport {
                outcome: Err(error),
                view: None,
                journal: Vec::new(),
            },
        });
    }

    let (outcome, journal) = execute(&job, path);
    Ok(JobReport {
        outcome,
        view: None,
        journal,
    })
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

/// The transfer itself — the same execution function the window's transfers
/// run, so the journal it returns is the same record the parent would have
/// written itself. Progress goes to a file the parent polls, and a watcher
/// thread turns the parent's flag files into the same pause/cancel calls an
/// in-process transfer answers.
fn execute(job: &ElevatedJob, job_path: &Path) -> (Result<(), FileSystemError>, Vec<TransferPair>) {
    if job.sources.is_empty() {
        return (
            Err(FileSystemError::InvalidInput(
                "The elevated job carries no sources".to_owned(),
            )),
            Vec::new(),
        );
    }

    let control = Arc::new(TransferControl::new(job.rate_limit));
    let done = Arc::new(AtomicBool::new(false));
    let watcher = spawn_flag_watcher(job_path, &control, &done);
    let progress = FileProgressReporter::new(
        job_path.with_extension("progress.json"),
        Arc::clone(&control),
    );

    let run = || match job.kind.as_str() {
        "delete" => {
            let result = job
                .sources
                .iter()
                .map(|item| {
                    Ok(TransferSource {
                        path: item.path.clone(),
                        backend: vfs::resolve(&item.path)?,
                        on_conflict: item.on_conflict,
                    })
                })
                .collect::<Result<Vec<_>, FileSystemError>>()
                .and_then(|sources| transfer::delete_entries(sources, &progress));
            (result, Vec::new())
        }
        "copy" | "move" => {
            let Some(destination) = job.destination.clone() else {
                return (
                    Err(FileSystemError::InvalidInput(
                        "The elevated job has no destination".to_owned(),
                    )),
                    Vec::new(),
                );
            };
            if job.kind == "move" {
                super::service::execute_move(job.sources.clone(), &destination, &progress)
            } else {
                super::service::execute_copy(job.sources.clone(), &destination, &progress)
            }
        }
        other => (
            Err(FileSystemError::InvalidInput(format!(
                "Unknown elevated job kind: {other}"
            ))),
            Vec::new(),
        ),
    };
    let (result, journal) = run();

    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    // Only a completed transfer forces its final reading past the throttle;
    // a cancelled or failed one keeps the last tick, where it actually stopped
    // (the engine itself only calls `finish` on its success path).
    if result.is_ok() {
        progress.finish();
    }
    (result, journal)
}

/// The child's side of the control seam: the parent cannot reach this
/// process's atomics, so it writes flag files and this thread turns them into
/// pause/cancel calls on the same [`TransferControl`] the engine gates on.
fn spawn_flag_watcher(
    job_path: &Path,
    control: &Arc<TransferControl>,
    done: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let cancel_path = job_path.with_extension("cancel");
    let pause_path = job_path.with_extension("pause");
    let control = Arc::clone(control);
    let done = Arc::clone(done);
    std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            if cancel_path.exists() {
                control.cancel();
            }
            if pause_path.exists() {
                control.pause();
            } else {
                control.resume();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}

/// The progress file the parent polls: the same fields the UI event carries.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressFile {
    completed: u64,
    total: u64,
    current_path: Option<String>,
}

/// A reporter with no event loop to reach: it writes the progress file,
/// throttled like the in-process reporter so a 35,000-entry copy does not
/// become 35,000 file writes.
struct FileProgressReporter {
    path: PathBuf,
    state: Mutex<ProgressState>,
    control: Arc<TransferControl>,
}

struct ProgressState {
    completed: u64,
    total: u64,
    current_path: Option<String>,
    last_write: Option<Instant>,
}

impl FileProgressReporter {
    fn new(path: PathBuf, control: Arc<TransferControl>) -> Self {
        Self {
            path,
            state: Mutex::new(ProgressState {
                completed: 0,
                total: 0,
                current_path: None,
                last_write: None,
            }),
            control,
        }
    }

    fn write(&self, force: bool) {
        let mut state = self.state.lock().expect("progress reporter lock poisoned");
        if !force
            && state
                .last_write
                .is_some_and(|last| last.elapsed() < Duration::from_millis(150))
        {
            return;
        }
        state.last_write = Some(Instant::now());
        let payload = ProgressFile {
            completed: state.completed.min(state.total),
            total: state.total,
            current_path: state.current_path.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&payload) {
            let _ = fs::write(&self.path, bytes);
        }
    }
}

impl FileOperationProgressReporterTrait for FileProgressReporter {
    fn start(&self, total: u64) {
        self.state.lock().expect("progress reporter lock poisoned").total = total;
        self.write(true);
    }

    fn begin_entry(&self, path: &Path) {
        let is_first = {
            let mut state = self.state.lock().expect("progress reporter lock poisoned");
            state.current_path = Some(path_to_string(path));
            state.completed == 0
        };
        self.write(is_first);
    }

    fn advance_by(&self, units: u64, path: &Path) {
        let is_done = {
            let mut state = self.state.lock().expect("progress reporter lock poisoned");
            state.completed = state.completed.saturating_add(units);
            state.current_path = Some(path_to_string(path));
            state.completed >= state.total
        };
        self.write(is_done);
    }

    fn finish(&self) {
        {
            let mut state = self.state.lock().expect("progress reporter lock poisoned");
            state.completed = state.total;
        }
        self.write(true);
    }

    fn control(&self) -> Option<&TransferControl> {
        Some(&self.control)
    }
}

fn read_result(path: &Path) -> Result<JobReport, FileSystemError> {
    let contents = fs::read(path).map_err(|error| {
        // The child exited without writing a result: either the user dismissed
        // the elevation prompt or the child died before it could report.
        FileSystemError::Internal(format!("The elevated operation did not report back: {error}"))
    })?;

    let result: JobResult = serde_json::from_slice(&contents)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let outcome = if result.ok {
        Ok(())
    } else if result.cancelled {
        Err(FileSystemError::Cancelled(
            result
                .error
                .unwrap_or_else(|| "The elevated operation was cancelled".to_owned()),
        ))
    } else {
        Err(FileSystemError::Internal(
            result
                .error
                .unwrap_or_else(|| "The elevated operation failed".to_owned()),
        ))
    };
    Ok(JobReport {
        outcome,
        view: result.view,
        journal: result.journal,
    })
}

/// The launched child, waited on without blocking the poll loop. On Windows
/// this is the process `ShellExecuteExW` hands back; elsewhere it is the
/// launcher (`pkexec`, `osascript`), which exits when the child does.
enum Waiter {
    #[cfg(windows)]
    Windows(windows::Win32::Foundation::HANDLE),
    #[cfg(not(windows))]
    Unix(std::process::Child),
}

impl Waiter {
    fn try_wait(&mut self) -> Result<bool, FileSystemError> {
        match self {
            #[cfg(windows)]
            Self::Windows(handle) => {
                use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
                use windows::Win32::System::Threading::WaitForSingleObject;
                // SAFETY: the handle came from `ShellExecuteExW` and stays
                // open until `finish` closes it.
                let status = unsafe { WaitForSingleObject(*handle, 0) };
                if status == WAIT_OBJECT_0 {
                    Ok(true)
                } else if status == WAIT_TIMEOUT {
                    Ok(false)
                } else {
                    Err(FileSystemError::Internal(
                        "Waiting on the elevated child failed".to_owned(),
                    ))
                }
            }
            #[cfg(not(windows))]
            Self::Unix(child) => child
                .try_wait()
                .map(|status| status.is_some())
                .map_err(|error| FileSystemError::Internal(error.to_string())),
        }
    }

    /// The child has exited: release the handle, or collect the exit status
    /// the result file could not explain (a dismissed prompt, a dead binary).
    fn finish(self) -> Result<(), FileSystemError> {
        match self {
            #[cfg(windows)]
            Self::Windows(handle) => {
                use windows::Win32::Foundation::CloseHandle;
                // SAFETY: the handle came from `ShellExecuteExW` and is
                // closed exactly once, here.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Ok(())
            }
            #[cfg(not(windows))]
            Self::Unix(mut child) => {
                let status = child
                    .wait()
                    .map_err(|error| FileSystemError::Internal(error.to_string()))?;
                let result = status_to_result(status);
                #[cfg(target_os = "macos")]
                let result = result.map_err(|error| match error {
                    FileSystemError::Internal(message) if message.contains("(-128)") => {
                        FileSystemError::PermissionDenied(
                            "The elevation prompt was dismissed".to_owned(),
                        )
                    }
                    other => other,
                });
                result
            }
        }
    }
}

/// Carries progress back and control across until the child exits: the child
/// writes a progress file and watches flag files; this loop reads one and
/// writes the other. 100 ms is under the child reporter's own throttle, so no
/// boundary tick is lost, and over a UI frame, so the row stays calm.
fn poll_child(
    mut waiter: Waiter,
    job_path: &Path,
    control: &TransferControl,
    host: &dyn Host,
    operation_id: &str,
    kind: FileOperationKind,
) -> Result<(), FileSystemError> {
    let progress_path = job_path.with_extension("progress.json");
    let cancel_path = job_path.with_extension("cancel");
    let pause_path = job_path.with_extension("pause");
    let mut cancel_written = false;
    let mut last: Option<(u64, Option<String>)> = None;

    loop {
        if control.is_cancelled() && !cancel_written {
            let _ = fs::write(&cancel_path, b"1");
            cancel_written = true;
        }
        if control.is_paused() {
            if !pause_path.exists() {
                let _ = fs::write(&pause_path, b"1");
            }
        } else {
            let _ = fs::remove_file(&pause_path);
        }

        if let Ok(bytes) = fs::read(&progress_path)
            && let Ok(progress) = serde_json::from_slice::<ProgressFile>(&bytes)
        {
            let tick = (progress.completed, progress.current_path.clone());
            if last.as_ref() != Some(&tick) {
                emit_file_operation_progress(
                    host,
                    operation_id,
                    kind,
                    FileOperationPhase::Running,
                    progress.completed,
                    Some(progress.total),
                    progress.current_path,
                );
                last = Some(tick);
            }
        }

        if waiter.try_wait()? {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    waiter.finish()
}

fn wait_blocking(mut waiter: Waiter) -> Result<(), FileSystemError> {
    while !waiter.try_wait()? {
        std::thread::sleep(Duration::from_millis(100));
    }
    waiter.finish()
}

#[cfg(windows)]
fn launch(job_path: &Path) -> Result<Waiter, FileSystemError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{ERROR_CANCELLED, GetLastError};
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

    if info.hProcess.is_invalid() {
        return Err(FileSystemError::Internal(
            "The elevation launch returned no process handle".to_owned(),
        ));
    }
    Ok(Waiter::Windows(info.hProcess))
}

#[cfg(target_os = "linux")]
fn launch(job_path: &Path) -> Result<Waiter, FileSystemError> {
    let executable = std::env::current_exe()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    // `pkexec` is the polkit CLI every desktop ships; without it there is no
    // supported way to raise one process's rights from inside a session.
    let child = std::process::Command::new("pkexec")
        .arg(&executable)
        .arg(JOB_FLAG)
        .arg(job_path)
        .spawn()
        .map_err(|error| {
            FileSystemError::Unsupported(format!(
                "Elevation needs polkit's pkexec, which could not be started: {error}"
            ))
        })?;

    Ok(Waiter::Unix(child))
}

#[cfg(target_os = "macos")]
fn launch(job_path: &Path) -> Result<Waiter, FileSystemError> {
    let executable = std::env::current_exe()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    // `quoted form of` is AppleScript's shell quoting; building the command by
    // hand would let a path with a quote break out of it.
    let script = format!(
        "do shell script (quoted form of \"{}\") & \" {JOB_FLAG} \" & (quoted form of \"{}\") with administrator privileges",
        executable.display(),
        job_path.display()
    );

    let child = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
        .map_err(|error| FileSystemError::Unsupported(error.to_string()))?;

    Ok(Waiter::Unix(child))
}

#[cfg(not(windows))]
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
fn launch(_job_path: &Path) -> Result<Waiter, FileSystemError> {
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
            rate_limit: Some(4 * 1024 * 1024),
        };

        let encoded = serde_json::to_string(&job).expect("encode the job");
        let decoded: ElevatedJob = serde_json::from_str(&encoded).expect("decode the job");

        assert_eq!(decoded.kind, "copy");
        assert_eq!(decoded.sources.len(), 1);
        assert_eq!(decoded.sources[0].on_conflict, ConflictAction::Replace);
        assert_eq!(decoded.destination.as_deref(), Some(r"C:\temp"));
        assert_eq!(decoded.rate_limit, Some(4 * 1024 * 1024));
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
            rate_limit: None,
        };

        let (outcome, journal) = execute(&job, &std::env::temp_dir().join("dae-elevated-unknown.json"));
        assert!(
            matches!(outcome, Err(FileSystemError::InvalidInput(_))),
            "expected an invalid-input rejection, got {outcome:?}"
        );
        assert!(journal.is_empty());
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

        let report = read_result(&path).expect("the result parses");
        let _ = fs::remove_file(&path);

        assert_eq!(
            report.outcome.err().map(|error| error.to_string()),
            Some(
                FileSystemError::Internal("Access is denied".to_owned())
                    .to_string()
            )
        );
        assert!(report.journal.is_empty());
    }

    /// A scratch directory unique to one test, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "dae-elevated-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("dst")).expect("scratch dirs");
            Self(dir)
        }

        fn path(&self, relative: &str) -> String {
            self.0.join(relative).to_string_lossy().into_owned()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn copy_job(scratch: &Scratch, sources: &[&str]) -> ElevatedJob {
        ElevatedJob {
            kind: "copy".to_owned(),
            sources: sources
                .iter()
                .map(|name| TransferItem {
                    path: scratch.path(name),
                    on_conflict: ConflictAction::Fail,
                })
                .collect(),
            destination: Some(scratch.path("dst")),
            rate_limit: None,
        }
    }

    #[test]
    fn a_successful_copy_reports_its_journal() {
        let scratch = Scratch::new("journal-ok");
        fs::write(scratch.path("a.txt"), b"a").expect("source a");
        fs::write(scratch.path("b.txt"), b"b").expect("source b");

        let (outcome, journal) = execute(
            &copy_job(&scratch, &["a.txt", "b.txt"]),
            &scratch.0.join("job.json"),
        );

        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(journal.len(), 2);
        assert!(journal.iter().all(|pair| pair.destination.contains("dst")));
        assert_eq!(fs::read(scratch.path("dst/a.txt")).expect("copied a"), b"a");

        // The finish tick is forced past the throttle, so the parent always
        // has a final reading to show.
        let progress: ProgressFile = serde_json::from_slice(
            &fs::read(scratch.0.join("job.progress.json")).expect("progress file"),
        )
        .expect("progress parses");
        assert_eq!(progress.total, 2);
        assert_eq!(progress.completed, 2);
    }

    #[test]
    fn a_fail_conflict_rejects_the_batch_before_any_byte_moves() {
        let scratch = Scratch::new("journal-failfast");
        fs::write(scratch.path("a.txt"), b"a").expect("source a");
        fs::write(scratch.path("b.txt"), b"new").expect("source b");
        fs::write(scratch.path("dst/b.txt"), b"old").expect("collision");

        let (outcome, journal) = execute(
            &copy_job(&scratch, &["a.txt", "b.txt"]),
            &scratch.0.join("job.json"),
        );

        // The plan check answers a Fail collision up front: nothing runs, so
        // there is nothing to undo and the destination is untouched.
        assert!(outcome.is_err());
        assert!(journal.is_empty());
        assert!(!std::path::Path::new(&scratch.path("dst/a.txt")).exists());
        assert_eq!(fs::read(scratch.path("dst/b.txt")).expect("untouched"), b"old");
    }

    /// A failure *mid-batch* — here a file another process holds open, the
    /// Windows sharing violation — must still hand back the entries that
    /// landed before it, because those are what the parent can undo.
    #[cfg(windows)]
    #[test]
    fn a_partial_failure_hands_back_the_entries_that_landed() {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Foundation::{CloseHandle, GENERIC_READ};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING,
        };
        use windows::core::PCWSTR;

        let scratch = Scratch::new("journal-partial");
        fs::write(scratch.path("a.txt"), b"a").expect("source a");
        fs::write(scratch.path("b.txt"), b"b").expect("source b");

        let wide: Vec<u16> = std::ffi::OsStr::new(&scratch.path("b.txt"))
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: the wide path is NUL-terminated and outlives the call; no
        // share mode means every other open of this file fails until close.
        let lock = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .expect("hold b.txt open");

        let (outcome, journal) = execute(
            &copy_job(&scratch, &["a.txt", "b.txt"]),
            &scratch.0.join("job.json"),
        );

        // SAFETY: the handle came from CreateFileW above and is closed once.
        unsafe {
            let _ = CloseHandle(lock);
        }

        assert!(outcome.is_err());
        assert_eq!(journal.len(), 1);
        assert!(journal[0].source.ends_with("a.txt"));
        assert_eq!(fs::read(scratch.path("dst/a.txt")).expect("copied a"), b"a");
    }

    #[test]
    fn the_result_file_round_trips_the_journal() {
        let scratch = Scratch::new("journal-wire");
        let result = JobResult {
            ok: false,
            cancelled: false,
            error: Some("halfway".to_owned()),
            view: None,
            journal: vec![TransferPair {
                source: scratch.path("a.txt"),
                destination: scratch.path("dst/a.txt"),
            }],
        };
        let path = scratch.0.join("result.json");
        fs::write(&path, serde_json::to_vec(&result).expect("encode")).expect("write");

        let report = read_result(&path).expect("the result parses");

        assert!(report.outcome.is_err());
        assert_eq!(report.journal.len(), 1);
        assert_eq!(report.journal[0].source, scratch.path("a.txt"));
    }

    #[test]
    fn a_cancelled_result_reads_back_as_cancelled_not_broken() {
        let scratch = Scratch::new("cancelled-wire");
        let path = scratch.0.join("result.json");
        fs::write(&path, br#"{"ok":false,"cancelled":true}"#).expect("write");

        let report = read_result(&path).expect("the result parses");
        assert!(
            matches!(report.outcome, Err(FileSystemError::Cancelled(_))),
            "{:?}",
            report.outcome
        );
    }

    /// The watcher is the child's half of the control seam; drive it with the
    /// same flag files the parent writes and assert the control answers.
    #[test]
    fn flag_files_reach_the_control_as_pause_and_cancel() {
        let scratch = Scratch::new("flags");
        let job_path = scratch.0.join("job.json");
        let control = Arc::new(TransferControl::new(None));
        let done = Arc::new(AtomicBool::new(false));
        let watcher = spawn_flag_watcher(&job_path, &control, &done);

        let wait_until = |predicate: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !predicate() {
                assert!(Instant::now() < deadline, "the watcher never answered");
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        fs::write(job_path.with_extension("pause"), b"1").expect("pause flag");
        wait_until(&|| control.is_paused());

        fs::remove_file(job_path.with_extension("pause")).expect("clear pause");
        wait_until(&|| !control.is_paused());

        fs::write(job_path.with_extension("cancel"), b"1").expect("cancel flag");
        wait_until(&|| control.is_cancelled());

        done.store(true, Ordering::Relaxed);
        watcher.join().expect("watcher exits");
    }
}
