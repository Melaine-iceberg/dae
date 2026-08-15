//! The SMB backend: one `SmbBackend` instance per server, reached through
//! `smb://host[:port]/share/path` URLs. All tree-shaped transfers are served
//! by the generic engine in `transfer.rs`; this module only implements
//! single-server operations.

use super::connections::{self, Protocol, SaveConnectionInput};
use super::error::FileSystemError;
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, NewEntryKind, SearchEntry,
    SearchResponse, entry_sort_key,
};
use super::vfs::{FileSystemBackend, SharedBackend};
use smb2::{ErrorKind as SmbErrorKind, FileReader, FileWriter, SmbClient, Tree};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

const DEFAULT_PORT: u16 = 445;
const READ_CHUNK: u64 = 256 * 1024;
const MAX_SEARCH_RESULTS: usize = 200;

pub struct SmbBackend {
    runtime: Arc<tokio::runtime::Runtime>,
    /// The smb2 crate takes `&mut self` for nearly every call, so the client
    /// and its share connections sit behind one lock. Lock guards never cross
    /// an `.await` that yields: each operation runs inside `block_on` on the
    /// calling thread.
    state: Mutex<SmbState>,
}

struct SmbState {
    client: SmbClient,
    trees: HashMap<String, Tree>,
}

/// The pieces of an `smb://host[:port]/share/path` URL after the scheme.
struct SmbPath {
    host: String,
    port: Option<u16>,
    /// Empty at the server root, where `read_dir` lists the available shares.
    share: String,
    /// Path beneath the share root, "" at the share root, "/"-separated.
    sub_path: String,
}

impl SmbPath {
    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }

    /// The canonical URL, normalizing away trailing separators.
    fn url(&self) -> String {
        if self.share.is_empty() {
            format!("smb://{}", self.authority())
        } else if self.sub_path.is_empty() {
            format!("smb://{}/{}", self.authority(), self.share)
        } else {
            format!("smb://{}/{}/{}", self.authority(), self.share, self.sub_path)
        }
    }

    fn join(&self, name: &str) -> String {
        let joined_sub = if self.sub_path.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{}", self.sub_path, name)
        };

        format!("smb://{}/{}/{}", self.authority(), self.share, joined_sub)
    }

    fn parent_and_name(&self) -> Result<(String, String), FileSystemError> {
        // Shares are server-side configuration; only entries inside a share
        // can be renamed.
        if self.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Shares cannot be renamed".into(),
            ));
        }

        let name = self
            .sub_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                FileSystemError::InvalidInput("The share root cannot be renamed".into())
            })?;
        let parent = match self.sub_path.rfind('/') {
            Some(index) => self.join_parent(&self.sub_path[..index]),
            None => self.join_parent(""),
        };
        Ok((parent, name.to_owned()))
    }

    fn join_parent(&self, parent_sub: &str) -> String {
        if parent_sub.is_empty() {
            format!("smb://{}/{}", self.authority(), self.share)
        } else {
            format!("smb://{}/{}/{}", self.authority(), self.share, parent_sub)
        }
    }
}

fn parse_smb_path(rest: &str) -> Result<SmbPath, FileSystemError> {
    let (authority, remainder) = split_authority(rest);
    let (host, port) = parse_authority(authority)?;

    let (share, sub_path) = match remainder {
        Some(remainder) => match remainder.split_once('/') {
            Some((share, sub)) => (share.to_owned(), sub.replace('\\', "/")),
            None => (remainder.to_owned(), String::new()),
        },
        None => (String::new(), String::new()),
    };

    Ok(SmbPath {
        host,
        port,
        share,
        sub_path: sub_path
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
            .collect::<Vec<_>>()
            .join("/"),
    })
}

fn split_authority(rest: &str) -> (&str, Option<&str>) {
    if let Some(stripped) = rest.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            let host = &rest[..end + 2];
            let remainder = &rest[end + 2..];
            return (host, remainder.strip_prefix('/'));
        }
    }

    match rest.split_once('/') {
        Some((authority, remainder)) => (authority, Some(remainder)),
        None => (rest, None),
    }
}

fn parse_authority(authority: &str) -> Result<(String, Option<u16>), FileSystemError> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| FileSystemError::InvalidInput("Unbalanced IPv6 address".into()))?;
        let host = &authority[..=end];
        let port = authority[end + 1..].strip_prefix(':');
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };

    if host.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "An SMB host is required".into(),
        ));
    }

    let port = match port {
        None => None,
        Some("") => None,
        Some(port) => Some(port.parse::<u16>().map_err(|_| {
            FileSystemError::InvalidInput(format!("Invalid SMB port: {port}"))
        })?),
    };

    Ok((host.to_owned(), port))
}

/// Opens (or reuses) the session for `smb://...` URL text and returns it as a
/// backend. Called by `vfs::resolve`.
pub fn open_backend(rest: &str) -> Result<SharedBackend, FileSystemError> {
    let path = parse_smb_path(rest)?;

    if let Some(existing) = connections::session_for(Protocol::Smb, &path.host, path.port) {
        return Ok(existing);
    }

    let backend: SharedBackend = Arc::new(SmbBackend::connect(&path.host, path.port)?);
    connections::store_session(Protocol::Smb, &path.host, path.port, backend.clone());
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
        let addr = format!("{}:{}", host, port.unwrap_or(DEFAULT_PORT));
        let runtime = Arc::new(
            tokio::runtime::Runtime::new().map_err(|error| FileSystemError::Io(error.to_string()))?,
        );

        let client = runtime
            .block_on(async { smb2::connect(&addr, username.unwrap_or(""), password.unwrap_or("")).await })
            .map_err(map_connect_error)?;

        Ok(Self {
            runtime,
            state: Mutex::new(SmbState {
                client,
                trees: HashMap::new(),
            }),
        })
    }

    fn share_names(&self) -> Result<Vec<String>, FileSystemError> {
        let mut state = self.state.lock().expect("smb session poisoned");
        let shares = self
            .runtime
            .block_on(state.client.list_shares())
            .map_err(map_smb_error)?;
        Ok(shares.into_iter().map(|share| share.name).collect())
    }
}

/// Runs `body` with the client and the share's tree connected. Locks the
/// session for the duration; connect-on-demand keeps idle sessions cheap.
macro_rules! smb_tree_op {
    ($self:expr, $share:expr, ($client:ident, $tree:ident) => $body:expr) => {{
        let mut state = $self.state.lock().expect("smb session poisoned");
        let SmbState { client, trees } = &mut *state;
        if !trees.contains_key($share) {
            let tree = $self
                .runtime
                .block_on(client.connect_share($share))
                .map_err(map_smb_error)?;
            trees.insert($share.to_owned(), tree);
        }
        let $tree = trees.get_mut($share).expect("tree just connected");
        let $client = &mut *client;
        $self
            .runtime
            .block_on(async { $body })
            .map_err(map_smb_error)
    }};
}

impl FileSystemBackend for SmbBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let (scheme, rest) = super::vfs::split_scheme(path)?;
        debug_assert_eq!(scheme, super::vfs::Scheme::Smb);
        let parsed = parse_smb_path(rest)?;

        let entries = if parsed.share.is_empty() {
            self.share_names()?
                .into_iter()
                .filter(|name| !name.ends_with('$'))
                .map(|name| DirectoryEntry {
                    path: format!("smb://{}/{}", parsed.authority(), name),
                    name,
                    kind: EntryKind::Directory,
                    modified_at: None,
                    size: None,
                })
                .collect::<Vec<_>>()
        } else {
            let listed = smb_tree_op!(self, &parsed.share, (_client, tree) => {
                tree_client_list(_client, tree, &parsed.sub_path).await
            })?;
            listed
                .into_iter()
                .filter(|entry| entry.name != "." && entry.name != "..")
                .map(|entry| DirectoryEntry {
                    path: parsed.join(&entry.name),
                    name: entry.name,
                    kind: if entry.is_directory {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    modified_at: modified_millis(entry.modified),
                    size: if entry.is_directory {
                        None
                    } else {
                        Some(entry.size)
                    },
                })
                .collect::<Vec<_>>()
        };

        let mut entries = entries;
        entries.sort_by_cached_key(|entry| entry_sort_key(entry));

        Ok(DirectoryView {
            path: parsed.url(),
            breadcrumbs: smb_breadcrumbs(&parsed),
            entries,
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

        if parsed.share.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Choose a share before creating entries".into(),
            ));
        }

        let target = if parsed.sub_path.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{}", parsed.sub_path, name)
        };

        match kind {
            NewEntryKind::File => {
                // create_file_writer truncates, so the absence check must
                // come first to preserve the never-overwrite contract.
                let existing = smb_tree_op!(self, &parsed.share, (client, tree) => {
                    client.stat(tree, &target).await
                });
                match existing {
                    Ok(_) => {
                        return Err(FileSystemError::AlreadyExists(parsed.join(name)));
                    }
                    Err(FileSystemError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }

                smb_tree_op!(self, &parsed.share, (client, tree) => {
                    let writer = client.create_file_writer(tree, &target).await?;
                    writer.finish().await.map(|_| ())
                })?;
            }
            NewEntryKind::Directory => {
                smb_tree_op!(self, &parsed.share, (client, tree) => {
                    client.create_directory(tree, &target).await
                })?;
            }
        }

        Ok(parsed.join(name))
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let parsed = parse_dir(path)?;
        let (parent, _old_name) = parsed.parent_and_name()?;
        let destination_sub = parse_dir(&parent)?.join(new_name);
        let to_sub = parse_dir(&destination_sub)?.sub_path;

        smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.rename(tree, &parsed.sub_path, &to_sub).await
        })
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

        if parsed.share.is_empty() {
            return Ok(EntryStat {
                kind: EntryKind::Directory,
                size: 0,
            });
        }

        let info = smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.stat(tree, &parsed.sub_path).await
        })?;

        Ok(EntryStat {
            kind: if info.is_directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: info.size,
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.share.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Choose a share before creating folders".into(),
            ));
        }
        if parsed.sub_path.is_empty() {
            return Ok(());
        }

        smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.create_directory(tree, &parsed.sub_path).await
        })
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.share.is_empty() || parsed.sub_path.is_empty() {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }

        let reader = smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.open_file_reader(tree, &parsed.sub_path).await
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
        let parsed = parse_dir(path)?;
        if parsed.share.is_empty() || parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Not a file path: {path}"
            )));
        }

        let writer = smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.create_file_writer(tree, &parsed.sub_path).await
        })?;

        Ok(Box::new(SmbWriteAdapter {
            runtime: Arc::clone(&self.runtime),
            writer,
        }))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = parse_dir(path)?;
        if parsed.share.is_empty() || parsed.sub_path.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "Refusing to delete a share root: {path}"
            )));
        }

        let info = smb_tree_op!(self, &parsed.share, (client, tree) => {
            client.stat(tree, &parsed.sub_path).await
        })?;

        if info.is_directory {
            smb_tree_op!(self, &parsed.share, (client, tree) => {
                client.delete_directory(tree, &parsed.sub_path).await
            })
        } else {
            smb_tree_op!(self, &parsed.share, (client, tree) => {
                client.delete_file(tree, &parsed.sub_path).await
            })
        }
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from = parse_dir(source)?;
        let to = parse_dir(destination)?;

        // The SMB protocol cannot rename across share boundaries; the
        // transfer engine falls back to copy + delete on error.
        if from.share != to.share {
            return Err(FileSystemError::InvalidInput(
                "Cannot rename across shares".into(),
            ));
        }

        smb_tree_op!(self, &from.share, (client, tree) => {
            client.rename(tree, &from.sub_path, &to.sub_path).await
        })
    }
}

fn parse_dir(path: &str) -> Result<SmbPath, FileSystemError> {
    let (scheme, rest) = super::vfs::split_scheme(path)?;
    if scheme != super::vfs::Scheme::Smb {
        return Err(FileSystemError::InvalidInput(format!(
            "Not an SMB path: {path}"
        )));
    }
    parse_smb_path(rest)
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

fn smb_breadcrumbs(path: &SmbPath) -> Vec<Breadcrumb> {
    let mut breadcrumbs = vec![Breadcrumb {
        name: path.host.clone(),
        path: format!("smb://{}", path.authority()),
    }];

    if path.share.is_empty() {
        return breadcrumbs;
    }

    breadcrumbs.push(Breadcrumb {
        name: path.share.clone(),
        path: format!("smb://{}/{}", path.authority(), path.share),
    });

    let mut accumulated = format!("smb://{}/{}", path.authority(), path.share);
    for segment in path.sub_path.split('/').filter(|segment| !segment.is_empty()) {
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
    backend: &'a SmbBackend,
    query: String,
    entries: Vec<SearchEntry>,
    truncated: bool,
}

impl SearchWalker<'_> {
    fn walk(
        &mut self,
        directory: &SmbPath,
        relative_prefix: &str,
        is_current: &dyn Fn() -> bool,
    ) -> Result<(), FileSystemError> {
        if self.truncated || !is_current() {
            return Ok(());
        }

        let children = if directory.share.is_empty() {
            self.backend
                .share_names()?
                .into_iter()
                .filter(|name| !name.ends_with('$'))
                .map(|name| DirectoryEntry {
                    path: format!("smb://{}/{}", directory.authority(), name),
                    name,
                    kind: EntryKind::Directory,
                    modified_at: None,
                    size: None,
                })
                .collect::<Vec<_>>()
        } else {
            let listed = smb_tree_op!(self.backend, &directory.share, (client, tree) => {
                client.list_directory(tree, &directory.sub_path).await
            })?;
            listed
                .into_iter()
                .filter(|entry| entry.name != "." && entry.name != "..")
                .map(|entry| DirectoryEntry {
                    path: directory.join(&entry.name),
                    name: entry.name,
                    kind: if entry.is_directory {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    modified_at: modified_millis(entry.modified),
                    size: if entry.is_directory {
                        None
                    } else {
                        Some(entry.size)
                    },
                })
                .collect::<Vec<_>>()
        };

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
        SmbErrorKind::AuthRequired | SmbErrorKind::SigningRequired => {
            FileSystemError::PermissionDenied(format!(
                "The server rejected the connection: {error}. \
                 Add an account and password in the connect dialog and try again."
            ))
        }
        SmbErrorKind::AccessDenied => FileSystemError::PermissionDenied(error.to_string()),
        SmbErrorKind::TimedOut | SmbErrorKind::ConnectionLost => FileSystemError::Io(format!(
            "Could not reach the SMB server: {error}"
        )),
        _ => FileSystemError::Io(error.to_string()),
    }
}

fn map_smb_error(error: smb2::Error) -> FileSystemError {
    match error.kind() {
        SmbErrorKind::NotFound => FileSystemError::NotFound(error.to_string()),
        SmbErrorKind::AlreadyExists => FileSystemError::AlreadyExists(error.to_string()),
        SmbErrorKind::AccessDenied => FileSystemError::PermissionDenied(error.to_string()),
        SmbErrorKind::AuthRequired | SmbErrorKind::SigningRequired => {
            FileSystemError::PermissionDenied(format!(
                "The server requires an account: {error}"
            ))
        }
        SmbErrorKind::NotADirectory => FileSystemError::NotDirectory(error.to_string()),
        SmbErrorKind::IsADirectory => FileSystemError::InvalidInput(error.to_string()),
        _ => FileSystemError::Io(error.to_string()),
    }
}

/// Verifies a connection attempt before it is saved, using the credentials
/// typed into the connect dialog rather than any stored ones.
#[tauri::command]
#[specta::specta]
pub async fn test_connection(
    input: SaveConnectionInput,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
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
            other => Err(FileSystemError::InvalidInput(format!(
                "The {} protocol is not supported in this build yet",
                other.as_str()
            ))),
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
        let path = parse_smb_path("nas.local:445/media/movies/2024").unwrap();
        assert_eq!(path.host, "nas.local");
        assert_eq!(path.port, Some(445));
        assert_eq!(path.share, "media");
        assert_eq!(path.sub_path, "movies/2024");
        assert_eq!(path.url(), "smb://nas.local:445/media/movies/2024");

        let root = parse_smb_path("nas.local").unwrap();
        assert_eq!(root.share, "");
        assert_eq!(root.url(), "smb://nas.local");

        let share_root = parse_smb_path("nas.local/media/").unwrap();
        assert_eq!(share_root.share, "media");
        assert_eq!(share_root.sub_path, "");
        assert_eq!(share_root.url(), "smb://nas.local/media");

        let dotted = parse_smb_path("nas.local/media/../public/./docs").unwrap();
        assert_eq!(dotted.sub_path, "public/docs");

        let ipv6 = parse_smb_path("[::1]/media").unwrap();
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.share, "media");
    }

    #[test]
    fn joins_child_paths_and_resolves_parents() {
        let path = parse_smb_path("nas.local/media/movies").unwrap();
        assert_eq!(path.join("new.txt"), "smb://nas.local/media/movies/new.txt");

        let (parent, name) = path.parent_and_name().unwrap();
        assert_eq!(parent, "smb://nas.local/media");
        assert_eq!(name, "movies");

        let share_root = parse_smb_path("nas.local/media").unwrap();
        assert!(matches!(
            share_root.parent_and_name(),
            Err(FileSystemError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejects_malformed_hosts_and_ports() {
        assert!(parse_smb_path(":445/media").is_err());
        let bad_port = parse_smb_path("nas.local:99999/media");
        assert!(matches!(
            bad_port,
            Err(FileSystemError::InvalidInput(_))
        ));
    }

    #[test]
    fn builds_share_aware_breadcrumbs() {
        let path = parse_smb_path("nas.local/media/movies/2024").unwrap();
        let breadcrumbs = smb_breadcrumbs(&path);

        let names: Vec<_> = breadcrumbs.iter().map(|crumb| crumb.name.as_str()).collect();
        assert_eq!(names, vec!["nas.local", "media", "movies", "2024"]);
        assert_eq!(breadcrumbs[0].path, "smb://nas.local");
        assert_eq!(breadcrumbs[1].path, "smb://nas.local/media");
        assert_eq!(breadcrumbs[3].path, "smb://nas.local/media/movies/2024");

        let root = parse_smb_path("nas.local").unwrap();
        assert_eq!(smb_breadcrumbs(&root).len(), 1);
    }
}
