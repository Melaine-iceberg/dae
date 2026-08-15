use super::error::FileSystemError;
use super::local::LocalBackend;
use super::progress::FileOperationProgressReporterTrait;
use super::types::{DirectoryView, NewEntryKind, SearchResponse};

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
// A single static backend serves everything today; network backends will be
// served from a per-connection registry once the first protocol lands.
pub fn resolve(path: &str) -> Result<&'static dyn FileSystemBackend, FileSystemError> {
    static LOCAL: LocalBackend = LocalBackend;

    match split_scheme(path)?.0 {
        Scheme::Local => Ok(&LOCAL),
        Scheme::Smb => Err(unsupported_scheme("SMB")),
        Scheme::Sftp => Err(unsupported_scheme("SFTP")),
        Scheme::Ftp => Err(unsupported_scheme("FTP")),
        Scheme::WebDav => Err(unsupported_scheme("WebDAV")),
    }
}

fn unsupported_scheme(name: &str) -> FileSystemError {
    FileSystemError::InvalidInput(format!("{name} storage is not supported in this build"))
}

/// Blocks `operation` whose sources and destination live on different backends.
// Extension point: a generic list/stream-based transfer engine built on backend
// primitives replaces this guard when the second protocol backend lands.
pub fn ensure_same_backend(
    sources: &[String],
    destination: &str,
    operation: &str,
) -> Result<(), FileSystemError> {
    let destination_scheme = split_scheme(destination)?.0;

    for source in sources {
        if split_scheme(source)?.0 != destination_scheme {
            return Err(FileSystemError::InvalidInput(format!(
                "{operation} between different storage backends is not supported yet"
            )));
        }
    }

    Ok(())
}

/// One storage protocol implementation behind the explorer's path strings.
///
/// All methods are blocking; commands run them on the async runtime's blocking
/// threads. Transfer methods only ever see sources and a destination on this
/// same backend (see [`ensure_same_backend`]).
pub trait FileSystemBackend: Send + Sync {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError>;

    fn create_entry(
        &self,
        directory: &str,
        name: &str,
        kind: NewEntryKind,
    ) -> Result<String, FileSystemError>;

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError>;

    fn copy(
        &self,
        sources: &[String],
        destination: &str,
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError>;

    fn move_entries(
        &self,
        sources: &[String],
        destination: &str,
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError>;

    fn delete(
        &self,
        paths: &[String],
        progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<(), FileSystemError>;

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError>;
}
