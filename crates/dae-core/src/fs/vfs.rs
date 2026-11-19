use super::cloud;
use super::error::FileSystemError;
use super::ftp;
use super::local::LocalBackend;
use super::progress::FileOperationProgressReporterTrait;
use super::sftp;
use super::smb;
use super::types::{
    DirectoryView, EntryStat, FileProperties, NewEntryKind, PropertyChanges,
    RecursivePropertyUpdateOutcome, SearchResponse,
};
use super::url::{EntryUrl, Scheme};
use super::webdav;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::LazyLock;

/// A backend handle shared between the command layer, the transfer engine,
/// and per-connection session caches.
pub type SharedBackend = Arc<dyn FileSystemBackend>;

/// Returns the backend that serves `path`, parsed once into an
/// [`EntryUrl`]: the local backend is static, network schemes open (or
/// reuse) a session through their backend module.
pub fn resolve(path: &str) -> Result<SharedBackend, FileSystemError> {
    static LOCAL: LazyLock<SharedBackend> = LazyLock::new(|| Arc::new(LocalBackend));

    let url = EntryUrl::parse(path)?;
    match url.scheme() {
        Scheme::Local => Ok(Arc::clone(&LOCAL)),
        Scheme::Smb => smb::open_backend(&url),
        Scheme::Sftp => sftp::open_backend(&url),
        // Both spellings reach the FTP backend; `ftps` is the one that
        // negotiates TLS, decided by the scheme the URL spelled.
        Scheme::Ftp | Scheme::Ftps => ftp::open_backend(&url),
        Scheme::WebDav | Scheme::WebDavS => webdav::open_backend(&url),
        Scheme::Cloud => cloud::open_backend(&url),
        // A read-only view inside one archive file on the local disk.
        Scheme::Archive => super::archive_view::open_backend(&url),
    }
}

/// The scheme serving `path`, without parsing anything else.
pub fn scheme_of(path: &str) -> Result<Scheme, FileSystemError> {
    EntryUrl::parse(path).map(|url| url.scheme())
}

/// True when `path` addresses the local disk rather than a remote scheme.
/// A path that does not parse is not local — an unknown scheme is the
/// usual cause, and the caller will hear the error at `resolve`.
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

    /// Recursive name search is a local-disk feature. A network backend has
    /// no index to answer it, and walking the tree would issue unbounded
    /// round trips, so every non-local backend inherits this rejection.
    fn search(
        &self,
        root: &str,
        query: &str,
        filters: &super::search::SearchFilters,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        let _ = (root, query, filters, is_current);
        Err(FileSystemError::InvalidInput("fs.search_local_only".into()))
    }

    // -- Primitive operations for the generic cross-backend transfer engine --

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError>;

    /// Creates a directory; parents must already exist.
    fn mkdir(&self, path: &str) -> Result<(), FileSystemError>;

    /// Opens a file for reading as a byte stream.
    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError>;
    /// Creates (or truncates) a file for sequential writing.
    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError>;

    /// Opens (or creates) `path` with the writer positioned at `offset`,
    /// keeping what is already there. The byte-level resume writes the
    /// remainder of a partial destination through this. A backend that
    /// cannot position a writer inherits the default, and the engine falls
    /// back to copying the whole file.
    fn open_write_at(
        &self,
        path: &str,
        offset: u64,
    ) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let _ = offset;
        Err(FileSystemError::Unsupported(format!(
            "This storage backend does not support resuming a partial file: {path}"
        )))
    }

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

    /// Applies property edits to a directory and everything beneath it.
    /// Progress is reported through `progress`; the outcome separates
    /// successful entries from failures so the UI can surface partial
    /// success. The default rejects the operation.
    fn update_properties_recursive(
        &self,
        path: &str,
        _changes: &PropertyChanges,
        _progress: &dyn FileOperationProgressReporterTrait,
    ) -> Result<RecursivePropertyUpdateOutcome, FileSystemError> {
        Err(FileSystemError::Unsupported(format!(
            "This storage backend does not support editing properties: {path}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_scheme_of_a_path() {
        assert_eq!(scheme_of("smb://nas/media").expect("smb"), Scheme::Smb);
        assert_eq!(
            scheme_of("sftp://user@nas:22/home").expect("sftp"),
            Scheme::Sftp
        );
        // The TLS spellings stay distinct; the FTP backend picks its mode
        // off the scheme, not off a re-sniffed string.
        assert_eq!(scheme_of("ftps://nas/pub").expect("ftps"), Scheme::Ftps);
        assert_eq!(
            scheme_of("webdavs://cloud.example/dav").expect("webdavs"),
            Scheme::WebDavS
        );
        assert_eq!(
            scheme_of("cloud://google_drive:user@example.com/folder-id").expect("cloud"),
            Scheme::Cloud
        );
        // `file://` is local, in URI clothing.
        assert_eq!(scheme_of("file:///home").expect("local"), Scheme::Local);
    }

    #[test]
    fn treats_scheme_less_paths_as_local() {
        for path in [
            r"C:\Users\test",
            "/home/user",
            r"\\nas\share\folder",
            r"relative\nested://segment",
        ] {
            assert!(is_local_path(path), "path should stay local: {path}");
        }
        assert!(!is_local_path("smb://nas/share"), "a remote scheme is not local");
    }

    #[test]
    fn rejects_unknown_and_unregistered_schemes() {
        let unknown = resolve("s3://bucket/data").err().expect("unknown scheme");
        assert!(matches!(unknown, FileSystemError::InvalidInput(_)));

        // Every scheme now has a backend: smb, sftp, ftp and webdav resolve to
        // a connect attempt. Only an unknown scheme is a parse error, which the
        // first assertion above covers.
    }

    #[test]
    fn serves_local_paths_through_the_backend_trait() {
        use std::fs;

        let directory =
            std::env::temp_dir().join(format!("dae-vfs-trait-test-{}", std::process::id()));
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
