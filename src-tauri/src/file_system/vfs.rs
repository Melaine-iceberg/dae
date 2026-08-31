use super::error::FileSystemError;
use super::local::LocalBackend;
use super::sftp;
use super::smb;
use super::types::{
    DirectoryView, EntryStat, FileProperties, NewEntryKind, PropertyChanges, SearchResponse,
};
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
    Cloud,
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
        "cloud" => Scheme::Cloud,
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
        Scheme::Sftp => {
            let (_, rest) = split_scheme(path)?;
            sftp::open_backend(rest)
        }
        Scheme::Ftp => Err(unsupported_scheme("FTP")),
        Scheme::WebDav => Err(unsupported_scheme("WebDAV")),
        Scheme::Cloud => {
            let (_, rest) = split_scheme(path)?;
            super::cloud::open_backend(rest)
        }
    }
}

fn unsupported_scheme(name: &str) -> FileSystemError {
    FileSystemError::InvalidInput(format!("{name} storage is not supported in this build"))
}

/// The scheme serving `path`, without the remainder.
pub fn scheme_of(path: &str) -> Result<Scheme, FileSystemError> {
    Ok(split_scheme(path)?.0)
}

/// True when `path` addresses the local disk rather than a remote scheme.
pub fn is_local_path(path: &str) -> bool {
    scheme_of(path).is_ok_and(|scheme| scheme == Scheme::Local)
}

/// One storage protocol implementation behind the explorer's path strings.
///
/// All methods are blocking; commands run them on the async runtime's blocking
/// threads. Backends only implement single-server operations; tree-shaped
/// transfers (copy/move/delete across directories and backends) are composed
/// by the generic engine in `transfer.rs` from the primitive methods below.
pub trait FileSystemBackend: Send + Sync {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError>;

    /// Display name of the entry at `path`. Defaults to the final path
    /// segment; backends whose segments are opaque ids (cloud storage)
    /// resolve the real name instead, so cross-backend transfers name
    /// destination entries correctly.
    fn entry_name(&self, path: &str) -> Result<String, FileSystemError> {
        super::transfer::last_segment(path)
            .map(str::to_owned)
            .ok_or_else(|| {
                FileSystemError::InvalidInput(format!(
                    "The root of a volume has no entry name: {path}"
                ))
            })
    }

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

    // -- Properties (permissions / ownership / attributes) --

    /// Full metadata for the properties dialog. Backends without a
    /// permission model inherit the default, which degrades to a view-only
    /// [`FileProperties`] built from [`FileSystemBackend::stat`].
    fn properties(&self, path: &str) -> Result<FileProperties, FileSystemError> {
        let stat = self.stat(path)?;
        Ok(FileProperties::basic(path, stat))
    }

    /// Applies permission/ownership/attribute edits. Fields left as `None`
    /// keep their current value. The default rejects every change for
    /// backends without a permission model.
    fn update_properties(
        &self,
        path: &str,
        _changes: &PropertyChanges,
    ) -> Result<(), FileSystemError> {
        Err(FileSystemError::Unsupported(format!(
            "This storage backend does not support editing properties: {path}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scheme_prefixes() {
        let (scheme, rest) = split_scheme("smb://nas/media").expect("smb scheme");
        assert_eq!(scheme, Scheme::Smb);
        assert_eq!(rest, "nas/media");

        let (scheme, rest) = split_scheme("sftp://user@nas:22/home").expect("sftp scheme");
        assert_eq!(scheme, Scheme::Sftp);
        assert_eq!(rest, "user@nas:22/home");

        let (scheme, rest) = split_scheme("webdavs://cloud.example/dav").expect("webdav scheme");
        assert_eq!(scheme, Scheme::WebDav);
        assert_eq!(rest, "cloud.example/dav");

        let (scheme, rest) =
            split_scheme("cloud://google_drive:user@example.com/folder-id")
                .expect("cloud scheme");
        assert_eq!(scheme, Scheme::Cloud);
        assert_eq!(rest, "google_drive:user@example.com/folder-id");

        let (scheme, rest) = split_scheme("file:///home").expect("file maps to local");
        assert_eq!(scheme, Scheme::Local);
        assert_eq!(rest, "/home");
    }

    #[test]
    fn treats_scheme_less_paths_as_local() {
        for path in [
            r"C:\Users\test",
            "/home/user",
            r"\\nas\share\folder",
            r"relative\nested://segment",
        ] {
            let (scheme, rest) = split_scheme(path).expect("scheme-less path");
            assert_eq!(scheme, Scheme::Local, "path should stay local: {path}");
            assert_eq!(rest, path);
        }
    }

    #[test]
    fn rejects_unknown_and_unregistered_schemes() {
        let unknown = resolve("s3://bucket/data")
            .err()
            .expect("unknown scheme");
        assert!(matches!(unknown, FileSystemError::InvalidInput(_)));

        // SMB and SFTP resolve to connect attempts these days; ftp is a scheme
        // that still has no backend.
        let unregistered = resolve("ftp://nas/media")
            .err()
            .expect("no ftp backend yet");
        assert!(matches!(unregistered, FileSystemError::InvalidInput(_)));
    }

    #[test]
    fn serves_local_paths_through_the_backend_trait() {
        use std::fs;

        let directory = std::env::temp_dir().join(format!("dae-vfs-trait-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");

        let path = directory.to_string_lossy().into_owned();
        let backend = resolve(&path).expect("local path resolves");

        let created = backend
            .create_entry(&path, "through-trait.txt", NewEntryKind::File)
            .expect("create through trait object");
        assert!(directory.join("through-trait.txt").is_file());

        let view = backend.read_dir(&path).expect("read through trait object");
        assert!(
            view.entries
                .iter()
                .any(|entry| entry.name == "through-trait.txt")
        );

        backend
            .remove(&created)
            .expect("delete through trait object");

        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
