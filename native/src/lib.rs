//! N-API core process for the dae Electron shell.
//!
//! The file-system surface is `dae-core`'s, not a copy of it: listings, packed
//! batches, and change patches all come from the same crate the Tauri shell
//! links. The N-API layer here only owns the transport — a pull-based event
//! queue plus channels for payloads the renderer wants as raw buffers.
//!
//! PTY sessions and image thumbnail decoding stay here because `dae-core` is a
//! file-system crate; the renderer calls these directly.

mod directory_size;
mod disks;
mod file_ops;
mod hashing;
mod listing;
mod media_meta;
mod preview;
mod recents;
mod search;
mod settings;
mod shell;
mod shell_commands;
mod system;
mod terminal;
mod volumes;
mod watch;

use std::io::Cursor;
use std::sync::OnceLock;

use dae_core::file_system::git::GitStatusCache;
use napi::bindgen_prelude::Buffer;
use napi::{Error, Result};
use napi_derive::napi;
use serde::Deserialize;
use tokio::sync::mpsc;

/// One cache for every Git read and watcher notification in this process.
pub(crate) fn git_cache() -> &'static GitStatusCache {
    static GIT_CACHE: OnceLock<GitStatusCache> = OnceLock::new();
    GIT_CACHE.get_or_init(GitStatusCache::default)
}

/// The config directory the Electron shell hands down. It mirrors Tauri's
/// `app_config_dir` (the identifier under the platform config root) so a
/// migrated install keeps reading the same files.
pub(crate) fn config_dir() -> std::path::PathBuf {
    std::env::var_os("DAE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::config_dir().map(|root| root.join("com.melaine.dae")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

pub(crate) struct RawEvent {
    name: String,
    target: Option<String>,
    payload_json: Option<String>,
    data: Option<Vec<u8>>,
    channel_id: Option<u32>,
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
    let tx = event_tx();
    // Commands emit from both blocking threads and the async runtime's own
    // workers; `blocking_send` panics on the latter. Async callers use the
    // non-blocking send and drop under a stalled pump instead of deadlocking.
    if tokio::runtime::Handle::try_current().is_ok() {
        let _ = tx.try_send(event);
    } else {
        let _ = tx.blocking_send(event);
    }
}

pub(crate) fn emit_event(name: &str, payload_json: String) {
    emit(RawEvent {
        name: name.to_owned(),
        target: None,
        payload_json: Some(payload_json),
        data: None,
        channel_id: None,
    });
}

/// Sends a message to one of the renderer's `Channel`s. Exactly one of
/// `payload_json` (a JSON value) or `data` (a raw buffer) is set.
pub(crate) fn emit_channel(channel_id: u32, payload_json: Option<String>, data: Option<Vec<u8>>) {
    emit(RawEvent {
        name: String::new(),
        target: None,
        payload_json,
        data,
        channel_id: Some(channel_id),
    });
}

#[napi(object)]
pub struct CoreEvent {
    pub name: String,
    pub target: Option<String>,
    pub payload_json: Option<String>,
    pub data: Option<Buffer>,
    pub channel_id: Option<u32>,
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
        channel_id: event.channel_id,
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
struct ChannelRef {
    #[serde(rename = "__dae_channel__")]
    id: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadDirectoryArgs {
    path: String,
    stream_id: String,
    #[serde(rename = "watcherId")]
    watcher_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadDirectoryPacketsArgs {
    path: String,
    stream_id: String,
    #[serde(rename = "watcherId")]
    watcher_id: Option<String>,
    on_batch: ChannelRef,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadDirectoryChangesArgs {
    directory: String,
    names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelListingArgs {
    stream_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordRecentArgs {
    path: String,
    kind: dae_core::file_system::types::EntryKind,
    source: recents::RecentSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DirectorySizeArgs {
    operation_id: String,
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelDirectorySizeArgs {
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchDirectoryArgs {
    path: String,
    query: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchFileContentsArgs {
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalCreateArgs {
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_output: Option<ChannelRef>,
    on_exit: Option<ChannelRef>,
}

#[napi(js_name = "getHomeDirectory")]
pub fn get_home_directory() -> String {
    dirs::home_dir()
        .or_else(|| std::env::var_os("USERPROFILE").map(Into::into))
        .or_else(|| std::env::var_os("HOME").map(Into::into))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_owned())
}

fn required_string(args: &serde_json::Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| Error::from_reason(format!("missing {key}")))
}

/// Starts the platform observers that report changes the renderer subscribes
/// to: system accent, desktop theme, and volume arrival/removal. Safe to call
/// once; repeated calls are ignored by the modules themselves.
#[napi(js_name = "initialize")]
pub fn initialize() -> bool {
    settings::ensure_loaded();
    system::start();
    volumes::start();
    true
}

/// Applies the renderer's appearance choice to the Electron window's system
/// backdrop. `hwnd` is the decimal HWND Electron reports for the window.
#[napi(js_name = "setWindowAppearance")]
pub fn set_window_appearance(hwnd: f64, dark: bool) -> bool {
    shell::set_window_appearance(hwnd as isize, dark)
}

async fn dispatch(command: &str, args: serde_json::Value) -> Result<serde_json::Value> {
    match command {
        "get_home_directory" => Ok(serde_json::Value::String(get_home_directory())),

        "read_directory" => {
            let args: ReadDirectoryArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let view = tokio::task::spawn_blocking(move || {
                listing::open(
                    args.path,
                    args.stream_id,
                    args.watcher_id,
                    listing::Sink::Events,
                )
            })
            .await
            .map_err(|error| Error::from_reason(format!("listing join: {error}")))?
            .map_err(Error::from_reason)?;
            serde_json::to_value(view).map_err(|error| Error::from_reason(error.to_string()))
        }

        "read_directory_packets" => {
            let args: ReadDirectoryPacketsArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let view = tokio::task::spawn_blocking(move || {
                listing::open(
                    args.path,
                    args.stream_id,
                    args.watcher_id,
                    listing::Sink::Channel(args.on_batch.id),
                )
            })
            .await
            .map_err(|error| Error::from_reason(format!("listing join: {error}")))?
            .map_err(Error::from_reason)?;
            serde_json::to_value(view).map_err(|error| Error::from_reason(error.to_string()))
        }

        "read_directory_changes" => {
            let args: ReadDirectoryChangesArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let changes = tokio::task::spawn_blocking(move || {
                listing::changes(args.directory, args.names)
            })
            .await
            .map_err(|error| Error::from_reason(format!("changes join: {error}")))?
            .map_err(Error::from_reason)?;
            serde_json::to_value(changes).map_err(|error| Error::from_reason(error.to_string()))
        }

        "cancel_directory_listing" => {
            let args: CancelListingArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            listing::cancel(&args.stream_id);
            Ok(serde_json::Value::Null)
        }

        "list_recents" => {
            let items = tokio::task::spawn_blocking(recents::list)
                .await
                .map_err(|error| Error::from_reason(format!("recents join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(items).map_err(|error| Error::from_reason(error.to_string()))
        }

        "record_recent" => {
            let args: RecordRecentArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let items = tokio::task::spawn_blocking(move || {
                recents::record(args.path, args.kind, args.source)
            })
            .await
            .map_err(|error| Error::from_reason(format!("recents join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(items).map_err(|error| Error::from_reason(error.to_string()))
        }

        "remove_recent" => {
            let path = required_string(&args, "path")?;
            let items = tokio::task::spawn_blocking(move || recents::remove(path))
                .await
                .map_err(|error| Error::from_reason(format!("recents join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(items).map_err(|error| Error::from_reason(error.to_string()))
        }

        "clear_recents" => {
            tokio::task::spawn_blocking(recents::clear)
                .await
                .map_err(|error| Error::from_reason(format!("recents join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "read_text_preview" => {
            let path = required_string(&args, "path")?;
            let max_bytes = args
                .get("maxBytes")
                .and_then(|value| value.as_u64())
                .unwrap_or(256 * 1024) as u32;
            let preview = tokio::task::spawn_blocking(move || {
                preview::read_text_preview(path, max_bytes)
            })
            .await
            .map_err(|error| Error::from_reason(format!("preview join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(preview).map_err(|error| Error::from_reason(error.to_string()))
        }

        "read_media_preview" => {
            let path = required_string(&args, "path")?;
            let preview = tokio::task::spawn_blocking(move || media_meta::read_media_preview(path))
                .await
                .map_err(|error| Error::from_reason(format!("media preview join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(preview).map_err(|error| Error::from_reason(error.to_string()))
        }

        "search_directory" => {
            let args: SearchDirectoryArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let generation = search::begin();
            let response = tokio::task::spawn_blocking(move || {
                search::directory(args.path, args.query, generation)
            })
            .await
            .map_err(|error| Error::from_reason(format!("search join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(response).map_err(|error| Error::from_reason(error.to_string()))
        }

        "search_file_contents" => {
            let args: SearchFileContentsArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let generation = search::begin();
            let response = tokio::task::spawn_blocking(move || {
                search::file_contents(
                    args.path,
                    args.query,
                    args.is_regex,
                    args.case_sensitive,
                    args.file_filter,
                    generation,
                )
            })
            .await
            .map_err(|error| Error::from_reason(format!("search join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(response).map_err(|error| Error::from_reason(error.to_string()))
        }

        "cancel_search" => {
            search::cancel();
            Ok(serde_json::Value::Null)
        }

        "start_directory_size_calculation" => {
            let args: DirectorySizeArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            directory_size::start(args.operation_id, args.paths)
                .map_err(Error::from_reason)?;
            Ok(serde_json::Value::Null)
        }

        "cancel_directory_size_calculation" => {
            let args: CancelDirectorySizeArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            directory_size::cancel(&args.operation_id);
            Ok(serde_json::Value::Null)
        }

        "start_file_hash_calculation" => {
            let operation_id = required_string(&args, "operationId")?;
            let path = required_string(&args, "path")?;
            hashing::start(operation_id, path)
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "cancel_file_hash_calculation" => {
            let operation_id = required_string(&args, "operationId")?;
            hashing::cancel(&operation_id);
            Ok(serde_json::Value::Null)
        }

        "unwatch_directory" => {
            let watcher_id = args
                .get("watcherId")
                .and_then(|value| value.as_str())
                .ok_or_else(|| Error::from_reason("unwatch_directory: missing watcherId"))?;
            watch::unwatch(watcher_id);
            Ok(serde_json::Value::Null)
        }

        "load_settings" => {
            let settings = settings::load()
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(settings).map_err(|error| Error::from_reason(error.to_string()))
        }

        "save_settings" => {
            let settings: settings::AppSettings = serde_json::from_value(
                args.get("settings")
                    .cloned()
                    .ok_or_else(|| Error::from_reason("save_settings: missing settings"))?,
            )
            .map_err(|error| Error::from_reason(format!("invalid settings: {error}")))?;
            let saved = settings::save(&settings)
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(saved).map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_system_accent" => {
            serde_json::to_value(system::accent())
                .map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_system_theme" => {
            serde_json::to_value(system::theme())
                .map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_git_status" => {
            let path = required_string(&args, "path")?;
            let status = tokio::task::spawn_blocking(move || {
                dae_core::file_system::git::get_git_status(path, git_cache())
            })
            .await
            .map_err(|error| Error::from_reason(format!("git join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(status).map_err(|error| Error::from_reason(error.to_string()))
        }

        "list_git_branches" => {
            let path = required_string(&args, "path")?;
            let branches = tokio::task::spawn_blocking(move || {
                dae_core::file_system::git::list_git_branches(path)
            })
            .await
            .map_err(|error| Error::from_reason(format!("git join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(branches).map_err(|error| Error::from_reason(error.to_string()))
        }

        "git_checkout_branch" => {
            let path = required_string(&args, "path")?;
            let name = required_string(&args, "name")?;
            let remote = args
                .get("remote")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            tokio::task::spawn_blocking(move || {
                dae_core::file_system::git::git_checkout_branch(path, name, remote)
            })
            .await
            .map_err(|error| Error::from_reason(format!("git join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "git_create_branch" => {
            let path = required_string(&args, "path")?;
            let name = required_string(&args, "name")?;
            let checkout = args
                .get("checkout")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            tokio::task::spawn_blocking(move || {
                dae_core::file_system::git::git_create_branch(path, name, checkout)
            })
            .await
            .map_err(|error| Error::from_reason(format!("git join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "git_fetch" | "git_push" | "git_pull" | "git_sync" => {
            let path = required_string(&args, "path")?;
            let command = command.to_owned();
            tokio::task::spawn_blocking(move || match command.as_str() {
                "git_fetch" => dae_core::file_system::git::git_fetch(path),
                "git_push" => dae_core::file_system::git::git_push(path),
                "git_pull" => dae_core::file_system::git::git_pull(path),
                _ => dae_core::file_system::git::git_sync(path),
            })
            .await
            .map_err(|error| Error::from_reason(format!("git join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "list_disks" => {
            let volumes = tokio::task::spawn_blocking(disks::list_disks)
                .await
                .map_err(|error| Error::from_reason(format!("disk join: {error}")))?
                .map_err(Error::from_reason)?;
            serde_json::to_value(volumes).map_err(|error| Error::from_reason(error.to_string()))
        }

        "list_wsl_distros" => {
            let distros = tokio::task::spawn_blocking(disks::list_wsl_distros)
                .await
                .map_err(|error| Error::from_reason(format!("wsl join: {error}")))?
                .map_err(Error::from_reason)?;
            serde_json::to_value(distros).map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_system_places" => {
            serde_json::to_value(shell::system_places())
                .map_err(|error| Error::from_reason(error.to_string()))
        }

        // The remaining workspace stores still live in the Tauri shell. Empty
        // lists let the real renderer paint its empty states instead of error
        // panels.
        "load_favorites" | "list_spaces" => {
            Ok(serde_json::Value::Array(Vec::new()))
        }

        "terminal_create" => {
            let args: TerminalCreateArgs = serde_json::from_value(args)
                .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
            let session = terminal::create(terminal::CreateArgs {
                cwd: args.cwd,
                cols: args.cols,
                rows: args.rows,
                on_output: args.on_output.map(|channel| channel.id),
                on_exit: args.on_exit.map(|channel| channel.id),
            })
            .map_err(Error::from_reason)?;
            serde_json::to_value(session).map_err(|error| Error::from_reason(error.to_string()))
        }

        "terminal_write" => {
            let id = args
                .get("id")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| Error::from_reason("terminal_write: missing id"))? as u32;
            let data = args
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            terminal::write(id, data).map_err(Error::from_reason)?;
            Ok(serde_json::Value::Null)
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
            terminal::resize(id, cols, rows).map_err(Error::from_reason)?;
            Ok(serde_json::Value::Null)
        }

        "terminal_kill" => {
            let id = args
                .get("id")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| Error::from_reason("terminal_kill: missing id"))? as u32;
            Ok(serde_json::Value::Bool(terminal::kill(id)))
        }

        "list_shell_commands" => {
            let paths: Vec<String> = args
                .get("paths")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("list_shell_commands: {error}")))?
                .unwrap_or_default();
            let primary = required_string(&args, "primary")?;
            let items = shell_commands::list_shell_commands(paths, primary)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(items).map_err(|error| Error::from_reason(error.to_string()))
        }

        "warm_shell_commands" => {
            shell_commands::warm_shell_commands()
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "invoke_shell_command" => {
            let id = required_string(&args, "id")?;
            let paths: Vec<String> = args
                .get("paths")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("invoke_shell_command: {error}")))?
                .unwrap_or_default();
            shell_commands::invoke_shell_command(id, paths)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "wait_for_tab_drag_release" => {
            let timeout_ms = args
                .get("timeoutMs")
                .and_then(|value| value.as_u64())
                .unwrap_or(60_000);
            Ok(serde_json::Value::Bool(
                file_ops::wait_for_mouse_release(timeout_ms).await,
            ))
        }

        other => match file_ops::dispatch(other, &args).await {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err(Error::from_reason(format!("unknown command: {other}"))),
            Err(error) => Err(Error::from_reason(error)),
        },
    }
}

#[napi(js_name = "invoke")]
pub async fn invoke(command: String, args_json: String) -> Result<String> {
    let args: serde_json::Value = serde_json::from_str(&args_json)
        .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
    let value = dispatch(&command, args).await?;
    serde_json::to_string(&value).map_err(|error| Error::from_reason(error.to_string()))
}

/// Returns the same JSON as `invoke` without parsing it on the Electron side.
#[napi(js_name = "invokeRaw")]
pub async fn invoke_raw(command: String, args_json: String) -> Result<Option<Buffer>> {
    let args: serde_json::Value = serde_json::from_str(&args_json)
        .map_err(|error| Error::from_reason(format!("invalid args: {error}")))?;
    let value = dispatch(&command, args).await?;
    let bytes = serde_json::to_vec(&value).map_err(|error| Error::from_reason(error.to_string()))?;
    Ok(Some(Buffer::from(bytes)))
}

/// Kills every PTY session. The core process calls this before exiting so no
/// shell survives the app.
#[napi(js_name = "shutdown")]
pub fn shutdown() -> bool {
    terminal::kill_all();
    watch::release_all();
    true
}
