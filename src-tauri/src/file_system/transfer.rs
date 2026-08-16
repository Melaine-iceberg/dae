//! Generic transfer engine that moves entries between any two backends
//! through the trait's primitive operations (stat, mkdir, streams, remove).
//! Local-to-local transfers keep the backend-native rayon path in
//! `local/operations.rs`; every other combination lands here.

use super::error::FileSystemError;
use super::progress::FileOperationProgressReporterTrait;
use super::types::{EntryKind, EntryStat};
use super::vfs::{FileSystemBackend, SharedBackend};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

const STREAM_CHUNK_BYTES: usize = 256 * 1024;

/// One source entry paired with the backend that serves it.
pub struct TransferSource {
    pub path: String,
    pub backend: SharedBackend,
}

struct PlanEntry {
    source: TransferSource,
    stat: EntryStat,
    destination: String,
}

/// Copies sources into `destination` (a directory on `destination_backend`).
/// Existing entries are never overwritten.
pub fn copy_entries(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let plan = build_plan(sources, destination, destination_backend)?;
    progress.start(
        plan.iter()
            .map(|entry| count_copy_units(entry.source.backend.as_ref(), &entry.source.path, &entry.stat))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    for entry in plan {
        progress.begin_entry(Path::new(&entry.source.path));
        copy_node(
            entry.source.backend.as_ref(),
            &entry.source.path,
            &entry.stat,
            destination_backend.as_ref(),
            &entry.destination,
            progress,
        )?;
    }

    progress.finish();
    Ok(())
}

/// Moves sources into `destination`, preferring a protocol-native rename when
/// source and destination share one backend, falling back to copy + delete.
pub fn move_entries(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    let plan = build_plan(sources, destination, destination_backend)?;
    progress.start(
        plan.iter()
            .map(|entry| {
                count_copy_units(entry.source.backend.as_ref(), &entry.source.path, &entry.stat)
                    .and_then(|copy| {
                        count_delete_units(
                            entry.source.backend.as_ref(),
                            &entry.source.path,
                            &entry.stat,
                        )
                        .map(|delete| copy + delete)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    for entry in plan {
        progress.begin_entry(Path::new(&entry.source.path));

        let native_rename = if Arc::ptr_eq(&entry.source.backend, destination_backend) {
            entry
                .source
                .backend
                .rename_to(&entry.source.path, &entry.destination)
                .is_ok()
        } else {
            false
        };

        if native_rename {
            let units = count_copy_units(
                entry.source.backend.as_ref(),
                &entry.source.path,
                &entry.stat,
            )? + count_delete_units(
                entry.source.backend.as_ref(),
                &entry.source.path,
                &entry.stat,
            )?;
            progress.advance_by(units, Path::new(&entry.source.path));
        } else {
            copy_node(
                entry.source.backend.as_ref(),
                &entry.source.path,
                &entry.stat,
                destination_backend.as_ref(),
                &entry.destination,
                progress,
            )?;
            delete_node(
                entry.source.backend.as_ref(),
                &entry.source.path,
                &entry.stat,
                progress,
            )?;
        }
    }

    progress.finish();
    Ok(())
}

/// Deletes entries anywhere in the VFS, depth-first.
pub fn delete_entries(
    targets: Vec<TransferSource>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if targets.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before deleting".into(),
        ));
    }

    ensure_unique_paths(&targets)?;

    let mut stats = Vec::with_capacity(targets.len());
    for target in &targets {
        stats.push(target.backend.stat(&target.path)?);
    }

    progress.start(
        targets
            .iter()
            .zip(&stats)
            .map(|(target, stat)| {
                count_delete_units(target.backend.as_ref(), &target.path, stat)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    for (target, stat) in targets.into_iter().zip(stats) {
        progress.begin_entry(Path::new(&target.path));
        delete_node(target.backend.as_ref(), &target.path, &stat, progress)?;
    }

    progress.finish();
    Ok(())
}

/// Duplicates entries next to their originals with "副本" suffixes
/// ("报告.txt" → "报告 副本.txt", then "报告 副本 2.txt", …). Works on any
/// backend because each copy runs through the same engine as copy/paste.
pub fn duplicate_sources(
    sources: Vec<TransferSource>,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<Vec<String>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before duplicating".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    let mut stats = Vec::with_capacity(sources.len());
    for source in &sources {
        stats.push(source.backend.stat(&source.path)?);
    }

    progress.start(
        sources
            .iter()
            .zip(&stats)
            .map(|(source, stat)| count_copy_units(source.backend.as_ref(), &source.path, stat))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum(),
    );

    let mut created = Vec::with_capacity(sources.len());
    for (source, stat) in sources.into_iter().zip(stats) {
        progress.begin_entry(Path::new(&source.path));
        let destination = unique_duplicate_path(&source)?;

        copy_node(
            source.backend.as_ref(),
            &source.path,
            &stat,
            source.backend.as_ref(),
            &destination,
            progress,
        )?;

        created.push(destination);
    }

    progress.finish();
    Ok(created)
}

/// Computes the first free sibling path for a duplicate of `source`.
fn unique_duplicate_path(source: &TransferSource) -> Result<String, FileSystemError> {
    let name = last_segment(&source.path).ok_or_else(|| {
        FileSystemError::InvalidInput(format!(
            "The root of a volume cannot be duplicated: {}",
            source.path
        ))
    })?;

    let parent = parent_path_of(&source.path).ok_or_else(|| {
        FileSystemError::InvalidInput(format!(
            "The root of a volume cannot be duplicated: {}",
            source.path
        ))
    })?;

    let mut attempt = 0_u32;
    loop {
        let candidate_name = duplicate_name(name, attempt);
        let candidate = join_path(&parent, &candidate_name);

        match source.backend.stat(&candidate) {
            Err(FileSystemError::NotFound(_)) => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => attempt += 1,
        }
    }
}

/// The directory containing `path`, keeping the trailing separator style.
fn parent_path_of(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let separator_index = trimmed.rfind(['/', '\\'])?;
    Some(trimmed[..=separator_index].to_owned())
}

/// "report.txt" → "report 副本.txt" → "report 副本 2.txt"; directories keep
/// their full name because they have no extension to preserve.
fn duplicate_name(name: &str, attempt: u32) -> String {
    let suffix = if attempt == 0 {
        "副本".to_owned()
    } else {
        format!("副本 {}", attempt + 1)
    };

    let Some((stem, extension)) = name.rsplit_once('.') else {
        return format!("{name} {suffix}");
    };

    if stem.is_empty() {
        return format!("{name} {suffix}");
    }

    format!("{stem} {suffix}.{extension}")
}

fn build_plan(
    sources: Vec<TransferSource>,
    destination: &str,
    destination_backend: &SharedBackend,
) -> Result<Vec<PlanEntry>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry before pasting".into(),
        ));
    }

    ensure_unique_paths(&sources)?;

    if destination_backend.stat(destination)?.kind != EntryKind::Directory {
        return Err(FileSystemError::NotDirectory(destination.to_owned()));
    }

    let mut planned_names = HashSet::new();
    let mut plan = Vec::with_capacity(sources.len());

    for source in sources {
        let stat = source.backend.stat(&source.path)?;
        let name = last_segment(&source.path).ok_or_else(|| {
            FileSystemError::InvalidInput(format!(
                "The root of a volume cannot be copied or moved: {}",
                source.path
            ))
        })?;

        if !planned_names.insert(name.to_owned()) {
            return Err(FileSystemError::AlreadyExists(format!(
                "Multiple selected entries have the same name: {name}"
            )));
        }

        let target = join_path(destination, name);

        match destination_backend.stat(&target) {
            Err(FileSystemError::NotFound(_)) => {}
            Err(error) => return Err(error),
            Ok(_) => return Err(FileSystemError::AlreadyExists(target)),
        }

        if stat.kind == EntryKind::Directory
            && Arc::ptr_eq(&source.backend, destination_backend)
            && path_contains(&source.path, &target)
        {
            return Err(FileSystemError::InvalidInput(format!(
                "Cannot paste a folder into itself: {}",
                source.path
            )));
        }

        plan.push(PlanEntry {
            source,
            stat,
            destination: target,
        });
    }

    Ok(plan)
}

fn copy_node(
    source: &dyn FileSystemBackend,
    source_path: &str,
    stat: &EntryStat,
    destination_backend: &dyn FileSystemBackend,
    destination_path: &str,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind == EntryKind::Directory {
        destination_backend.mkdir(destination_path)?;
        progress.advance(Path::new(destination_path));

        let view = source.read_dir(source_path)?;
        for entry in &view.entries {
            let entry_stat = source.stat(&entry.path)?;
            copy_node(
                source,
                &entry.path,
                &entry_stat,
                destination_backend,
                &join_path(destination_path, &entry.name),
                progress,
            )?;
        }

        return Ok(());
    }

    // Files and symlinks both stream as content; a symlink materializes as a
    // regular file on the destination, which is the only portable mapping.
    let mut reader = source.open_read(source_path)?;
    let mut writer = destination_backend.open_write(destination_path)?;
    let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        writer.write_all(&buffer[..read])?;
        progress.advance_by(read as u64, Path::new(source_path));
    }

    writer.flush()?;
    Ok(())
}

fn delete_node(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
    progress: &dyn FileOperationProgressReporterTrait,
) -> Result<(), FileSystemError> {
    if stat.kind == EntryKind::Directory {
        let view = backend.read_dir(path)?;
        for entry in &view.entries {
            let entry_stat = backend.stat(&entry.path)?;
            delete_node(backend, &entry.path, &entry_stat, progress)?;
        }
    }

    backend.remove(path)?;
    progress.advance(Path::new(path));
    Ok(())
}

/// Work units for copying: file bytes plus one per directory created, summed
/// over the whole tree so progress totals match what execution advances.
fn count_copy_units(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
) -> Result<u64, FileSystemError> {
    if stat.kind != EntryKind::Directory {
        return Ok(stat.size.max(1));
    }

    let mut total = 1;
    for entry in backend.read_dir(path)?.entries {
        let entry_stat = backend.stat(&entry.path)?;
        total += count_copy_units(backend, &entry.path, &entry_stat)?;
    }

    Ok(total)
}

/// Work units for deleting: one per removed node, summed over the whole tree.
fn count_delete_units(
    backend: &dyn FileSystemBackend,
    path: &str,
    stat: &EntryStat,
) -> Result<u64, FileSystemError> {
    if stat.kind != EntryKind::Directory {
        return Ok(1);
    }

    let mut total = 1;
    for entry in backend.read_dir(path)?.entries {
        let entry_stat = backend.stat(&entry.path)?;
        total += count_delete_units(backend, &entry.path, &entry_stat)?;
    }

    Ok(total)
}

fn ensure_unique_paths(sources: &[TransferSource]) -> Result<(), FileSystemError> {
    let mut unique_paths = HashSet::new();
    for source in sources {
        if !unique_paths.insert(source.path.clone()) {
            return Err(FileSystemError::InvalidInput(format!(
                "The same entry was selected more than once: {}",
                source.path
            )));
        }
    }

    Ok(())
}

fn last_segment(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|segment| !segment.is_empty())
}

fn join_path(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches(['/', '\\']), name)
}

/// True when `descendant` lies inside `ancestor`, on the same backend.
fn path_contains(ancestor: &str, descendant: &str) -> bool {
    let ancestor = ancestor.trim_end_matches(['/', '\\']);
    let descendant = descendant.trim_end_matches(['/', '\\']);
    descendant.len() > ancestor.len()
        && descendant.starts_with(ancestor)
        && descendant[ancestor.len()..]
            .chars()
            .next()
            .is_some_and(|next| next == '/' || next == '\\')
}
