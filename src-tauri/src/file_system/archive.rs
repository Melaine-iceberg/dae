use super::error::FileSystemError;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::types::{EntryKind, display_name_from_path, path_to_string};
use super::vfs::{self, FileSystemBackend};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sevenz_rust2::{ArchiveEntry, ArchiveReader, ArchiveWriter, Password};
use specta::Type;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const STREAM_CHUNK_BYTES: usize = 256 * 1024;

fn zip_error(error: zip::result::ZipError) -> FileSystemError {
    FileSystemError::Io(error.to_string())
}

fn sevenz_error(error: sevenz_rust2::Error) -> FileSystemError {
    FileSystemError::Io(error.to_string())
}

/// Archive containers the explorer can create and extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Zip,
    Tar,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "7z")]
    SevenZip,
}

impl ArchiveFormat {
    /// The file extension (without leading dot) archives of this format use.
    fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::Tar => "tar",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::SevenZip => "7z",
        }
    }

    /// Detects the format of an existing file from its name (case-insensitive),
    /// covering common aliases such as `.tgz`.
    fn from_file_name(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_lowercase();

        if name.ends_with(".zip") {
            Some(ArchiveFormat::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else if name.ends_with(".tar") {
            Some(ArchiveFormat::Tar)
        } else if name.ends_with(".7z") {
            Some(ArchiveFormat::SevenZip)
        } else {
            None
        }
    }
}

/// Compresses entries into an archive of the requested format inside
/// `destination_dir` and returns the archive path. Sources may live on any
/// backend (streamed through the VFS primitives); the archive itself is
/// always a local file.
#[tauri::command]
#[specta::specta]
pub async fn compress_entries(
    sources: Vec<String>,
    destination_dir: String,
    format: ArchiveFormat,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before compressing".into(),
        ));
    }

    emit_preparing(&app, &operation_id, FileOperationKind::Compress);

    tauri::async_runtime::spawn_blocking(move || {
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Compress);
        compress_sync(sources, &destination_dir, format, &progress)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Extracts a local archive. Without `destination_dir` the entries land in a
/// fresh folder named after the archive stem next to the archive; the folder
/// path is returned.
#[tauri::command]
#[specta::specta]
pub async fn extract_archive(
    archive_path: String,
    destination_dir: Option<String>,
    operation_id: String,
    app: tauri::AppHandle,
) -> Result<String, FileSystemError> {
    emit_preparing(&app, &operation_id, FileOperationKind::Extract);

    tauri::async_runtime::spawn_blocking(move || {
        let progress =
            FileOperationProgressReporter::new(app, operation_id, FileOperationKind::Extract);
        extract_sync(&archive_path, destination_dir.as_deref(), &progress)
    })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

pub(super) fn compress_sync(
    sources: Vec<String>,
    destination_dir: &str,
    format: ArchiveFormat,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<String, FileSystemError> {
    if vfs::scheme_of(destination_dir)? != vfs::Scheme::Local {
        return Err(FileSystemError::InvalidInput(
            "Archives can only be created in local folders".into(),
        ));
    }

    let resolved = sources
        .into_iter()
        .map(|path| {
            let backend = vfs::resolve(&path)?;
            Ok((path, backend))
        })
        .collect::<Result<Vec<_>, FileSystemError>>()?;

    let archive_stem = if resolved.len() == 1 {
        archive_stem_of(&resolved[0].0)
    } else {
        "Archive".to_owned()
    };

    let archive_path = unique_archive_path(destination_dir, &archive_stem, format.extension())?;
    let mut sink = ArchiveSink::create(format, &archive_path)?;

    // Sizing pass so the progress bar knows the uncompressed byte total.
    let mut total = 0_u64;
    for (path, backend) in &resolved {
        total += entry_tree_size(backend.as_ref(), path)?;
    }
    progress.start(total);

    for (path, backend) in resolved {
        let root_name = display_name_from_path(&path);
        add_tree_to_archive(backend.as_ref(), &path, &root_name, &mut sink, progress)?;
    }

    sink.finish()?;
    progress.finish();

    Ok(path_to_string(&archive_path))
}

pub(super) fn extract_sync(
    archive_path: &str,
    destination_dir: Option<&str>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<String, FileSystemError> {
    if vfs::scheme_of(archive_path)? != vfs::Scheme::Local {
        return Err(FileSystemError::InvalidInput(
            "Archives can only be extracted from local folders".into(),
        ));
    }

    let archive = PathBuf::from(archive_path);
    if !archive.is_file() {
        return Err(FileSystemError::NotFound(archive_path.to_owned()));
    }

    let format = ArchiveFormat::from_file_name(&archive).ok_or_else(|| {
        FileSystemError::InvalidInput(format!(
            "Unsupported archive format: {}",
            display_name_from_path(archive_path)
        ))
    })?;

    // Sizing pass first: it also rejects unsafe entry names before any part of
    // the destination is created, so a malicious archive leaves nothing behind.
    let total = measure_uncompressed_size(&archive, format)?;
    progress.start(total);

    let destination_root = match destination_dir {
        Some(directory) => {
            if vfs::scheme_of(directory)? != vfs::Scheme::Local {
                return Err(FileSystemError::InvalidInput(
                    "Archives can only be extracted into local folders".into(),
                ));
            }

            let path = PathBuf::from(directory);
            std::fs::create_dir_all(&path)?;
            path
        }
        None => unique_destination_dir(&archive, &extraction_stem_of(&archive))?,
    };

    match format {
        ArchiveFormat::Zip => extract_zip(&archive, &destination_root, progress)?,
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            extract_tar(&archive, &destination_root, format, progress)?
        }
        ArchiveFormat::SevenZip => extract_sevenzip(&archive, &destination_root, progress)?,
    }

    progress.finish();

    Ok(path_to_string(&destination_root))
}

// -- Compression ----------------------------------------------------------------

/// Sink that hides the per-format writer differences behind one interface.
enum ArchiveSink {
    Zip {
        writer: Box<ZipWriter<std::fs::File>>,
        options: SimpleFileOptions,
    },
    Tar {
        builder: tar::Builder<TarBackend>,
    },
    SevenZip {
        writer: ArchiveWriter<std::fs::File>,
    },
}

/// Destination of a tar builder; gzip needs an explicit finish to write the
/// stream trailer, which plain `flush` cannot express through `dyn Write`.
enum TarBackend {
    Plain(std::fs::File),
    Gz(GzEncoder<std::fs::File>),
}

impl Write for TarBackend {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            TarBackend::Plain(file) => file.write(buffer),
            TarBackend::Gz(encoder) => encoder.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            TarBackend::Plain(file) => file.flush(),
            TarBackend::Gz(encoder) => encoder.flush(),
        }
    }
}

impl ArchiveSink {
    fn create(format: ArchiveFormat, path: &Path) -> Result<Self, FileSystemError> {
        match format {
            ArchiveFormat::Zip => {
                let file = std::fs::File::create(path)?;
                Ok(ArchiveSink::Zip {
                    writer: Box::new(ZipWriter::new(file)),
                    options: SimpleFileOptions::default()
                        .compression_method(CompressionMethod::Deflated)
                        .large_file(true),
                })
            }
            ArchiveFormat::Tar => {
                let file = std::fs::File::create(path)?;
                Ok(ArchiveSink::Tar {
                    builder: tar::Builder::new(TarBackend::Plain(file)),
                })
            }
            ArchiveFormat::TarGz => {
                let file = std::fs::File::create(path)?;
                let encoder = GzEncoder::new(file, Compression::default());
                Ok(ArchiveSink::Tar {
                    builder: tar::Builder::new(TarBackend::Gz(encoder)),
                })
            }
            ArchiveFormat::SevenZip => {
                let writer = ArchiveWriter::create(path).map_err(sevenz_error)?;
                Ok(ArchiveSink::SevenZip { writer })
            }
        }
    }

    fn add_directory(&mut self, arc_name: &str) -> Result<(), FileSystemError> {
        match self {
            ArchiveSink::Zip { writer, options } => writer
                .add_directory(arc_name.to_owned(), *options)
                .map_err(zip_error),
            ArchiveSink::Tar { builder } => {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                header.set_mtime(0);
                builder
                    .append_data(&mut header, arc_name, std::io::empty())
                    .map_err(FileSystemError::from)
            }
            ArchiveSink::SevenZip { writer } => writer
                .push_archive_entry(ArchiveEntry::new_directory(arc_name), None::<std::io::Empty>)
                .map(|_| ())
                .map_err(sevenz_error),
        }
    }

    fn add_file(
        &mut self,
        arc_name: &str,
        size: u64,
        reader: &mut dyn Read,
    ) -> Result<(), FileSystemError> {
        match self {
            ArchiveSink::Zip { writer, options } => {
                writer
                    .start_file(arc_name.to_owned(), *options)
                    .map_err(zip_error)?;
                copy_chunked(reader, writer)?;
                Ok(())
            }
            ArchiveSink::Tar { builder } => {
                let mut header = tar::Header::new_gnu();
                header.set_mode(0o644);
                header.set_size(size);
                header.set_mtime(0);
                builder
                    .append_data(&mut header, arc_name, reader)
                    .map_err(FileSystemError::from)
            }
            ArchiveSink::SevenZip { writer } => {
                let mut entry = ArchiveEntry::new_file(arc_name);
                entry.size = size;
                writer
                    .push_archive_entry(entry, Some(reader))
                    .map(|_| ())
                    .map_err(sevenz_error)
            }
        }
    }

    fn finish(self) -> Result<(), FileSystemError> {
        match self {
            ArchiveSink::Zip { writer, .. } => {
                writer.finish().map_err(zip_error)?;
            }
            ArchiveSink::Tar { builder } => {
                let backend = builder.into_inner().map_err(FileSystemError::from)?;
                match backend {
                    TarBackend::Plain(mut file) => file.flush()?,
                    TarBackend::Gz(encoder) => {
                        let mut file = encoder.finish()?;
                        file.flush()?;
                    }
                }
            }
            ArchiveSink::SevenZip { writer } => {
                let mut file = writer.finish()?;
                file.flush()?;
            }
        }

        Ok(())
    }
}

/// Streams `reader` to `writer` in fixed chunks so memory stays flat for
/// arbitrarily large entries.
fn copy_chunked(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
) -> Result<(), FileSystemError> {
    let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
    }
}

/// Reader proxy that reports consumed bytes to the progress reporter.
struct CountingReader<'a> {
    inner: Box<dyn Read + Send>,
    progress: &'a dyn FileOperationProgressReporterTrait,
    path: String,
}

impl<'a> CountingReader<'a> {
    fn new(
        inner: Box<dyn Read + Send>,
        progress: &'a dyn FileOperationProgressReporterTrait,
        path: &str,
    ) -> Self {
        Self {
            inner,
            progress,
            path: path.to_owned(),
        }
    }
}

impl Read for CountingReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read > 0 {
            self.progress.advance_by(read as u64, Path::new(&self.path));
        }
        Ok(read)
    }
}

/// Writer proxy that reports produced bytes to the progress reporter.
struct CountingWriter<'a> {
    inner: std::fs::File,
    progress: &'a dyn FileOperationProgressReporterTrait,
    path: String,
}

impl<'a> CountingWriter<'a> {
    fn new(
        inner: std::fs::File,
        progress: &'a dyn FileOperationProgressReporterTrait,
        path: &str,
    ) -> Self {
        Self {
            inner,
            progress,
            path: path.to_owned(),
        }
    }
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        if written > 0 {
            self.progress
                .advance_by(written as u64, Path::new(&self.path));
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// "报告.doc" → "报告"; "photos" → "photos"; hidden ".gitignore" → ".gitignore".
fn archive_stem_of(path: &str) -> String {
    let name = display_name_from_path(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name,
    }
}

/// Stem used for the default extraction folder. Double extensions are stripped
/// whole ("photos.tar.gz" → "photos") and the original casing is preserved.
pub(super) fn extraction_stem_of(path: &Path) -> String {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return archive_stem_of(&path_to_string(path));
    };

    for suffix in [".tar.gz", ".tgz", ".tar", ".zip", ".7z"] {
        let stem_length = name.len().saturating_sub(suffix.len());
        if name.len() > suffix.len()
            && name[stem_length..].eq_ignore_ascii_case(suffix)
            && !name[..stem_length].is_empty()
        {
            return name[..stem_length].to_owned();
        }
    }

    name.to_owned()
}

fn unique_archive_path(
    directory: &str,
    stem: &str,
    extension: &str,
) -> Result<PathBuf, FileSystemError> {
    let local = vfs::resolve(directory)?;

    let mut attempt = 0_u32;
    loop {
        let file_name = if attempt == 0 {
            format!("{stem}.{}", extension)
        } else {
            format!("{stem} {attempt}.{}", extension)
        };

        let candidate = PathBuf::from(directory).join(file_name);
        match local.stat(&path_to_string(&candidate)) {
            Err(FileSystemError::NotFound(_)) => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => attempt += 1,
        }
    }
}

/// Creates a fresh folder named `stem` (suffixed " 2", " 3", … on clashes)
/// next to `archive` and returns its path.
fn unique_destination_dir(archive: &Path, stem: &str) -> Result<PathBuf, FileSystemError> {
    let parent = archive
        .parent()
        .ok_or_else(|| FileSystemError::InvalidInput("The archive has no parent folder".into()))?;

    let mut attempt = 0_u32;
    loop {
        let folder_name = if attempt == 0 {
            stem.to_owned()
        } else {
            format!("{stem} {attempt}")
        };

        let candidate = parent.join(folder_name);
        match candidate.try_exists() {
            Ok(false) => {
                std::fs::create_dir_all(&candidate)?;
                return Ok(candidate);
            }
            Ok(true) => attempt += 1,
            Err(error) => return Err(FileSystemError::from(error)),
        }
    }
}

fn entry_tree_size(backend: &dyn FileSystemBackend, path: &str) -> Result<u64, FileSystemError> {
    let stat = backend.stat(path)?;

    match stat.kind {
        EntryKind::Directory => {
            let mut total = 0;
            for entry in backend.read_dir(path)?.entries {
                total += entry_tree_size(backend, &entry.path)?;
            }
            Ok(total)
        }
        EntryKind::File => Ok(stat.size),
        _ => Ok(0),
    }
}

fn add_tree_to_archive(
    backend: &dyn FileSystemBackend,
    path: &str,
    arc_name: &str,
    sink: &mut ArchiveSink,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let stat = backend.stat(path)?;

    match stat.kind {
        EntryKind::Directory => {
            sink.add_directory(arc_name)?;

            for entry in backend.read_dir(path)?.entries {
                let child_arc = format!("{arc_name}/{}", entry.name);
                add_tree_to_archive(backend, &entry.path, &child_arc, sink, progress)?;
            }

            Ok(())
        }
        EntryKind::File => {
            let reader = backend.open_read(path)?;
            let mut counting = CountingReader::new(reader, progress, path);
            sink.add_file(arc_name, stat.size, &mut counting)
        }
        // Symlinks and special nodes are skipped; archives stay portable.
        _ => Ok(()),
    }
}

// -- Extraction -----------------------------------------------------------------

/// Splits an archive entry name into path components, rejecting absolute
/// paths, drive prefixes, and parent traversal ("zip slip" attacks).
fn validated_components(name: &str) -> Result<Vec<&str>, FileSystemError> {
    let mut components = Vec::new();

    for component in name.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                return Err(FileSystemError::InvalidInput(format!(
                    "Blocked unsafe archive entry: {name}"
                )));
            }
            part if part.contains(':') => {
                return Err(FileSystemError::InvalidInput(format!(
                    "Blocked unsafe archive entry: {name}"
                )));
            }
            part => components.push(part),
        }
    }

    Ok(components)
}

/// Maps an archive entry name onto a path inside `root`.
fn sanitized_entry_path(root: &Path, name: &str) -> Result<PathBuf, FileSystemError> {
    let mut path = root.to_path_buf();
    for component in validated_components(name)? {
        path.push(component);
    }
    Ok(path)
}

/// Sums the uncompressed file sizes without writing anything, so the progress
/// bar knows its total — and every entry name is safe — before extraction
/// starts.
fn measure_uncompressed_size(
    archive_path: &Path,
    format: ArchiveFormat,
) -> Result<u64, FileSystemError> {
    match format {
        ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive_path)?;
            let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;

            let mut total = 0;
            for index in 0..archive.len() {
                let entry = archive.by_index(index).map_err(zip_error)?;
                validated_components(entry.name())?;
                if !entry.is_dir() {
                    total += entry.size();
                }
            }
            Ok(total)
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            let reader = open_tar_reader(archive_path, format)?;
            let mut archive = tar::Archive::new(reader);

            let mut total = 0;
            for entry in archive.entries().map_err(FileSystemError::from)? {
                let entry = entry?;
                if matches!(
                    entry.header().entry_type(),
                    tar::EntryType::Regular | tar::EntryType::Continuous
                ) {
                    validated_components(&entry.path()?.to_string_lossy())?;
                    total += entry.header().size().map_err(FileSystemError::from)?;
                }
            }
            Ok(total)
        }
        ArchiveFormat::SevenZip => {
            let reader = ArchiveReader::open(archive_path, Password::empty())
                .map_err(sevenz_error)?;
            let mut total = 0;
            for entry in &reader.archive().files {
                validated_components(&entry.name)?;
                if !entry.is_directory {
                    total += entry.size;
                }
            }
            Ok(total)
        }
    }
}

fn open_tar_reader(archive_path: &Path, format: ArchiveFormat) -> Result<Box<dyn Read>, FileSystemError> {
    let file = std::fs::File::open(archive_path)?;
    match format {
        ArchiveFormat::TarGz => Ok(Box::new(GzDecoder::new(file))),
        _ => Ok(Box::new(file)),
    }
}

fn extract_zip(
    archive_path: &Path,
    root: &Path,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;

        // Symlinks and special nodes are skipped; extraction stays portable.
        if entry.is_symlink() {
            continue;
        }

        let destination = sanitized_entry_path(root, entry.name())?;

        if entry.is_dir() {
            std::fs::create_dir_all(&destination)?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = std::fs::File::create(&destination)?;
        let mut writer = CountingWriter::new(output, progress, entry.name());
        copy_chunked(&mut entry, &mut writer)?;
    }

    Ok(())
}

fn extract_tar(
    archive_path: &Path,
    root: &Path,
    format: ArchiveFormat,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let reader = open_tar_reader(archive_path, format)?;
    let mut archive = tar::Archive::new(reader);

    for entry in archive.entries().map_err(FileSystemError::from)? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();

        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                let destination = sanitized_entry_path(root, &name)?;
                std::fs::create_dir_all(&destination)?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                let destination = sanitized_entry_path(root, &name)?;
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let output = std::fs::File::create(&destination)?;
                let mut writer = CountingWriter::new(output, progress, &name);
                copy_chunked(&mut entry, &mut writer)?;
            }
            // Hard links, symlinks, devices, and fifos are skipped.
            _ => {}
        }
    }

    Ok(())
}

fn extract_sevenzip(
    archive_path: &Path,
    root: &Path,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let mut reader =
        ArchiveReader::open(archive_path, Password::empty()).map_err(sevenz_error)?;

    reader
        .for_each_entries(|entry, data| {
            let name = entry.name.as_str();

            if entry.is_directory {
                let destination = sanitized_entry_path(root, name).map_err(std::io::Error::other)?;
                std::fs::create_dir_all(&destination)?;
                return Ok(true);
            }

            let destination = sanitized_entry_path(root, name).map_err(std::io::Error::other)?;
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let output = std::fs::File::create(&destination)?;
            let mut writer = CountingWriter::new(output, progress, name);
            std::io::copy(data, &mut writer)?;

            Ok(true)
        })
        .map_err(sevenz_error)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_system::test_support::TestProgress;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::Ordering as AtomicOrdering;

    fn make_archive_source_tree(root: &Path) {
        fs::create_dir_all(root.join("资料").join("nested")).expect("create nested directory");
        fs::write(root.join("root.txt"), "root content").expect("write root file");
        fs::write(root.join("资料/nested/leaf.txt"), "leaf content").expect("write leaf file");
    }

    #[test]
    fn compresses_and_extracts_every_archive_format() {
        let root = std::env::temp_dir().join(format!("dae-archive-test-{}", std::process::id()));
        let source = root.join("bundle");
        let output = root.join("output");
        fs::create_dir_all(&source).expect("create source directory");
        fs::create_dir_all(&output).expect("create output directory");
        make_archive_source_tree(&source);

        for format in [
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::TarGz,
            ArchiveFormat::SevenZip,
        ] {
            let compress_progress = TestProgress::new();
            let archive_path = compress_sync(
                vec![source.to_string_lossy().into_owned()],
                &output.to_string_lossy(),
                format,
                &compress_progress,
            )
            .expect("compress archive");

            let created = Path::new(&archive_path);
            assert!(created.is_file(), "archive exists: {archive_path}");
            assert_eq!(
                compress_progress.completed.load(AtomicOrdering::Relaxed),
                compress_progress.total.load(AtomicOrdering::Relaxed),
                "compression progress completes for {format:?}"
            );

            let extract_progress = TestProgress::new();
            let destination = extract_sync(&archive_path, None, &extract_progress)
                .expect("extract archive");

            let extracted_root = Path::new(&destination).join("bundle");
            assert_eq!(
                fs::read_to_string(extracted_root.join("root.txt")).expect("read extracted root"),
                "root content",
                "round-trip preserves files for {format:?}"
            );
            assert_eq!(
                fs::read_to_string(extracted_root.join("资料/nested/leaf.txt"))
                    .expect("read extracted leaf"),
                "leaf content",
                "round-trip preserves nested unicode paths for {format:?}"
            );
            assert_eq!(
                extract_progress.completed.load(AtomicOrdering::Relaxed),
                extract_progress.total.load(AtomicOrdering::Relaxed),
                "extraction progress completes for {format:?}"
            );

            fs::remove_dir_all(destination).expect("remove extraction folder");
        }

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn rejects_path_traversal_entries_during_extraction() {
        use std::io::Write;

        let root = std::env::temp_dir().join(format!("dae-zip-slip-test-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create test directory");

        let archive_path = root.join("malicious.zip");
        let file = fs::File::create(&archive_path).expect("create malicious archive");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("../escaped.txt", SimpleFileOptions::default())
            .expect("start traversal entry");
        writer.write_all(b"escaped").expect("write traversal entry");
        writer.finish().expect("finish malicious archive");

        let error = extract_sync(&archive_path.to_string_lossy(), None, &TestProgress::new())
            .expect_err("traversal entries must be blocked");

        fs::remove_file(&archive_path).expect("remove malicious archive");
        fs::remove_dir(&root).expect("remove test directory");

        assert!(matches!(error, FileSystemError::InvalidInput(_)));
        assert!(!root.join("../escaped.txt").exists());
    }

    #[test]
    fn derives_extraction_folder_stems_from_archive_names() {
        assert_eq!(extraction_stem_of(Path::new("Photos.tar.gz")), "Photos");
        assert_eq!(extraction_stem_of(Path::new("backup.TGZ")), "backup");
        assert_eq!(extraction_stem_of(Path::new("报告.zip")), "报告");
        assert_eq!(extraction_stem_of(Path::new("bundle.7z")), "bundle");
        assert_eq!(extraction_stem_of(Path::new("archive.tar")), "archive");
    }
}
