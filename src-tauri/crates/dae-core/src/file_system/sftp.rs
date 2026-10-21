//! The SFTP backend: one `SftpBackend` per server, reached through
//! `sftp://host[:port]/abs/path` URLs. Tree-shaped transfers are served by
//! the generic engine in `transfer.rs`; this module implements the
//! single-server primitives, mirroring the SMB backend.
//!
//! Auth is password-based today; the username falls back to the local
//! account name when the connection store has none. Host keys are verified
//! against the user's OpenSSH `known_hosts` file and recorded on first
//! contact — see [`super::known_hosts`] for the policy and its limits.

use super::connections::{self, Protocol};
use super::error::FileSystemError;
use super::known_hosts::{HostKeyVerdict, verify_host_key};
use super::smb::{parse_authority, split_authority};
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, FileProperties, NewEntryKind,
    PlatformProperties, PropertyChanges, SearchEntry, SearchResponse, UnixProperties,
    entry_sort_key,
};
use super::vfs::{FileSystemBackend, SharedBackend};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::File as SftpFile;
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEFAULT_PORT: u16 = 22;
/// Per-request response timeout; transfers of large files issue many small
/// requests, so this only bounds each one, not the whole stream.
const SESSION_TIMEOUT_SECS: u64 = 30;
const READ_CHUNK: usize = 256 * 1024;
const MAX_SEARCH_RESULTS: usize = 200;

pub struct SftpBackend {
    runtime: Arc<tokio::runtime::Runtime>,
    /// All `SftpSession` methods take `&self` (requests are multiplexed over
    /// one channel), so unlike SMB no session lock is needed.
    sftp: SftpSession,
}

/// Verifies the server's host key against the user's OpenSSH `known_hosts`
/// file, recording it on first contact (trust on first use) and refusing a key
/// that changed since. See [`super::known_hosts`] for why that file and what
/// the policy cannot protect against.
///
/// A `russh` handler can only answer the handshake with yes/no, and `Ok(false)`
/// surfaces as a generic "unknown key" abort. The refusal's real sentence —
/// which fingerprints disagreed, and how to replace the line — is therefore
/// left in `refusal` for [`connect_session`] to report.
struct KnownHostsHandler {
    host: String,
    port: u16,
    refusal: Arc<Mutex<Option<String>>>,
    /// Test seam: the file to verify against instead of the user's
    /// `~/.ssh/known_hosts`. `None` everywhere outside tests.
    #[cfg(test)]
    known_hosts_path: Option<std::path::PathBuf>,
}

impl KnownHostsHandler {
    /// Anything that is not a clean match refuses the handshake and records
    /// why, so [`connect_session`] can report the actual sentence rather than
    /// russh's generic abort.
    fn accept(&mut self, key: &russh::keys::PublicKey, certificate: bool) -> bool {
        // Synchronous I/O on a runtime worker, deliberately: this is one read
        // (and at most one append) of a file measured in kilobytes, once per
        // connection, and the handshake has nothing else to do until it
        // answers. A `spawn_blocking` hop would only add a task for it.
        #[cfg(test)]
        let verdict = match &self.known_hosts_path {
            Some(path) => super::known_hosts::verify_host_key_at(path, &self.host, self.port, key),
            None => verify_host_key(&self.host, self.port, key),
        };
        #[cfg(not(test))]
        let verdict = verify_host_key(&self.host, self.port, key);

        match verdict {
            Ok(HostKeyVerdict::Known) => {
                if certificate {
                    log::info!(
                        "{}:{} presented a host certificate; its key matches the recorded one (the signing CA was not consulted)",
                        self.host,
                        self.port
                    );
                }
                true
            }
            Ok(HostKeyVerdict::Learned) => {
                log::info!(
                    "Trusted the host key of {}:{} on first contact ({}{})",
                    self.host,
                    self.port,
                    key.fingerprint(russh::keys::HashAlg::Sha256),
                    if certificate {
                        ", presented as a certificate; the signing CA was not consulted"
                    } else {
                        ""
                    }
                );
                true
            }
            Err(reason) => {
                if let Ok(mut slot) = self.refusal.lock() {
                    *slot = Some(reason);
                }
                false
            }
        }
    }
}

impl russh::client::Handler for KnownHostsHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // A host *certificate* is pinned by its underlying key: the CA that
        // signed it is not consulted (we have no `@cert-authority` support),
        // so a renewed certificate for the same key keeps working while a
        // different key is refused like any other change.
        let certificate = server_public_key.certificate().is_some();
        Ok(self.accept(&server_public_key.public_key(), certificate))
    }
}

/// The pieces of an `sftp://host[:port]/abs/path` URL after the scheme.
/// There is no share segment (unlike SMB): an empty `sub_path` is the
/// server root `/`, which is the user's home on chrooted servers.
struct SftpPath {
    host: String,
    port: Option<u16>,
    /// Path beneath the server root, "" at the root, "/"-separated.
    sub_path: String,
}

impl SftpPath {
    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }

    /// The canonical URL, normalizing away trailing separators.
    fn url(&self) -> String {
        if self.sub_path.is_empty() {
            format!("sftp://{}", self.authority())
        } else {
            format!("sftp://{}/{}", self.authority(), self.sub_path)
        }
    }

    /// The absolute path on the remote server.
    fn remote(&self) -> String {
        if self.sub_path.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", self.sub_path)
        }
    }

    fn join(&self, name: &str) -> String {
        if self.sub_path.is_empty() {
            format!("sftp://{}/{}", self.authority(), name)
        } else {
            format!("sftp://{}/{}/{}", self.authority(), self.sub_path, name)
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
            format!("sftp://{}", self.authority())
        } else {
            format!("sftp://{}/{}", self.authority(), parent_sub)
        };
        Ok((parent, name.to_owned()))
    }
}

fn parse_sftp_path(rest: &str) -> Result<SftpPath, FileSystemError> {
    let (authority, remainder) = split_authority(rest);
    let (host, port) = parse_authority(authority)?;

    let sub_path = remainder
        .unwrap_or("")
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/");

    Ok(SftpPath {
        host,
        port,
        sub_path,
    })
}

/// Opens (or reuses) the session for `sftp://...` URL text and returns it as
/// a backend. Called by `vfs::resolve`.
pub fn open_backend(rest: &str) -> Result<SharedBackend, FileSystemError> {
    let path = parse_sftp_path(rest)?;

    if let Some(existing) = connections::session_for(Protocol::Sftp, &path.host, path.port) {
        return Ok(existing);
    }

    let backend: SharedBackend = Arc::new(SftpBackend::connect(&path.host, path.port)?);
    connections::store_session(Protocol::Sftp, &path.host, path.port, backend.clone());
    Ok(backend)
}

impl SftpBackend {
    /// Connects using credentials from the connection store; SFTP has no
    /// anonymous mode, so the username falls back to the local account name.
    pub fn connect(host: &str, port: Option<u16>) -> Result<Self, FileSystemError> {
        let (username, password) = connections::resolve_credentials(Protocol::Sftp, host, port);
        Self::connect_with(host, port, username.as_deref(), password.as_deref())
    }

    pub fn connect_with(
        host: &str,
        port: Option<u16>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, FileSystemError> {
        let port = port.unwrap_or(DEFAULT_PORT);
        let username = username
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(local_user_name)
            .ok_or_else(|| {
                FileSystemError::InvalidInput(
                    "An SFTP account is required. Add one in the connect dialog and try again."
                        .into(),
                )
            })?;

        let runtime = Arc::new(
            tokio::runtime::Runtime::new()
                .map_err(|error| FileSystemError::Io(error.to_string()))?,
        );

        let sftp = runtime.block_on(connect_session(
            host,
            port,
            &username,
            password.unwrap_or(""),
        ))?;
        sftp.set_timeout(SESSION_TIMEOUT_SECS);

        Ok(Self { runtime, sftp })
    }

    fn read_dir_entries(&self, parsed: &SftpPath) -> Result<Vec<DirectoryEntry>, FileSystemError> {
        let read_dir = self
            .runtime
            .block_on(self.sftp.read_dir(parsed.remote()))
            .map_err(map_sftp_error)?;

        let entries = read_dir
            .filter(|entry| {
                let name = entry.file_name();
                name != "." && name != ".."
            })
            .map(|entry| {
                let attrs = entry.metadata();
                let kind = kind_of(&attrs);
                let name = entry.file_name();
                DirectoryEntry {
                    name: name.clone(),
                    path: parsed.join(&name),
                    kind,
                    modified_at: modified_millis(&attrs),
                    size: (kind == EntryKind::File).then(|| attrs.size.unwrap_or(0)),
                    hidden: name.starts_with('.'),
                    read_only: attrs.permissions.is_some_and(|mode| mode & 0o200 == 0),
                }
            })
            .collect::<Vec<_>>();
        Ok(entries)
    }
}

async fn connect_session(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<SftpSession, FileSystemError> {
    let config = Arc::new(russh::client::Config::default());
    let refusal = Arc::new(Mutex::new(None));
    let handler = KnownHostsHandler {
        host: host.to_owned(),
        port,
        refusal: refusal.clone(),
        #[cfg(test)]
        known_hosts_path: None,
    };
    let mut session = russh::client::connect(config, (host, port), handler)
        .await
        .map_err(|error| {
            // The host-key refusal is the one connect failure whose reason is
            // ours; everything else is genuinely "could not reach".
            let reason = refusal.lock().ok().and_then(|mut slot| slot.take());
            match reason {
                Some(reason) => FileSystemError::PermissionDenied(reason),
                None => FileSystemError::Io(format!("Could not reach the SFTP server: {error}")),
            }
        })?;

    let authenticated = session
        .authenticate_password(username, password)
        .await
        .map_err(|error| FileSystemError::Io(format!("SFTP authentication failed: {error}")))?;
    if !authenticated.success() {
        return Err(FileSystemError::PermissionDenied(
            "The server rejected the account or password. Check the credentials in the connect dialog.".into(),
        ));
    }

    let channel = session
        .channel_open_session()
        .await
        .map_err(|error| FileSystemError::Io(format!("Could not open an SFTP channel: {error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| {
            FileSystemError::Io(format!("The server has no SFTP subsystem: {error}"))
        })?;

    SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| FileSystemError::Io(format!("Could not start the SFTP session: {error}")))
}

/// Verifies a connection attempt before it is saved, using the credentials
/// typed into the connect dialog rather than any stored ones.
pub fn test_backend(
    host: &str,
    port: Option<u16>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), FileSystemError> {
    let backend = SftpBackend::connect_with(host, port, username, password)?;
    backend
        .runtime
        .block_on(backend.sftp.canonicalize("/"))
        .map(|_| ())
        .map_err(map_sftp_error)
}

impl FileSystemBackend for SftpBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let parsed = parse_dir(path)?;
        let mut entries = self.read_dir_entries(&parsed)?;
        entries.sort_by_cached_key(entry_sort_key);

        Ok(DirectoryView {
            path: parsed.url(),
            breadcrumbs: sftp_breadcrumbs(&parsed),
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
        let target = parse_dir(&parsed.join(name))?;

        match kind {
            NewEntryKind::File => {
                // `create` truncates, so the absence check must come first
                // to preserve the never-overwrite contract.
                let exists = self
                    .runtime
                    .block_on(self.sftp.try_exists(target.remote()))
                    .map_err(map_sftp_error)?;
                if exists {
                    return Err(FileSystemError::AlreadyExists(parsed.join(name)));
                }

                let file = self
                    .runtime
                    .block_on(self.sftp.create(target.remote()))
                    .map_err(map_sftp_error)?;
                self.runtime
                    .block_on(file.close())
                    .map_err(|error| FileSystemError::Io(error.to_string()))?;
            }
            NewEntryKind::Directory => {
                self.runtime
                    .block_on(self.sftp.create_dir(target.remote()))
                    .map_err(map_sftp_error)?;
            }
        }

        Ok(parsed.join(name))
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let parsed = parse_dir(path)?;
        let (parent, _old_name) = parsed.parent_and_name()?;
        let destination = parse_dir(&parse_dir(&parent)?.join(new_name))?;

        self.runtime
            .block_on(self.sftp.rename(parsed.remote(), destination.remote()))
            .map_err(map_sftp_error)
    }

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(SearchResponse {
                entries: Vec::new(),
                truncated: false,
            });
        }

        let parsed = parse_dir(root)?;
        let mut walker = SearchWalker {
            backend: self,
            query,
            entries: Vec::new(),
            truncated: false,
        };
        walker.walk(&parsed, "", is_current)?;

        Ok(SearchResponse {
            entries: walker.entries,
            truncated: walker.truncated,
        })
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let parsed = parse_dir(path)?;
        let attrs = self
            .runtime
            .block_on(self.sftp.metadata(parsed.remote()))
            .map_err(map_sftp_error)?;

        Ok(EntryStat {
            kind: kind_of(&attrs),
            size: attrs.size.unwrap_or(0),
            modified_at: modified_millis(&attrs),
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Ok(());
        }

        self.runtime
            .block_on(self.sftp.create_dir(parsed.remote()))
            .map_err(map_sftp_error)
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }

        let file = self
            .runtime
            .block_on(self.sftp.open(parsed.remote()))
            .map_err(map_sftp_error)?;

        Ok(Box::new(SftpReadAdapter {
            runtime: Arc::clone(&self.runtime),
            file,
            buffer: Vec::new(),
            position: 0,
            eof: false,
        }))
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Not a file path: {path}"
            )));
        }

        let file = self
            .runtime
            .block_on(self.sftp.create(parsed.remote()))
            .map_err(map_sftp_error)?;

        Ok(Box::new(SftpWriteAdapter {
            runtime: Arc::clone(&self.runtime),
            file: Some(file),
        }))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Refusing to delete the server root: {path}"
            )));
        }

        // Symlinks are removed as files even when they point at directories.
        let attrs = self
            .runtime
            .block_on(self.sftp.symlink_metadata(parsed.remote()))
            .map_err(map_sftp_error)?;

        self.runtime
            .block_on(async {
                if attrs.is_dir() && !attrs.is_symlink() {
                    self.sftp.remove_dir(parsed.remote()).await
                } else {
                    self.sftp.remove_file(parsed.remote()).await
                }
            })
            .map_err(map_sftp_error)
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from = parse_dir(source)?;
        let to = parse_dir(destination)?;

        // SFTP cannot rename across servers; the transfer engine falls back
        // to copy + delete on error.
        if from.authority() != to.authority() {
            return Err(FileSystemError::InvalidInput(
                "Cannot rename across SFTP servers".into(),
            ));
        }

        self.runtime
            .block_on(self.sftp.rename(from.remote(), to.remote()))
            .map_err(map_sftp_error)
    }

    fn properties(&self, path: &str) -> Result<FileProperties, FileSystemError> {
        let parsed = parse_dir(path)?;
        let attrs = self
            .runtime
            .block_on(self.sftp.symlink_metadata(parsed.remote()))
            .map_err(map_sftp_error)?;

        let kind = kind_of(&attrs);
        let target = if kind == EntryKind::Symlink {
            self.runtime
                .block_on(self.sftp.read_link(parsed.remote()))
                .ok()
        } else {
            None
        };

        // SFTP reports POSIX attributes; servers that omit them (rare) fall
        // back to the view-only basic section.
        let platform = match attrs.permissions {
            Some(mode) => PlatformProperties::Unix(UnixProperties {
                mode,
                uid: attrs.uid.unwrap_or(0),
                gid: attrs.gid.unwrap_or(0),
                user_name: attrs.user.clone(),
                group_name: attrs.group.clone(),
            }),
            None => PlatformProperties::Basic,
        };

        let name = parsed
            .sub_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| parsed.host.clone());

        Ok(FileProperties {
            path: parsed.url(),
            name,
            kind,
            size: attrs.size,
            created_at: None, // SFTP v3 has no birth time
            modified_at: modified_millis(&attrs),
            accessed_at: accessed_millis(&attrs),
            target,
            platform,
        })
    }

    fn update_properties(
        &self,
        path: &str,
        changes: &PropertyChanges,
    ) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        let remote = parsed.remote();
        let mut attrs = FileAttributes::empty();
        let mut has_change = false;

        if let Some(mode) = changes.mode {
            attrs.permissions = Some(mode & 0o7777);
            has_change = true;
        }

        if let Some(owner) = &changes.owner {
            // The SFTP protocol only accepts numeric uid/gid.
            let uid = match &owner.user {
                Some(user) => Some(parse_account_id(user)?),
                None => None,
            };
            let gid = match &owner.group {
                Some(group) => Some(parse_account_id(group)?),
                None => None,
            };
            if uid.is_some() || gid.is_some() {
                attrs.uid = uid;
                attrs.gid = gid;
                has_change = true;
            }
        }

        if has_change {
            self.runtime
                .block_on(self.sftp.set_metadata(remote, attrs))
                .map_err(map_sftp_error)?;
        }

        Ok(())
    }
}

fn parse_dir(path: &str) -> Result<SftpPath, FileSystemError> {
    let (scheme, rest) = super::vfs::split_scheme(path)?;
    if scheme != super::vfs::Scheme::Sftp {
        return Err(FileSystemError::InvalidInput(format!(
            "Not an SFTP path: {path}"
        )));
    }
    parse_sftp_path(rest)
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(FileSystemError::InvalidInput(format!(
            "Invalid entry name: {name}"
        )));
    }
    Ok(())
}

fn kind_of(attrs: &FileAttributes) -> EntryKind {
    if attrs.is_symlink() {
        EntryKind::Symlink
    } else if attrs.is_dir() {
        EntryKind::Directory
    } else if attrs.is_regular() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn modified_millis(attrs: &FileAttributes) -> Option<u64> {
    attrs
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn accessed_millis(attrs: &FileAttributes) -> Option<u64> {
    attrs
        .accessed()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

/// SFTP setstat accepts numeric ids only; account names cannot be resolved
/// through the protocol, unlike the local backend.
fn parse_account_id(value: &str) -> Result<u32, FileSystemError> {
    value.trim().parse::<u32>().map_err(|_| {
        FileSystemError::InvalidInput(format!(
            "SFTP ownership changes need a numeric id, got: {value}"
        ))
    })
}

fn local_user_name() -> Option<String> {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

fn map_sftp_error(error: SftpError) -> FileSystemError {
    match &error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => FileSystemError::NotFound(error.to_string()),
            StatusCode::PermissionDenied => FileSystemError::PermissionDenied(error.to_string()),
            // OpenSSH reports "File exists" as a generic failure.
            StatusCode::Failure if status.error_message.contains("exists") => {
                FileSystemError::AlreadyExists(error.to_string())
            }
            _ => FileSystemError::Io(error.to_string()),
        },
        SftpError::Timeout => {
            FileSystemError::Io(format!("The SFTP server did not respond in time: {error}"))
        }
        _ => FileSystemError::Io(error.to_string()),
    }
}

fn sftp_breadcrumbs(path: &SftpPath) -> Vec<Breadcrumb> {
    let mut breadcrumbs = vec![Breadcrumb {
        name: path.host.clone(),
        path: format!("sftp://{}", path.authority()),
    }];

    let mut accumulated = format!("sftp://{}", path.authority());
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

struct SearchWalker<'a> {
    backend: &'a SftpBackend,
    query: String,
    entries: Vec<SearchEntry>,
    truncated: bool,
}

impl SearchWalker<'_> {
    fn walk(
        &mut self,
        directory: &SftpPath,
        relative_prefix: &str,
        is_current: &dyn Fn() -> bool,
    ) -> Result<(), FileSystemError> {
        if self.truncated || !is_current() {
            return Ok(());
        }

        let children = self.backend.read_dir_entries(directory)?;

        for child in children {
            if self.entries.len() >= MAX_SEARCH_RESULTS {
                self.truncated = true;
                return Ok(());
            }
            if !is_current() {
                return Ok(());
            }

            if child.name.to_lowercase().contains(&self.query) {
                let relative = if relative_prefix.is_empty() {
                    child.name.clone()
                } else {
                    format!("{relative_prefix}/{}", child.name)
                };
                self.entries.push(SearchEntry {
                    name: child.name.clone(),
                    path: child.path.clone(),
                    relative_path: relative,
                    kind: child.kind,
                    modified_at: child.modified_at,
                    size: child.size,
                    hidden: child.hidden,
                    read_only: child.read_only,
                });
            }

            if child.kind == EntryKind::Directory {
                let child_path = parse_dir(&child.path)?;
                let child_relative = if relative_prefix.is_empty() {
                    child.name.clone()
                } else {
                    format!("{relative_prefix}/{}", child.name)
                };
                self.walk(&child_path, &child_relative, is_current)?;
            }
        }

        Ok(())
    }
}

/// Adapts the async sequential `File` reader to a byte-stream `Read`,
/// buffering one chunk per round trip.
struct SftpReadAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    file: SftpFile,
    buffer: Vec<u8>,
    position: usize,
    eof: bool,
}

impl Read for SftpReadAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position >= self.buffer.len() && !self.eof {
            let mut chunk = vec![0u8; READ_CHUNK];
            let read = self
                .runtime
                .block_on(self.file.read(&mut chunk))
                .map_err(|error| io::Error::other(error.to_string()))?;
            if read == 0 {
                self.eof = true;
                break;
            }
            chunk.truncate(read);
            self.buffer = chunk;
            self.position = 0;
        }

        let available = &self.buffer[self.position..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count;
        Ok(count)
    }
}

/// Adapts the async `File` writer to a byte-stream `Write`. The handle is
/// closed on drop so pending writes are confirmed by the server.
struct SftpWriteAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    file: Option<SftpFile>,
}

impl Write for SftpWriteAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self.file.as_mut().expect("sftp file already closed");
        self.runtime
            .block_on(file.write_all(buf))
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let file = self.file.as_mut().expect("sftp file already closed");
        self.runtime
            .block_on(file.flush())
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

impl Drop for SftpWriteAdapter {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Dropping without close discards pending write errors.
            let _ = self.runtime.block_on(file.close());
        }
    }
}

#[cfg(test)]
mod sftp_tests {
    use super::*;

    /// A known-hosts file in a scratch directory, so the handler tests never
    /// touch the real `~/.ssh/known_hosts`.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("dae-sftp-handler-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create the scratch directory");
            Self(path.join("known_hosts"))
        }

        fn handler(
            &self,
            host: &str,
            port: u16,
        ) -> (KnownHostsHandler, Arc<Mutex<Option<String>>>) {
            let refusal = Arc::new(Mutex::new(None));
            (
                KnownHostsHandler {
                    host: host.to_owned(),
                    port,
                    refusal: refusal.clone(),
                    known_hosts_path: Some(self.0.clone()),
                },
                refusal,
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA6rWI3G2sz07DnfFlrouTcysQlj2P+jpNSOEWD9OJ3X";

    fn key(openssh: &str) -> russh::keys::PublicKey {
        russh::keys::PublicKey::from_openssh(openssh).expect("parse the fixture key")
    }

    #[test]
    fn the_handler_accepts_first_contact_then_requires_the_same_key() {
        let scratch = Scratch::new("known");
        let (mut handler, refusal) = scratch.handler("nas.local", 2222);

        assert!(
            handler.accept(&key(KEY_A), false),
            "an unknown host is trusted on first contact"
        );
        assert!(refusal.lock().unwrap().is_none());

        assert!(
            handler.accept(&key(KEY_A), false),
            "the same key stays accepted"
        );

        assert!(
            !handler.accept(&key(KEY_B), false),
            "a changed key must refuse the handshake"
        );
        let reason = refusal
            .lock()
            .unwrap()
            .clone()
            .expect("the reason is recorded");
        assert!(
            reason.contains("ssh-keygen -R \"[nas.local]:2222\""),
            "the refusal tells the user how to repair it: {reason}"
        );
    }

    #[test]
    fn a_certificate_is_pinned_by_its_key() {
        let scratch = Scratch::new("cert");
        let (mut handler, _) = scratch.handler("nas.local", 22);

        // The same key presented inside a certificate is the same pin: the
        // flag only changes what is logged, never whether the key is trusted.
        assert!(handler.accept(&key(KEY_A), false));
        assert!(handler.accept(&key(KEY_A), true));
        assert!(!handler.accept(&key(KEY_B), true));
    }

    #[test]
    fn parses_sftp_urls() {
        let path = parse_sftp_path("nas.local:2222/home/user/docs").unwrap();
        assert_eq!(path.host, "nas.local");
        assert_eq!(path.port, Some(2222));
        assert_eq!(path.sub_path, "home/user/docs");
        assert_eq!(path.url(), "sftp://nas.local:2222/home/user/docs");
        assert_eq!(path.remote(), "/home/user/docs");

        let root = parse_sftp_path("nas.local").unwrap();
        assert_eq!(root.sub_path, "");
        assert_eq!(root.url(), "sftp://nas.local");
        assert_eq!(root.remote(), "/");

        let trailing = parse_sftp_path("nas.local/home/user/").unwrap();
        assert_eq!(trailing.sub_path, "home/user");

        let dotted = parse_sftp_path("nas.local/home/../user/./docs").unwrap();
        assert_eq!(dotted.sub_path, "home/user/docs");

        let ipv6 = parse_sftp_path("[::1]/home").unwrap();
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.sub_path, "home");
    }

    #[test]
    fn joins_child_paths_and_resolves_parents() {
        let path = parse_sftp_path("nas.local/home/user").unwrap();
        assert_eq!(path.join("new.txt"), "sftp://nas.local/home/user/new.txt");

        let (parent, name) = path.parent_and_name().unwrap();
        assert_eq!(parent, "sftp://nas.local/home");
        assert_eq!(name, "user");

        let child = parse_sftp_path("nas.local/home").unwrap();
        let (root_parent, child_name) = child.parent_and_name().unwrap();
        assert_eq!(root_parent, "sftp://nas.local");
        assert_eq!(child_name, "home");

        let root = parse_sftp_path("nas.local").unwrap();
        assert!(matches!(
            root.parent_and_name(),
            Err(FileSystemError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejects_malformed_hosts_and_ports() {
        assert!(parse_sftp_path(":22/home").is_err());
        let bad_port = parse_sftp_path("nas.local:99999/home");
        assert!(matches!(bad_port, Err(FileSystemError::InvalidInput(_))));
    }

    #[test]
    fn builds_breadcrumbs() {
        let path = parse_sftp_path("nas.local:2222/home/user/docs").unwrap();
        let breadcrumbs = sftp_breadcrumbs(&path);

        let names: Vec<_> = breadcrumbs
            .iter()
            .map(|crumb| crumb.name.as_str())
            .collect();
        assert_eq!(names, vec!["nas.local", "home", "user", "docs"]);
        assert_eq!(breadcrumbs[0].path, "sftp://nas.local:2222");
        assert_eq!(breadcrumbs[3].path, "sftp://nas.local:2222/home/user/docs");

        let root = parse_sftp_path("nas.local").unwrap();
        assert_eq!(sftp_breadcrumbs(&root).len(), 1);
    }

    #[test]
    fn parses_numeric_owner_ids_only() {
        assert_eq!(parse_account_id("1000").unwrap(), 1000);
        assert_eq!(parse_account_id(" 1001 ").unwrap(), 1001);
        assert!(matches!(
            parse_account_id("alice"),
            Err(FileSystemError::InvalidInput(_))
        ));
    }

    #[test]
    fn maps_status_codes() {
        let not_found = SftpError::Status(russh_sftp::protocol::Status {
            id: 1,
            status_code: StatusCode::NoSuchFile,
            error_message: "no such file".into(),
            language_tag: String::new(),
        });
        assert!(matches!(
            map_sftp_error(not_found),
            FileSystemError::NotFound(_)
        ));

        let exists = SftpError::Status(russh_sftp::protocol::Status {
            id: 2,
            status_code: StatusCode::Failure,
            error_message: "File exists".into(),
            language_tag: String::new(),
        });
        assert!(matches!(
            map_sftp_error(exists),
            FileSystemError::AlreadyExists(_)
        ));

        assert!(matches!(
            map_sftp_error(SftpError::Timeout),
            FileSystemError::Io(_)
        ));
    }
}
