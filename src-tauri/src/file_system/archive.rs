use super::error::FileSystemError;
use super::progress::{
    FileOperationKind, FileOperationProgressReporter, FileOperationProgressReporterTrait,
    emit_preparing,
};
use super::types::{EntryKind, display_name_from_path, path_to_string};
use super::vfs::{self, FileSystemBackend};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const STREAM_CHUNK_BYTES: usize = 256 * 1024;

fn zip_error(error: zip::result::ZipError) -> FileSystemError {
    FileSystemError::Io(error.to_string())
}

/// Compresses entries into a zip archive inside `destination_dir` and returns
/// the archive path. Sources may live on any backend (streamed through the
/// VFS primitives); the archive itself is always a local file.
#[tauri::command]
#[specta::specta]
pub async fn compress_entries(
    sources: Vec<String>,
    destination_dir: String,
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
        compress_sync(sources, &destination_dir, &progress)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn compress_sync(
    sources: Vec<String>,
    destination_dir: &str,
    progress: &FileOperationProgressReporter,
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

    let archive_path = unique_archive_path(destination_dir, &archive_stem)?;
    let file = std::fs::File::create(&archive_path)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);

    // Sizing pass so the progress bar knows the uncompressed byte total.
    let mut total = 0_u64;
    for (path, backend) in &resolved {
        total += entry_tree_size(backend.as_ref(), path)?;
    }
    progress.start(total);

    for (path, backend) in resolved {
        let root_name = display_name_from_path(&path);
        add_tree_to_zip(
            backend.as_ref(),
            &path,
            &root_name,
            &mut writer,
            options,
            progress,
        )?;
    }

    writer.finish().map_err(zip_error)?;
    progress.finish();

    Ok(path_to_string(&archive_path))
}

/// "报告.doc" → "报告"; "photos" → "photos"; hidden ".gitignore" → ".gitignore".
fn archive_stem_of(path: &str) -> String {
    let name = display_name_from_path(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name,
    }
}

fn unique_archive_path(directory: &str, stem: &str) -> Result<PathBuf, FileSystemError> {
    let local = vfs::resolve(directory)?;

    let mut attempt = 0_u32;
    loop {
        let file_name = if attempt == 0 {
            format!("{stem}.zip")
        } else {
            format!("{stem} {attempt}.zip")
        };

        let candidate = PathBuf::from(directory).join(file_name);
        match local.stat(&path_to_string(&candidate)) {
            Err(FileSystemError::NotFound(_)) => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => attempt += 1,
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

fn add_tree_to_zip(
    backend: &dyn FileSystemBackend,
    path: &str,
    arc_name: &str,
    writer: &mut ZipWriter<std::fs::File>,
    options: SimpleFileOptions,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let stat = backend.stat(path)?;

    match stat.kind {
        EntryKind::Directory => {
            writer
                .add_directory(arc_name.to_owned(), options)
                .map_err(zip_error)?;

            for entry in backend.read_dir(path)?.entries {
                let child_arc = format!("{arc_name}/{}", entry.name);
                add_tree_to_zip(backend, &entry.path, &child_arc, writer, options, progress)?;
            }

            Ok(())
        }
        EntryKind::File => {
            let mut reader = backend.open_read(path)?;
            writer
                .start_file(arc_name.to_owned(), options)
                .map_err(zip_error)?;

            let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }

                writer.write_all(&buffer[..read])?;
                progress.advance_by(read as u64, Path::new(path));
            }

            Ok(())
        }
        // Symlinks and special nodes are skipped; archives stay portable.
        _ => Ok(()),
    }
}
