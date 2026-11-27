//! The `archive:` backend: one archive file on the local disk, one read-only
//! tree (docs/decisions/archive-browse.md).
//!
//! Browsing is index-based, never extraction: ZIP's central directory and
//! TAR's header scan produce a flat member list, and `read_dir` slices the
//! children out of it. `open_read` reads byte ranges out of the archive
//! file — `Stored` verbatim, `Deflated` through `flate2` — so Quick Look and
//! drag-out copy work without ever materializing a file.
//!
//! The one thing this backend refuses to guess is encryption: an encrypted
//! ZIP answers `fs.archive_encrypted` for the whole tree, because listing
//! names while withholding content is a worse answer than "extract me".

use super::archive::zip_error;
use super::error::FileSystemError;
use super::local::build_breadcrumbs;
use super::search::SearchFilters;
use super::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, FileProperties, NewEntryKind,
    SearchResponse,
};
use super::url::{EntryUrl, Location};
use super::vfs::{FileSystemBackend, SharedBackend};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

/// The two formats this backend can browse. Everything else keeps the
/// extract path; the format check runs once, at `resolve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowsedFormat {
    Zip,
    Tar,
}

impl BrowsedFormat {
    /// The browsable subset of `archive::ArchiveFormat`'s name detection.
    /// A remote target is rejected by `open_backend`, not here — the reason
    /// ("no seekable file") belongs to the whole archive, not its name.
    fn of_path(path: &Path) -> Result<Self, FileSystemError> {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_lowercase();
        if name.ends_with(".zip") {
            Ok(Self::Zip)
        } else if name.ends_with(".tar") {
            Ok(Self::Tar)
        } else {
            Err(FileSystemError::InvalidInput(format!(
                "fs.archive_format_unsupported: {name}"
            )))
        }
    }
}
#[derive(Debug, Clone)]
enum MemberData {
    /// The byte range after the local header, plus how it was compressed.
    Zip {
        start: u64,
        compressed: u64,
        method: zip::CompressionMethod,
    },
    /// The byte offset of the member's data inside the archive file.
    Tar { data_start: u64 },
}

/// One archive member with its inner path split into sanitized segments.
#[derive(Debug, Clone)]
struct Member {
    segments: Vec<String>,
    is_dir: bool,
    size: u64,
    modified_ms: Option<u64>,
    data: MemberData,
}

/// The backend for one archive file. Stateless beyond the target: both ZIP
/// central directories and TAR scans share the process-wide LRU index cache
/// keyed by path, modification time, and size.
pub struct ArchiveBackend {
    target: PathBuf,
    /// The verbatim local path text, so child paths round-trip exactly the
    /// way the shell typed the archive.
    target_text: String,
    format: BrowsedFormat,
}

/// Opens the backend for one `archive:` URL. Only a local target can browse:
/// a remote target has no seekable file, and random access — not streaming —
/// is what both formats' indexes assume.
pub fn open_backend(url: &EntryUrl) -> Result<SharedBackend, FileSystemError> {
    let Location::Archive { target, .. } = url.location() else {
        return Err(FileSystemError::InvalidInput(format!(
            "fs.archive_local_only: {url}"
        )));
    };
    let Some(target_text) = target.local_path() else {
        return Err(FileSystemError::InvalidInput(format!(
            "fs.archive_local_only: {url}"
        )));
    };
    let format = BrowsedFormat::of_path(Path::new(target_text))?;
    Ok(Arc::new(ArchiveBackend {
        target: PathBuf::from(target_text),
        target_text: target_text.to_owned(),
        format,
    }))
}

impl ArchiveBackend {
    /// Splits one archive path back into the backend's own target and the
    /// inner segments. Every method re-parses — paths are the only state a
    /// caller carries, and re-parsing is cheaper than caching per call.
    fn split(&self, path: &str) -> Result<Vec<String>, FileSystemError> {
        let url = EntryUrl::parse(path)?;
        match url.location() {
            Location::Archive { segments, .. } => Ok(segments.clone()),
            _ => Err(FileSystemError::InvalidInput(format!(
                "fs.archive_local_only: {path}"
            ))),
        }
    }

    /// The member index for this backend's target.
    fn members(&self) -> Result<Arc<Vec<Member>>, FileSystemError> {
        cached_members(&self.target, self.format)
    }

    fn breadcrumbs(&self, inner: &[String]) -> Vec<Breadcrumb> {
        let mut crumbs = build_breadcrumbs(&self.target);
        // The archive file itself is replaced by its `archive:` root, so the
        // last crumb never points back at the container as a plain file.
        crumbs.pop();
        let mut archive_path = format!("archive:{}!", self.target_text);
        crumbs.push(Breadcrumb {
            name: self
                .target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.target_text.clone()),
            path: archive_path.clone(),
        });
        for segment in inner {
            archive_path.push('/');
            archive_path.push_str(segment);
            crumbs.push(Breadcrumb {
                name: segment.clone(),
                path: archive_path.clone(),
            });
        }
        crumbs
    }
}

/// A read_dir child: a member, or a directory the index only implies (ZIP
/// routinely omits explicit entries for intermediate folders).
struct Child<'a> {
    name: String,
    member: Option<&'a Member>,
    directory: bool,
}

fn children_of<'a>(members: &'a [Member], prefix: &[String]) -> Vec<Child<'a>> {
    let mut children: Vec<Child<'a>> = Vec::new();
    for member in members {
        let Some(rest) = member.segments.strip_prefix(prefix) else {
            continue;
        };
        let Some(name) = rest.first() else {
            continue;
        };
        let directory = rest.len() > 1 || member.is_dir;
        match children.iter_mut().find(|child| child.name == *name) {
            // An explicit member outranks an earlier implied directory.
            Some(existing) => {
                if existing.member.is_none() && !directory {
                    *existing = Child {
                        name: name.clone(),
                        member: Some(member),
                        directory: false,
                    };
                }
            }
            None => children.push(Child {
                name: name.clone(),
                member: Some(member),
                directory,
            }),
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));
    children
}

fn directory_entry(
    url: &EntryUrl,
    name: &str,
    child: &Child<'_>,
) -> Result<DirectoryEntry, FileSystemError> {
    let member = child.member;
    let directory = child.directory || member.is_some_and(|member| member.is_dir);
    Ok(DirectoryEntry {
        name: name.to_owned(),
        path: url.join(name).to_string(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        modified_at: member.and_then(|member| member.modified_ms),
        created_at: None,
        created_fallback: false,
        size: member.filter(|_| !directory).map(|member| member.size),
        hidden: name.starts_with('.'),
        read_only: false,
    })
}

fn zip_members(target: &Path) -> Result<Vec<Member>, FileSystemError> {
    let file = std::fs::File::open(target)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    let mut members = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        // Raw: the index reads metadata only, and by_index would insist on
        // a password for AES members before `encrypted()` could be seen.
        let entry = archive.by_index_raw(index).map_err(zip_error)?;
        if entry.encrypted() {
            return Err(FileSystemError::InvalidInput(
                "fs.archive_encrypted".into(),
            ));
        }
        let raw_name = entry.name().map_err(zip_error)?;
        let segments = super::url::normalize_segments(&raw_name);
        if segments.is_empty() {
            continue;
        }
        members.push(Member {
            is_dir: entry.is_dir(),
            size: entry.size(),
            modified_ms: entry.last_modified().and_then(zip_modified_ms),
            data: MemberData::Zip {
                start: entry.data_start().ok_or_else(|| {
                    FileSystemError::Io(format!("zip data start unavailable: {raw_name}"))
                })?,
                compressed: entry.compressed_size(),
                method: entry.compression(),
            },
            segments,
        });
    }
    Ok(members)
}

/// ZIP's local timestamp is DOS-era civil fields with no epoch handy and no
/// `chrono` in this crate; days-from-civil is five lines and keeps the
/// conversion where the data lives.
fn zip_modified_ms(time: zip::DateTime) -> Option<u64> {
    let year = i64::from(time.year());
    let month = i64::from(time.month());
    let day = i64::from(time.day());
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days.checked_mul(86_400)?
        + i64::from(time.hour()) * 3_600
        + i64::from(time.minute()) * 60
        + i64::from(time.second());
    seconds.checked_mul(1000)?.try_into().ok()
}
/// A reader that shares its byte position, so a `tar::Archive` scan can
/// record where each member's data starts without reading the data.
struct CountingReader {
    inner: std::fs::File,
    position: Arc<std::sync::atomic::AtomicU64>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.position
            .fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(read)
    }
}

fn tar_members(target: &Path) -> Result<Vec<Member>, FileSystemError> {
    let file = std::fs::File::open(target)?;
    let position = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counting = CountingReader {
        inner: file,
        position: Arc::clone(&position),
    };
    let mut archive = tar::Archive::new(counting);
    let mut members = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let segments = super::url::normalize_segments(&entry.path()?.to_string_lossy());
        if segments.is_empty() {
            continue;
        }
        // The iterator yields each entry with its data still unread, so the
        // shared counter holds the member's data start exactly here.
        let data_start = position.load(std::sync::atomic::Ordering::Relaxed);
        members.push(Member {
            is_dir: entry.header().entry_type().is_dir(),
            size: entry.size(),
            modified_ms: entry.header().mtime().ok().and_then(|s| s.checked_mul(1000)),
            data: MemberData::Tar { data_start },
            segments,
        });
    }
    Ok(members)
}

struct CachedIndex {
    format: BrowsedFormat,
    mtime_ms: u64,
    size: u64,
    last_used: u64,
    members: Arc<Vec<Member>>,
}

#[derive(Default)]
struct IndexCache {
    clock: u64,
    entries: HashMap<PathBuf, CachedIndex>,
}

const INDEX_CACHE_CAP: usize = 32;

/// The archive index cache: one parse per `(path, format, mtime, size)`,
/// process-wide with LRU eviction at [`INDEX_CACHE_CAP`].
fn cached_members(
    target: &Path,
    format: BrowsedFormat,
) -> Result<Arc<Vec<Member>>, FileSystemError> {
    let metadata = std::fs::metadata(target)?;
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let size = metadata.len();

    static CACHE: LazyLock<Mutex<IndexCache>> = LazyLock::new(|| Mutex::new(IndexCache::default()));
    let mut guard = CACHE.lock().expect("archive index cache poisoned");
    if let Some(entry) = guard.entries.get(target)
        && entry.format == format
        && entry.mtime_ms == mtime_ms
        && entry.size == size
    {
        let members = Arc::clone(&entry.members);
        guard.clock = guard.clock.wrapping_add(1);
        let stamp = guard.clock;
        if let Some(entry) = guard.entries.get_mut(target) {
            entry.last_used = stamp;
        }
        return Ok(members);
    }
    // Parse outside the cache lock so a large archive does not stall other tabs.
    drop(guard);

    let members = Arc::new(match format {
        BrowsedFormat::Zip => zip_members(target)?,
        BrowsedFormat::Tar => tar_members(target)?,
    });

    let mut guard = CACHE.lock().expect("archive index cache poisoned");
    if guard.entries.len() >= INDEX_CACHE_CAP
        && !guard.entries.contains_key(target)
        && let Some(oldest_key) = guard
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
    {
        guard.entries.remove(&oldest_key);
    }
    guard.clock = guard.clock.wrapping_add(1);
    let stamp = guard.clock;
    guard.entries.insert(
        target.to_owned(),
        CachedIndex {
            format,
            mtime_ms,
            size,
            last_used: stamp,
            members: Arc::clone(&members),
        },
    );
    Ok(members)
}

#[cfg(test)]
fn tar_index(target: &Path) -> Result<Arc<Vec<Member>>, FileSystemError> {
    cached_members(target, BrowsedFormat::Tar)
}

#[cfg(test)]
fn zip_index(target: &Path) -> Result<Arc<Vec<Member>>, FileSystemError> {
    cached_members(target, BrowsedFormat::Zip)
}

fn read_only(path: &str) -> FileSystemError {
    FileSystemError::Unsupported(format!("fs.archive_read_only: {path}"))
}

impl FileSystemBackend for ArchiveBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let url = EntryUrl::parse(path)?;
        let inner = self.split(path)?;
        let members = self.members()?;

        // `read_dir` on a member file is the same mistake it is on the local
        // disk; say so rather than handing back a one-row view.
        if !inner.is_empty()
            && let Some(member) = members.iter().find(|member| member.segments == inner)
            && !member.is_dir
            && !members.iter().any(|candidate| {
                candidate.segments.len() > inner.len() && candidate.segments.starts_with(&inner)
            })
        {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }

        let children = children_of(&members, &inner);
        if !inner.is_empty()
            && children.is_empty()
            && !members
                .iter()
                .any(|member| member.segments == inner && member.is_dir)
        {
            return Err(FileSystemError::NotFound(path.to_owned()));
        }
        let entries = children
            .iter()
            .map(|child| directory_entry(&url, &child.name, child))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DirectoryView {
            path: url.to_string(),
            breadcrumbs: self.breadcrumbs(&inner),
            entries,
            stream_id: None,
        })
    }

    fn create_entry(
        &self,
        _directory: &str,
        _name: &str,
        _kind: NewEntryKind,
    ) -> Result<String, FileSystemError> {
        Err(read_only(_directory))
    }

    fn rename_entry(&self, path: &str, _new_name: &str) -> Result<(), FileSystemError> {
        Err(read_only(path))
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let inner = self.split(path)?;
        let members = self.members()?;
        if let Some(member) = members.iter().find(|member| member.segments == inner) {
            return Ok(EntryStat {
                kind: if member.is_dir {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: member.size,
                modified_at: member.modified_ms,
            });
        }
        // No member owns the path, but members live under it: the index
        // implies the directory the archive never wrote down.
        if members
            .iter()
            .any(|member| member.segments.len() > inner.len() && member.segments.starts_with(&inner))
        {
            return Ok(EntryStat {
                kind: EntryKind::Directory,
                size: 0,
                modified_at: None,
            });
        }
        Err(FileSystemError::NotFound(path.to_owned()))
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        Err(read_only(path))
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let inner = self.split(path)?;
        let members = self.members()?;
        let Some(member) = members.iter().find(|member| member.segments == inner) else {
            return Err(FileSystemError::NotFound(path.to_owned()));
        };
        if member.is_dir {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }
        let mut file = std::fs::File::open(&self.target)?;
        match &member.data {
            MemberData::Zip {
                start,
                compressed,
                method,
            } => {
                file.seek(SeekFrom::Start(*start))?;
                let range = file.take(*compressed);
                match method {
                    zip::CompressionMethod::Stored => Ok(Box::new(range)),
                    zip::CompressionMethod::Deflated => {
                        Ok(Box::new(flate2::read::DeflateDecoder::new(range)))
                    }
                    other => Err(FileSystemError::Unsupported(format!(
                        "fs.archive_method_unsupported: {other:?}"
                    ))),
                }
            }
            // `data_start` was recorded at scan time; the iterator had
            // already consumed the header and any long-name extension.
            MemberData::Tar { data_start } => {
                file.seek(SeekFrom::Start(*data_start))?;
                Ok(Box::new(file.take(member.size)))
            }
        }
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn std::io::Write + Send>, FileSystemError> {
        Err(read_only(path))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        Err(read_only(path))
    }

    fn rename_to(&self, source: &str, _destination: &str) -> Result<(), FileSystemError> {
        Err(read_only(source))
    }

    fn properties(&self, path: &str) -> Result<FileProperties, FileSystemError> {
        // Read-only is the default's answer for edits; this override exists
        // only so the properties dialog keeps its basic page.
        let stat = self.stat(path)?;
        Ok(FileProperties::basic(path, stat))
    }

    fn search(
        &self,
        _root: &str,
        _query: &str,
        _filters: &SearchFilters,
        _is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        Err(FileSystemError::InvalidInput(
            "fs.search_local_only".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dae-archive-view-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.add_directory("docs", options).unwrap();
        zip.start_file("docs/readme.txt", options).unwrap();
        zip.write_all(b"the quick brown fox").unwrap();
        zip.start_file("docs/data.bin", options).unwrap();
        zip.write_all(&vec![7u8; 100_000]).unwrap();
        zip.start_file("docs/nested/deep.txt", options).unwrap();
        zip.write_all(b"deep").unwrap();
        zip.start_file("top.txt", options).unwrap();
        zip.write_all(b"top level").unwrap();
        zip.finish().unwrap();
    }

    fn build_tar(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut tar = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        for (name, body) in [("docs/readme.txt", b"tar member" as &[u8]), ("top.bin", b"\x00\x01")] {
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, std::io::Cursor::new(body)).unwrap();
        }
        tar.finish().unwrap();
    }

    fn resolve(path: &str) -> SharedBackend {
        super::super::vfs::resolve(path).unwrap()
    }

    fn names(view: &DirectoryView) -> Vec<String> {
        view.entries.iter().map(|entry| entry.name.clone()).collect()
    }

    #[test]
    fn browses_a_zip_tree_from_the_central_directory() {
        let dir = scratch("zip");
        let zip_path = dir.join("notes.zip");
        build_zip(&zip_path);
        let root = format!("archive:{}!", zip_path.to_string_lossy());

        let backend = resolve(&root);
        let view = backend.read_dir(&root).unwrap();
        assert_eq!(names(&view), ["docs", "top.txt"]);
        assert_eq!(view.entries[0].kind, EntryKind::Directory);
        assert_eq!(view.entries[1].size, Some(9));
        assert_eq!(view.stream_id, None);
        // Breadcrumbs: local ancestors, then the archive root by name.
        let last = view.breadcrumbs.last().unwrap();
        assert_eq!(last.name, "notes.zip");
        assert_eq!(last.path, root);

        let docs = format!("{root}/docs");
        let view = backend.read_dir(&docs).unwrap();
        assert_eq!(names(&view), ["data.bin", "nested", "readme.txt"]);

        let nested = format!("{docs}/nested");
        let view = backend.read_dir(&nested).unwrap();
        assert_eq!(names(&view), ["deep.txt"]);

        let deep = format!("{nested}/deep.txt");
        let stat = backend.stat(&deep).unwrap();
        assert_eq!(stat.kind, EntryKind::File);
        let mut text = String::new();
        backend.open_read(&deep).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "deep");

        // A zip without explicit directory entries still implies them.
        let readme = format!("{docs}/readme.txt");
        let mut text = String::new();
        backend.open_read(&readme).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "the quick brown fox");
    }

    #[test]
    fn open_read_serves_stored_and_deflated_members() {
        let dir = scratch("zip-range");
        let zip_path = dir.join("mixed.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let stored =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("stored.bin", stored).unwrap();
        zip.write_all(&vec![9u8; 50_000]).unwrap();
        zip.start_file("deflated.bin", deflated).unwrap();
        zip.write_all(&vec![3u8; 50_000]).unwrap();
        zip.finish().unwrap();

        let backend = resolve(&format!("archive:{}!", zip_path.to_string_lossy()));
        for name in ["stored.bin", "deflated.bin"] {
            let path = format!("archive:{}!/{name}", zip_path.to_string_lossy());
            let mut bytes = Vec::new();
            backend.open_read(&path).unwrap().read_to_end(&mut bytes).unwrap();
            let expected = match name {
                "stored.bin" => vec![9u8; 50_000],
                _ => vec![3u8; 50_000],
            };
            assert_eq!(bytes, expected, "{name} round-trips byte for byte");
        }
    }

    #[test]
    fn browses_a_tar_with_a_cached_index() {
        let dir = scratch("tar");
        let tar_path = dir.join("notes.tar");
        build_tar(&tar_path);
        let root = format!("archive:{}!", tar_path.to_string_lossy());

        let backend = resolve(&root);
        let view = backend.read_dir(&root).unwrap();
        assert_eq!(names(&view), ["docs", "top.bin"]);

        let member = format!("{root}/docs/readme.txt");
        let mut text = String::new();
        backend.open_read(&member).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "tar member");

        // The second read goes through the cache; a rewrite that changes the
        // file's millisecond timestamp drops it. The sleep is what makes the
        // mtime move: both writes happen inside this one test.
        let before = tar_index(&tar_path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&tar_path, std::fs::read(&tar_path).unwrap()).unwrap();
        let after = tar_index(&tar_path).unwrap();
        assert!(!Arc::ptr_eq(&before, &after), "mtime change re-scans");
    }

    #[test]
    fn rejects_encrypted_zips_unsupported_formats_and_writes() {
        let dir = scratch("guards");
        // Any non-zip extension is rejected at resolve, before any read.
        let junk = dir.join("plain.txt");
        std::fs::write(&junk, b"not an archive").unwrap();
        let error = super::super::vfs::resolve(&format!("archive:{}!", junk.to_string_lossy()))
            .err()
            .expect("a plain text file is not browsable");
        assert!(error.to_string().contains("fs.archive_format_unsupported"));

        // tar.gz parses as an archive path but is not browsable.
        let tgz = dir.join("bundle.tar.gz");
        std::fs::write(&tgz, b"pretend").unwrap();
        let error = super::super::vfs::resolve(&format!("archive:{}!", tgz.to_string_lossy()))
            .err()
            .expect("tar.gz is not browsable in-process");
        assert!(error.to_string().contains("fs.archive_format_unsupported"));

        let zip_path = dir.join("open.zip");
        build_zip(&zip_path);
        let backend = resolve(&format!("archive:{}!", zip_path.to_string_lossy()));
        let root = format!("archive:{}!", zip_path.to_string_lossy());
        assert!(matches!(
            backend.create_entry(&root, "x", NewEntryKind::File),
            Err(FileSystemError::Unsupported(_))
        ));
        assert!(matches!(
            backend.remove(&format!("{root}/top.txt")),
            Err(FileSystemError::Unsupported(_))
        ));

        // An encrypted member refuses the whole tree.
        let secret = dir.join("secret.zip");
        let file = std::fs::File::create(&secret).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .with_aes_encryption(zip::AesMode::Aes256, "swordfish");
        zip.start_file("hidden.txt", options).unwrap();
        zip.write_all(b"classified").unwrap();
        zip.finish().unwrap();
        let error = resolve(&format!("archive:{}!", secret.to_string_lossy()))
            .read_dir(&format!("archive:{}!", secret.to_string_lossy()))
            .unwrap_err();
        assert!(error.to_string().contains("fs.archive_encrypted"));
    }

    #[test]
    fn archive_paths_parse_join_and_clamp() {
        let url = EntryUrl::parse("archive:/K:/code/notes.zip!/docs").unwrap();
        assert_eq!(url.scheme(), crate::fs::url::Scheme::Archive);
        assert_eq!(url.segments(), ["docs"]);
        assert_eq!(url.file_name().unwrap(), "docs");
        let joined = url.join("2026");
        assert_eq!(joined.to_string(), "archive:/K:/code/notes.zip!/docs/2026");
        let (parent, name) = joined.pop().unwrap();
        assert_eq!(name, "2026");
        assert_eq!(parent.to_string(), "archive:/K:/code/notes.zip!/docs");

        // A remote target parses; the backend rejects it, not the parser.
        let remote = EntryUrl::parse("archive:smb://nas/share/x.zip!/a").unwrap();
        assert_eq!(remote.to_string(), "archive:smb://nas/share/x.zip!/a");
        let error = super::super::vfs::resolve(&remote.to_string())
            .err()
            .expect("a remote archive target cannot be browsed");
        assert!(error.to_string().contains("fs.archive_local_only"));

        // `..` clamps at the archive root; nested archives and the `://`
        // spelling are refused.
        assert_eq!(
            EntryUrl::parse("archive:/K:/x.zip!/../../escape").unwrap().to_string(),
            "archive:/K:/x.zip!/escape"
        );
        assert!(EntryUrl::parse("archive:archive:/K:/x.zip!/a").is_err());
        assert!(EntryUrl::parse("archive://K:/x.zip!/a").is_err());
        assert!(EntryUrl::parse("archive:/K:/x.zip").is_err());
    }

    #[test]
    fn zip_cache_and_missing_or_file_read_dir_behave_consistently() {
        let dir = scratch("zip-cache-errors");
        let zip_path = dir.join("tree.zip");
        build_zip(&zip_path);
        let root = format!("archive:{}!", zip_path.to_string_lossy());
        let backend = resolve(&root);

        let first = zip_index(&zip_path).unwrap();
        let second = zip_index(&zip_path).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "second zip index read hits cache");

        assert!(matches!(
            backend.read_dir(&format!("{root}/top.txt")),
            Err(FileSystemError::NotDirectory(_))
        ));
        assert!(matches!(
            backend.read_dir(&format!("{root}/does-not-exist")),
            Err(FileSystemError::NotFound(_))
        ));

        // Explicit empty directories remain browsable.
        let empty_zip = dir.join("empty-dir.zip");
        {
            let file = std::fs::File::create(&empty_zip).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.add_directory("blank", options).unwrap();
            zip.finish().unwrap();
        }
        let empty_root = format!("archive:{}!", empty_zip.to_string_lossy());
        let empty_backend = resolve(&empty_root);
        let view = empty_backend.read_dir(&format!("{empty_root}/blank")).unwrap();
        assert!(view.entries.is_empty());
    }
}
