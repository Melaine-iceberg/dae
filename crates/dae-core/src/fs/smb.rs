//! The SMB backend: one `SmbBackend` instance per server, reached through
//! `smb://host[:port]/share/path` URLs. All tree-shaped transfers are served
//! by the generic engine in `transfer.rs`; this module only implements
//! single-server operations.

use super::connections::{self, Protocol, SaveConnectionInput};
use super::error::FileSystemError;
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, NewEntryKind, entry_sort_key,
};
use super::url::{EntryUrl, Scheme};
use super::vfs::{FileSystemBackend, SharedBackend};
use smb2::{ErrorKind as SmbErrorKind, FileReader, FileWriter, SmbClient, Tree};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

const DEFAULT_PORT: u16 = 445;
const READ_CHUNK: u64 = 256 * 1024;

/// Sessions one backend keeps open at a time.
///
/// The `smb2` client takes `&mut self` for every metadata call, so a single
/// session carries exactly one of them at a time and concurrency has to come
/// from having more than one. Four is the width the cross-backend transfer
/// engine runs at (see `transfer::CROSS_BACKEND_CONCURRENCY`), so a batch
/// keeps every worker fed without opening a session per file on servers that
/// count sessions per user.
///
/// Sizing it to the engine is also what keeps waiting rare, and waiting short:
/// a session is held for one metadata round trip, never for the bytes that
/// follow it, so the four are free most of the time even mid-transfer. No
/// caller ever holds two at once — every operation takes one and gives it back
/// before the next — which is why a pool this size cannot run out of sessions
/// among the engine's own workers.
const MAX_SESSIONS: usize = 4;

/// How long a caller waits for a session before giving up. Generous, because
/// the wait is only ever as long as one metadata round trip; it exists so a
/// session that is somehow never handed back reports an error instead of
/// hanging the operation forever.
const SESSION_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct SmbBackend {
    runtime: Arc<tokio::runtime::Runtime>,
    /// Kept so the pool can open another session once the existing ones are all
    /// busy. Credentials already live in memory for the connection registry, so
    /// holding them here adds no new exposure.
    host: String,
    port: Option<u16>,
    username: String,
    password: String,
    pool: ConnectionPool<SmbState>,
}

/// One authenticated client, with the shares it has connected on demand.
///
/// Nothing outside the pool holds this: a session is checked out for the
/// duration of one operation and handed straight back, so a slow transfer never
/// pins one. Connection-wide state (credits, waiters, session keys) is shared
/// by the `Connection` handle inside `client`, so the open file handles a
/// transfer keeps streaming from stay valid after their session goes back.
struct SmbState {
    client: SmbClient,
    trees: HashMap<String, Tree>,
}

/// Why a [`ConnectionPool::checkout`] did not produce a connection.
enum PoolError<E> {
    /// The pool tried to open one and the backend refused.
    Open(E),
    /// The cap was reached and nothing came back in time.
    TimedOut,
}

/// A bounded pool that hands out one connection per operation.
///
/// The bookkeeping — the cap, the wait, and the return-on-drop — is where a
/// pool deadlocks or leaks when it is wrong, so it lives here as one piece
/// generic over the connection type. That keeps it testable without a server
/// to talk to, which matters because the bugs it could otherwise hide are a
/// hang or a slot that is never usable again.
struct ConnectionPool<C> {
    state: Mutex<PoolState<C>>,
    /// Signalled whenever a connection comes back, waking a caller that found
    /// the pool empty and at its cap.
    returned: Condvar,
}

/// What the pool is currently holding. `idle` and `live` only ever move
/// together: a connection is in exactly one of the two, never both.
struct PoolState<C> {
    idle: Vec<C>,
    /// Connections handed out plus the ones sitting in `idle`.
    live: usize,
}

/// A checked-out connection. Dropping it hands the connection back to the pool
/// unless it was discarded, in which case its slot is freed instead.
struct Checkout<'a, C> {
    pool: &'a ConnectionPool<C>,
    connection: Option<C>,
    reusable: bool,
}

impl<C> ConnectionPool<C> {
    fn new() -> Self {
        Self {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
            }),
            returned: Condvar::new(),
        }
    }

    /// Puts the connection opened at startup into the pool, so the first
    /// operation reuses the handshake the backend already paid for.
    fn seed(&self, connection: C) {
        let mut state = self.lock();
        state.idle.push(connection);
        state.live = 1;
    }

    /// Hands out a connection, opening another when the pool is empty and the
    /// cap allows it, and waiting up to `wait` for one to come back otherwise.
    fn checkout<E>(
        &self,
        max: usize,
        wait: Duration,
        open: impl Fn() -> Result<C, E>,
    ) -> Result<Checkout<'_, C>, PoolError<E>> {
        let deadline = Instant::now() + wait;

        loop {
            let mut state = self.lock();

            if let Some(connection) = state.idle.pop() {
                return Ok(Checkout {
                    pool: self,
                    connection: Some(connection),
                    reusable: true,
                });
            }

            if state.live < max {
                // Claim the slot before opening: holding the lock across the
                // handshake would stall every other operation, and claiming
                // afterwards would let two callers open past the cap.
                state.live += 1;
                drop(state);

                return match open() {
                    Ok(connection) => Ok(Checkout {
                        pool: self,
                        connection: Some(connection),
                        reusable: true,
                    }),
                    Err(error) => {
                        let mut state = self.lock();
                        state.live -= 1;
                        drop(state);
                        self.returned.notify_one();
                        Err(PoolError::Open(error))
                    }
                };
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(PoolError::TimedOut);
            }

            // `wait_timeout` releases the lock as part of waiting, so the check
            // above and this wait cannot be split by a return that would be
            // missed — which is what keeps the wakeup from being lost.
            let (state, _) = self
                .returned
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            drop(state);
        }
    }

    /// The pool lock. A panic while a connection was checked out must not wedge
    /// the backend for the rest of the process, so poisoning is recovered from
    /// rather than propagated.
    fn lock(&self) -> MutexGuard<'_, PoolState<C>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl<C> Checkout<'_, C> {
    fn get_mut(&mut self) -> &mut C {
        self.connection.as_mut().expect("connection checked out")
    }

    /// Drops the connection instead of pooling it. One that just failed below
    /// the protocol is not worth handing to the next caller, who would only
    /// inherit the corpse.
    fn discard(&mut self) {
        self.reusable = false;
    }
}

impl<C> Drop for Checkout<'_, C> {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };

        let mut dropped = None;
        {
            let mut state = self.pool.lock();
            if self.reusable {
                state.idle.push(connection);
            } else {
                state.live -= 1;
                dropped = Some(connection);
            }
        }

        // Closing a discarded connection touches the network, so it happens off
        // the lock; waking a waiter afterwards avoids notifying while the lock
        // is still held.
        drop(dropped);
        self.pool.returned.notify_one();
    }
}

/// Splits off the parent URL and the entry name for a rename. Shares are
/// server-side configuration, so only entries inside a share can be renamed;
/// the server root, where `read_dir` lists the shares, is not an entry at all.
fn parent_and_name(url: &EntryUrl) -> Result<(EntryUrl, String), FileSystemError> {
    let Some((parent, name)) = url.pop() else {
        return Err(FileSystemError::InvalidInput(
            "The share root cannot be renamed".into(),
        ));
    };

    // A parent with no segments means the entry being renamed is a share.
    if parent.segments().is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Shares cannot be renamed".into(),
        ));
    }

    Ok((parent, name))
}

/// Opens (or reuses) the session for the server an `smb://…` URL points at
/// and returns it as a backend. Called by `vfs::resolve`.
pub fn open_backend(url: &EntryUrl) -> Result<SharedBackend, FileSystemError> {
    let server = url.server().expect("server");

    if let Some(existing) = connections::session_for(Protocol::Smb, &server.host, server.port) {
        return Ok(existing);
    }

    let backend: SharedBackend = Arc::new(SmbBackend::connect(&server.host, server.port)?);
    connections::store_session(Protocol::Smb, &server.host, server.port, backend.clone());
    Ok(backend)
}

impl SmbBackend {
    /// Connects using credentials from the connection store, falling back to
    /// an anonymous (guest) session when nothing was saved.
    pub fn connect(host: &str, port: Option<u16>) -> Result<Self, FileSystemError> {
        let (username, password) = connections::resolve_credentials(Protocol::Smb, host, port);
        Self::connect_with(host, port, username.as_deref(), password.as_deref())
    }

    pub fn connect_with(
        host: &str,
        port: Option<u16>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, FileSystemError> {
        let runtime = Arc::new(
            tokio::runtime::Runtime::new()
                .map_err(|error| FileSystemError::Io(error.to_string()))?,
        );

        let backend = Self {
            runtime,
            host: host.to_owned(),
            port,
            username: username.unwrap_or_default().to_owned(),
            password: password.unwrap_or_default().to_owned(),
            pool: ConnectionPool::new(),
        };

        // Authenticate once up front so bad credentials or an unreachable host
        // surface on connect rather than on the first listing.
        backend.pool.seed(backend.open_session()?);

        Ok(backend)
    }

    /// Opens and authenticates a fresh session. Shares attach on first use.
    fn open_session(&self) -> Result<SmbState, FileSystemError> {
        let addr = format!("{}:{}", self.host, self.port.unwrap_or(DEFAULT_PORT));
        let client = self
            .runtime
            .block_on(async { smb2::connect(&addr, &self.username, &self.password).await })
            .map_err(map_connect_error)?;

        Ok(SmbState {
            client,
            trees: HashMap::new(),
        })
    }

    /// Takes a session out of the pool for one operation.
    fn session(&self) -> Result<Checkout<'_, SmbState>, FileSystemError> {
        self.pool
            .checkout(MAX_SESSIONS, SESSION_WAIT_TIMEOUT, || self.open_session())
            .map_err(|error| match error {
                PoolError::Open(error) => error,
                PoolError::TimedOut => {
                    FileSystemError::Io("Timed out waiting for a free SMB session".into())
                }
            })
    }

    fn share_names(&self) -> Result<Vec<String>, FileSystemError> {
        let mut session = self.session()?;
        let shares = self
            .runtime
            .block_on(session.get_mut().client.list_shares());

        match shares {
            Ok(shares) => Ok(shares.into_iter().map(|share| share.name).collect()),
            Err(error) => {
                if !session_survives(&error) {
                    session.discard();
                }
                Err(map_smb_error(error))
            }
        }
    }
}

/// Runs `body` with the client and the share's tree connected, on a session of
/// its own so several operations can be in flight against the same server.
///
/// The lock is only held long enough to find (or open) the tree; the operation
/// then runs on a checked-out session. Connect-on-demand keeps idle sessions
/// cheap, and a failure below the protocol retires the session rather than
/// returning it to the pool.
macro_rules! smb_tree_op {
    ($self:expr, $share:expr, ($client:ident, $tree:ident) => $body:expr) => {{
        let mut session = $self.session()?;
        let SmbState { client, trees } = session.get_mut();

        if !trees.contains_key($share) {
            let connected = $self.runtime.block_on(client.connect_share($share));
            match connected {
                Ok(tree) => {
                    trees.insert($share.to_owned(), tree);
                }
                Err(error) => {
                    if !session_survives(&error) {
                        session.discard();
                    }
                    return Err(map_smb_error(error));
                }
            }
        }

        let $tree = trees.get_mut($share).expect("tree just connected");
        let $client = &mut *client;

        match $self.runtime.block_on(async { $body }) {
            Ok(value) => Ok(value),
            Err(error) => {
                if !session_survives(&error) {
                    session.discard();
                }
                Err(map_smb_error(error))
            }
        }
    }};
}

/// Whether a failure leaves the session fit to reuse.
///
/// Phrased as an allow-list of errors the server answered with — those leave
/// the connection in a clean state. Anything else, transport or protocol or a
/// kind this build has never heard of, retires the session: reconnecting costs
/// one handshake, while reusing a desynchronised connection fails every
/// operation that follows it.
fn session_survives(error: &smb2::Error) -> bool {
    matches!(
        error.kind(),
        SmbErrorKind::NotFound
            | SmbErrorKind::AlreadyExists
            | SmbErrorKind::AccessDenied
            | SmbErrorKind::SharingViolation
            | SmbErrorKind::IsADirectory
            | SmbErrorKind::NotADirectory
            | SmbErrorKind::DiskFull
            | SmbErrorKind::AuthRequired
            | SmbErrorKind::SigningRequired
    )
}

impl FileSystemBackend for SmbBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let url = parse_dir(path)?;
        let segments = url.segments();

        let entries = match segments.first() {
            // The server root lists the shares themselves.
            None => self
                .share_names()?
                .into_iter()
                .filter(|name| !name.ends_with('$'))
                .map(|name| DirectoryEntry {
                    path: url.join(&name).to_string(),
                    name,
                    kind: EntryKind::Directory,
                    modified_at: None,
                    created_at: None,
                    created_fallback: false,
                    size: None,
                    // The smb2 crate exposes no DOS attributes; stay neutral.
                    hidden: false,
                    read_only: false,
                })
                .collect::<Vec<_>>(),
            Some(share) => {
                let sub_path = segments[1..].join("/");
                let listed = smb_tree_op!(self, share.as_str(), (_client, tree) => {
                    tree_client_list(_client, tree, &sub_path).await
                })?;
                listed
                    .into_iter()
                    .filter(|entry| entry.name != "." && entry.name != "..")
                    .map(|entry| DirectoryEntry {
                        path: url.join(&entry.name).to_string(),
                        name: entry.name,
                        kind: if entry.is_directory {
                            EntryKind::Directory
                        } else {
                            EntryKind::File
                        },
                        modified_at: modified_millis(entry.modified),
                        // SMB carries no creation time through this client.
                        created_at: None,
                        created_fallback: false,
                        size: if entry.is_directory {
                            None
                        } else {
                            Some(entry.size)
                        },
                        hidden: false,
                        read_only: false,
                    })
                    .collect::<Vec<_>>()
            }
        };

        let mut entries = entries;
        entries.sort_by_cached_key(entry_sort_key);

        Ok(DirectoryView {
            path: url.to_string(),
            breadcrumbs: smb_breadcrumbs(&url),
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
        let url = parse_dir(directory)?;
        let segments = url.segments();
        let Some(share) = segments.first() else {
            return Err(FileSystemError::InvalidInput(
                "Choose a share before creating entries".into(),
            ));
        };

        let target = if segments.len() == 1 {
            name.to_owned()
        } else {
            format!("{}/{}", segments[1..].join("/"), name)
        };

        match kind {
            NewEntryKind::File => {
                // create_file_writer truncates, so the absence check must
                // come first to preserve the never-overwrite contract.
                let existing = smb_tree_op!(self, share.as_str(), (client, tree) => {
                    client.stat(tree, &target).await
                });
                match existing {
                    Ok(_) => {
                        return Err(FileSystemError::AlreadyExists(url.join(name).to_string()));
                    }
                    Err(FileSystemError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }

                smb_tree_op!(self, share.as_str(), (client, tree) => {
                    let writer = client.create_file_writer(tree, &target).await?;
                    writer.finish().await.map(|_| ())
                })?;
            }
            NewEntryKind::Directory => {
                smb_tree_op!(self, share.as_str(), (client, tree) => {
                    client.create_directory(tree, &target).await
                })?;
            }
        }

        Ok(url.join(name).to_string())
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let url = parse_dir(path)?;
        let (parent, _old_name) = parent_and_name(&url)?;
        let destination = parent.join(new_name);

        let share = url.segments().first().map(String::as_str).unwrap_or("");
        let from_sub = url.segments()[1..].join("/");
        let to_sub = destination.segments()[1..].join("/");

        smb_tree_op!(self, share, (client, tree) => {
            client.rename(tree, &from_sub, &to_sub).await
        })
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let url = parse_dir(path)?;
        let segments = url.segments();

        // The server root is the share list, which is always a directory.
        let Some(share) = segments.first() else {
            return Ok(EntryStat {
                kind: EntryKind::Directory,
                size: 0,
                modified_at: None,
            });
        };

        let sub_path = segments[1..].join("/");
        let info = smb_tree_op!(self, share.as_str(), (client, tree) => {
            client.stat(tree, &sub_path).await
        })?;

        Ok(EntryStat {
            kind: if info.is_directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: info.size,
            modified_at: None,
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let url = parse_dir(path)?;
        let segments = url.segments();
        let Some(share) = segments.first() else {
            return Err(FileSystemError::InvalidInput(
                "Choose a share before creating folders".into(),
            ));
        };

        // The share root exists by definition; creating it is a no-op.
        let sub_path = segments[1..].join("/");
        if sub_path.is_empty() {
            return Ok(());
        }

        smb_tree_op!(self, share.as_str(), (client, tree) => {
            client.create_directory(tree, &sub_path).await
        })
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let url = parse_dir(path)?;
        let (share, sub_path) = match url.segments() {
            [share, rest @ ..] if !rest.is_empty() => (share.as_str(), rest.join("/")),
            _ => return Err(FileSystemError::NotDirectory(path.to_owned())),
        };

        let reader = smb_tree_op!(self, share, (client, tree) => {
            client.open_file_reader(tree, &sub_path).await
        })?;

        Ok(Box::new(SmbReadAdapter {
            runtime: Arc::clone(&self.runtime),
            reader,
            offset: 0,
            buffer: Vec::new(),
            position: 0,
        }))
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let url = parse_dir(path)?;
        let (share, sub_path) = match url.segments() {
            [share, rest @ ..] if !rest.is_empty() => (share.as_str(), rest.join("/")),
            _ => {
                return Err(FileSystemError::InvalidInput(format!(
                    "Not a file path: {path}"
                )));
            }
        };

        let writer = smb_tree_op!(self, share, (client, tree) => {
            client.create_file_writer(tree, &sub_path).await
        })?;

        Ok(Box::new(SmbWriteAdapter {
            runtime: Arc::clone(&self.runtime),
            writer,
        }))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let url = parse_dir(path)?;
        let (share, sub_path) = match url.segments() {
            [share, rest @ ..] if !rest.is_empty() => (share.as_str(), rest.join("/")),
            _ => {
                return Err(FileSystemError::InvalidInput(format!(
                    "Refusing to delete a share root: {path}"
                )));
            }
        };

        let info = smb_tree_op!(self, share, (client, tree) => {
            client.stat(tree, &sub_path).await
        })?;

        if info.is_directory {
            smb_tree_op!(self, share, (client, tree) => {
                client.delete_directory(tree, &sub_path).await
            })
        } else {
            smb_tree_op!(self, share, (client, tree) => {
                client.delete_file(tree, &sub_path).await
            })
        }
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from = parse_dir(source)?;
        let to = parse_dir(destination)?;

        // The SMB protocol cannot rename across share boundaries; the
        // transfer engine falls back to copy + delete on error.
        if from.segments().first() != to.segments().first() {
            return Err(FileSystemError::InvalidInput(
                "Cannot rename across shares".into(),
            ));
        }

        let share = from.segments().first().map(String::as_str).unwrap_or("");
        let from_sub = from.segments()[1..].join("/");
        let to_sub = to.segments()[1..].join("/");
        smb_tree_op!(self, share, (client, tree) => {
            client.rename(tree, &from_sub, &to_sub).await
        })
    }
}

/// Parses one SMB URL, rejecting the paths of every other scheme.
fn parse_dir(path: &str) -> Result<EntryUrl, FileSystemError> {
    let url = EntryUrl::parse(path)?;
    if url.scheme() != Scheme::Smb {
        return Err(FileSystemError::InvalidInput(format!(
            "Not an SMB path: {path}"
        )));
    }
    Ok(url)
}

/// `list_directory` lives on the client, not the tree, so the macro's closure
/// forwards to this free function to keep the borrow shapes simple.
async fn tree_client_list(
    client: &mut SmbClient,
    tree: &mut Tree,
    sub_path: &str,
) -> smb2::Result<Vec<smb2::DirectoryEntry>> {
    client.list_directory(tree, sub_path).await
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(FileSystemError::InvalidInput(format!(
            "Invalid entry name: {name}"
        )));
    }
    Ok(())
}

fn modified_millis(time: smb2::pack::FileTime) -> Option<u64> {
    if time.0 == 0 {
        return None;
    }
    time.to_system_time()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
}

/// Breadcrumbs from the server root down, one per segment: the host, the
/// share, then every directory beneath it.
fn smb_breadcrumbs(url: &EntryUrl) -> Vec<Breadcrumb> {
    let server = url.server().expect("server");
    let mut breadcrumbs = vec![Breadcrumb {
        name: server.host.clone(),
        path: url.with_segments(Vec::new()).to_string(),
    }];

    let mut segments = Vec::new();
    for segment in url.segments() {
        segments.push(segment.clone());
        breadcrumbs.push(Breadcrumb {
            name: segment.clone(),
            path: url.with_segments(segments.clone()).to_string(),
        });
    }

    breadcrumbs
}

/// Adapts the positioned async `FileReader` to a byte-stream `Read`.
struct SmbReadAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    reader: FileReader,
    offset: u64,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for SmbReadAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.buffer.len() {
            if self.offset >= self.reader.size() {
                return Ok(0);
            }

            let chunk = self
                .runtime
                .block_on(self.reader.read_at(self.offset, READ_CHUNK))
                .map_err(|error| io::Error::other(error.to_string()))?;
            if chunk.is_empty() {
                return Ok(0);
            }
            self.buffer = chunk;
            self.position = 0;
        }

        let available = &self.buffer[self.position..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count;
        self.offset += count as u64;
        Ok(count)
    }
}

/// Adapts the push-based async `FileWriter` to a byte-stream `Write`.
struct SmbWriteAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    writer: FileWriter,
}

impl Write for SmbWriteAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.runtime
            .block_on(self.writer.write_chunk(buf))
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn map_connect_error(error: smb2::Error) -> FileSystemError {
    match error.kind() {
        // `GuestSessionRefused` used to arrive as `AuthRequired`, so it belongs
        // in this arm: the server answered a named login by offering a guest or
        // anonymous session, and the same advice applies — name an account, or
        // connect without one on purpose.
        SmbErrorKind::AuthRequired
        | SmbErrorKind::SigningRequired
        | SmbErrorKind::GuestSessionRefused => FileSystemError::PermissionDenied(format!(
            "The server rejected the connection: {error}. \
             Add an account and password in the connect dialog and try again."
        )),
        // Also split out of `AuthRequired`. This one is not a credential
        // problem — the response failed signature verification — so it gets its
        // own wording instead of advice that would send the user to retype a
        // password that was never the issue.
        SmbErrorKind::SignatureInvalid => FileSystemError::PermissionDenied(format!(
            "The server's response could not be verified: {error}"
        )),
        SmbErrorKind::AccessDenied => FileSystemError::PermissionDenied(error.to_string()),
        SmbErrorKind::TimedOut | SmbErrorKind::ConnectionLost => {
            FileSystemError::Io(format!("Could not reach the SMB server: {error}"))
        }
        _ => FileSystemError::Io(error.to_string()),
    }
}

fn map_smb_error(error: smb2::Error) -> FileSystemError {
    match error.kind() {
        SmbErrorKind::NotFound => FileSystemError::NotFound(error.to_string()),
        SmbErrorKind::AlreadyExists => FileSystemError::AlreadyExists(error.to_string()),
        SmbErrorKind::AccessDenied => FileSystemError::PermissionDenied(error.to_string()),
        SmbErrorKind::AuthRequired | SmbErrorKind::SigningRequired => {
            FileSystemError::PermissionDenied(format!("The server requires an account: {error}"))
        }
        // Reachable outside the handshake: any refused command whose response
        // failed signature verification lands here, and reporting it as an I/O
        // failure would blame the wrong thing.
        SmbErrorKind::SignatureInvalid => FileSystemError::PermissionDenied(format!(
            "The server's response could not be verified: {error}"
        )),
        SmbErrorKind::NotADirectory => FileSystemError::NotDirectory(error.to_string()),
        SmbErrorKind::IsADirectory => FileSystemError::InvalidInput(error.to_string()),
        _ => FileSystemError::Io(error.to_string()),
    }
}

/// See `commands::test_connection` for the contract; this half is engine-free.
pub async fn test_connection(input: SaveConnectionInput) -> Result<(), FileSystemError> {
    tokio::task::spawn_blocking(move || {
        let host = input.host.trim().to_owned();
        if host.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "A server host is required".into(),
            ));
        }

        match input.protocol {
            Protocol::Smb => {
                let backend = SmbBackend::connect_with(
                    &host,
                    input.port,
                    input.username.as_deref(),
                    input.password.as_deref(),
                )?;
                backend.share_names().map(|_| ())
            }
            Protocol::Sftp => super::sftp::test_backend(
                &host,
                input.port,
                input.username.as_deref(),
                input.password.as_deref(),
            ),
            Protocol::Ftp => super::ftp::test_backend(
                &host,
                input.port,
                input.username.as_deref(),
                input.password.as_deref(),
            ),
            Protocol::WebDav => super::webdav::test_backend(
                &host,
                input.port,
                input.username.as_deref(),
                input.password.as_deref(),
            ),
        }
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[cfg(test)]
mod smb_tests {
    use super::*;

    #[test]
    fn parses_smb_urls() {
        let path = EntryUrl::parse("smb://nas.local:445/media/movies/2024").unwrap();
        let server = path.server().expect("server");
        assert_eq!(server.host, "nas.local");
        assert_eq!(server.port, Some(445));
        assert_eq!(path.segments(), ["media", "movies", "2024"]);
        assert_eq!(path.to_string(), "smb://nas.local:445/media/movies/2024");

        let root = EntryUrl::parse("smb://nas.local").unwrap();
        assert!(root.segments().is_empty());
        assert_eq!(root.to_string(), "smb://nas.local");

        let share_root = EntryUrl::parse("smb://nas.local/media/").unwrap();
        assert_eq!(share_root.segments(), ["media"]);
        assert_eq!(share_root.to_string(), "smb://nas.local/media");

        let dotted = EntryUrl::parse("smb://nas.local/media/../public/./docs").unwrap();
        assert_eq!(dotted.segments(), ["media", "public", "docs"]);

        let ipv6 = EntryUrl::parse("smb://[::1]/media").unwrap();
        assert_eq!(ipv6.server().expect("server").host, "[::1]");
        assert_eq!(ipv6.segments(), ["media"]);
    }

    #[test]
    fn joins_child_paths_and_resolves_parents() {
        let path = EntryUrl::parse("smb://nas.local/media/movies").unwrap();
        assert_eq!(
            path.join("new.txt").to_string(),
            "smb://nas.local/media/movies/new.txt"
        );

        let (parent, name) = parent_and_name(&path).unwrap();
        assert_eq!(parent.to_string(), "smb://nas.local/media");
        assert_eq!(name, "movies");

        // Shares are server-side configuration, and the server root is not
        // an entry at all; neither can be renamed.
        let share_root = EntryUrl::parse("smb://nas.local/media").unwrap();
        match parent_and_name(&share_root) {
            Err(FileSystemError::InvalidInput(message)) => {
                assert_eq!(message, "Shares cannot be renamed");
            }
            other => panic!("renaming a share must be refused: {other:?}"),
        }

        let server_root = EntryUrl::parse("smb://nas.local").unwrap();
        match parent_and_name(&server_root) {
            Err(FileSystemError::InvalidInput(message)) => {
                assert_eq!(message, "The share root cannot be renamed");
            }
            other => panic!("renaming the server root must be refused: {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_hosts_and_ports() {
        assert!(EntryUrl::parse("smb://:445/media").is_err());
        let bad_port = EntryUrl::parse("smb://nas.local:99999/media");
        assert!(matches!(bad_port, Err(FileSystemError::InvalidInput(_))));
    }

    #[test]
    fn builds_share_aware_breadcrumbs() {
        let path = EntryUrl::parse("smb://nas.local/media/movies/2024").unwrap();
        let breadcrumbs = smb_breadcrumbs(&path);

        let names: Vec<_> = breadcrumbs
            .iter()
            .map(|crumb| crumb.name.as_str())
            .collect();
        assert_eq!(names, vec!["nas.local", "media", "movies", "2024"]);
        assert_eq!(breadcrumbs[0].path, "smb://nas.local");
        assert_eq!(breadcrumbs[1].path, "smb://nas.local/media");
        assert_eq!(breadcrumbs[3].path, "smb://nas.local/media/movies/2024");

        let root = EntryUrl::parse("smb://nas.local").unwrap();
        assert_eq!(smb_breadcrumbs(&root).len(), 1);
    }

    /// The pool is what lets several operations reach one server at once, and
    /// it cannot be exercised through `SmbBackend` without a server to connect
    /// to. The bookkeeping is generic for exactly that reason, so these drive it
    /// with plain integers standing in for sessions.
    mod pool {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        /// Long enough that a passing test never trips it, short enough that a
        /// hang still fails the suite.
        const PATIENT: Duration = Duration::from_secs(5);
        /// For the call that is meant to give up.
        const IMPATIENT: Duration = Duration::from_millis(30);

        #[test]
        fn reuses_the_connection_seeded_at_connect() {
            let pool = ConnectionPool::<usize>::new();
            pool.seed(42);

            {
                let mut first = pool
                    .checkout(4, PATIENT, || Ok::<_, ()>(0))
                    .ok()
                    .expect("checkout the seeded connection");
                assert_eq!(*first.get_mut(), 42);
            }

            // Opening would hand out 0, so seeing 42 again proves the connection
            // was pooled rather than replaced.
            let mut second = pool
                .checkout(4, PATIENT, || Ok::<_, ()>(0))
                .ok()
                .expect("checkout again");
            assert_eq!(*second.get_mut(), 42);
        }

        #[test]
        fn opens_up_to_the_cap_and_then_waits() {
            let pool = ConnectionPool::<usize>::new();
            let opens = AtomicUsize::new(0);
            let open = || {
                opens.fetch_add(1, AtomicOrdering::SeqCst);
                Ok::<_, ()>(opens.load(AtomicOrdering::SeqCst))
            };

            let mut first = pool
                .checkout(2, PATIENT, open)
                .ok()
                .expect("first checkout");
            let mut second = pool
                .checkout(2, PATIENT, open)
                .ok()
                .expect("second checkout");
            let first_id = *first.get_mut();
            let second_id = *second.get_mut();
            assert_ne!(first_id, second_id, "each concurrent caller gets its own");
            assert_eq!(opens.load(AtomicOrdering::SeqCst), 2);

            // A third caller must not push past the cap.
            let refused = pool
                .checkout(2, IMPATIENT, open)
                .err()
                .expect("the cap must hold");
            assert!(matches!(refused, PoolError::TimedOut));
            assert_eq!(opens.load(AtomicOrdering::SeqCst), 2);

            // Once one comes back it is reused rather than reopened.
            drop(first);
            let mut third = pool.checkout(2, PATIENT, open).ok().expect("reuse");
            assert_eq!(*third.get_mut(), first_id);
            assert_eq!(opens.load(AtomicOrdering::SeqCst), 2);

            drop(second);
            drop(third);
        }

        #[test]
        fn wakes_a_waiter_instead_of_timing_it_out() {
            let pool = ConnectionPool::<usize>::new();
            pool.seed(7);

            let held = pool
                .checkout(1, PATIENT, || Ok::<_, ()>(0))
                .ok()
                .expect("take the only session");

            std::thread::scope(|scope| {
                scope.spawn(|| {
                    std::thread::sleep(Duration::from_millis(50));
                    drop(held);
                });

                // The assertion is on how *promptly* the handover happens, not
                // just on it happening: a lost wakeup still eventually succeeds,
                // because `checkout` re-reads the pool once its wait expires —
                // it just sits there for the whole timeout first.
                let started = Instant::now();
                let mut released = pool
                    .checkout(1, PATIENT, || Ok::<_, ()>(0))
                    .ok()
                    .expect("the released session must be handed over");
                let waited = started.elapsed();

                assert_eq!(*released.get_mut(), 7);
                assert!(
                    waited < Duration::from_secs(1),
                    "waited {waited:?} for a session released after 50ms, so the wakeup was lost"
                );
            });
        }

        #[test]
        fn a_discarded_connection_frees_its_slot() {
            let pool = ConnectionPool::<usize>::new();
            let opens = AtomicUsize::new(0);
            let open = || {
                opens.fetch_add(1, AtomicOrdering::SeqCst);
                Ok::<_, ()>(opens.load(AtomicOrdering::SeqCst))
            };

            {
                let mut broken = pool.checkout(1, PATIENT, open).ok().expect("checkout");
                broken.discard();
            }

            // The slot has to be free again, so the next operation opens a fresh
            // connection instead of waiting for the one that was thrown away.
            let mut replacement = pool
                .checkout(1, PATIENT, open)
                .ok()
                .expect("a discarded session must release its slot");
            assert_eq!(*replacement.get_mut(), 2);
            assert_eq!(opens.load(AtomicOrdering::SeqCst), 2);
        }

        #[test]
        fn reports_a_refused_connection_without_leaking_the_slot() {
            let pool = ConnectionPool::<usize>::new();

            for _ in 0..2 {
                let refused = pool
                    .checkout(1, PATIENT, || Err::<usize, &str>("no server"))
                    .err()
                    .expect("the open must fail");
                assert!(matches!(refused, PoolError::Open("no server")));
            }

            // Each failure released its slot, so the pool can still hand one out.
            let mut connection = pool
                .checkout(1, PATIENT, || Ok::<_, &str>(9))
                .ok()
                .expect("a failed open must not consume the cap");
            assert_eq!(*connection.get_mut(), 9);
        }
    }
}
