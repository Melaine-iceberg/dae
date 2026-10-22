//! N-API core process for the dae Electron shell.
//!
//! The file-system surface is `dae-core`'s, not a copy of it: listings, packed
//! batches, and change patches all come from the same crate the Tauri shell
//! links. The N-API layer here only owns the transport — a pull-based event
//! queue plus channels for payloads the renderer wants as raw buffers.
//!
//! PTY sessions and image thumbnail decoding stay here because `dae-core` is a
//! file-system crate; the renderer calls these directly.

mod archive;
mod default_manager;
mod directory_size;
mod disks;
mod file_ops;
mod hashing;
mod listing;
mod media_meta;
mod network;
mod open_with;
mod preview;
mod recents;
mod render_pool;
mod search;
mod settings;
mod shell;
mod shell_commands;
mod snap_layout;
mod system;
mod system_files;
mod terminal;
mod thumbnails;
mod trash;
mod volumes;
mod watch;
mod workspace;

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

/// One rendered image with the MIME type its bytes have.
#[napi(object)]
pub struct RenderedImage {
    pub mime: String,
    pub bytes: Buffer,
}

impl From<thumbnails::Rendered> for RenderedImage {
    fn from(rendered: thumbnails::Rendered) -> Self {
        Self {
            mime: rendered.mime.to_owned(),
            bytes: Buffer::from(rendered.bytes),
        }
    }
}

/// Runs one blocking render on the dedicated pool, which keeps decodes and
/// shell COM off both the Node event loop and the listing pool.
async fn run_on_render_pool<T, F>(job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    render_pool::submit(Box::new(move || {
        let _ = sender.send(job());
    }));

    receiver.await.map_err(|error| {
        Error::from_reason(format!("render worker stopped before the job ran: {error}"))
    })
}

/// The `thumbnail://` response body: null is a 404 the frontend falls back
/// from, an error is a failed read the protocol turns into a 500.
#[napi(js_name = "thumbnail")]
pub async fn thumbnail(path: String, size: u32) -> Result<Option<RenderedImage>> {
    let rendered = run_on_render_pool(move || thumbnails::render_thumbnail(&path, size))
        .await?
        .map_err(Error::from_reason)?;
    Ok(rendered.map(RenderedImage::from))
}

/// The `fileicon://` response for one path, file or folder.
#[napi(js_name = "fileIcon")]
pub async fn file_icon(
    path: String,
    is_dir: bool,
    size: u32,
) -> Result<Option<RenderedImage>> {
    let rendered =
        run_on_render_pool(move || thumbnails::render_file_icon(&path, is_dir, size)).await?;
    Ok(rendered.map(RenderedImage::from))
}

/// The `fileicon://` response for a whole type, named by its extension.
#[napi(js_name = "typeIcon")]
pub async fn type_icon(extension: String, size: u32) -> Result<Option<RenderedImage>> {
    let rendered =
        run_on_render_pool(move || thumbnails::render_type_icon(&extension, size)).await?;
    Ok(rendered.map(RenderedImage::from))
}

/// The `fileicon://` response for a freedesktop icon-theme name.
#[napi(js_name = "namedIcon")]
pub async fn named_icon(name: String, size: u32) -> Result<Option<RenderedImage>> {
    let rendered = run_on_render_pool(move || thumbnails::render_named_icon(&name, size)).await?;
    Ok(rendered.map(RenderedImage::from))
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

fn convert_arg<T: serde::de::DeserializeOwned>(
    args: &serde_json::Value,
    key: &str,
    command: &str,
) -> Result<T> {
    args.get(key)
        .cloned()
        .ok_or_else(|| Error::from_reason(format!("{command}: missing {key}")))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|error| Error::from_reason(format!("{command}: {error}")))
        })
}

/// Starts the platform observers that report changes the renderer subscribes
/// to: system accent, desktop theme, and volume arrival/removal. Safe to call
/// once; repeated calls are ignored by the modules themselves.
#[napi(js_name = "initialize")]
pub fn initialize() -> bool {
    settings::ensure_loaded();
    network::init(config_dir());
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

/// Starts the OLE drag-out on the Electron main thread. The core process must
/// not run this: DoDragDrop needs the message loop of the window's own thread,
/// so the main process loads this addon and calls the function synchronously.
#[napi(js_name = "startFileDrag")]
pub fn start_file_drag(paths: Vec<String>, mode: String, hwnd: f64) -> Result<bool> {
    system_files::start_drag_out(paths, &mode, hwnd as isize)
        .map(|()| true)
        .map_err(|error| Error::from_reason(error.to_string()))
}

/// Validates a macOS "应用扩展" row and returns the service title it runs.
/// Called by the Electron main process before it queues `performShellService`.
#[napi(js_name = "resolveShellService")]
pub fn resolve_shell_service(id: String) -> Result<String> {
    shell_commands::resolve_service_title(&id)
        .map_err(|error| Error::from_reason(error.to_string()))
}

/// Runs a macOS service through `NSPerformService`. Only the Electron main
/// process may call this: AppKit reaches the services machinery through its
/// application object and UI thread. Blocks until the service finishes.
#[napi(js_name = "performShellService")]
pub fn perform_shell_service(title: String, paths: Vec<String>) -> Result<bool> {
    shell_commands::perform_service(&title, &paths)
        .map_err(|error| Error::from_reason(error.to_string()))
}

/// Lays a Windows 11 snap-layout hotspot over the maximize button. `x`, `y`,
/// `width` and `height` are CSS pixels inside the window's client area; the
/// main process calls this on its own (UI) thread whenever the button moves.
#[napi(js_name = "installSnapLayout")]
pub fn install_snap_layout(hwnd: f64, x: i32, y: i32, width: i32, height: i32) -> bool {
    snap_layout::install(hwnd as isize, x, y, width, height)
}

/// Removes the snap-layout hotspot; called when the button unmounts.
#[napi(js_name = "destroySnapLayout")]
pub fn destroy_snap_layout(hwnd: f64) -> bool {
    snap_layout::destroy(hwnd as isize)
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
            serde_json::to_value(system::accent().await)
                .map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_system_theme" => {
            serde_json::to_value(system::theme().await)
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

        "get_package_family_name" => {
            Ok(serde_json::Value::String(system_files::package_family_name()))
        }

        "get_key_modifiers" => serde_json::to_value(system_files::get_key_modifiers())
            .map_err(|error| Error::from_reason(error.to_string())),

        "open_terminal" => {
            let path = required_string(&args, "path")?;
            tokio::task::spawn_blocking(move || system_files::open_terminal(path))
                .await
                .map_err(|error| Error::from_reason(format!("terminal join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "write_files_to_clipboard" => {
            let paths: Vec<String> = args
                .get("paths")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| {
                    Error::from_reason(format!("write_files_to_clipboard: {error}"))
                })?
                .unwrap_or_default();
            let cut = args
                .get("cut")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            tokio::task::spawn_blocking(move || {
                system_files::write_files_to_clipboard(paths, cut)
            })
            .await
            .map_err(|error| Error::from_reason(format!("clipboard join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "read_files_from_clipboard" => {
            let files = tokio::task::spawn_blocking(system_files::read_files_from_clipboard)
                .await
                .map_err(|error| Error::from_reason(format!("clipboard join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(files).map_err(|error| Error::from_reason(error.to_string()))
        }

        "create_shortcuts" => {
            let sources: Vec<String> = args
                .get("sources")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("create_shortcuts: {error}")))?
                .unwrap_or_default();
            let destination = required_string(&args, "destination")?;
            let created = tokio::task::spawn_blocking(move || {
                system_files::create_shortcuts(sources, destination)
            })
            .await
            .map_err(|error| Error::from_reason(format!("shortcut join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(created).map_err(|error| Error::from_reason(error.to_string()))
        }

        "load_favorites" => {
            let favorites = tokio::task::spawn_blocking(workspace::load_favorites)
                .await
                .map_err(|error| Error::from_reason(format!("favorites join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(favorites).map_err(|error| Error::from_reason(error.to_string()))
        }

        "save_favorites" => {
            let favorites: Vec<workspace::Favorite> = args
                .get("favorites")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("save_favorites: {error}")))?
                .unwrap_or_default();
            tokio::task::spawn_blocking(move || workspace::save_favorites(favorites))
                .await
                .map_err(|error| Error::from_reason(format!("favorites join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "list_spaces" => {
            let spaces = tokio::task::spawn_blocking(workspace::list_spaces)
                .await
                .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(spaces).map_err(|error| Error::from_reason(error.to_string()))
        }

        "create_space" => {
            let name = required_string(&args, "name")?;
            let space = tokio::task::spawn_blocking(move || workspace::create_space(name))
                .await
                .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(space).map_err(|error| Error::from_reason(error.to_string()))
        }

        "rename_space" => {
            let space_id = required_string(&args, "spaceId")?;
            let name = required_string(&args, "name")?;
            let space =
                tokio::task::spawn_blocking(move || workspace::rename_space(space_id, name))
                    .await
                    .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                    .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(space).map_err(|error| Error::from_reason(error.to_string()))
        }

        "delete_space" => {
            let space_id = required_string(&args, "spaceId")?;
            tokio::task::spawn_blocking(move || workspace::delete_space(space_id))
                .await
                .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "add_space_item" => {
            let space_id = required_string(&args, "spaceId")?;
            let path = required_string(&args, "path")?;
            let space =
                tokio::task::spawn_blocking(move || workspace::add_space_item(space_id, path))
                    .await
                    .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                    .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(space).map_err(|error| Error::from_reason(error.to_string()))
        }

        "remove_space_item" => {
            let space_id = required_string(&args, "spaceId")?;
            let path = required_string(&args, "path")?;
            let space =
                tokio::task::spawn_blocking(move || workspace::remove_space_item(space_id, path))
                    .await
                    .map_err(|error| Error::from_reason(format!("spaces join: {error}")))?
                    .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(space).map_err(|error| Error::from_reason(error.to_string()))
        }

        "list_connections" => {
            let connections = tokio::task::spawn_blocking(network::list_connections)
                .await
                .map_err(|error| Error::from_reason(format!("connections join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(connections).map_err(|error| Error::from_reason(error.to_string()))
        }

        "save_connection" => {
            let input: dae_core::file_system::connections::SaveConnectionInput = args
                .get("input")
                .cloned()
                .ok_or_else(|| Error::from_reason("save_connection: missing input"))
                .and_then(|value| {
                    serde_json::from_value(value)
                        .map_err(|error| Error::from_reason(format!("save_connection: {error}")))
                })?;
            let saved = tokio::task::spawn_blocking(move || network::save_connection(input))
                .await
                .map_err(|error| Error::from_reason(format!("connections join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(saved).map_err(|error| Error::from_reason(error.to_string()))
        }

        "delete_connection" => {
            let id = required_string(&args, "id")?;
            tokio::task::spawn_blocking(move || network::delete_connection(id))
                .await
                .map_err(|error| Error::from_reason(format!("connections join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "test_connection" => {
            let input = convert_arg::<dae_core::file_system::connections::SaveConnectionInput>(
                &args,
                "input",
                "test_connection",
            )?;
            network::test_connection(input)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "list_cloud_accounts" => {
            let accounts = tokio::task::spawn_blocking(network::list_cloud_accounts)
                .await
                .map_err(|error| Error::from_reason(format!("cloud join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(accounts).map_err(|error| Error::from_reason(error.to_string()))
        }

        "delete_cloud_account" => {
            let id = required_string(&args, "id")?;
            tokio::task::spawn_blocking(move || network::delete_cloud_account(id))
                .await
                .map_err(|error| Error::from_reason(format!("cloud join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "authorize_cloud_account" => {
            let input = convert_arg::<
                dae_core::file_system::cloud::oauth::AuthorizeCloudAccountInput,
            >(&args, "input", "authorize_cloud_account")?;
            let account = network::authorize_cloud_account(input)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(account).map_err(|error| Error::from_reason(error.to_string()))
        }

        "get_default_file_manager_status" => {
            let status = tokio::task::spawn_blocking(default_manager::get_status)
                .await
                .map_err(|error| Error::from_reason(format!("default manager join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(status).map_err(|error| Error::from_reason(error.to_string()))
        }

        "set_default_file_manager" => {
            let status = tokio::task::spawn_blocking(default_manager::set)
                .await
                .map_err(|error| Error::from_reason(format!("default manager join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(status).map_err(|error| Error::from_reason(error.to_string()))
        }

        "unset_default_file_manager" => {
            let status = tokio::task::spawn_blocking(default_manager::unset)
                .await
                .map_err(|error| Error::from_reason(format!("default manager join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(status).map_err(|error| Error::from_reason(error.to_string()))
        }

        "open_with" => {
            let path = required_string(&args, "path")?;
            tokio::task::spawn_blocking(move || open_with::open_with(path))
                .await
                .map_err(|error| Error::from_reason(format!("open with join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "list_open_with_apps" => {
            let path = required_string(&args, "path")?;
            let choices = tokio::task::spawn_blocking(move || open_with::list_open_with_apps(path))
                .await
                .map_err(|error| Error::from_reason(format!("open with join: {error}")))?
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(choices).map_err(|error| Error::from_reason(error.to_string()))
        }

        "open_with_app" => {
            let path = required_string(&args, "path")?;
            let app_id = required_string(&args, "appId")?;
            let set_default = args
                .get("setDefault")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            tokio::task::spawn_blocking(move || {
                open_with::open_with_app(path, app_id, set_default)
            })
            .await
            .map_err(|error| Error::from_reason(format!("open with join: {error}")))?
            .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::Null)
        }

        "list_trash" => {
            let entries = trash::list_trash()
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            serde_json::to_value(entries).map_err(|error| Error::from_reason(error.to_string()))
        }

        "restore_trash_entries" => {
            let ids: Vec<String> = args
                .get("ids")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("restore_trash_entries: {error}")))?
                .unwrap_or_default();
            let operation_id = required_string(&args, "operationId")?;
            let restored = trash::restore_trash_entries(ids, operation_id)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::from(restored))
        }

        "delete_trash_entries" => {
            let ids: Vec<String> = args
                .get("ids")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("delete_trash_entries: {error}")))?
                .unwrap_or_default();
            let operation_id = required_string(&args, "operationId")?;
            let purged = trash::delete_trash_entries(ids, operation_id)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::from(purged))
        }

        "empty_trash" => {
            let operation_id = required_string(&args, "operationId")?;
            let purged = trash::empty_trash(operation_id)
                .await
                .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::from(purged))
        }

        "compress_entries" => {
            let sources: Vec<String> = args
                .get("sources")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::from_reason(format!("compress_entries: {error}")))?
                .unwrap_or_default();
            let destination_dir = required_string(&args, "destinationDir")?;
            let format =
                convert_arg::<archive::ArchiveFormat>(&args, "format", "compress_entries")?;
            let password = args
                .get("password")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let operation_id = required_string(&args, "operationId")?;
            let path =
                archive::compress_entries(sources, destination_dir, format, password, operation_id)
                    .await
                    .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::String(path))
        }

        "extract_archive" => {
            let archive_path = required_string(&args, "archivePath")?;
            let destination_dir = args
                .get("destinationDir")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let password = args
                .get("password")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let operation_id = required_string(&args, "operationId")?;
            let path =
                archive::extract_archive(archive_path, destination_dir, password, operation_id)
                    .await
                    .map_err(|error| Error::from_reason(error.to_string()))?;
            Ok(serde_json::Value::String(path))
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
