//! Phase 0 N-API core for the dae Electron migration.
//!
//! This crate deliberately implements only the seams the migration plan labels
//! as risky: a directory listing (whole and streamed), a PTY session whose
//! output can be coalesced, thumbnail decoding, and a pull-based event queue.
//! The real `dae-core` extraction replaces this crate once the spike passes.

use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use napi::bindgen_prelude::Buffer;
use napi::{Error, Result};
use napi_derive::napi;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Coalescing window for PTY output, copied from the Tauri terminal so the
/// spike measures the same shape of IPC traffic.
const FLUSH_INTERVAL_MS: u64 = 8;
const MAX_BATCH_BYTES: usize = 256 * 1024;
const DEFAULT_BATCH_SIZE: usize = 512;

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

struct RawEvent {
    name: String,
    target: Option<String>,
    payload_json: Option<String>,
    data: Option<Vec<u8>>,
}

/// Events produced on OS threads, pulled by the core process's JS pump. A
/// bounded channel is deliberate: if the pump stalls, PTY and listing
/// producers block instead of growing an unbounded queue.
static EVENT_TX: OnceLock<mpsc::Sender<RawEvent>> = OnceLock::new();
static EVENT_RX: OnceLock<tokio::sync::Mutex<mpsc::Receiver<RawEvent>>> = OnceLock::new();

fn event_tx() -> &'static mpsc::Sender<RawEvent> {
    EVENT_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel(8192);
        let _ = EVENT_RX.set(tokio::sync::Mutex::new(rx));
        tx
    })
}

fn emit(event: RawEvent) {
    let _ = event_tx().blocking_send(event);
}

#[napi(object)]
pub struct CoreEvent {
    pub name: String,
    pub target: Option<String>,
    pub payload_json: Option<String>,
    pub data: Option<Buffer>,
}

/// Pulls at most `max` queued events. The first one is awaited; the rest are
/// drained without waiting so a burst costs one crossing instead of N.
#[napi(js_name = "nextEvents")]
pub async fn next_events(max: u32) -> Result<Vec<CoreEvent>> {
    let max = max.clamp(1, 256) as usize;
    let _ = event_tx();
    let Some(receiver) = EVENT_RX.get() else {
        return Ok(Vec::new());
    };
    let mut receiver = receiver.lock().await;
    let Some(first) = receiver.recv().await else {
        return Ok(Vec::new());
    };

    let mut events = Vec::with_capacity(max);
    events.push(into_js(first));
    while events.len() < max {
        match receiver.try_recv() {
            Ok(event) => events.push(into_js(event)),
            Err(_) => break,
        }
    }
    Ok(events)
}

fn into_js(event: RawEvent) -> CoreEvent {
    CoreEvent {
        name: event.name,
        target: event.target,
        payload_json: event.payload_json,
        data: event.data.map(Buffer::from),
    }
}

// ---------------------------------------------------------------------------
// Directory listing
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntryInfo {
    name: String,
    path: String,
    is_directory: bool,
    size: u64,
    modified_ms: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryListing {
    path: String,
    entries: Vec<DirectoryEntryInfo>,
    elapsed_ms: f64,
}

fn entry_info(entry: std::fs::DirEntry) -> Option<DirectoryEntryInfo> {
    let metadata = entry.metadata().ok()?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);

    Some(DirectoryEntryInfo {
        name: entry.file_name().to_string_lossy().into_owned(),
        path: entry.path().to_string_lossy().into_owned(),
        is_directory: metadata.is_dir(),
        size: if metadata.is_file() { metadata.len() } else { 0 },
        modified_ms,
    })
}

fn read_directory_impl(path: &str) -> std::result::Result<DirectoryListing, String> {
    let started = Instant::now();
    let read_dir = std::fs::read_dir(path)
        .map_err(|error| format!("failed to read {path}: {error}"))?;
    let entries = read_dir
        .filter_map(|entry| entry.ok())
        .filter_map(entry_info)
        .collect::<Vec<_>>();

    Ok(DirectoryListing {
        path: path.to_owned(),
        entries,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn emit_stream_event(name: &str, payload: serde_json::Value, data: Option<Vec<u8>>) {
    emit(RawEvent {
        name: name.to_owned(),
        target: None,
        payload_json: Some(payload.to_string()),
        data,
    });
}

/// Reads the directory in batches on a dedicated thread, sending each batch
/// as raw JSON bytes. This is the transport that must stay out of the main
/// process's JSON parser in the real app.
fn stream_directory(path: String, stream_id: String, batch_size: usize) {
    let started = Instant::now();
    let batch_size = batch_size.clamp(1, 4096);
    let result = (|| -> std::result::Result<usize, String> {
        let read_dir = std::fs::read_dir(&path)
            .map_err(|error| format!("failed to read {path}: {error}"))?;
        let mut batch = Vec::with_capacity(batch_size);
        let mut total = 0usize;

        for entry in read_dir.flatten() {
            if let Some(info) = entry_info(entry) {
                batch.push(info);
                total += 1;
                if batch.len() == batch_size {
                    send_batch(&stream_id, &mut batch)?;
                }
            }
        }
        if !batch.is_empty() {
            send_batch(&stream_id, &mut batch)?;
        }
        Ok(total)
    })();

    match result {
        Ok(total) => emit_stream_event(
            "directory_done",
            serde_json::json!({
                "streamId": stream_id,
                "entries": total,
                "elapsedMs": started.elapsed().as_secs_f64() * 1000.0,
            }),
            None,
        ),
        Err(message) => emit_stream_event(
            "directory_error",
            serde_json::json!({ "streamId": stream_id, "message": message }),
            None,
        ),
    }
}

fn send_batch(
    stream_id: &str,
    batch: &mut Vec<DirectoryEntryInfo>,
) -> std::result::Result<(), String> {
    let count = batch.len();
    let bytes = serde_json::to_vec(batch).map_err(|error| error.to_string())?;
    batch.clear();
    emit_stream_event(
        "directory_batch",
        serde_json::json!({ "streamId": stream_id, "count": count }),
        Some(bytes),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Terminal
// ---------------------------------------------------------------------------

static SESSIONS: OnceLock<Mutex<HashMap<u32, TerminalSession>>> = OnceLock::new();
static NEXT_SESSION_ID: AtomicU32 = AtomicU32::new(1);

fn sessions() -> &'static Mutex<HashMap<u32, TerminalSession>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalCreateArgs {
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    program: Option<String>,
    args: Option<Vec<String>>,
}

fn shell_command(program: Option<String>, args: Option<Vec<String>>) -> CommandBuilder {
    let program = program.unwrap_or_else(default_program);
    let mut command = CommandBuilder::new(program);
    if let Some(args) = args {
        for arg in args {
            command.arg(arg);
        }
    }
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

fn create_terminal(args: TerminalCreateArgs) -> std::result::Result<u32, String> {
    let mut command = shell_command(args.program, args.args);
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
    eprintln!("[dae-native] terminal spawned pid={pid:?}");
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

    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let pending = std::sync::Arc::new(Mutex::new(Vec::<u8>::new()));
    let closed = std::sync::Arc::new(AtomicBool::new(false));
    let exited = std::sync::Arc::new(AtomicBool::new(false));
    let exit_code = std::sync::Arc::new(AtomicI32::new(-1));

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

    thread::spawn({
        let pending = pending.clone();
        let closed = closed.clone();
        let exited = exited.clone();
        let exit_code = exit_code.clone();
        let meta = serde_json::json!({ "sessionId": id }).to_string();
        move || {
            // ConPTY can keep the output pipe open after the shell exits, so a
            // reader EOF is not a reliable end-of-session signal on Windows.
            // Once the child is gone, give the reader a short grace period to
            // drain what the console buffered, then close the session anyway.
            let mut drain_deadline: Option<Instant> = None;
            loop {
                thread::sleep(Duration::from_millis(FLUSH_INTERVAL_MS));
                if let Some(chunk) = drain_pending(&pending, MAX_BATCH_BYTES) {
                    emit(RawEvent {
                        name: "terminal_output".to_owned(),
                        target: None,
                        payload_json: Some(meta.clone()),
                        data: Some(chunk),
                    });
                }

                let exited_now = exited.load(Ordering::Acquire);
                let reader_closed = closed.load(Ordering::Acquire);
                if !exited_now {
                    continue;
                }

                let deadline = *drain_deadline.get_or_insert_with(|| {
                    Instant::now() + Duration::from_millis(400)
                });
                if !reader_closed && Instant::now() < deadline {
                    continue;
                }

                if let Some(rest) = drain_pending(&pending, usize::MAX) {
                    emit(RawEvent {
                        name: "terminal_output".to_owned(),
                        target: None,
                        payload_json: Some(meta.clone()),
                        data: Some(rest),
                    });
                }
                let code = exit_code.load(Ordering::Acquire);
                emit_stream_event(
                    "terminal_exit",
                    serde_json::json!({ "sessionId": id, "exitCode": code.max(0) }),
                    None,
                );
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

fn kill_session(session: &mut TerminalSession) {
    // Taskkill first: it needs the root process alive to walk the tree. A bare
    // TerminateProcess (the killer's implementation) leaves children behind,
    // and the acceptance script checks for exactly that.
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

fn write_terminal(id: u32, data: &str) -> std::result::Result<(), String> {
    let mut sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| "term.session_missing".to_owned())?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|error| format!("term.write_failed: {error}"))?;    session
        .writer
        .flush()
        .map_err(|error| format!("term.flush_failed: {error}"))
}

fn resize_terminal(id: u32, cols: u16, rows: u16) -> std::result::Result<(), String> {
    let sessions = sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = sessions
        .get(&id)
        .ok_or_else(|| "term.session_missing".to_owned())?;
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

fn kill_terminal(id: u32) -> bool {
    let mut session = match sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&id)
    {
        Some(session) => session,
        None => return false,
    };
    kill_session(&mut session);
    true
}

fn kill_all_terminals() {
    let drained: Vec<TerminalSession> = {
        let mut sessions = sessions()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.drain().map(|(_, session)| session).collect()
    };
    for mut session in drained {
        kill_session(&mut session);
    }
}

// ---------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------

#[napi(js_name = "thumbnail")]
pub async fn thumbnail(path: String, size: u32) -> Result<Buffer> {
    let bytes = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, String> {
        let image = image::open(&path).map_err(|error| format!("thumbnail: {error}"))?;
        let edge = size.clamp(1, 1024);
        let thumb = image.thumbnail(edge, edge);
        let mut out = Cursor::new(Vec::<u8>::new());
        thumb
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|error| format!("thumbnail encode: {error}"))?;
        Ok(out.into_inner())
    })
    .await
    .map_err(|error| Error::from_reason(format!("thumbnail join: {error}")))?
    .map_err(Error::from_reason)?;

    Ok(Buffer::from(bytes))
}

// ---------------------------------------------------------------------------
// Command surface
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadDirectoryArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartListingArgs {
    path: String,
    stream_id: String,
    #[serde(default = "default_batch_size")]
    batch_size: usize,
}

fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}

#[napi(js_name = "getHomeDirectory")]
pub fn get_home_directory() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_owned())
}

#[napi(js_name = "invoke")]
pub async fn invoke(command: String, args_json: String) -> Result<String> {
    let args: serde_json::Value = serde_json::from_str(&args_json)
        .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;

    match command.as_str() {
        "ping" => Ok(args_json),
        "get_home_directory" => Ok(serde_json::to_string(&get_home_directory())
            .map_err(|error| Error::from_reason(error.to_string()))?),
        "read_directory" => {
            let ReadDirectoryArgs { path } = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let listing = tokio::task::spawn_blocking(move || read_directory_impl(&path))
                .await
                .map_err(|error| Error::from_reason(format!("listing join: {error}")))?
                .map_err(Error::from_reason)?;
            Ok(serde_json::to_string(&listing)
                .map_err(|error| Error::from_reason(error.to_string()))?)
        }
        "start_directory_listing" => {
            let StartListingArgs {
                path,
                stream_id,
                batch_size,
            } = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            thread::spawn(move || stream_directory(path, stream_id, batch_size));
            Ok("{\"started\":true}".to_owned())
        }
        "terminal_create" => {
            let args: TerminalCreateArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let id = create_terminal(args).map_err(Error::from_reason)?;
            Ok(serde_json::to_string(&id).map_err(|error| Error::from_reason(error.to_string()))?)
        }
        "terminal_write" => {
            let id = args.get("id").and_then(|value| value.as_u64());
            let data = args
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let id = id.ok_or_else(|| Error::from_reason("terminal_write: missing id"))? as u32;
            write_terminal(id, data).map_err(Error::from_reason)?;
            Ok("true".to_owned())
        }
        "terminal_resize" => {
            let id = args
                .get("id")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| Error::from_reason("terminal_resize: missing id"))? as u32;
            let cols = args
                .get("cols")
                .and_then(|value| value.as_u64())
                .unwrap_or(80) as u16;
            let rows = args
                .get("rows")
                .and_then(|value| value.as_u64())
                .unwrap_or(24) as u16;
            resize_terminal(id, cols, rows).map_err(Error::from_reason)?;
            Ok("true".to_owned())
        }
        "terminal_kill" => {
            let id = args
                .get("id")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| Error::from_reason("terminal_kill: missing id"))? as u32;
            Ok(serde_json::to_string(&kill_terminal(id))
                .map_err(|error| Error::from_reason(error.to_string()))?)
        }
        other => Err(Error::from_reason(format!("unknown command: {other}"))),
    }
}

/// Returns raw JSON bytes for commands whose payload must not be parsed on the
/// Electron side. Phase 0 only needs the directory listing here.
#[napi(js_name = "invokeRaw")]
pub async fn invoke_raw(command: String, args_json: String) -> Result<Option<Buffer>> {
    match command.as_str() {
        "read_directory" => {
            let ReadDirectoryArgs { path } = serde_json::from_str(&args_json)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let bytes = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, String> {
                let listing = read_directory_impl(&path)?;
                serde_json::to_vec(&listing).map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| Error::from_reason(format!("listing join: {error}")))?
            .map_err(Error::from_reason)?;
            Ok(Some(Buffer::from(bytes)))
        }
        other => Err(Error::from_reason(format!(
            "unknown raw command: {other}"
        ))),
    }
}

/// Kills every PTY session. The core process calls this before exiting so no
/// shell survives the app; the acceptance script verifies that independently.
#[napi(js_name = "shutdown")]
pub fn shutdown() -> bool {
    kill_all_terminals();
    true
}
