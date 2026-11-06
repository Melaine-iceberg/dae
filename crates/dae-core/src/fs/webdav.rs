//! The WebDAV backend: one `WebDavBackend` per server, reached through
//! `webdav://host[:port]/abs/path` (plain) and `webdavs://` (TLS) URLs.
//! Tree-shaped transfers are served by the generic engine in `transfer.rs`;
//! this module implements the single-server primitives, mirroring the SMB and
//! SFTP backends.
//!
//! Unlike SMB and SFTP there is no session to establish: WebDAV is HTTP, so
//! every operation is one request carrying Basic credentials from the
//! connection store (an empty password means anonymous). Two consequences
//! shape the rest of the module:
//!
//! * The HTTP client is process-wide and scheme-agnostic, so the TLS choice is
//!   read from the path of each operation (`webdav://` → `http://`,
//!   `webdavs://` → `https://`) instead of being fixed when the backend opens.
//!   [`open_backend`] therefore performs no round trip, and a server that is
//!   down or a credential that is wrong reports on the first listing.
//! * Paths are held decoded, as everywhere else in the engine, and encoded
//!   per segment only when a request goes out; `href`s coming back are
//!   percent-decoded once.
//!
//! Metadata comes from `PROPFIND`: Depth 1 for a listing, Depth 0 for
//! [`FileSystemBackend::stat`]. Responses are parsed by local element name
//! because servers disagree on the `DAV:` prefix (`d:`, `D:`, `ns0:`, or
//! none). Only `resourcetype`, `getcontentlength` and `getlastmodified` are
//! requested: WebDAV defines no creation time, hidden flag or permission bits
//! that could fill the rest of [`DirectoryEntry`] without a vendor extension,
//! so those stay empty and neutral. `getlastmodified` is RFC 1123 and is
//! parsed by hand — this crate carries no date-time library.
//!
//! Uploading streams through `reqwest`'s blocking client, whose `Body::new`
//! takes a reader: [`open_write`] hands the transfer engine a bounded channel
//! writer, and a worker thread feeds those bytes to a `PUT` as they arrive.
//! The queue is four 64 KiB chunks, so a large copy never sits in memory, and
//! [`flush`](FileSystemBackend::open_write) waits for the server's final status
//! instead of reporting success before the bytes have landed. Creating an
//! empty file uses the same path with no chunks.

use super::connections::{self, Protocol};
use super::error::FileSystemError;
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, NewEntryKind, entry_sort_key,
};
use super::vfs::{FileSystemBackend, SharedBackend};
use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Bounds one connection attempt; a listing or a download issues many requests
/// and only each individual request is bounded by this, never the whole stream.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Upload chunks waiting for the PUT thread before `write` blocks. Four chunks
/// is the same pipeline depth the transfer engine uses for its own streams.
const UPLOAD_QUEUE_DEPTH: usize = 4;

/// Bytes buffered per upload chunk. Large enough that the per-request overhead
/// disappears, small enough that the bounded queue stays a few hundred KB.
const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// The properties a listing needs, in the `DAV:` namespace.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

pub struct WebDavBackend {
    runtime: Arc<tokio::runtime::Runtime>,
    /// `None` for anonymous access, or when the saved connection has no
    /// password; both cases send no `Authorization` header.
    username: Option<String>,
    password: Option<String>,
}

/// The pieces of a `webdav[s]://host[:port]/abs/path` URL after the scheme.
/// There is no share segment (unlike SMB): an empty `sub_path` is the server
/// root, which is the collection the server exposes as its entry point.
struct WebDavPath {
    /// `webdavs://` — TLS, so requests go out over `https://`.
    tls: bool,
    host: String,
    port: Option<u16>,
    /// Path beneath the server root, "" at the root, "/"-separated.
    sub_path: String,
}

impl WebDavPath {
    fn scheme(&self) -> &'static str {
        if self.tls { "webdavs" } else { "webdav" }
    }

    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }

    /// The canonical explorer path, normalizing away trailing separators.
    fn url(&self) -> String {
        if self.sub_path.is_empty() {
            format!("{}://{}", self.scheme(), self.authority())
        } else {
            format!("{}://{}/{}", self.scheme(), self.authority(), self.sub_path)
        }
    }

    /// The absolute URL sent on the wire. `reqwest` speaks HTTP, so the scheme
    /// is `http`/`https` rather than `webdav`/`webdavs`, and each segment is
    /// percent-encoded.
    fn request_url(&self) -> String {
        let origin = format!(
            "{}://{}",
            if self.tls { "https" } else { "http" },
            self.authority()
        );
        if self.sub_path.is_empty() {
            format!("{origin}/")
        } else {
            format!("{origin}/{}", encode_path(&self.sub_path))
        }
    }

    /// The decoded absolute path on the server, used to recognize the
    /// directory's own `href` in a Depth 1 listing.
    fn remote(&self) -> String {
        if self.sub_path.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", self.sub_path)
        }
    }

    fn join(&self, name: &str) -> String {
        if self.sub_path.is_empty() {
            format!("{}://{}/{}", self.scheme(), self.authority(), name)
        } else {
            format!(
                "{}://{}/{}/{}",
                self.scheme(),
                self.authority(),
                self.sub_path,
                name
            )
        }
    }

    fn parent_and_name(&self) -> Result<(String, String), FileSystemError> {
        if self.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "The server root cannot be renamed".into(),
            ));
        }

        let name = self.sub_path.rsplit('/').next().unwrap_or_default();
        let parent_sub = match self.sub_path.rfind('/') {
            Some(index) => &self.sub_path[..index],
            None => "",
        };
        let parent = if parent_sub.is_empty() {
            format!("{}://{}", self.scheme(), self.authority())
        } else {
            format!("{}://{}/{}", self.scheme(), self.authority(), parent_sub)
        };
        Ok((parent, name.to_owned()))
    }
}

fn parse_webdav_path(rest: &str, tls: bool) -> Result<WebDavPath, FileSystemError> {
    let (authority, remainder) = super::smb::split_authority(rest);
    let (host, port) = super::smb::parse_authority(authority)?;

    let sub_path = remainder
        .unwrap_or("")
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/");

    Ok(WebDavPath {
        tls,
        host,
        port,
        sub_path,
    })
}

/// Parses a full explorer path, keeping the `webdavs`/`webdav` distinction
/// that [`super::vfs::split_scheme`] folds into one scheme.
fn parse_dir(path: &str) -> Result<WebDavPath, FileSystemError> {
    let (scheme, rest) = super::vfs::split_scheme(path)?;
    if scheme != super::vfs::Scheme::WebDav {
        return Err(FileSystemError::InvalidInput(format!(
            "Not a WebDAV path: {path}"
        )));
    }

    let tls = path
        .split_once("://")
        .is_some_and(|(prefix, _)| prefix.eq_ignore_ascii_case("webdavs"));
    parse_webdav_path(rest, tls)
}

/// Opens (or reuses) the backend for `webdav://...` URL text and returns it as
/// a backend. Called by `vfs::resolve`.
///
/// No request is made here: a WebDAV backend holds no connection, and the
/// path's TLS choice is only known once an operation names its own path.
pub fn open_backend(rest: &str) -> Result<SharedBackend, FileSystemError> {
    let (authority, _) = super::smb::split_authority(rest);
    let (host, port) = super::smb::parse_authority(authority)?;

    if let Some(existing) = connections::session_for(Protocol::WebDav, &host, port) {
        return Ok(existing);
    }

    let backend: SharedBackend = Arc::new(WebDavBackend::connect(&host, port)?);
    connections::store_session(Protocol::WebDav, &host, port, Arc::clone(&backend));
    Ok(backend)
}

/// Probes a server with the credentials the connect dialog holds, before they
/// are saved. Blocking, because the caller already runs on a blocking thread.
/// Port 443 selects `webdavs`; every other port is plain `webdav`.
pub fn test_backend(
    host: &str,
    port: Option<u16>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), FileSystemError> {
    let host = host.trim().to_owned();
    if host.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "A server host is required".into(),
        ));
    }
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host.clone(),
    };
    let scheme = if port == Some(443) {
        "webdavs"
    } else {
        "webdav"
    };
    let path = format!("{scheme}://{authority}/");
    let backend = WebDavBackend::connect_with(username, password)?;
    backend.read_dir(&path).map(|_| ())
}

impl WebDavBackend {
    /// Builds a backend using credentials from the connection store.
    pub fn connect(host: &str, port: Option<u16>) -> Result<Self, FileSystemError> {
        let (username, password) = connections::resolve_credentials(Protocol::WebDav, host, port);
        Self::connect_with(username.as_deref(), password.as_deref())
    }

    pub fn connect_with(
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, FileSystemError> {
        let runtime = Arc::new(
            tokio::runtime::Runtime::new()
                .map_err(|error| FileSystemError::Io(error.to_string()))?,
        );
        // Fail on a broken TLS stack here rather than on the first listing.
        http_client()?;

        Ok(Self {
            runtime,
            username: username.map(str::to_owned),
            password: password.map(str::to_owned),
        })
    }

    /// Attaches Basic credentials unless the connection is anonymous. An empty
    /// password is the anonymous marker the connect dialog writes, so it must
    /// not turn into a `user:` header.
    fn authenticated(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match (self.username.as_deref(), self.password.as_deref()) {
            (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
                builder.basic_auth(username, Some(password))
            }
            _ => builder,
        }
    }

    fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, FileSystemError> {
        self.runtime.block_on(async {
            request.send().await.map_err(|error| {
                FileSystemError::Io(format!("The WebDAV server did not answer: {error}"))
            })
        })
    }

    /// Rejects a non-2xx answer, keeping the server's own explanation (if any)
    /// in the message.
    fn expect_success(
        &self,
        response: reqwest::Response,
        context: &str,
    ) -> Result<reqwest::Response, FileSystemError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let detail = self
            .runtime
            .block_on(async { response.text().await.unwrap_or_default() });
        Err(map_status(status, context, &detail))
    }

    /// Runs a `PROPFIND` at `depth` ("0" or "1") and returns the resources the
    /// server reported, in its own order.
    fn propfind(
        &self,
        path: &WebDavPath,
        depth: &'static str,
    ) -> Result<Vec<DavResource>, FileSystemError> {
        let request = self
            .authenticated(http_client()?.request(custom_method("PROPFIND")?, path.request_url()))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY);

        let response = self.send(request)?;
        let response = self.expect_success(response, &path.url())?;
        let body = self.runtime.block_on(async {
            response.text().await.map_err(|error| {
                FileSystemError::Io(format!("Could not read the WebDAV response: {error}"))
            })
        })?;

        Ok(parse_multistatus(&body))
    }

    /// The metadata of one path, or `None` when the server says it is absent.
    fn resource_at(&self, path: &str) -> Result<Option<DavResource>, FileSystemError> {
        let parsed = parse_dir(path)?;
        match self.propfind(&parsed, "0") {
            Ok(resources) => Ok(resources.into_iter().next()),
            Err(FileSystemError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `MOVE` keeps the bytes on the server, so a rename inside one collection
    /// costs one round trip. An existing destination is refused
    /// (`Overwrite: F`), matching the SMB backend; the transfer engine decides
    /// whether a destination should be replaced before it gets here.
    fn move_entry(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from = parse_dir(source)?;
        let to = parse_dir(destination)?;
        if from.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "The server root cannot be moved".into(),
            ));
        }

        let request = self
            .authenticated(http_client()?.request(custom_method("MOVE")?, from.request_url()))
            .header("Destination", to.request_url())
            .header("Overwrite", "F");

        let response = self.send(request)?;
        self.expect_success(response, &from.url()).map(|_| ())
    }
}

impl FileSystemBackend for WebDavBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let parsed = parse_dir(path)?;
        // Depth 1 reports the collection itself first; it is not an entry.
        let requested = parsed.remote();
        let requested = requested.trim_end_matches('/');

        let mut entries = Vec::new();
        for resource in self.propfind(&parsed, "1")? {
            if decoded_href_path(&resource.href).trim_end_matches('/') == requested {
                continue;
            }
            let Some((name, trailing_slash)) = href_entry(&resource.href) else {
                continue;
            };
            let kind = if trailing_slash || resource.is_collection {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push(DirectoryEntry {
                path: parsed.join(&name),
                name,
                kind,
                modified_at: resource.last_modified,
                // WebDAV reports no creation time in the standard properties
                // this backend reads.
                created_at: None,
                created_fallback: false,
                size: match kind {
                    EntryKind::Directory => None,
                    _ => resource.content_length,
                },
                // Neither flag exists in the protocol; a dot-prefixed name is
                // the only hidden marker and the UI applies that itself.
                hidden: false,
                read_only: false,
            });
        }

        entries.sort_by_cached_key(entry_sort_key);

        Ok(DirectoryView {
            path: parsed.url(),
            breadcrumbs: webdav_breadcrumbs(&parsed),
            entries,
            // Remote backends list in one piece; only local reads stream.
            stream_id: None,
        })
    }

    fn create_entry(
        &self,
        directory: &str,
        name: &str,
        kind: NewEntryKind,
    ) -> Result<String, FileSystemError> {
        validate_entry_name(name)?;
        let parsed = parse_dir(directory)?;
        let target = parsed.join(name);

        // PUT truncates and MKCOL answers 405 on an existing collection, so
        // the absence check has to come first to keep the never-overwrite
        // contract the engine relies on.
        if self.resource_at(&target)?.is_some() {
            return Err(FileSystemError::AlreadyExists(target));
        }

        match kind {
            NewEntryKind::File => {
                let request = self
                    .authenticated(
                        http_client()?
                            .request(reqwest::Method::PUT, parse_dir(&target)?.request_url()),
                    )
                    .body(Vec::<u8>::new());
                let response = self.send(request)?;
                self.expect_success(response, &target)?;
            }
            NewEntryKind::Directory => self.mkdir(&target)?,
        }

        Ok(target)
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let parsed = parse_dir(path)?;
        let (parent, _old_name) = parsed.parent_and_name()?;
        let destination = parse_dir(&parent)?.join(new_name);
        self.move_entry(path, &destination)
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let parsed = parse_dir(path)?;
        let resource = self
            .propfind(&parsed, "0")?
            .into_iter()
            .next()
            .ok_or_else(|| FileSystemError::NotFound(parsed.url()))?;

        // The root is a collection even when a server omits `resourcetype`.
        let kind = if resource.is_collection || parsed.sub_path.is_empty() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };

        Ok(EntryStat {
            kind,
            size: match kind {
                EntryKind::Directory => 0,
                _ => resource.content_length.unwrap_or(0),
            },
            modified_at: resource.last_modified,
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "The server root already exists".into(),
            ));
        }

        let request = self
            .authenticated(http_client()?.request(custom_method("MKCOL")?, parsed.request_url()));
        let response = self.send(request)?;
        self.expect_success(response, &parsed.url()).map(|_| ())
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        let request =
            self.authenticated(http_client()?.request(reqwest::Method::GET, parsed.request_url()));
        let response = self.send(request)?;
        let response = self.expect_success(response, &parsed.url())?;

        Ok(Box::new(WebDavReadAdapter {
            runtime: Arc::clone(&self.runtime),
            response: Some(response),
            buffer: Vec::new(),
            position: 0,
        }))
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        let url = parsed.request_url();
        let display = parsed.url();
        let username = self.username.clone();
        let password = self.password.clone();

        let (sender, receiver) = sync_channel::<Vec<u8>>(UPLOAD_QUEUE_DEPTH);
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let worker = thread::Builder::new()
            .name("webdav-put".into())
            .spawn(move || {
                let outcome = upload_stream(
                    &url,
                    &display,
                    username.as_deref(),
                    password.as_deref(),
                    receiver,
                );
                let _ = result_sender.send(outcome);
            })
            .map_err(|error| {
                FileSystemError::Internal(format!(
                    "Could not start the WebDAV upload thread: {error}"
                ))
            })?;

        Ok(Box::new(WebDavWriteAdapter {
            sender: Some(sender),
            result: Some(result_receiver),
            worker: Some(worker),
            pending: Vec::with_capacity(UPLOAD_CHUNK_BYTES),
            finished: None,
        }))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "The server root cannot be deleted".into(),
            ));
        }

        let request = self
            .authenticated(http_client()?.request(reqwest::Method::DELETE, parsed.request_url()));
        let response = self.send(request)?;
        self.expect_success(response, &parsed.url()).map(|_| ())
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        self.move_entry(source, destination)
    }
}

/// The blocking client used for uploads. It is a different client from
/// [`http_client`] because a streamed request body needs a reader, and
/// `blocking::Body::new` is the only constructor in this build that takes one.
/// The client owns a runtime thread and must not be created or dropped inside
/// an async context; every caller here is a dedicated upload thread.
fn blocking_client() -> Result<&'static reqwest::blocking::Client, FileSystemError> {
    static CLIENT: LazyLock<Result<reqwest::blocking::Client, String>> = LazyLock::new(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())
    });

    match &*CLIENT {
        Ok(client) => Ok(client),
        Err(message) => Err(FileSystemError::Internal(format!(
            "Could not build the WebDAV upload client: {message}"
        ))),
    }
}

/// Runs one PUT on the upload thread, reading the body from `receiver` until
/// the writer closes it. The final status is the upload's result: a 2xx means
/// every byte landed, anything else keeps the server's explanation.
fn upload_stream(
    url: &str,
    display: &str,
    username: Option<&str>,
    password: Option<&str>,
    receiver: Receiver<Vec<u8>>,
) -> Result<(), FileSystemError> {
    let client = blocking_client()?;
    let mut request = client.put(url);
    if let (Some(username), Some(password)) = (username, password)
        && !username.is_empty()
        && !password.is_empty()
    {
        request = request.basic_auth(username, Some(password));
    }

    let body = reqwest::blocking::Body::new(ChannelReader {
        receiver,
        buffer: Vec::new(),
        position: 0,
    });
    let response = request.body(body).send().map_err(|error| {
        FileSystemError::Io(format!("The WebDAV upload did not complete: {error}"))
    })?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let detail = response.text().unwrap_or_default();
    Err(map_status(status, display, &detail))
}

/// The request body: whatever the writer has handed over, one chunk at a time.
/// A closed channel is the end of the body, which is how `flush`/`drop` end it.
struct ChannelReader {
    receiver: Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.position >= self.buffer.len() {
            match self.receiver.recv() {
                Ok(bytes) => {
                    self.buffer = bytes;
                    self.position = 0;
                }
                // The writer is done: an empty read ends the chunked body.
                Err(_) => return Ok(0),
            }
        }

        let count = (self.buffer.len() - self.position).min(buf.len());
        buf[..count].copy_from_slice(&self.buffer[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// The write half of a WebDAV upload. Bytes are buffered into chunk-sized
/// pieces and pushed to the PUT thread over a bounded channel, so `write`
/// blocks when the network falls behind instead of growing a buffer. `flush`
/// closes the body and waits for the server's status: the transfer engine
/// treats a successful flush as "the file is there", and it would be a lie to
/// return before the final response.
struct WebDavWriteAdapter {
    sender: Option<SyncSender<Vec<u8>>>,
    result: Option<Receiver<Result<(), FileSystemError>>>,
    worker: Option<JoinHandle<()>>,
    pending: Vec<u8>,
    /// Sticky failure, so a second flush or a write after an error reports the
    /// original cause instead of a channel-closed guess.
    finished: Option<FileSystemError>,
}

impl WebDavWriteAdapter {
    fn fail(&mut self, error: FileSystemError) -> io::Error {
        self.finished = Some(error.clone());
        io::Error::other(error.to_string())
    }

    fn send_chunk(&mut self, chunk: Vec<u8>) -> io::Result<()> {
        let Some(sender) = self.sender.clone() else {
            return Err(self.fail(FileSystemError::Io(
                "The WebDAV upload was already finished".into(),
            )));
        };
        sender.send(chunk).map_err(|_| {
            self.fail(FileSystemError::Io(
                "The WebDAV upload stopped before the file was sent".into(),
            ))
        })
    }

    fn send_pending(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::take(&mut self.pending);
        self.send_chunk(chunk)
    }

    /// Ends the body, joins the PUT thread and returns its verdict. Idempotent:
    /// the transfer engine calls `flush` and then drops the writer.
    fn finish(&mut self) -> Result<(), FileSystemError> {
        if let Some(error) = &self.finished {
            return Err(error.clone());
        }

        // Dropping the sender is what makes `ChannelReader::read` return 0.
        self.sender = None;
        let Some(result) = self.result.take() else {
            return Ok(());
        };
        let outcome = result.recv().unwrap_or_else(|_| {
            Err(FileSystemError::Io(
                "The WebDAV upload ended without a server status".into(),
            ))
        });
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        match outcome {
            Ok(()) => Ok(()),
            Err(error) => {
                self.finished = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl Write for WebDavWriteAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(error) = &self.finished {
            return Err(io::Error::other(error.to_string()));
        }

        // A caller that already hands over chunk-sized pieces skips the copy.
        if self.pending.is_empty() && buf.len() >= UPLOAD_CHUNK_BYTES {
            self.send_chunk(buf.to_vec())?;
            return Ok(buf.len());
        }

        self.pending.extend_from_slice(buf);
        while self.pending.len() >= UPLOAD_CHUNK_BYTES {
            let rest = self.pending.split_off(UPLOAD_CHUNK_BYTES);
            let chunk = std::mem::replace(&mut self.pending, rest);
            self.send_chunk(chunk)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Err(error) = self.send_pending() {
            return Err(error);
        }
        self.finish().map_err(|error| io::Error::other(error.to_string()))
    }
}

impl Drop for WebDavWriteAdapter {
    fn drop(&mut self) {
        // A dropped writer still has to close the body and collect the result;
        // otherwise a failed upload could be reported as success. The transfer
        // engine flushes first, so this normally has nothing left to do.
        if let Err(error) = self.finish() {
            log::warn!("WebDAV upload failed after the writer was dropped: {error}");
        }
    }
}

/// The process-wide HTTP client. Installs the rustls provider (ring) before the
/// first client exists, the same way the cloud backend does; the client itself
/// carries no per-server state, because credentials are attached per request.
fn http_client() -> Result<&'static reqwest::Client, FileSystemError> {
    static CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
        // Fails harmlessly when another module already installed a provider.
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())
    });

    match &*CLIENT {
        Ok(client) => Ok(client),
        Err(message) => Err(FileSystemError::Internal(format!(
            "Could not build the WebDAV HTTP client: {message}"
        ))),
    }
}

/// A WebDAV method outside the standard set. The token is a compile-time
/// constant, so a rejection here would be a programming error, not bad input.
fn custom_method(name: &str) -> Result<reqwest::Method, FileSystemError> {
    reqwest::Method::from_bytes(name.as_bytes())
        .map_err(|_| FileSystemError::Internal(format!("Invalid HTTP method: {name}")))
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(FileSystemError::InvalidInput(format!(
            "Invalid entry name: {name}"
        )));
    }
    Ok(())
}

fn webdav_breadcrumbs(path: &WebDavPath) -> Vec<Breadcrumb> {
    let mut breadcrumbs = vec![Breadcrumb {
        name: path.host.clone(),
        path: format!("{}://{}", path.scheme(), path.authority()),
    }];

    let mut accumulated = breadcrumbs[0].path.clone();
    for segment in path
        .sub_path
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        accumulated.push('/');
        accumulated.push_str(segment);
        breadcrumbs.push(Breadcrumb {
            name: segment.to_owned(),
            path: accumulated.clone(),
        });
    }

    breadcrumbs
}

/// Maps a non-success status onto a `FileSystemError`. WebDAV's own codes are
/// the interesting ones: 409 is MKCOL's "parent missing", 412 is a MOVE that
/// refused to overwrite, and 423 is a server-side lock.
fn map_status(status: reqwest::StatusCode, context: &str, detail: &str) -> FileSystemError {
    let detail = summarize(detail);
    let detail = if detail.is_empty() {
        context.to_owned()
    } else {
        format!("{context}: {detail}")
    };

    match status.as_u16() {
        401 => FileSystemError::PermissionDenied(format!(
            "The WebDAV server rejected the credentials: {detail}"
        )),
        403 => FileSystemError::PermissionDenied(detail),
        404 => FileSystemError::NotFound(detail),
        409 => {
            FileSystemError::NotFound(format!("{detail} (the parent collection does not exist)"))
        }
        412 => FileSystemError::AlreadyExists(detail),
        423 => FileSystemError::Io(format!("The entry is locked on the server: {detail}")),
        405 | 501 => FileSystemError::Unsupported(format!(
            "The WebDAV server does not allow this operation: {detail}"
        )),
        _ => FileSystemError::Io(format!("The WebDAV server returned {status}: {detail}")),
    }
}

/// Keeps an error page from filling a dialog; servers answer with HTML often
/// enough that the raw body is not worth showing whole.
fn summarize(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 300 {
        return trimmed.to_owned();
    }
    let shortened: String = trimmed.chars().take(300).collect();
    format!("{shortened}…")
}

/// One `<response>` element of a PROPFIND multistatus body, reduced to what a
/// listing or a `stat` needs.
#[derive(Debug, Default, PartialEq, Eq)]
struct DavResource {
    href: String,
    is_collection: bool,
    content_length: Option<u64>,
    last_modified: Option<u64>,
}

/// The property whose text is currently being collected.
#[derive(Clone, Copy)]
enum DavField {
    Href,
    ContentLength,
    LastModified,
}

/// Parses the multistatus body of a PROPFIND response.
///
/// Servers disagree on prefixes: Apache answers with `d:` bound to `DAV:`,
/// others use `D:`, `ns0:`, or no prefix at all, so elements are compared by
/// local name. That is unambiguous here because every element this reads is
/// unique to the DAV namespace and `propstat` groups repeat nothing it keeps.
fn parse_multistatus(xml: &str) -> Vec<DavResource> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut resources = Vec::new();
    let mut current: Option<DavResource> = None;
    let mut field: Option<DavField> = None;
    let mut text = String::new();
    let mut in_resource_type = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                "response" => {
                    current = Some(DavResource::default());
                    in_resource_type = false;
                    field = None;
                }
                "resourcetype" => in_resource_type = true,
                "collection" if in_resource_type => {
                    if let Some(resource) = current.as_mut() {
                        resource.is_collection = true;
                    }
                }
                name @ ("href" | "getcontentlength" | "getlastmodified") => {
                    field = Some(match name {
                        "href" => DavField::Href,
                        "getcontentlength" => DavField::ContentLength,
                        _ => DavField::LastModified,
                    });
                    text.clear();
                }
                _ => {}
            },
            // `<d:collection/>` closes at once, so it never reaches `End`.
            Ok(Event::Empty(element)) => {
                if in_resource_type
                    && element.local_name().as_ref() == "collection"
                    && let Some(resource) = current.as_mut()
                {
                    resource.is_collection = true;
                }
            }
            Ok(Event::Text(value)) => {
                if field.is_some() {
                    text.push_str(&value);
                }
            }
            Ok(Event::CData(value)) => {
                if field.is_some() {
                    text.push_str(&value.xml10_content());
                }
            }
            // `quick-xml` hands `&amp;`-style references over as their own
            // events, so text between tags is incomplete without folding them
            // back in — hrefs carry them often enough to matter.
            Ok(Event::GeneralRef(value)) => {
                if field.is_some() {
                    push_entity(&mut text, &value);
                }
            }
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                "href" => {
                    if let Some(resource) = current.as_mut() {
                        resource.href = text.trim().to_owned();
                    }
                    field = None;
                }
                "getcontentlength" => {
                    if let Some(resource) = current.as_mut() {
                        resource.content_length = text.trim().parse().ok();
                    }
                    field = None;
                }
                "getlastmodified" => {
                    if let Some(resource) = current.as_mut() {
                        resource.last_modified = parse_rfc1123_millis(&text);
                    }
                    field = None;
                }
                "resourcetype" => in_resource_type = false,
                "response" => {
                    if let Some(resource) = current.take()
                        && !resource.href.is_empty()
                    {
                        resources.push(resource);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            // A truncated body yields what was read before the damage: one
            // malformed property must not blank a whole folder.
            Err(_) => break,
        }
    }

    resources
}

/// Appends the character an entity reference stands for. Character references
/// are resolved by `quick-xml`; named entities are the five XML ones, and a
/// DTD-declared name is dropped rather than guessed at.
fn push_entity(text: &mut String, reference: &quick_xml::events::BytesRef<'_>) {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        text.push(character);
        return;
    }

    match reference.xml10_content().as_ref() {
        "amp" => text.push('&'),
        "lt" => text.push('<'),
        "gt" => text.push('>'),
        "quot" => text.push('"'),
        "apos" => text.push('\''),
        _ => {}
    }
}

/// The path part of a PROPFIND `href`, which servers send as an absolute URL
/// (`https://host/dav/a.txt`), an absolute path (`/dav/a.txt`), and sometimes
/// with a query or fragment appended.
fn href_path(href: &str) -> &str {
    let href = href.trim();
    let href = href.split(['?', '#']).next().unwrap_or(href);

    match href.find("://") {
        Some(index) => {
            let rest = &href[index + 3..];
            match rest.find('/') {
                Some(slash) => &rest[slash..],
                None => "/",
            }
        }
        None => href,
    }
}

/// The percent-decoded path a PROPFIND `href` names, comparable with the
/// decoded path the explorer holds.
fn decoded_href_path(href: &str) -> String {
    percent_decode(href_path(href))
}

/// The entry name and "is a directory" flag a PROPFIND `href` carries. WebDAV
/// marks collections with a trailing slash, which is also what a server that
/// omits `<resourcetype>` leaves behind.
fn href_entry(href: &str) -> Option<(String, bool)> {
    let path = href_path(href);
    let is_directory = path.ends_with('/');
    let segment = path.trim_end_matches('/').rsplit('/').next()?;
    if segment.is_empty() {
        return None;
    }

    Some((percent_decode(segment), is_directory))
}

/// Percent-encodes one path segment. Everything outside the unreserved set is
/// escaped: over-encoding a sub-delimiter still names the same resource, while
/// leaving `?` or `#` alone would turn a file name into a query or fragment.
fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn encode_path(sub_path: &str) -> String {
    sub_path
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Decodes `%XX` escapes into UTF-8. `+` is a literal plus in a path, so it is
/// deliberately not treated as a space.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).copied().and_then(hex_value),
                bytes.get(index + 2).copied().and_then(hex_value),
            )
        {
            decoded.push(high * 16 + low);
            index += 3;
            continue;
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Parses an RFC 1123 `getlastmodified` value (`Tue, 15 Nov 1994 12:45:26 GMT`)
/// into epoch milliseconds. The weekday is ignored — the date itself is
/// authoritative — and a missing weekday is tolerated because servers send
/// both forms.
fn parse_rfc1123_millis(value: &str) -> Option<u64> {
    let mut tokens = value
        .split_whitespace()
        .filter(|token| !token.ends_with(','));

    let day: u32 = tokens.next()?.parse().ok()?;
    let month = month_number(tokens.next()?)?;
    let year: i32 = tokens.next()?.parse().ok()?;
    let mut clock = tokens.next()?.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next()?.parse().ok()?;
    let second: u32 = clock.next()?.parse().ok()?;

    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);

    u64::try_from(seconds).ok().map(|seconds| seconds * 1_000)
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as u32 + 1)
}

/// Days from 1970-01-01 to the given civil date (Howard Hinnant's algorithm),
/// so a calendar date becomes an epoch offset without a date-time library.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(if month <= 2 { year - 1 } else { year });
    let era = {
        let shifted = if year >= 0 { year } else { year - 399 };
        shifted / 400
    };
    let year_of_era = year - era * 400;
    // March is month 0 here, which puts the leap day at the end of the year.
    let month_index = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_index + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

/// Streams an authenticated `GET` response as a blocking `Read`, buffering one
/// network chunk per round trip.
struct WebDavReadAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    response: Option<reqwest::Response>,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for WebDavReadAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position >= self.buffer.len() {
            let Some(response) = self.response.as_mut() else {
                return Ok(0);
            };
            match self
                .runtime
                .block_on(response.chunk())
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                Some(bytes) => {
                    self.buffer = bytes.to_vec();
                    self.position = 0;
                }
                None => {
                    self.response = None;
                    return Ok(0);
                }
            }
        }

        let available = &self.buffer[self.position..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count;
        Ok(count)
    }
}

#[cfg(test)]
mod webdav_tests {
    use super::*;

    const PREFIXED_MULTISTATUS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
        <d:getlastmodified>Tue, 15 Nov 1994 12:45:26 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/notes%20and%20things.txt</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>1024</d:getcontentlength>
        <d:getlastmodified>Tue, 15 Nov 1994 12:45:26 GMT</d:getlastmodified>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/a&amp;b.txt</d:href>
    <d:propstat>
      <d:prop><d:resourcetype/></d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

    const PLAIN_MULTISTATUS: &str = r#"<multistatus xmlns="DAV:">
  <response>
    <href>https://nas.example/dav/Photos/</href>
    <propstat>
      <prop><resourcetype><collection/></resourcetype></prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
  <response>
    <href>https://nas.example/dav/Photos/cat.jpg</href>
    <propstat>
      <prop><resourcetype/><getcontentlength>2048</getcontentlength></prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
</multistatus>"#;

    #[test]
    fn parses_prefixed_multistatus() {
        let resources = parse_multistatus(PREFIXED_MULTISTATUS);
        assert_eq!(resources.len(), 3);

        assert_eq!(resources[0].href, "/dav/");
        assert!(resources[0].is_collection);
        assert_eq!(resources[0].content_length, None);
        assert_eq!(resources[0].last_modified, Some(784_903_526_000));

        assert_eq!(resources[1].href, "/dav/notes%20and%20things.txt");
        assert!(!resources[1].is_collection);
        assert_eq!(resources[1].content_length, Some(1024));

        // `&amp;` arrives as its own event and must be folded back in.
        assert_eq!(resources[2].href, "/dav/a&b.txt");
    }

    #[test]
    fn parses_unprefixed_multistatus() {
        let resources = parse_multistatus(PLAIN_MULTISTATUS);
        assert_eq!(resources.len(), 2);

        assert_eq!(resources[0].href, "https://nas.example/dav/Photos/");
        assert!(resources[0].is_collection);
        assert_eq!(
            href_entry(&resources[0].href),
            Some(("Photos".into(), true))
        );

        assert!(!resources[1].is_collection);
        assert_eq!(resources[1].content_length, Some(2048));
        assert_eq!(
            href_entry(&resources[1].href),
            Some(("cat.jpg".into(), false))
        );
    }

    #[test]
    fn ignores_a_truncated_multistatus() {
        let truncated = "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>/dav/x";
        assert!(parse_multistatus(truncated).is_empty());
    }

    #[test]
    fn decodes_hrefs_into_entry_names() {
        assert_eq!(
            href_entry("/dav/My%20Folder/"),
            Some(("My Folder".to_owned(), true))
        );
        assert_eq!(
            href_entry("https://host/dav/a%2Bb.txt"),
            Some(("a+b.txt".to_owned(), false))
        );
        assert_eq!(
            href_entry("/dav/%E6%96%87%E4%BB%B6.txt"),
            Some(("文件.txt".to_owned(), false))
        );
        // Query and fragment are not part of the name.
        assert_eq!(
            href_entry("http://host/dav/file.txt?version=2"),
            Some(("file.txt".to_owned(), false))
        );
        // The root names nothing.
        assert_eq!(href_entry("/"), None);
        assert_eq!(href_entry(""), None);
    }

    #[test]
    fn decodes_href_paths_for_self_detection() {
        assert_eq!(decoded_href_path("/dav/My%20Folder/"), "/dav/My Folder/");
        assert_eq!(decoded_href_path("https://nas/dav/"), "/dav/");
        assert_eq!(href_path("https://nas"), "/");
    }

    #[test]
    fn parses_rfc1123_dates() {
        assert_eq!(
            parse_rfc1123_millis("Tue, 15 Nov 1994 12:45:26 GMT"),
            Some(784_903_526_000)
        );
        assert_eq!(
            parse_rfc1123_millis("Sat, 29 Feb 2020 23:59:59 GMT"),
            Some(1_583_020_799_000)
        );
        // Epoch itself, and the weekday-less form some servers send.
        assert_eq!(
            parse_rfc1123_millis("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(0)
        );
        assert_eq!(
            parse_rfc1123_millis("15 Nov 1994 12:45:26 GMT"),
            Some(784_903_526_000)
        );

        assert_eq!(parse_rfc1123_millis(""), None);
        assert_eq!(parse_rfc1123_millis("yesterday"), None);
        assert_eq!(parse_rfc1123_millis("Tue, 15 Foo 1994 12:45:26 GMT"), None);
        assert_eq!(parse_rfc1123_millis("Tue, 32 Nov 1994 12:45:26 GMT"), None);
        assert_eq!(parse_rfc1123_millis("Tue, 15 Nov 1994 24:45:26 GMT"), None);
    }

    #[test]
    fn builds_request_urls_from_explorer_paths() {
        let parsed = parse_dir("webdavs://nas:8443/dav/My Files").expect("tls path");
        assert_eq!(parsed.url(), "webdavs://nas:8443/dav/My Files");
        assert_eq!(parsed.request_url(), "https://nas:8443/dav/My%20Files");
        assert_eq!(parsed.remote(), "/dav/My Files");
        assert_eq!(
            parsed.join("a?b.txt"),
            "webdavs://nas:8443/dav/My Files/a?b.txt"
        );

        let parsed = parse_dir("webdav://nas/dav/").expect("plain path");
        assert_eq!(parsed.url(), "webdav://nas/dav");
        assert_eq!(parsed.request_url(), "http://nas/dav");

        let parsed = parse_dir("webdav://nas").expect("server root");
        assert_eq!(parsed.url(), "webdav://nas");
        assert_eq!(parsed.request_url(), "http://nas/");
        assert_eq!(parsed.remote(), "/");
        assert_eq!(
            crumb_tuples(&parsed),
            vec![("nas".to_owned(), "webdav://nas".to_owned())]
        );
    }

    #[test]
    fn breadcrumbs_walk_the_path_segments() {
        let parsed = parse_dir("webdavs://nas/dav/My Files").expect("tls path");
        assert_eq!(
            crumb_tuples(&parsed),
            vec![
                ("nas".to_owned(), "webdavs://nas".to_owned()),
                ("dav".to_owned(), "webdavs://nas/dav".to_owned()),
                (
                    "My Files".to_owned(),
                    "webdavs://nas/dav/My Files".to_owned(),
                ),
            ]
        );
    }

    /// `Breadcrumb` carries no equality of its own; compare its two fields.
    fn crumb_tuples(path: &WebDavPath) -> Vec<(String, String)> {
        webdav_breadcrumbs(path)
            .into_iter()
            .map(|crumb| (crumb.name, crumb.path))
            .collect()
    }

    #[test]
    fn rejects_paths_from_other_schemes() {
        assert!(parse_dir("smb://nas/share").is_err());
    }

    /// One in-process HTTP/1.1 server: reads a chunked PUT, answers with
    /// `response`, then hands the received body back to the test. This is the
    /// whole upload contract without a real WebDAV server in CI.
    fn serve_one_put(response: &'static [u8]) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
        use std::io::{BufRead, BufReader, Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the test server");
        let port = listener.local_addr().expect("local address").port();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept the upload");
            let mut reader = BufReader::new(stream.try_clone().expect("clone the socket"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the request line");
            assert!(
                line.starts_with("PUT /file.bin HTTP/1.1"),
                "unexpected request line: {line:?}"
            );

            let mut chunked = false;
            loop {
                line.clear();
                reader.read_line(&mut line).expect("read a header");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                    chunked = true;
                }
            }
            assert!(chunked, "the upload must stream without a content length");

            let mut body = Vec::new();
            loop {
                line.clear();
                reader.read_line(&mut line).expect("read a chunk size");
                let size = usize::from_str_radix(line.trim(), 16).expect("hex chunk size");
                if size == 0 {
                    // The terminating chunk is followed by an empty line.
                    line.clear();
                    reader.read_line(&mut line).expect("read the trailer");
                    break;
                }
                let start = body.len();
                body.resize(start + size, 0);
                reader
                    .read_exact(&mut body[start..])
                    .expect("read the chunk body");
                let mut crlf = [0_u8; 2];
                reader.read_exact(&mut crlf).expect("read the chunk terminator");
                assert_eq!(&crlf, b"\r\n");
            }

            stream.write_all(response).expect("answer the upload");
            body
        });
        (port, worker)
    }

    #[test]
    fn streams_an_upload_as_chunked_put() {
        let (port, worker) = serve_one_put(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n");
        let backend = WebDavBackend::connect_with(None, None).expect("backend");
        let path = format!("webdav://127.0.0.1:{port}/file.bin");

        let payload: Vec<u8> = (0..200 * 1024).map(|index| (index % 251) as u8).collect();
        {
            let mut writer = backend.open_write(&path).expect("open the upload");
            writer.write_all(&payload).expect("stream the payload");
            writer.flush().expect("wait for the server");
        }

        let received = worker.join().expect("server thread");
        assert_eq!(received, payload);
    }

    #[test]
    fn reports_the_server_status_when_an_upload_is_refused() {
        let (port, worker) = serve_one_put(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 16\r\n\r\nThe disk is full",
        );
        let backend = WebDavBackend::connect_with(None, None).expect("backend");
        let path = format!("webdav://127.0.0.1:{port}/file.bin");
        let mut writer = backend.open_write(&path).expect("open the upload");
        writer.write_all(b"data").expect("buffer the payload");
        let error = writer.flush().expect_err("a refused upload must fail");
        assert!(
            error.to_string().contains("disk is full"),
            "the server's reason is kept: {error}"
        );
        drop(writer);
        worker.join().expect("server thread");
    }
}
