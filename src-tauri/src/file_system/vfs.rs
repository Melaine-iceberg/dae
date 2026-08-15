use super::error::FileSystemError;
use super::local::LocalBackend;
use super::smb;
use super::types::{DirectoryView, EntryStat, NewEntryKind, SearchResponse};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::LazyLock;

/// A backend handle shared between the command layer, the transfer engine,
/// and per-connection session caches.
pub type SharedBackend = Arc<dyn FileSystemBackend>;

/// Storage protocols addressable through a scheme prefix on explorer paths.
///
/// Paths without a scheme (drive letters, POSIX paths, UNC shares) are local.
/// Network schemes parse today but fail at [`resolve`] until a backend is
/// registered for them, which keeps the path contract forward compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Local,
    Smb,
    Sftp,
    Ftp,
    WebDav,
}

/// Parses the `scheme://` prefix of an explorer path, returning the scheme and
/// the remainder. `file://` and scheme-less paths resolve to [`Scheme::Local`].
pub fn split_scheme(path: &str) -> Result<(Scheme, &str), FileSystemError> {
    let Some((scheme, rest)) = path.split_once("://") else {
        return Ok((Scheme::Local, path));
    };

    // A separator before "://" means this is a regular path that merely
    // contains a "://" segment, not a scheme prefix.
    if scheme.contains(['/', '\\']) || scheme.is_empty() {
        return Ok((Scheme::Local, path));
    }

    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "file" => Scheme::Local,
        "smb" => Scheme::Smb,
        "sftp" => Scheme::Sftp,
        "ftp" => Scheme::Ftp,
        "webdav" | "webdavs" => Scheme::WebDav,
        other => {
            return Err(FileSystemError::InvalidInput(format!(
                "Unsupported storage protocol: {other}"
            )));
        }
    };

    Ok((scheme, rest))
}

/// Returns the backend that serves `path`.
// The local backend is static; network schemes consult the connection
// registry once the first protocol lands.
pub fn resolve(path: &str) -> Result<SharedBackend, FileSystemError> {
    static LOCAL: LazyLock<SharedBackend> = LazyLock::new(|| Arc::new(LocalBackend));

    match split_scheme(path)?.0 {
        Scheme::Local => Ok(Arc::clone(&LOCAL)),
        Scheme::Smb => {
            let (_, rest) = split_scheme(path)?;
            smb::open_backend(rest)
        }
        Scheme::Sftp => Err(unsupported_scheme("SFTP")),
        Scheme::Ftp => Err(unsupported_scheme("FTP")),
        Scheme::WebDav => Err(unsupported_scheme("WebDAV")),
    }
}

fn unsupported_scheme(name: &str) -> FileSystemError {
    FileSystemError::InvalidInput(format!("{name} storage is not supported in this build"))
}

/// The scheme serving `path`, without the remainder.
pub fn scheme_of(path: &str) -> Result<Scheme, FileSystemError> {
    Ok(split_scheme(path)?.0)
}

/// One storage protocol implementation behind the explorer's path strings.
///
/// All methods are blocking; commands run them on the async runtime's blocking
/// threads. Backends only implement single-server operations; tree-shaped
/// transfers (copy/move/delete across directories and backends) are composed
/// by the generic engine in `transfer.rs` from the primitive methods below.
pub trait FileSystemBackend: Send + Sync {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError>;

    fn create_entry(
        &self,
        directory: &str,
        name: &str,
        kind: NewEntryKind,
    ) -> Result<String, FileSystemError>;

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError>;

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError>;

    // -- Primitive operations for the generic cross-backend transfer engine --

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError>;

    /// Creates a directory; parents must already exist.
    fn mkdir(&self, path: &str) -> Result<(), FileSystemError>;

    /// Opens a file for reading as a byte stream.
    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError>;

    /// Creates (or truncates) a file for sequential writing.
    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError>;

    /// Deletes a file or an empty directory.
    fn remove(&self, path: &str) -> Result<(), FileSystemError>;

    /// Moves an entry to another path on the same backend when the protocol
    /// supports it; the engine falls back to copy + delete on failure.
    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError>;
}
