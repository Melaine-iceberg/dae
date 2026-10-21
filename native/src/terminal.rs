//! PTY sessions for the renderer's terminal panel.
//!
//! The shell's `terminal_create` hands back two channels — raw output bytes and
//! the exit code — so this module writes to channels rather than the generic
//! event stream.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{
    ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system,
};

use crate::emit_channel;

const FLUSH_INTERVAL_MS: u64 = 12;
const MAX_BATCH_BYTES: usize = 64 * 1024;
static NEXT_SESSION_ID: AtomicI32 = AtomicI32::new(1);

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

fn sessions() -> &'static Mutex<HashMap<u32, TerminalSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<u32, TerminalSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct CreateArgs {
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub on_output: Option<u32>,
    pub on_exit: Option<u32>,
}

pub fn create(args: CreateArgs) -> Result<u32, String> {
    let mut command = shell_command();
    if let Some(cwd) = args.cwd {
        let candidate = Path::new(&cwd);
        if candidate.is_dir() {
            command.cwd(candidate);
        }
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: args.rows.max(1),
            cols: args.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("term.pty_create_failed: {error}"))?;

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("term.shell_spawn_failed: {error}"))?;
    let pid = child.process_id();
    let killer = child.clone_killer();
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("term.pty_input_failed: {error}"))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("term.pty_output_failed: {error}"))?;
    // Dropping the slave keeps reads from hanging once the shell exits.
    drop(pair.slave);

    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed) as u32;
    let pending = Arc::new(Mutex::new(Vec::<u8>::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let exited = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(AtomicI32::new(-1));

    thread::spawn({
        let pending = pending.clone();
        let closed = closed.clone();
        move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if let Ok(mut pending) = pending.lock() {
                            pending.extend_from_slice(&buffer[..read]);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        eprintln!("[dae-native] terminal {id} read error: {error}");
                        break;
                    }
                }
            }
            closed.store(true, Ordering::Release);
        }
    });

    thread::spawn({
        let exit_code = exit_code.clone();
        let exited = exited.clone();
        let mut child = child;
        move || {
            if let Ok(status) = child.wait() {
                exit_code.store(status.exit_code() as i32, Ordering::Release);
            }
            exited.store(true, Ordering::Release);
        }
    });

    let output_channel = args.on_output;
    let exit_channel = args.on_exit;
    thread::spawn({
        let pending = pending.clone();
        let closed = closed.clone();
        let exited = exited.clone();
        let exit_code = exit_code.clone();
        move || {
            // ConPTY can keep the output pipe open after the shell exits, so a
            // reader EOF is not a reliable end-of-session signal on Windows.
            // Once the child is gone, give the reader a short grace period to
            // drain what the console buffered, then close the session anyway.
            let mut drain_deadline: Option<Instant> = None;
            loop {
                thread::sleep(Duration::from_millis(FLUSH_INTERVAL_MS));
                if let Some(chunk) = drain_pending(&pending, MAX_BATCH_BYTES) {
                    if let Some(channel) = output_channel {
                        emit_channel(channel, None, Some(chunk));
                    }
                }

                let exited_now = exited.load(Ordering::Acquire);
                let reader_closed = closed.load(Ordering::Acquire);
                if !exited_now {
                    continue;
                }

                let deadline =
                    *drain_deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(400));
                if !reader_closed && Instant::now() < deadline {
                    continue;
                }

                if let Some(rest) = drain_pending(&pending, usize::MAX) {
                    if let Some(channel) = output_channel {
                        emit_channel(channel, None, Some(rest));
                    }
                }
                if let Some(channel) = exit_channel {
                    emit_channel(
                        channel,
                        Some(exit_code.load(Ordering::Acquire).max(0).to_string()),
                        None,
                    );
                }
                // Dropping the session closes the master, which wakes the
                // reader if it is still blocked on the open ConPTY pipe.
                sessions()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                break;
            }
        }
    });

    let _ = writer.flush();

    sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            id,
            TerminalSession {
                writer,
                master: pair.master,
                killer,
                pid,
            },
        );

    Ok(id)
}

fn drain_pending(pending: &Mutex<Vec<u8>>, limit: usize) -> Option<Vec<u8>> {
    let mut pending = pending.lock().ok()?;
    if pending.is_empty() {
        return None;
    }
    if pending.len() <= limit {
        return Some(std::mem::take(&mut *pending));
    }
    Some(pending.drain(..limit).collect())
}

pub fn write(id: u32, data: &str) -> Result<(), String> {
    let mut sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| format!("term.session_not_found: {id}"))?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|error| format!("term.input_failed: {error}"))?;
    session
        .writer
        .flush()
        .map_err(|error| format!("term.input_failed: {error}"))
}

pub fn resize(id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = sessions
        .get(&id)
        .ok_or_else(|| format!("term.session_not_found: {id}"))?;
    session
        .master
        .resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("term.resize_failed: {error}"))
}

pub fn kill(id: u32) -> bool {
    let mut sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(mut session) = sessions.remove(&id) else {
        return false;
    };
    kill_session(&mut session);
    true
}

pub fn kill_all() {
    let mut sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (_, mut session) in sessions.drain() {
        kill_session(&mut session);
    }
}

fn kill_session(session: &mut TerminalSession) {
    // Taskkill first: it needs the root process alive to walk the tree. A bare
    // TerminateProcess (the killer's implementation) leaves children behind.
    #[cfg(target_os = "windows")]
    if let Some(pid) = session.pid {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = session.killer.kill();
}

fn shell_command() -> CommandBuilder {
    let mut command = CommandBuilder::new(default_program());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command
}

#[cfg(target_os = "windows")]
fn default_program() -> String {
    if is_in_path("pwsh.exe") {
        "pwsh.exe".to_owned()
    } else if is_in_path("powershell.exe") {
        "powershell.exe".to_owned()
    } else {
        "cmd.exe".to_owned()
    }
}

#[cfg(target_os = "windows")]
fn is_in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn default_program() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
}
