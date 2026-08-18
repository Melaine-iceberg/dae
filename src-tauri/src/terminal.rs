//! Integrated terminal sessions backed by portable-pty.
//!
//! One PTY session powers the terminal panel. Output is coalesced into
//! ~8ms batches and pushed to the webview as raw bytes over an IPC channel
//! (no JSON number-array overhead); keystrokes travel back as plain strings.
//! A hidden panel keeps its session alive, a closed shell is reported through
//! the exit channel, and everything is torn down when the app exits.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

/// Coalescing window for PTY output. 8ms keeps latency below one frame while
/// merging bursty output into a handful of IPC messages.
const FLUSH_INTERVAL_MS: u64 = 8;
/// Upper bound for a single channel message; bigger bursts are split.
const MAX_BATCH_BYTES: usize = 256 * 1024;

pub struct TerminalState {
    sessions: Mutex<HashMap<u32, TerminalSession>>,
    next_id: AtomicU32,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        }
    }
}

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    /// Split from the child so kills never race the blocking wait thread.
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// Spawns a shell in a new PTY and streams its output over `on_output`.
///
/// The shell is chosen per platform: PowerShell (pwsh, then Windows
/// PowerShell, then cmd) on Windows, `$SHELL` (with zsh/bash/sh fallbacks)
/// on macOS and Linux.
#[tauri::command]
pub fn terminal_create(
    state: State<'_, TerminalState>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, String> {
    let mut command = shell_command();
    if let Some(dir) = cwd.and_then(|path| {
        let candidate = PathBuf::from(&path);
        // Only local directories can host a shell; \\wsl$ style virtual
        // paths and missing folders fall back to the home directory.
        (candidate.is_dir() && !path.starts_with("\\\\")).then_some(candidate)
    }) {
        command.cwd(dir);
    } else if let Some(home) = home_directory() {
        command.cwd(home);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("创建 PTY 失败：{error}"))?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("启动 shell 失败：{error}"))?;
    let killer = child.clone_killer();
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("接管 PTY 输入失败：{error}"))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("接管 PTY 输出失败：{error}"))?;
    // Dropping the slave keeps reads from hanging once the shell exits.
    drop(pair.slave);

    let pending = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(AtomicI32::new(-1));

    thread::spawn({
        let pending = Arc::clone(&pending);
        let closed = Arc::clone(&closed);
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
                    Err(_) => break,
                }
            }
            closed.store(true, Ordering::Release);
        }
    });

    thread::spawn({
        let exit_code = Arc::clone(&exit_code);
        move || {
            if let Ok(status) = child.wait() {
                exit_code.store(status.exit_code() as i32, Ordering::Release);
            }
        }
    });

    tauri::async_runtime::spawn({
        let pending = Arc::clone(&pending);
        let closed = Arc::clone(&closed);
        let exit_code = Arc::clone(&exit_code);
        async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
            loop {
                ticker.tick().await;
                if let Some(chunk) = drain_pending(&pending, MAX_BATCH_BYTES) {
                    let _ = on_output.send(InvokeResponseBody::Raw(chunk));
                }
                if closed.load(Ordering::Acquire) {
                    // All output is already queued once `closed` is observed,
                    // so a final drain delivers the tail before reporting exit.
                    if let Some(rest) = drain_pending(&pending, usize::MAX) {
                        let _ = on_output.send(InvokeResponseBody::Raw(rest));
                    }
                    let code = exit_code.load(Ordering::Acquire);
                    let _ = on_exit.send(if code < 0 { 0 } else { code as u32 });
                    break;
                }
            }
        }
    });

    let _ = writer.flush();

    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    state.sessions.lock().unwrap().insert(
        id,
        TerminalSession {
            writer,
            master: pair.master,
            killer,
        },
    );
    Ok(id)
}

/// Feeds keystrokes (UTF-8 text) into the session's PTY.
#[tauri::command]
pub fn terminal_write(state: State<'_, TerminalState>, id: u32, data: String) -> Result<(), String> {
    let mut sessions = state.sessions.lock().unwrap();
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| "终端会话不存在".to_string())?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|error| format!("写入终端失败：{error}"))
}

/// Resizes the session's PTY to match the xterm viewport.
#[tauri::command]
pub fn terminal_resize(
    state: State<'_, TerminalState>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state.sessions.lock().unwrap();
    let session = sessions
        .get(&id)
        .ok_or_else(|| "终端会话不存在".to_string())?;
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("调整终端尺寸失败：{error}"))
}

/// Kills the shell and releases every PTY handle for the session.
#[tauri::command]
pub fn terminal_kill(state: State<'_, TerminalState>, id: u32) {
    if let Some(mut session) = state.sessions.lock().unwrap().remove(&id) {
        let _ = session.killer.kill();
        // Dropping master + writer forces the reader to EOF, which ends the
        // flush task; nothing else references the session afterwards.
    }
}

/// Kills every live session; used on app exit so no shells are orphaned.
pub fn kill_all(state: &TerminalState) {
    let sessions: Vec<TerminalSession> =
        std::mem::take(&mut *state.sessions.lock().unwrap())
            .into_values()
            .collect();
    for mut session in sessions {
        let _ = session.killer.kill();
    }
}

/// Takes at most `limit` bytes from the front of the pending buffer.
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

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(target_os = "windows")]
fn shell_command() -> CommandBuilder {
    let (program, args): (&str, &[&str]) = if is_in_path("pwsh.exe") {
        ("pwsh.exe", &["-NoLogo"])
    } else if is_in_path("powershell.exe") {
        ("powershell.exe", &["-NoLogo"])
    } else {
        ("cmd.exe", &[])
    };
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command
}

#[cfg(target_os = "windows")]
fn is_in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn shell_command() -> CommandBuilder {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|path| !path.is_empty() && PathBuf::from(path).exists())
        .or_else(|| {
            ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash", "/bin/sh"]
                .into_iter()
                .find(|path| PathBuf::from(path).exists())
                .map(str::to_string)
        });
    CommandBuilder::new(shell.unwrap_or_else(|| "/bin/sh".to_string()))
}

/// Dispatches a terminal IPC invoke by hand, mirroring what
/// `tauri::generate_handler!` expands to. The generated `__cmd__*` macros
/// must be invoked in their textual scope, which ends with this file.
pub fn handle_invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    match invoke.message.command() {
        "terminal_create" => __cmd__terminal_create!(terminal_create, invoke),
        "terminal_write" => __cmd__terminal_write!(terminal_write, invoke),
        "terminal_resize" => __cmd__terminal_resize!(terminal_resize, invoke),
        "terminal_kill" => __cmd__terminal_kill!(terminal_kill, invoke),
        _ => false,
    }
}
