//! The FTP/FTPS backend: one `FtpBackend` per server, reached through
//! `ftp://host[:port]/path` URLs. Tree-shaped transfers are served by the
//! generic engine in `transfer.rs`; this module implements the single-server
//! primitives, mirroring the SFTP and SMB backends.
//!
//! # Addressing
//!
//! The URL root is the *login directory* — the path `PWD` reports right after
//! login — so a chrooted server whose root is `/` and a plain server whose home
//! is `/home/user` both show their root as `ftp://host`. Every operation
//! resolves its path against that base and `CWD`s into the containing
//! directory, because a fair number of servers mishandle absolute pathnames in
//! `LIST`/`RETR`/`STOR`. A server that refuses `PWD` falls back to its `/`.
//!
//! # TLS
//!
//! Plain FTP is the default. FTPS is selected three ways:
//!
//! - implicit TLS on port 990 (`ftp://host:990/...`),
//! - an `ftps://` URL, which [`open_backend`] also accepts with its scheme
//!   prefix still attached, and
//! - [`open_backend_with_tls`], for a caller that parses `ftps://` itself.
//!
//! Explicit TLS issues `AUTH TLS` before the login; implicit TLS wraps the
//! control connection from the first byte. Certificates are verified against
//! the OS trust store (`rustls-native-certs`); there is no trust-on-first-use
//! fallback, so a server with a self-signed certificate cannot be reached
//! through this backend yet.
//!
//! This module needs suppaftp's sync rustls backend:
//!
//! ```toml
//! suppaftp = { version = "12", default-features = false, features = ["rustls-ring", "deprecated"] }
//! rustls-native-certs = "0.8"
//! ```
//!
//! `deprecated` is what gates `connect_secure_implicit`, the only implicit-TLS
//! constructor the crate exposes.
//!
//! # Connections
//!
//! FTP multiplexes one command channel per session, so every operation checks
//! a connection out of a bounded pool and returns it when it is done. The pool
//! exists rather than a single mutex because the transfer engine copies
//! sibling files in parallel and runs the writing half on its own thread: a
//! same-server copy needs four reading and four writing connections at once,
//! and a lone connection would stall it. The cap matches that width, and a
//! server that refuses extra sessions reports its refusal rather than hanging.
//!
//! A transfer keeps its connection until the transfer finishes, because FTP
//! refuses to open a second data connection on one command channel. Reads
//! stream through `retr_as_stream` and writes through `put_with_stream`, so
//! neither buffers a file in memory.
//!
//! # What the protocol does not give us
//!
//! No birth time, no permission model (the properties dialog stays view-only),
//! and no reliable file type: listings come from `MLSD` when the server
//! advertises it through `FEAT`, and from a heuristic `LIST` parser otherwise.
//! The parser covers the Unix `ls -l` shape every server shares; a line it
//! cannot read is skipped and counted in a debug log. `SIZE` is refused for
//! directories on most servers, so [`stat`](FileSystemBackend::stat) probes
//! with `CWD` when `MLST` is unavailable.

use super::connections::{self, Protocol};
use super::error::FileSystemError;
use super::smb::{parse_authority, split_authority};
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, NewEntryKind, entry_sort_key,
};
use super::vfs::{FileSystemBackend, SharedBackend};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use suppaftp::rustls::{ClientConfig, RootCertStore};
use suppaftp::types::FileType;
use suppaftp::{
    FtpError, FtpResult, FtpStream, RustlsConnector, RustlsFtpStream, TlsStream, TransferStream,
};

const DEFAULT_PORT: u16 = 21;
/// The registered port for implicit FTPS.
const IMPLICIT_TLS_PORT: u16 = 990;
const ANONYMOUS_USER: &str = "anonymous";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Bounds one control reply, not one transfer: the data connection is separate.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
const DATA_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A data connection that moves no byte for this long is treated as dead
/// rather than blocking a transfer worker for the rest of the process.
const DATA_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Sessions one backend keeps open at a time.
///
/// A same-server copy runs four files at once (the transfer engine's width),
/// each holding one connection for reading and one for writing, so eight is
/// what lets that copy finish without waiting on itself. Metadata operations
/// share the same pool and only hold a connection for one round trip.
const MAX_CONNECTIONS: usize = 8;
/// How long a caller waits for a connection before reporting that the server
/// is not letting go of one. The wait is normally bounded by a transfer.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(60);

const MLSD_UNKNOWN: u8 = 0;
const MLSD_SUPPORTED: u8 = 1;
const MLSD_UNSUPPORTED: u8 = 2;

const MILLIS_PER_DAY: u64 = 86_400_000;

/// How a connection secures its control channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plain FTP on port 21.
    Plain,
    /// `AUTH TLS` before the login (the `ftps://` scheme).
    Explicit,
    /// TLS from the first byte (port 990).
    Implicit,
}

/// The FTP backend for one server, shared through the connection registry.
pub struct FtpBackend {
    config: Arc<ServerConfig>,
    pool: Arc<ConnectionPool>,
    /// Whether the server advertises `MLSD`, learned once per backend: the
    /// answer is a property of the server, not of a connection.
    mlsd: AtomicU8,
}

/// What every pooled connection is opened from. Credentials already live in
/// memory for the connection registry, so holding them here adds no exposure.
struct ServerConfig {
    /// As written in the URL, bracketed for IPv6.
    host: String,
    port: u16,
    tls: TlsMode,
    username: String,
    password: String,
    /// The login directory, normalized: "/" or "/a/b", never with a trailing
    /// separator. URL paths resolve against it.
    base: String,
    /// Built once per backend when TLS is in use.
    tls_config: Option<Arc<ClientConfig>>,
}

/// Opens (or reuses) the session for `ftp://...` URL text and returns it as a
/// backend. Called by `vfs::resolve`.
///
/// `rest` is the URL without its scheme prefix. A caller that passes the whole
/// URL (`ftps://host/path`) is understood too, which keeps the `ftps` scheme
/// working without a second entry point.
pub fn open_backend(rest: &str) -> Result<SharedBackend, FileSystemError> {
    let (rest, requested) = split_scheme_prefix(rest);
    let path = parse_ftp_path(rest)?;
    let tls = requested.unwrap_or(match path.port {
        Some(IMPLICIT_TLS_PORT) => TlsMode::Implicit,
        _ => TlsMode::Plain,
    });
    open_with(&path, tls)
}

/// Probes a server with the credentials the connect dialog holds, before they
/// are saved. Blocking; the caller is already on a blocking thread. Port 990
/// selects implicit FTPS, every other port is plain FTP.
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
    let tls = if port == Some(IMPLICIT_TLS_PORT) {
        TlsMode::Implicit
    } else {
        TlsMode::Plain
    };
    let config = ServerConfig {
        host,
        port: port.unwrap_or(match tls {
            TlsMode::Implicit => IMPLICIT_TLS_PORT,
            _ => DEFAULT_PORT,
        }),
        tls,
        username: username
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(ANONYMOUS_USER)
            .to_owned(),
        password: password.unwrap_or_default().to_owned(),
        base: "/".to_owned(),
        tls_config: match tls {
            TlsMode::Plain => None,
            TlsMode::Explicit | TlsMode::Implicit => Some(Arc::new(tls_client_config()?)),
        },
    };
    let mut client = connect_client(&config)?;
    // The pool normally learns this; a probe only needs the login to succeed.
    let _ = client.pwd();
    Ok(())
}

/// Opens (or reuses) the session for `rest` with an explicit TLS choice, for a
/// caller that parses the scheme itself.
pub fn open_backend_with_tls(rest: &str, tls: TlsMode) -> Result<SharedBackend, FileSystemError> {
    let (rest, _) = split_scheme_prefix(rest);
    open_with(&parse_ftp_path(rest)?, tls)
}

fn open_with(path: &FtpPath, tls: TlsMode) -> Result<SharedBackend, FileSystemError> {
    if let Some(existing) = connections::session_for(Protocol::Ftp, &path.host, path.port) {
        return Ok(existing);
    }

    let port = path.port.unwrap_or(match tls {
        TlsMode::Implicit => IMPLICIT_TLS_PORT,
        TlsMode::Plain | TlsMode::Explicit => DEFAULT_PORT,
    });
    // A username in the URL is an explicit choice; otherwise the connection
    // store supplies one, and nothing at all means anonymous.
    let (stored_user, stored_password) =
        connections::resolve_credentials(Protocol::Ftp, &path.host, path.port);
    let username = path
        .username
        .clone()
        .or(stored_user)
        .unwrap_or_else(|| ANONYMOUS_USER.to_owned());
    let password = stored_password.unwrap_or_default();
    let tls_config = match tls {
        TlsMode::Plain => None,
        TlsMode::Explicit | TlsMode::Implicit => Some(Arc::new(tls_client_config()?)),
    };

    let mut config = ServerConfig {
        host: path.host.clone(),
        port,
        tls,
        username,
        password,
        base: "/".to_owned(),
        tls_config,
    };

    // Connect once up front: an unreachable host or a rejected account belongs
    // to the open call, not to the first listing. The same connection also
    // tells us the login directory the URL root maps to. A server that refuses
    // `PWD` (RFC 959 requires it to answer with a quoted path) still gets a
    // backend; its root is then the server's `/`.
    let mut client = connect_client(&config)?;
    config.base = match client.pwd() {
        Ok(pwd) => normalize_base(&pwd),
        Err(error) => {
            log::debug!("The FTP server did not report its login directory: {error}");
            "/".to_owned()
        }
    };

    let config = Arc::new(config);
    let pool = Arc::new(ConnectionPool::new(Arc::clone(&config)));
    pool.seed(client);

    let backend: SharedBackend = Arc::new(FtpBackend {
        config,
        pool,
        mlsd: AtomicU8::new(MLSD_UNKNOWN),
    });
    connections::store_session(Protocol::Ftp, &path.host, path.port, backend.clone());
    Ok(backend)
}

/// Opens and authenticates one connection. The caller owns it from here; the
/// pool hands it out and takes it back.
fn connect_client(config: &ServerConfig) -> Result<FtpClient, FileSystemError> {
    let address = format!("{}:{}", config.host, config.port);
    let socket = address
        .to_socket_addrs()
        .map_err(|error| {
            FileSystemError::Io(format!("Could not resolve {}: {error}", config.host))
        })?
        .next()
        .ok_or_else(|| {
            FileSystemError::Io(format!("Could not resolve {}: no addresses", config.host))
        })?;

    let domain = tls_domain(&config.host);
    let mut client = match config.tls {
        TlsMode::Plain => FtpClient::Plain(
            FtpStream::connect_timeout(socket, CONNECT_TIMEOUT).map_err(map_ftp_error)?,
        ),
        TlsMode::Explicit => {
            let stream =
                RustlsFtpStream::connect_timeout(socket, CONNECT_TIMEOUT).map_err(map_ftp_error)?;
            FtpClient::Secure(
                stream
                    .into_secure(tls_connector(config)?, &domain)
                    .map_err(map_ftp_error)?,
            )
        }
        TlsMode::Implicit => FtpClient::Secure(
            RustlsFtpStream::connect_secure_implicit(socket, tls_connector(config)?, &domain)
                .map_err(map_ftp_error)?,
        ),
    };

    // Timeouts belong to the sockets, so they are set before the first command
    // rather than after it: a server that accepts the TCP connection and then
    // says nothing must not hang the login.
    client = client.with_data_socket(data_socket);
    client.set_control_timeouts();
    client
        .login(&config.username, &config.password)
        .map_err(map_login_error)?;
    // Binary is not the crate's default (ASCII converts line endings), and
    // every transfer this backend starts is a byte stream.
    client
        .transfer_type(FileType::Binary)
        .map_err(map_ftp_error)?;
    Ok(client)
}

/// The OS trust store, once per backend. `DAE_TLS_CA_FILE`, when set, adds
/// every certificate in that PEM file as an extra anchor: private CA operators
/// (and the FTPS smoke test) can reach a server the OS store does not know
/// without turning verification off. The file only adds trust; it never
/// replaces the system roots.
fn tls_client_config() -> Result<ClientConfig, FileSystemError> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    for certificate in native.certs {
        let _ = roots.add(certificate);
    }

    if let Some(path) = std::env::var_os("DAE_TLS_CA_FILE") {
        use rustls::pki_types::CertificateDer;
        use rustls::pki_types::pem::PemObject;

        let path = std::path::PathBuf::from(path);
        let certificates = CertificateDer::pem_file_iter(&path).map_err(|error| {
            FileSystemError::Io(format!(
                "Could not read DAE_TLS_CA_FILE ({}): {error}",
                path.display()
            ))
        })?;
        for certificate in certificates {
            let certificate = certificate.map_err(|error| {
                FileSystemError::Io(format!(
                    "Could not parse a certificate in DAE_TLS_CA_FILE ({}): {error}",
                    path.display()
                ))
            })?;
            roots.add(certificate).map_err(|error| {
                FileSystemError::Io(format!(
                    "Could not trust a certificate in DAE_TLS_CA_FILE ({}): {error}",
                    path.display()
                ))
            })?;
        }
    }

    if roots.is_empty() {
        return Err(FileSystemError::Io(
            "No trusted TLS certificates were found on this system, so FTPS cannot be verified"
                .into(),
        ));
    }
    Ok(ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

fn tls_connector(config: &ServerConfig) -> Result<RustlsConnector, FileSystemError> {
    let tls_config = config.tls_config.as_ref().ok_or_else(|| {
        FileSystemError::Internal("TLS was not configured for this FTP connection".into())
    })?;
    Ok(RustlsConnector::from(Arc::clone(tls_config)))
}

/// The name a certificate is checked against: the URL host without the
/// brackets that mark an IPv6 literal.
fn tls_domain(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned()
}

/// Passive data sockets, with the timeouts the crate's default builder lacks.
fn data_socket(address: std::net::SocketAddr) -> FtpResult<TcpStream> {
    let stream = TcpStream::connect_timeout(&address, DATA_CONNECT_TIMEOUT)
        .map_err(FtpError::ConnectionError)?;
    let _ = stream.set_read_timeout(Some(DATA_IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(DATA_IDLE_TIMEOUT));
    Ok(stream)
}

/// `PWD` comes back as the server sees it; a trailing or doubled separator
/// would produce `//` in every path we build, so it is normalized here.
fn normalize_base(pwd: &str) -> String {
    let segments = pwd
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{segments}")
    }
}

/// A bounded pool of authenticated connections.
///
/// A transfer holds its connection until it finishes, so this cannot be a
/// single mutex: the engine copies sibling files concurrently and each copy
/// needs one connection for reading and one for writing.
struct ConnectionPool {
    config: Arc<ServerConfig>,
    state: Mutex<PoolState>,
    /// Signalled whenever a connection comes back, waking a caller that found
    /// the pool empty and at its cap.
    returned: Condvar,
}

struct PoolState {
    idle: Vec<FtpClient>,
    /// Connections handed out plus the ones sitting in `idle`.
    live: usize,
}

impl ConnectionPool {
    fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            config,
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
            }),
            returned: Condvar::new(),
        }
    }

    /// Puts the connection opened at startup into the pool, so the first
    /// operation reuses the login the backend already paid for.
    fn seed(&self, client: FtpClient) {
        let mut state = self.lock();
        state.idle.push(client);
        state.live = 1;
    }

    /// Hands out a connection, opening another when the pool is empty and the
    /// cap allows it, and waiting up to [`ACQUIRE_TIMEOUT`] otherwise.
    fn acquire(self: &Arc<Self>) -> Result<Lease, FileSystemError> {
        let deadline = Instant::now() + ACQUIRE_TIMEOUT;

        loop {
            let mut state = self.lock();
            if let Some(client) = state.idle.pop() {
                return Ok(Lease::new(Arc::clone(self), client));
            }

            if state.live < MAX_CONNECTIONS {
                // Claim the slot before connecting: holding the lock across the
                // handshake would stall every other operation, and claiming
                // afterwards would let two callers open past the cap.
                state.live += 1;
                drop(state);

                return match connect_client(&self.config) {
                    Ok(client) => Ok(Lease::new(Arc::clone(self), client)),
                    Err(error) => {
                        self.forget();
                        Err(error)
                    }
                };
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FileSystemError::Io(
                    "The FTP server has no free connection: every session is busy with a transfer"
                        .into(),
                ));
            }

            // `wait_timeout` releases the lock as part of waiting, so a return
            // that happens between the check above and this wait is still seen.
            let (state, _) = self
                .returned
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            drop(state);
        }
    }

    /// Hands a healthy connection back.
    fn release(&self, client: FtpClient) {
        {
            let mut state = self.lock();
            state.idle.push(client);
        }
        self.returned.notify_one();
    }

    /// Frees the slot of a connection that failed below the protocol instead
    /// of pooling a corpse for the next caller.
    fn forget(&self) {
        {
            let mut state = self.lock();
            state.live = state.live.saturating_sub(1);
        }
        self.returned.notify_one();
    }

    /// A panic while a connection was checked out must not wedge the backend
    /// for the rest of the process, so poisoning is recovered from.
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// A checked-out connection. Dropping it returns the connection, unless it was
/// discarded or moved into a transfer adapter.
struct Lease {
    pool: Arc<ConnectionPool>,
    client: Option<FtpClient>,
    /// Set when the connection was handed to a transfer adapter, which returns
    /// it itself; the lease must then stay out of the pool's accounting.
    handed_off: bool,
}

impl Lease {
    fn new(pool: Arc<ConnectionPool>, client: FtpClient) -> Self {
        Self {
            pool,
            client: Some(client),
            handed_off: false,
        }
    }

    fn client(&mut self) -> &mut FtpClient {
        self.client.as_mut().expect("FTP connection is checked out")
    }

    fn discard(&mut self) {
        self.client = None;
    }

    /// Moves the connection into a transfer adapter, which returns it once the
    /// transfer has finished.
    fn detach(mut self) -> (FtpClient, Arc<ConnectionPool>) {
        let client = self.client.take().expect("FTP connection is checked out");
        self.handed_off = true;
        (client, Arc::clone(&self.pool))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.handed_off {
            return;
        }

        match self.client.take() {
            Some(client) => self.pool.release(client),
            None => self.pool.forget(),
        }
    }
}

/// One authenticated control channel, in either of the two flavours the crate
/// has separate types for.
enum FtpClient {
    Plain(FtpStream),
    Secure(RustlsFtpStream),
}

/// Runs a command on whichever stream the client wraps. Both arms must produce
/// the same type, which is why the two transfer constructors are free
/// functions instead of arms of this macro.
macro_rules! on_client {
    ($client:expr, $stream:ident => $body:expr) => {
        match $client {
            FtpClient::Plain($stream) => $body,
            FtpClient::Secure($stream) => $body,
        }
    };
}

impl FtpClient {
    fn cwd(&mut self, path: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.cwd(path))
    }

    fn pwd(&mut self) -> FtpResult<String> {
        on_client!(self, stream => stream.pwd())
    }

    fn login(&mut self, username: &str, password: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.login(username, password))
    }

    fn transfer_type(&mut self, file_type: FileType) -> FtpResult<()> {
        on_client!(self, stream => stream.transfer_type(file_type))
    }

    fn list(&mut self, path: Option<&str>) -> FtpResult<Vec<String>> {
        on_client!(self, stream => stream.list(path))
    }

    fn mlsd(&mut self, path: Option<&str>) -> FtpResult<Vec<String>> {
        on_client!(self, stream => stream.mlsd(path))
    }

    fn mlst(&mut self, path: &str) -> FtpResult<String> {
        on_client!(self, stream => stream.mlst(Some(path)))
    }

    fn size(&mut self, path: &str) -> FtpResult<usize> {
        on_client!(self, stream => stream.size(path))
    }

    fn mdtm_millis(&mut self, path: &str) -> FtpResult<Option<u64>> {
        on_client!(self, stream => stream.mdtm(path).map(|time| {
            u64::try_from(time.and_utc().timestamp_millis()).ok()
        }))
    }

    fn mkdir(&mut self, path: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.mkdir(path))
    }

    fn rm(&mut self, path: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.rm(path))
    }

    fn rmdir(&mut self, path: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.rmdir(path))
    }

    fn rename(&mut self, from: &str, to: &str) -> FtpResult<()> {
        on_client!(self, stream => stream.rename(from, to))
    }

    fn put_empty_file(&mut self, path: &str) -> FtpResult<()> {
        on_client!(self, stream => {
            let mut empty = io::empty();
            stream.put_file(path, &mut empty).map(|_| ())
        })
    }

    fn feat(&mut self) -> FtpResult<Vec<String>> {
        on_client!(self, stream => stream.feat().map(|features| features.into_keys().collect()))
    }

    /// `None` when `FEAT` itself failed, so a transient failure is not recorded
    /// as "this server has no MLSD".
    fn advertises_mlsd(&mut self) -> Option<bool> {
        self.feat().ok().map(|features| {
            features
                .iter()
                .any(|name| name.eq_ignore_ascii_case("MLSD"))
        })
    }

    fn set_control_timeouts(&mut self) {
        on_client!(self, stream => {
            let _ = stream.get_ref().set_read_timeout(Some(CONTROL_TIMEOUT));
            let _ = stream.get_ref().set_write_timeout(Some(CONTROL_TIMEOUT));
        });
    }

    fn with_data_socket(self, builder: fn(std::net::SocketAddr) -> FtpResult<TcpStream>) -> Self {
        match self {
            FtpClient::Plain(stream) => FtpClient::Plain(stream.passive_stream_builder(builder)),
            FtpClient::Secure(stream) => FtpClient::Secure(stream.passive_stream_builder(builder)),
        }
    }
}

/// A download in progress. The control connection stays checked out until the
/// transfer finishes, because FTP allows one data connection per command
/// channel; the completion reply is read when this is dropped.
struct FtpReadStream<T: TlsStream> {
    transfer: Option<TransferStream<T>>,
    connection: Option<(FtpClient, Arc<ConnectionPool>)>,
    remote: String,
}

impl<T: TlsStream> Read for FtpReadStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.transfer.as_mut() {
            Some(transfer) => transfer.read(buf).map_err(io::Error::other),
            None => Ok(0),
        }
    }
}

impl<T: TlsStream> Drop for FtpReadStream<T> {
    fn drop(&mut self) {
        let reusable = match self.transfer.take() {
            Some(transfer) => match transfer.finish() {
                Ok(()) => true,
                Err(error) => {
                    log::debug!("FTP download of {} did not complete: {error}", self.remote);
                    connection_survives(&error)
                }
            },
            None => true,
        };

        if let Some((client, pool)) = self.connection.take() {
            if reusable {
                pool.release(client);
            } else {
                pool.forget();
            }
        }
    }
}

/// An upload in progress. `flush` pushes the bytes; the server's completion
/// reply can only be read once the data connection closes, so it is checked
/// when this is dropped and a failure is logged rather than returned.
struct FtpWriteStream<T: TlsStream> {
    transfer: Option<TransferStream<T>>,
    connection: Option<(FtpClient, Arc<ConnectionPool>)>,
    remote: String,
}

impl<T: TlsStream> Write for FtpWriteStream<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.transfer.as_mut() {
            Some(transfer) => transfer.write(buf).map_err(io::Error::other),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the FTP upload has already finished",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.transfer.as_mut() {
            Some(transfer) => transfer.flush().map_err(io::Error::other),
            None => Ok(()),
        }
    }
}

impl<T: TlsStream> Drop for FtpWriteStream<T> {
    fn drop(&mut self) {
        let reusable = match self.transfer.take() {
            Some(transfer) => match transfer.finish() {
                Ok(()) => true,
                Err(error) => {
                    // The engine flushed before dropping, so this is the only
                    // report a refused upload gets.
                    log::warn!("FTP upload of {} did not complete: {error}", self.remote);
                    connection_survives(&error)
                }
            },
            None => true,
        };

        if let Some((client, pool)) = self.connection.take() {
            if reusable {
                pool.release(client);
            } else {
                pool.forget();
            }
        }
    }
}

fn begin_read(
    client: FtpClient,
    pool: Arc<ConnectionPool>,
    directory: &str,
    name: &str,
) -> Result<Box<dyn Read + Send>, FileSystemError> {
    let remote = format!("{directory}/{name}");
    match client {
        FtpClient::Plain(mut stream) => match stream.retr_as_stream(name) {
            Ok(transfer) => Ok(Box::new(FtpReadStream {
                transfer: Some(transfer),
                connection: Some((FtpClient::Plain(stream), pool)),
                remote,
            })),
            Err(error) => {
                pool.forget();
                Err(map_ftp_error(error))
            }
        },
        FtpClient::Secure(mut stream) => match stream.retr_as_stream(name) {
            Ok(transfer) => Ok(Box::new(FtpReadStream {
                transfer: Some(transfer),
                connection: Some((FtpClient::Secure(stream), pool)),
                remote,
            })),
            Err(error) => {
                pool.forget();
                Err(map_ftp_error(error))
            }
        },
    }
}

fn begin_write(
    client: FtpClient,
    pool: Arc<ConnectionPool>,
    directory: &str,
    name: &str,
) -> Result<Box<dyn Write + Send>, FileSystemError> {
    let remote = format!("{directory}/{name}");
    match client {
        FtpClient::Plain(mut stream) => match stream.put_with_stream(name) {
            Ok(transfer) => Ok(Box::new(FtpWriteStream {
                transfer: Some(transfer),
                connection: Some((FtpClient::Plain(stream), pool)),
                remote,
            })),
            Err(error) => {
                pool.forget();
                Err(map_ftp_error(error))
            }
        },
        FtpClient::Secure(mut stream) => match stream.put_with_stream(name) {
            Ok(transfer) => Ok(Box::new(FtpWriteStream {
                transfer: Some(transfer),
                connection: Some((FtpClient::Secure(stream), pool)),
                remote,
            })),
            Err(error) => {
                pool.forget();
                Err(map_ftp_error(error))
            }
        },
    }
}

impl FtpBackend {
    /// The absolute path on the server that a URL sub-path names.
    fn remote(&self, sub_path: &str) -> String {
        if sub_path.is_empty() {
            return self.config.base.clone();
        }
        if self.config.base == "/" {
            format!("/{sub_path}")
        } else {
            format!("{}/{}", self.config.base, sub_path)
        }
    }

    /// The absolute containing directory and the entry name within it.
    fn split_remote(&self, sub_path: &str) -> (String, String) {
        match sub_path.rfind('/') {
            Some(index) => (
                self.remote(&sub_path[..index]),
                sub_path[index + 1..].to_owned(),
            ),
            None => (self.config.base.clone(), sub_path.to_owned()),
        }
    }

    fn mlsd_supported(&self) -> bool {
        match self.mlsd.load(Ordering::Relaxed) {
            MLSD_SUPPORTED => true,
            MLSD_UNSUPPORTED => false,
            _ => match self.run(|client| Ok(client.advertises_mlsd())) {
                Ok(Some(supported)) => {
                    self.record_mlsd(supported);
                    supported
                }
                _ => false,
            },
        }
    }

    fn mark_mlsd_unsupported(&self) {
        self.record_mlsd(false);
    }

    fn record_mlsd(&self, supported: bool) {
        self.mlsd.store(
            if supported {
                MLSD_SUPPORTED
            } else {
                MLSD_UNSUPPORTED
            },
            Ordering::Relaxed,
        );
    }

    /// Checks out a connection positioned in `directory`, retrying once when a
    /// pooled socket turns out to be dead — the usual symptom of a server that
    /// dropped an idle session. The `CWD` is idempotent, so replaying it is
    /// safe; whatever the caller sends next is never replayed.
    fn lease_in(&self, directory: &str) -> Result<Lease, FileSystemError> {
        let mut attempt = 0;
        loop {
            let mut lease = self.pool.acquire()?;
            match lease.client().cwd(directory) {
                Ok(()) => return Ok(lease),
                Err(error) => {
                    if connection_survives(&error) || attempt == 1 {
                        return Err(map_ftp_error(error));
                    }
                    lease.discard();
                    attempt += 1;
                }
            }
        }
    }

    /// Runs a read-only sequence on a checked-out connection, retrying once on
    /// a socket that was dead before the first command. A server *reply* is
    /// never retried — the answer is the answer.
    fn run<T>(&self, op: impl Fn(&mut FtpClient) -> FtpResult<T>) -> Result<T, FileSystemError> {
        let mut attempt = 0;
        loop {
            let mut lease = self.pool.acquire()?;
            match op(lease.client()) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if connection_survives(&error) || attempt == 1 {
                        return Err(map_ftp_error(error));
                    }
                    lease.discard();
                    attempt += 1;
                }
            }
        }
    }

    /// Runs a sequence that may change the server, on a connection already
    /// positioned in `directory`.
    ///
    /// Only the positioning is retried: a command that may have taken effect
    /// before the connection dropped must not be replayed, or a lost reply
    /// would turn one rename into two.
    fn run_in<T>(
        &self,
        directory: &str,
        op: impl Fn(&mut FtpClient) -> FtpResult<T>,
    ) -> Result<T, FileSystemError> {
        let mut lease = self.lease_in(directory)?;
        match op(lease.client()) {
            Ok(value) => Ok(value),
            Err(error) => {
                if !connection_survives(&error) {
                    lease.discard();
                }
                Err(map_ftp_error(error))
            }
        }
    }
}

impl FileSystemBackend for FtpBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let parsed = parse_dir(path)?;
        let remote_dir = self.remote(&parsed.sub_path);
        let use_mlsd = self.mlsd_supported();

        let (used_mlsd, lines) = self.run_in(&remote_dir, |client| {
            if use_mlsd {
                match client.mlsd(None) {
                    Ok(lines) => return Ok((true, lines)),
                    Err(error) if is_command_unsupported(&error) => {
                        // Advertised but refused: remember that and list the
                        // human-readable way instead of failing the view.
                        self.mark_mlsd_unsupported();
                    }
                    Err(error) => return Err(error),
                }
            }
            client.list(None).map(|lines| (false, lines))
        })?;

        let now = SystemTime::now();
        let mut entries = Vec::with_capacity(lines.len());
        let mut skipped = 0_usize;
        for line in &lines {
            let parsed_entry = if used_mlsd {
                parse_mlsx_line(line)
            } else {
                parse_list_line(line, now)
            };
            let Some(entry) = parsed_entry else {
                skipped += 1;
                continue;
            };

            entries.push(DirectoryEntry {
                name: entry.name.clone(),
                path: parsed.join(&entry.name),
                kind: entry.kind,
                modified_at: entry.modified_at,
                // FTP reports no birth time.
                created_at: None,
                created_fallback: false,
                size: (entry.kind == EntryKind::File).then(|| entry.size.unwrap_or(0)),
                hidden: entry.name.starts_with('.'),
                read_only: entry.read_only,
            });
        }
        if skipped > 0 {
            log::debug!(
                "Skipped {skipped} unreadable {} line(s) in {remote_dir}",
                if used_mlsd { "MLSD" } else { "LIST" }
            );
        }

        entries.sort_by_cached_key(entry_sort_key);

        Ok(DirectoryView {
            path: parsed.url(),
            breadcrumbs: ftp_breadcrumbs(&parsed),
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
        let remote_dir = self.remote(&parsed.sub_path);
        let target = parsed.join(name);

        // `STOR` truncates, so the absence check has to come first to keep the
        // never-overwrite contract.
        let exists = self.run_in(&remote_dir, |client| Ok(probe_exists(client, name)))?;
        if exists {
            return Err(FileSystemError::AlreadyExists(target));
        }

        match kind {
            NewEntryKind::File => self.run_in(&remote_dir, |client| client.put_empty_file(name)),
            NewEntryKind::Directory => self.run_in(&remote_dir, |client| client.mkdir(name)),
        }?;

        Ok(target)
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "The server root cannot be renamed".into(),
            ));
        }

        let (directory, name) = self.split_remote(&parsed.sub_path);
        self.run_in(&directory, |client| client.rename(&name, new_name))
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Ok(EntryStat {
                kind: EntryKind::Directory,
                size: 0,
                modified_at: None,
            });
        }

        let (directory, name) = self.split_remote(&parsed.sub_path);
        let use_mlst = self.mlsd_supported();

        self.run_in(&directory, |client| {
            // `MLST` answers kind, size and mtime in one command on the
            // servers that have it.
            if use_mlst
                && let Ok(line) = client.mlst(&name)
                && let Some(entry) = parse_mlsx_line(&line)
            {
                return Ok(EntryStat {
                    kind: entry.kind,
                    size: entry.size.unwrap_or(0),
                    modified_at: entry.modified_at,
                });
            }

            match client.size(&name) {
                Ok(size) => Ok(EntryStat {
                    kind: EntryKind::File,
                    size: size as u64,
                    modified_at: client.mdtm_millis(&name).ok().flatten(),
                }),
                // Most servers refuse SIZE for a directory, so the directory
                // probe is a CWD into it.
                Err(error) if is_missing(&error) => {
                    let modified_at = client.mdtm_millis(&name).ok().flatten();
                    match client.cwd(&name) {
                        Ok(()) => Ok(EntryStat {
                            kind: EntryKind::Directory,
                            size: 0,
                            modified_at,
                        }),
                        Err(_) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Ok(());
        }

        let (directory, name) = self.split_remote(&parsed.sub_path);
        let exists = self.run_in(&directory, |client| Ok(probe_exists(client, &name)))?;
        if exists {
            return Err(FileSystemError::AlreadyExists(parsed.url()));
        }

        self.run_in(&directory, |client| client.mkdir(&name))
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }

        let (directory, name) = self.split_remote(&parsed.sub_path);
        let (client, pool) = self.lease_in(&directory)?.detach();
        begin_read(client, pool, &directory, &name)
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Not a file path: {path}"
            )));
        }

        let (directory, name) = self.split_remote(&parsed.sub_path);
        let (client, pool) = self.lease_in(&directory)?.detach();
        begin_write(client, pool, &directory, &name)
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Refusing to delete the server root: {path}"
            )));
        }

        // `DELE` on a directory is refused by every server worth talking to,
        // so the kind decides the command.
        let kind = self.stat(path)?.kind;
        let (directory, name) = self.split_remote(&parsed.sub_path);
        self.run_in(&directory, |client| {
            if kind == EntryKind::Directory {
                client.rmdir(&name)
            } else {
                client.rm(&name)
            }
        })
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from = parse_dir(source)?;
        let to = parse_dir(destination)?;

        // FTP cannot rename across servers; the transfer engine falls back to
        // copy + delete on error.
        if from.authority() != to.authority() {
            return Err(FileSystemError::InvalidInput(
                "Cannot rename across FTP servers".into(),
            ));
        }

        let (directory, name) = self.split_remote(&from.sub_path);
        let target = self.remote(&to.sub_path);
        self.run_in(&directory, |client| client.rename(&name, &target))
    }
}

/// Whether an entry of `name` exists in the current directory.
///
/// `SIZE` is the cheapest answer for a file, `MLST` the cheapest for a
/// directory, and `MDTM` the fallback for a server that refuses both — it
/// answers for files *and* directories. `CWD` catches the last case, a
/// directory-only server, because `CWD` into a file always fails.
fn probe_exists(client: &mut FtpClient, name: &str) -> bool {
    if client.size(name).is_ok() {
        return true;
    }
    if client.mlst(name).is_ok() {
        return true;
    }
    if client.mdtm_millis(name).is_ok() {
        return true;
    }
    client.cwd(name).is_ok()
}

/// The pieces of an `ftp://[user@]host[:port]/path` URL after the scheme.
struct FtpPath {
    /// Only ever set when the URL named an account; the backend keeps it, the
    /// canonical URL does not.
    username: Option<String>,
    host: String,
    port: Option<u16>,
    /// Path beneath the login directory, "" at the root, "/"-separated.
    sub_path: String,
}

impl FtpPath {
    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }

    /// The canonical URL, normalizing away trailing separators.
    fn url(&self) -> String {
        if self.sub_path.is_empty() {
            format!("ftp://{}", self.authority())
        } else {
            format!("ftp://{}/{}", self.authority(), self.sub_path)
        }
    }

    fn join(&self, name: &str) -> String {
        if self.sub_path.is_empty() {
            format!("ftp://{}/{}", self.authority(), name)
        } else {
            format!("ftp://{}/{}/{}", self.authority(), self.sub_path, name)
        }
    }
}

/// Splits a leading scheme off `rest`, for callers that pass the whole URL.
fn split_scheme_prefix(rest: &str) -> (&str, Option<TlsMode>) {
    if let Some(rest) = rest.strip_prefix("ftps://") {
        (rest, Some(TlsMode::Explicit))
    } else if let Some(rest) = rest.strip_prefix("ftp://") {
        (rest, Some(TlsMode::Plain))
    } else {
        (rest, None)
    }
}

fn parse_ftp_path(rest: &str) -> Result<FtpPath, FileSystemError> {
    let (authority, remainder) = split_authority(rest);
    let (username, host_port) = match authority.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() => (Some(user.to_owned()), host),
        _ => (None, authority),
    };
    let (host, port) = parse_authority(host_port)?;

    let sub_path = remainder
        .unwrap_or("")
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/");

    Ok(FtpPath {
        username,
        host,
        port,
        sub_path,
    })
}

fn parse_dir(path: &str) -> Result<FtpPath, FileSystemError> {
    let (scheme, rest) = super::vfs::split_scheme(path)?;
    if scheme != super::vfs::Scheme::Ftp {
        return Err(FileSystemError::InvalidInput(format!(
            "Not an FTP path: {path}"
        )));
    }
    parse_ftp_path(rest)
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(FileSystemError::InvalidInput(format!(
            "Invalid entry name: {name}"
        )));
    }
    Ok(())
}

fn ftp_breadcrumbs(path: &FtpPath) -> Vec<Breadcrumb> {
    let mut breadcrumbs = vec![Breadcrumb {
        name: path.host.clone(),
        path: format!("ftp://{}", path.authority()),
    }];

    let mut accumulated = format!("ftp://{}", path.authority());
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

/// A connection whose control channel is still in sync can be reused. A reply
/// that arrived (even an error) leaves the stream aligned; anything below that
/// — a socket error, an unparsable reply, a refused data connection — does not.
fn connection_survives(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(_) | FtpError::DataConnectionAlreadyOpen
    )
}

fn map_login_error(error: FtpError) -> FileSystemError {
    if let FtpError::UnexpectedResponse(response) = &error
        && response.status.code() == 530
    {
        return FileSystemError::PermissionDenied(
            "The FTP server rejected the account or password. Check the credentials in the connect dialog.".into(),
        );
    }
    map_ftp_error(error)
}

fn map_ftp_error(error: FtpError) -> FileSystemError {
    match &error {
        FtpError::UnexpectedResponse(response) => {
            let message = format!("{response}");
            match response.status.code() {
                530 => FileSystemError::PermissionDenied(message),
                550 | 551 => FileSystemError::NotFound(message),
                552 | 553 => FileSystemError::InvalidInput(message),
                500 | 501 | 502 | 504 => FileSystemError::Unsupported(message),
                _ => FileSystemError::Io(message),
            }
        }
        FtpError::SecureError(reason) => {
            FileSystemError::PermissionDenied(format!("The TLS handshake failed: {reason}"))
        }
        FtpError::InvalidAddress(reason) => {
            FileSystemError::InvalidInput(format!("Invalid FTP server address: {reason}"))
        }
        FtpError::ConnectionError(_)
        | FtpError::BadResponse
        | FtpError::DataConnectionAlreadyOpen => FileSystemError::Io(error.to_string()),
    }
}

/// The server does not know the command (as opposed to refusing the path).
fn is_command_unsupported(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(response)
            if matches!(response.status.code(), 500 | 502 | 504)
    )
}

/// The path is absent, or the command does not apply to it (`SIZE` on a
/// directory). Either way the caller falls back to probing.
fn is_missing(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(response)
            if matches!(response.status.code(), 450 | 550)
    )
}

/// One directory entry as the listing parsers see it.
#[derive(Debug)]
struct ParsedEntry {
    name: String,
    kind: EntryKind,
    size: Option<u64>,
    modified_at: Option<u64>,
    read_only: bool,
}

/// Parses one `MLSD` line: `fact=value;...; name`, where the name may contain
/// spaces and the fact list may carry keys we ignore.
fn parse_mlsx_line(line: &str) -> Option<ParsedEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() || !facts.contains('=') {
        return None;
    }

    let mut entry_type: Option<String> = None;
    let mut size = None;
    let mut modified_at = None;
    let mut mode = None;
    let mut perm: Option<String> = None;

    for fact in facts.split(';') {
        let Some((key, value)) = fact.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "type" => entry_type = Some(value.to_ascii_lowercase()),
            "size" => size = value.parse::<u64>().ok(),
            "modify" => modified_at = parse_mlsx_stamp(value),
            "unix.mode" => mode = u32::from_str_radix(value, 8).ok(),
            "perm" => perm = Some(value.to_owned()),
            _ => {}
        }
    }

    let kind = match entry_type.as_deref() {
        // The directory itself and its parent: not entries of the listing.
        Some("cdir") | Some("pdir") => return None,
        Some("dir") => EntryKind::Directory,
        Some("file") => EntryKind::File,
        Some(kind) if kind.starts_with("os.unix=slink") => EntryKind::Symlink,
        Some(_) => EntryKind::Other,
        None if size.is_some() => EntryKind::File,
        None => EntryKind::Other,
    };

    let read_only = match mode {
        Some(mode) => mode & 0o200 == 0,
        None => perm.as_deref().is_some_and(|perm| !perm.contains('w')),
    };

    Some(ParsedEntry {
        name: name.to_owned(),
        kind,
        size: (kind != EntryKind::Directory).then_some(size).flatten(),
        modified_at,
        read_only,
    })
}

/// Parses one Unix `ls -l` style `LIST` line.
///
/// Only the part every server agrees on is required: the type character, a
/// numeric size somewhere before the month, the month/day/time-or-year stamp,
/// and then the name to the end of the line. Owner, group and link count are
/// skipped whatever they contain, which also covers servers that omit one of
/// them, and the name keeps its spaces because it is sliced out of the line.
fn parse_list_line(line: &str, now: SystemTime) -> Option<ParsedEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line == "total" || line.starts_with("total ") {
        return None;
    }

    let fields = field_ranges(line);
    if fields.len() < 6 {
        return None;
    }

    let perms = field(line, fields[0])?;
    let kind = match perms.as_bytes().first()? {
        b'd' => EntryKind::Directory,
        b'l' => EntryKind::Symlink,
        b'-' => EntryKind::File,
        b'b' | b'c' | b'p' | b's' => EntryKind::Other,
        _ => return None,
    };

    let month_index = (2..fields.len()).find(|index| {
        field(line, fields[*index]).is_some_and(|token| month_number(token).is_some())
    })?;

    let size = field(line, *fields.get(month_index.checked_sub(1)?)?)?
        .parse::<u64>()
        .ok()?;
    let month = month_number(field(line, *fields.get(month_index)?)?)?;
    let day = field(line, *fields.get(month_index + 1)?)?
        .parse::<u32>()
        .ok()?;
    let stamp = field(line, *fields.get(month_index + 2)?)?;
    let name_field = fields.get(month_index + 3)?;
    let raw_name = line.get(name_field.0..)?;

    // A symlink is listed as "name -> target"; the entry is the link itself.
    let name = if kind == EntryKind::Symlink {
        raw_name
            .split_once(" -> ")
            .map_or(raw_name, |(name, _)| name)
    } else {
        raw_name
    };
    if name.is_empty() {
        return None;
    }

    Some(ParsedEntry {
        name: name.to_owned(),
        kind,
        size: (kind != EntryKind::Directory).then_some(size),
        modified_at: parse_list_stamp(month, day, stamp, now),
        read_only: perms.as_bytes().get(2) == Some(&b'-'),
    })
}

/// Byte ranges of the space/tab separated fields of `line`.
fn field_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, character) in line.char_indices() {
        if character == ' ' || character == '\t' {
            if let Some(start) = start.take() {
                ranges.push((start, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(start) = start {
        ranges.push((start, line.len()));
    }
    ranges
}

fn field(line: &str, range: (usize, usize)) -> Option<&str> {
    line.get(range.0..range.1)
}

/// `Jan`..`Dec`, matched on the first three letters because servers spell the
/// month out both ways.
fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = name.get(..3)?;
    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(prefix))
        .map(|index| index as u32 + 1)
}

/// `MMM dd HH:MM` is this year's time, unless that would be in the future —
/// `ls` omits the year only for recent files — and `MMM dd YYYY` is midnight.
fn parse_list_stamp(month: u32, day: u32, stamp: &str, now: SystemTime) -> Option<u64> {
    let mut parts = stamp.split(':');
    if let Some(hour) = parts.next().filter(|_| stamp.contains(':')) {
        let hour: u32 = hour.parse().ok()?;
        let minute: u32 = parts.next()?.parse().ok()?;
        let year = utc_year(now);
        let candidate = civil_to_epoch_millis(year, month, day, hour, minute, 0)?;
        let now_millis = now
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        if candidate > now_millis.saturating_add(MILLIS_PER_DAY) {
            civil_to_epoch_millis(year - 1, month, day, hour, minute, 0)
        } else {
            Some(candidate)
        }
    } else {
        let year: i64 = stamp.parse().ok()?;
        civil_to_epoch_millis(year, month, day, 0, 0, 0)
    }
}

/// `YYYYMMDDHHMMSS[.sss]`, always UTC.
fn parse_mlsx_stamp(value: &str) -> Option<u64> {
    let digits = value.split('.').next()?;
    if digits.len() != 14 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = digits.get(0..4)?.parse::<i64>().ok()?;
    let month = digits.get(4..6)?.parse::<u32>().ok()?;
    let day = digits.get(6..8)?.parse::<u32>().ok()?;
    let hour = digits.get(8..10)?.parse::<u32>().ok()?;
    let minute = digits.get(10..12)?.parse::<u32>().ok()?;
    let second = digits.get(12..14)?.parse::<u32>().ok()?;
    civil_to_epoch_millis(year, month, day, hour, minute, second)
}

fn civil_to_epoch_millis(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    if second > 60 {
        return None;
    }

    let seconds = days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3_600
        + minute as i64 * 60
        + second as i64;
    u64::try_from(seconds).ok().map(|seconds| seconds * 1_000)
}

fn utc_year(now: SystemTime) -> i64 {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    civil_from_days((seconds / 86_400) as i64).0
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month as i64 + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = (if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    }) / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }) as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod ftp_tests {
    use super::*;

    /// 2025-06-15T15:06:40Z, the "now" the year-less LIST stamps resolve
    /// against: mid-year, so a January stamp is in the past and a December one
    /// is in the future.
    fn now_2025() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_750_000_000)
    }

    #[test]
    fn parses_posix_listings() {
        let entry = parse_list_line(
            "-rw-r--r--    1 1000     1000         1234 Jan 02 12:34 report.pdf",
            now_2025(),
        )
        .expect("a file line");
        assert_eq!(entry.name, "report.pdf");
        assert_eq!(entry.kind, EntryKind::File);
        assert_eq!(entry.size, Some(1234));
        assert_eq!(entry.modified_at, Some(1_735_821_240_000));
        assert!(!entry.read_only);

        let entry = parse_list_line(
            "drwxr-xr-x    2 0        0            4096 Nov 05  2023 archive",
            now_2025(),
        )
        .expect("a directory line");
        assert_eq!(entry.kind, EntryKind::Directory);
        assert_eq!(entry.name, "archive");
        assert_eq!(entry.size, None, "a directory has no file size");
        assert_eq!(entry.modified_at, Some(1_699_142_400_000));

        let entry = parse_list_line(
            "-rw-r--r--    1 ftp      ftp            12 Jan  2  2024 notes with spaces.txt",
            now_2025(),
        )
        .expect("a name with spaces");
        assert_eq!(entry.name, "notes with spaces.txt");
        assert_eq!(entry.size, Some(12));
        assert_eq!(entry.modified_at, Some(1_704_153_600_000));

        let entry = parse_list_line(
            "lrwxrwxrwx    1 ftp      ftp             6 Jan 02 12:34 latest -> report.pdf",
            now_2025(),
        )
        .expect("a symlink line");
        assert_eq!(entry.kind, EntryKind::Symlink);
        assert_eq!(entry.name, "latest");

        // Servers that omit the group column are common enough to accept.
        let entry = parse_list_line(
            "-rw-r--r-- 1 owner 1234 Jan 2 12:34 no-group.txt",
            now_2025(),
        )
        .expect("a group-less line");
        assert_eq!(entry.name, "no-group.txt");
        assert_eq!(entry.size, Some(1234));

        let entry = parse_list_line(
            "-r--r--r-- 1 ftp ftp 5 Jan 02 12:34 read-only.txt",
            now_2025(),
        )
        .expect("a read-only line");
        assert!(entry.read_only);
    }

    #[test]
    fn resolves_year_less_list_stamps_backwards() {
        // `ls` prints a time only for recent files, so a stamp that would land
        // in the future is last year's.
        let entry = parse_list_line("-rw-r--r-- 1 ftp ftp 5 Dec 31 23:00 old.txt", now_2025())
            .expect("a line from last year");
        assert_eq!(entry.modified_at, Some(1_735_686_000_000));
    }

    #[test]
    fn ignores_lines_that_are_not_entries() {
        for line in ["total 24", "total", "", "garbage", "drwxr-xr-x"] {
            assert!(
                parse_list_line(line, now_2025()).is_none(),
                "should not parse: {line:?}"
            );
        }
    }

    #[test]
    fn parses_mlsx_listings() {
        let entry =
            parse_mlsx_line("type=file;size=1024;modify=20240102153045;UNIX.mode=0644; report.pdf")
                .expect("a file line");
        assert_eq!(entry.name, "report.pdf");
        assert_eq!(entry.kind, EntryKind::File);
        assert_eq!(entry.size, Some(1024));
        assert_eq!(entry.modified_at, Some(1_704_209_445_000));
        assert!(!entry.read_only);

        let entry = parse_mlsx_line("type=dir;modify=20240102153045; photos").expect("a directory");
        assert_eq!(entry.kind, EntryKind::Directory);
        assert_eq!(entry.name, "photos");
        assert_eq!(entry.size, None);

        let entry =
            parse_mlsx_line("type=file;size=0; my holiday photo.jpg").expect("spaces in the name");
        assert_eq!(entry.name, "my holiday photo.jpg");
        assert_eq!(entry.size, Some(0));

        let entry =
            parse_mlsx_line("type=OS.unix=slink:/etc/passwd;size=11; passwd").expect("a symlink");
        assert_eq!(entry.kind, EntryKind::Symlink);
        assert_eq!(entry.name, "passwd");

        let entry =
            parse_mlsx_line("type=file;size=1;perm=r; readme.txt").expect("a read-only file");
        assert!(entry.read_only);

        assert!(parse_mlsx_line("type=cdir;modify=20240102153045; .").is_none());
        assert!(parse_mlsx_line("type=pdir;modify=20240102153045; ..").is_none());
        assert!(parse_mlsx_line("not a listing").is_none());
    }

    #[test]
    fn parses_ftp_urls() {
        let path = parse_ftp_path("user@nas.local:2121/pub/docs").expect("a URL with an account");
        assert_eq!(path.username.as_deref(), Some("user"));
        assert_eq!(path.host, "nas.local");
        assert_eq!(path.port, Some(2121));
        assert_eq!(path.sub_path, "pub/docs");
        assert_eq!(path.url(), "ftp://nas.local:2121/pub/docs");

        let root = parse_ftp_path("nas.local").expect("a bare authority");
        assert_eq!(root.sub_path, "");
        assert_eq!(root.url(), "ftp://nas.local");

        let trailing = parse_ftp_path("nas.local/pub/docs/").expect("a trailing separator");
        assert_eq!(trailing.sub_path, "pub/docs");

        let dotted = parse_ftp_path("nas.local/pub/../docs/./").expect("dot segments");
        assert_eq!(dotted.sub_path, "pub/docs");

        let ipv6 = parse_ftp_path("[::1]:2121/pub").expect("an IPv6 literal");
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.port, Some(2121));
        assert_eq!(tls_domain(&ipv6.host), "::1");
    }

    #[test]
    fn rejects_malformed_hosts_and_ports() {
        assert!(parse_ftp_path(":21/pub").is_err());
        assert!(matches!(
            parse_ftp_path("nas.local:99999/pub"),
            Err(FileSystemError::InvalidInput(_))
        ));
    }

    #[test]
    fn recognizes_the_ftps_scheme() {
        assert_eq!(
            split_scheme_prefix("ftps://nas.local/pub"),
            ("nas.local/pub", Some(TlsMode::Explicit))
        );
        assert_eq!(
            split_scheme_prefix("ftp://nas.local/pub"),
            ("nas.local/pub", Some(TlsMode::Plain))
        );
        assert_eq!(
            split_scheme_prefix("nas.local/pub"),
            ("nas.local/pub", None)
        );
    }

    #[test]
    fn normalizes_the_login_directory() {
        assert_eq!(normalize_base("/"), "/");
        assert_eq!(normalize_base("/home/user"), "/home/user");
        assert_eq!(normalize_base("/home/user/"), "/home/user");
        assert_eq!(normalize_base("//home//user"), "/home/user");
        assert_eq!(normalize_base(""), "/");
    }

    #[test]
    fn joins_child_paths_and_builds_breadcrumbs() {
        let path = parse_ftp_path("nas.local:2121/pub").expect("a URL");
        assert_eq!(path.join("new.txt"), "ftp://nas.local:2121/pub/new.txt");

        let breadcrumbs = ftp_breadcrumbs(&path);
        let names: Vec<_> = breadcrumbs
            .iter()
            .map(|crumb| crumb.name.as_str())
            .collect();
        assert_eq!(names, vec!["nas.local", "pub"]);
        assert_eq!(breadcrumbs[0].path, "ftp://nas.local:2121");
        assert_eq!(breadcrumbs[1].path, "ftp://nas.local:2121/pub");

        let root = parse_ftp_path("nas.local").expect("a bare authority");
        assert_eq!(ftp_breadcrumbs(&root).len(), 1);
    }

    #[test]
    fn converts_civil_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));

        assert_eq!(
            civil_to_epoch_millis(2025, 1, 2, 12, 34, 0),
            Some(1_735_821_240_000)
        );
        assert_eq!(
            civil_to_epoch_millis(2024, 1, 2, 15, 30, 45),
            Some(1_704_209_445_000)
        );
        assert_eq!(civil_to_epoch_millis(1969, 12, 31, 23, 59, 59), None);
        assert_eq!(civil_to_epoch_millis(2025, 13, 1, 0, 0, 0), None);
    }
}
