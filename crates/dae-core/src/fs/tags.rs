//! File tags.
//!
//! Two documents, deliberately separate:
//!
//! * `tags.toml` in the config directory holds the *definitions* — id, name,
//!   colour. It is small, hand-editable, and exportable.
//! * The association between a path and its tag ids lives either on the file
//!   itself (an alternate data stream on Windows, an xattr on Unix) or, when
//!   the file system cannot carry one, in a sharded index under
//!   `<config>/tags-index/`.
//!
//! The index is written on *every* change, not only when the file store is
//! unavailable. That is what lets `tag:` search, the sidebar counts and the tag
//! sort answer without walking the disk or touching one file per row. The file
//! store is what makes the tags travel with the file; the index is what makes
//! them queryable.
//!
//! The association is keyed by the folded path (case-folded on Windows and
//! macOS) and the entry keeps the original spelling. Renames made through dae
//! move the entry (`note_rename`); a rename made outside dae is recovered from
//! the stored file identity (volume serial + file index on Windows, `dev:ino`
//! on Unix) the next time the new path is read, because that identity is what
//! survives the rename. A file replaced in place by a different file does not
//! inherit the old entry: the identity mismatch makes the read fall back to the
//! file's own stream.
//!
//! Nothing here touches the listing hot path. [`tags_for`] is meant to be called
//! for the visible rows (or, for a tag sort, on a worker), never per enumerated
//! entry.

use crate::error::FileSystemError;
use crate::fs::atomic::write_atomic;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFINITIONS_FILE: &str = "tags.toml";
const INDEX_DIR: &str = "tags-index";

/// The xattr / ADS name the ids are stored under. Namespaced with `dae` on
/// every platform: Linux has no shared tag convention and inventing one would
/// only collide with tools that do.
#[cfg(windows)]
const STREAM_NAME: &str = "dae.tags";
#[cfg(target_os = "macos")]
const STREAM_NAME: &str = "com.dae.tags";
#[cfg(all(unix, not(target_os = "macos")))]
const STREAM_NAME: &str = "user.dae.tags";

/// One tag a user can assign. `id` is stable across renames; `name` and
/// `color` are what the UI shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinition {
    pub id: String,
    pub name: String,
    /// `#rrggbb`, normalised on the way in. Defaulted so a hand-edited or
    /// older document without a colour still parses instead of losing every
    /// definition to one missing field.
    #[serde(default)]
    pub color: String,
}

/// Where a path's ids were found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagSource {
    /// NTFS alternate data stream.
    Ads,
    /// POSIX extended attribute.
    Xattr,
    /// The per-volume index; the file itself carries nothing.
    Index,
}

/// The tags of one path, as a batch read reports them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryTags {
    pub path: String,
    pub tag_ids: Vec<String>,
    pub source: TagSource,
}

/// What a write actually managed to do.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagWriteReport {
    /// Paths whose tags were stored on the file itself.
    pub saved_with_file: usize,
    /// Paths stored in the index only (remote, archive, or a file system that
    /// refused the stream/xattr).
    pub index_only: usize,
    /// Paths that could not be written anywhere.
    pub failed: Vec<String>,
    /// At least one path lives on a volume that cannot carry tags with the
    /// file. The editor uses this to say "these tags stay in dae" instead of
    /// silently pretending they travel.
    pub file_store_unavailable: bool,
}

/// What importing a `tags.toml` document did. `skipped` carries one line per
/// definition the merge refused, so the settings page can say why instead of
/// dropping it silently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagImportReport {
    /// Definitions the local list did not have.
    pub added: usize,
    /// Definitions whose id was already known; name and colour now come from
    /// the imported document.
    pub updated: usize,
    /// Definitions the merge refused: empty id/name, a name over the length
    /// limit, or a name another id already owns.
    pub skipped: Vec<String>,
}

/// What rebuilding the index did. The rebuild reads only what the index
/// already mentions — it never walks a directory tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRepairReport {
    /// Entries left in the index.
    pub entries: usize,
    /// Entries whose file no longer exists at that path.
    pub dropped_missing: usize,
    /// Entries that named the same file identity as a newer entry.
    pub dropped_duplicate: usize,
    /// Entries whose ids were re-read from the file's own stream, which is
    /// the truth when the index and the file disagree.
    pub adopted_from_file: usize,
    /// Shard documents that could not be parsed and were moved aside as
    /// `*.corrupt-<millis>` instead of being deleted.
    pub corrupt_shards: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DefinitionsFile {
    #[serde(default)]
    tags: Vec<TagDefinition>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexEntry {
    /// The path in the spelling it first arrived in.
    #[serde(default)]
    path: String,
    #[serde(default)]
    tag_ids: Vec<String>,
    #[serde(default)]
    updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexShard {
    #[serde(default)]
    entries: BTreeMap<String, IndexEntry>,
}

/// Serialises read-modify-write cycles over the index shards. Definitions and
/// entries are small; holding this across a whole batch is cheaper than a
/// per-file lock and keeps concurrent tag edits from losing each other.
static INDEX_LOCK: Mutex<()> = Mutex::new(());

/// Per-volume answer to "can this file system carry a tag stream?". A negative
/// answer is cached, so a read-only or exFAT volume costs one failed probe for
/// the process, not one per write.
static STREAM_SUPPORT: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The store rooted at the app config directory.
#[derive(Clone, Debug)]
pub struct TagStore {
    config_dir: PathBuf,
}

impl TagStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// The process-wide store. Falls back to an in-memory-free "no config"
    /// reading of defaults when the config directory cannot be resolved: tags
    /// then behave as if none were defined instead of failing navigation.
    pub fn app() -> Option<Self> {
        paths::config_dir().ok().map(Self::new)
    }

    fn definitions_path(&self) -> PathBuf {
        self.config_dir.join(DEFINITIONS_FILE)
    }

    /// The definitions on disk. A missing file is an empty set; an unreadable
    /// or corrupt one is reported and treated as empty rather than taking the
    /// shell down with it.
    pub fn definitions(&self) -> Vec<TagDefinition> {
        match fs::read_to_string(self.definitions_path()) {
            Ok(contents) => match toml::from_str::<DefinitionsFile>(&contents) {
                Ok(file) => file.tags,
                Err(error) => {
                    log::warn!(target: "dae::tags", "tags.toml is not readable: {error}");
                    Vec::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                log::warn!(target: "dae::tags", "could not read tags.toml: {error}");
                Vec::new()
            }
        }
    }

    /// Replaces the definitions. Names are trimmed and de-duplicated
    /// case-insensitively; a duplicate keeps its first spelling. Colours are
    /// normalised to `#rrggbb`; an invalid one gets the palette colour for the
    /// tag's position.
    pub fn save_definitions(
        &self,
        mut definitions: Vec<TagDefinition>,
    ) -> Result<Vec<TagDefinition>, FileSystemError> {
        let mut seen_names: Vec<String> = Vec::new();
        let mut seen_ids: Vec<String> = Vec::new();
        definitions.retain_mut(|definition| {
            let name = definition.name.trim().to_owned();
            let id = definition.id.trim().to_owned();
            if name.is_empty() || id.is_empty() {
                return false;
            }
            let folded = name.to_lowercase();
            if seen_names.contains(&folded) || seen_ids.contains(&id) {
                return false;
            }
            seen_names.push(folded);
            seen_ids.push(id.clone());
            definition.name = name;
            definition.id = id;
            definition.color = normalise_color(&definition.color, seen_ids.len() - 1);
            true
        });
        self.write_definitions(&definitions)?;
        Ok(definitions)
    }

    fn write_definitions(&self, definitions: &[TagDefinition]) -> Result<(), FileSystemError> {
        fs::create_dir_all(&self.config_dir)?;
        let document = DefinitionsFile {
            tags: definitions.to_vec(),
        };
        let contents = toml::to_string_pretty(&document)
            .map_err(|error| FileSystemError::Internal(format!("tags.save_failed: {error}")))?;
        write_atomic(&self.definitions_path(), contents.as_bytes())?;
        Ok(())
    }

    /// Creates one tag and returns it together with the full definition list.
    pub fn create_tag(
        &self,
        name: &str,
        color: &str,
    ) -> Result<(TagDefinition, Vec<TagDefinition>), FileSystemError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "A tag needs a name".to_owned(),
            ));
        }
        if name.chars().count() > 32 {
            return Err(FileSystemError::InvalidInput(
                "A tag name is limited to 32 characters".to_owned(),
            ));
        }
        let mut definitions = self.definitions();
        if definitions
            .iter()
            .any(|definition| definition.name.eq_ignore_ascii_case(name))
        {
            return Err(FileSystemError::AlreadyExists(name.to_owned()));
        }
        let id = unique_id(name, &definitions);
        let definition = TagDefinition {
            id,
            name: name.to_owned(),
            color: normalise_color(color, definitions.len()),
        };
        definitions.push(definition.clone());
        self.write_definitions(&definitions)?;
        Ok((definition, definitions))
    }

    /// Renames or recolours a definition. Unknown ids are refused rather than
    /// appending a definition the caller did not mean to create.
    pub fn update_tag(
        &self,
        id: &str,
        name: &str,
        color: &str,
    ) -> Result<Vec<TagDefinition>, FileSystemError> {
        let name = name.trim();
        let mut definitions = self.definitions();
        let Some(index) = definitions.iter().position(|tag| tag.id == id) else {
            return Err(FileSystemError::NotFound(id.to_owned()));
        };
        if name.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "A tag needs a name".to_owned(),
            ));
        }
        if definitions
            .iter()
            .enumerate()
            .any(|(other, tag)| other != index && tag.name.eq_ignore_ascii_case(name))
        {
            return Err(FileSystemError::AlreadyExists(name.to_owned()));
        }
        definitions[index].name = name.to_owned();
        definitions[index].color = normalise_color(color, index);
        self.write_definitions(&definitions)?;
        Ok(definitions)
    }

    /// Removes a definition and strips its id from every index entry. File
    /// streams are left alone: an id that no longer names a definition is
    /// ignored on read, and rewriting every tagged file to forget it would be
    /// the one unbounded walk this module refuses to make.
    pub fn remove_tag(&self, id: &str) -> Result<Vec<TagDefinition>, FileSystemError> {
        let mut definitions = self.definitions();
        definitions.retain(|tag| tag.id != id);
        self.write_definitions(&definitions)?;

        let _guard = INDEX_LOCK.lock().expect("tag index poisoned");
        for path in self.shard_paths() {
            let Ok(mut shard) = read_shard(&path) else {
                continue;
            };
            let mut changed = false;
            for entry in shard.entries.values_mut() {
                let before = entry.tag_ids.len();
                entry.tag_ids.retain(|tag| tag != id);
                changed |= entry.tag_ids.len() != before;
            }
            if changed && let Err(error) = write_shard(&path, &shard) {
                log::warn!(target: "dae::tags", "could not update {}: {error}", path.display());
            }
        }
        Ok(definitions)
    }

    /// Tags for a batch of paths. The index is consulted first, one shard parse
    /// per volume; a path with no index entry falls back to the stream on the
    /// file, which is what a file copied in from elsewhere carries.
    ///
    /// Two identity checks keep the index honest without a watcher. An entry
    /// whose stored file identity no longer matches the file at that path is
    /// treated as a miss — the name was reused by a different file, which must
    /// not inherit the old tags. A miss whose identity matches an entry under
    /// another path is adopted under the new path: that is a rename made
    /// outside dae, and identity is what survives it.
    pub fn tags_for(&self, paths: &[String]) -> Vec<EntryTags> {
        let mut shard_cache: HashMap<PathBuf, IndexShard> = HashMap::new();
        let mut answers = Vec::with_capacity(paths.len());
        for path in paths {
            let shard_path = shard_path(&self.config_dir, &volume_key(path));
            let shard = shard_cache
                .entry(shard_path.clone())
                .or_insert_with(|| read_shard(&shard_path).unwrap_or_default());
            let key = index_key(path);
            if let Some(entry) = shard.entries.get(&key) {
                let replaced = match (&entry.file_id, file_identity(path)) {
                    (Some(stored), Some(current)) => stored != &current,
                    _ => false,
                };
                if !replaced {
                    answers.push(EntryTags {
                        path: path.clone(),
                        tag_ids: entry.tag_ids.clone(),
                        source: stored_source(path),
                    });
                    continue;
                }
            }
            if let Ok(Some(ids)) = read_stream(path)
                && !ids.is_empty()
            {
                answers.push(EntryTags {
                    path: path.clone(),
                    tag_ids: ids,
                    source: stream_source(),
                });
                continue;
            }
            if let Some(identity) = file_identity(path)
                && let Some((old_key, mut entry)) = shard
                    .entries
                    .iter()
                    .find(|(other, entry)| {
                        *other != &key && entry.file_id.as_deref() == Some(identity.as_str())
                    })
                    .map(|(other, entry)| (other.clone(), entry.clone()))
            {
                shard.entries.remove(&old_key);
                entry.path = path.clone();
                entry.file_id = Some(identity);
                let ids = entry.tag_ids.clone();
                shard.entries.insert(key, entry);
                if let Err(error) = write_shard(&shard_path, shard) {
                    log::warn!(
                        target: "dae::tags",
                        "could not move a renamed entry to {path}: {error}"
                    );
                }
                answers.push(EntryTags {
                    path: path.clone(),
                    tag_ids: ids,
                    source: stored_source(path),
                });
                continue;
            }
            answers.push(EntryTags {
                path: path.clone(),
                tag_ids: Vec::new(),
                source: TagSource::Index,
            });
        }
        answers
    }

    /// Replaces the ids of every path in one batch. Empty `tag_ids` clears both
    /// stores.
    pub fn set_tags(
        &self,
        paths: &[String],
        tag_ids: &[String],
    ) -> Result<TagWriteReport, FileSystemError> {
        let known: Vec<String> = self
            .definitions()
            .into_iter()
            .map(|definition| definition.id)
            .collect();
        let mut ids: Vec<String> = Vec::new();
        for id in tag_ids {
            if !known.iter().any(|known| known == id) {
                return Err(FileSystemError::InvalidInput(format!("Unknown tag: {id}")));
            }
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids.truncate(32);

        let mut report = TagWriteReport::default();
        let _guard = INDEX_LOCK.lock().expect("tag index poisoned");
        let now = now_millis();

        for path in paths {
            if !file_store_possible(path) {
                report.file_store_unavailable = true;
                if self.index_set(path, &ids, now).is_ok() {
                    report.index_only += 1;
                } else {
                    report.failed.push(path.clone());
                }
                continue;
            }

            let stored = if ids.is_empty() {
                // Clearing means the file carries no tag stream at all, not an
                // empty one.
                remove_stream(path)
            } else {
                write_stream(path, &ids)
            };
            match stored {
                Ok(()) => match self.index_set(path, &ids, now) {
                    Ok(()) => report.saved_with_file += 1,
                    Err(error) => {
                        // The file carries the tags even if the index write
                        // failed; the next read finds them through the stream.
                        log::warn!(target: "dae::tags", "index write failed for {path}: {error}");
                        report.saved_with_file += 1;
                    }
                },
                Err(error) => {
                    log::debug!(target: "dae::tags", "stream write failed for {path}: {error}");
                    mark_stream_unsupported(path);
                    report.file_store_unavailable = true;
                    if self.index_set(path, &ids, now).is_ok() {
                        report.index_only += 1;
                    } else {
                        report.failed.push(path.clone());
                    }
                }
            }
        }
        Ok(report)
    }

    fn index_set(&self, path: &str, ids: &[String], now: u64) -> Result<(), FileSystemError> {
        let shard_path = shard_path(&self.config_dir, &volume_key(path));
        let mut shard = read_shard(&shard_path).unwrap_or_default();
        let key = index_key(path);
        if ids.is_empty() {
            shard.entries.remove(&key);
        } else {
            shard.entries.insert(
                key,
                IndexEntry {
                    path: path.to_owned(),
                    tag_ids: ids.to_vec(),
                    updated_at: now,
                    file_id: file_identity(path),
                },
            );
        }
        write_shard(&shard_path, &shard)
    }

    fn shard_paths(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.config_dir.join(INDEX_DIR)) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect()
    }

    /// Every indexed path carrying all of `tag_ids`, in path order.
    pub fn paths_with_tags(&self, tag_ids: &[String]) -> Vec<String> {
        if tag_ids.is_empty() {
            return Vec::new();
        }
        let mut paths: Vec<String> = Vec::new();
        for shard_path in self.shard_paths() {
            let Ok(shard) = read_shard(&shard_path) else {
                continue;
            };
            for entry in shard.entries.values() {
                if tag_ids.iter().all(|id| entry.tag_ids.contains(id)) {
                    paths.push(entry.path.clone());
                }
            }
        }
        paths.sort_unstable();
        paths.dedup();
        paths
    }

    /// How many indexed paths carry each tag, keyed by id. Tags with no files
    /// are simply absent.
    pub fn counts(&self) -> HashMap<String, u64> {
        let mut counts: HashMap<String, u64> = HashMap::new();
        for shard_path in self.shard_paths() {
            let Ok(shard) = read_shard(&shard_path) else {
                continue;
            };
            for entry in shard.entries.values() {
                for id in &entry.tag_ids {
                    *counts.entry(id.clone()).or_default() += 1;
                }
            }
        }
        counts
    }

    /// The definitions as a `tags.toml` document. Export is the same format
    /// the config directory holds, so an export can be dropped in as
    /// `tags.toml`, edited by hand, or imported on another machine.
    pub fn export_document(&self) -> Result<String, FileSystemError> {
        let document = DefinitionsFile {
            tags: self.definitions(),
        };
        toml::to_string_pretty(&document)
            .map_err(|error| FileSystemError::Internal(format!("tags.export_failed: {error}")))
    }

    /// Writes the definitions document to `path`, atomically.
    pub fn export_to(&self, path: &Path) -> Result<(), FileSystemError> {
        write_atomic(path, self.export_document()?.as_bytes())
    }

    /// Merges a `tags.toml` document into the local definitions. A known id
    /// overwrites name and colour; an unknown id is appended. A name another
    /// id already owns is refused rather than renamed — the caller sees it in
    /// `skipped` and nothing is guessed.
    pub fn import_document(&self, document: &str) -> Result<TagImportReport, FileSystemError> {
        let incoming: DefinitionsFile = toml::from_str(document).map_err(|error| {
            FileSystemError::InvalidInput(format!("tags.import_invalid: {error}"))
        })?;
        let mut definitions = self.definitions();
        let mut report = TagImportReport::default();
        for tag in incoming.tags {
            let id = tag.id.trim();
            let name = tag.name.trim();
            if id.is_empty() || name.is_empty() {
                report
                    .skipped
                    .push(format!("{:?}: empty id or name", tag.id));
                continue;
            }
            if name.chars().count() > 32 {
                report
                    .skipped
                    .push(format!("{id}: name over 32 characters"));
                continue;
            }
            if definitions
                .iter()
                .any(|other| other.id != id && other.name.eq_ignore_ascii_case(name))
            {
                report
                    .skipped
                    .push(format!("{id}: the name {name} belongs to another tag"));
                continue;
            }
            match definitions.iter().position(|other| other.id == id) {
                Some(index) => {
                    definitions[index].name = name.to_owned();
                    definitions[index].color = normalise_color(&tag.color, index);
                    report.updated += 1;
                }
                None => {
                    let color = normalise_color(&tag.color, definitions.len());
                    definitions.push(TagDefinition {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        color,
                    });
                    report.added += 1;
                }
            }
        }
        self.write_definitions(&definitions)?;
        Ok(report)
    }

    /// Reads and merges a `tags.toml` document from `path`.
    pub fn import_from(&self, path: &Path) -> Result<TagImportReport, FileSystemError> {
        let document = fs::read_to_string(path)?;
        self.import_document(&document)
    }

    /// Reconciles the index with the files it names. Only paths the index
    /// already mentions are visited: a rebuild is not a disk walk, so it can
    /// never cost more than the index itself.
    ///
    /// It fixes what the read path cannot: entries for deleted files, entries
    /// left under an old key, entries whose ids disagree with the file's own
    /// stream, and shard documents that no longer parse (moved aside as
    /// `*.corrupt-<millis>`, never deleted).
    pub fn repair_index(&self) -> Result<IndexRepairReport, FileSystemError> {
        let _guard = INDEX_LOCK.lock().expect("tag index poisoned");
        let shard_files = self.shard_paths();
        let mut report = IndexRepairReport::default();
        let mut groups: BTreeMap<String, IndexShard> = BTreeMap::new();

        for shard_file in &shard_files {
            let shard = match read_shard(shard_file) {
                Ok(shard) => shard,
                Err(error) => {
                    log::warn!(
                        target: "dae::tags",
                        "index shard {} does not parse: {error}",
                        shard_file.display()
                    );
                    report.corrupt_shards += 1;
                    let name = shard_file
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(INDEX_DIR);
                    let quarantine =
                        shard_file.with_file_name(format!("{name}.corrupt-{}", now_millis()));
                    if let Err(error) = fs::rename(shard_file, &quarantine) {
                        log::warn!(
                            target: "dae::tags",
                            "could not quarantine {}: {error}",
                            shard_file.display()
                        );
                    }
                    continue;
                }
            };
            for entry in shard.entries.into_values() {
                let mut entry = entry;
                let path = entry.path.trim().to_owned();
                if path.is_empty() || !Path::new(&path).exists() {
                    report.dropped_missing += 1;
                    continue;
                }
                entry.file_id = file_identity(&path);
                // The file's own stream is the truth when it carries ids; the
                // index may be stale after a write from another machine.
                if let Ok(Some(ids)) = read_stream(&path)
                    && !ids.is_empty()
                    && ids != entry.tag_ids
                {
                    entry.tag_ids = ids;
                    report.adopted_from_file += 1;
                }
                let volume = volume_key(&path);
                let key = index_key(&path);
                let group = groups.entry(volume).or_default();
                let keep_new = group
                    .entries
                    .get(&key)
                    .is_none_or(|existing| existing.updated_at <= entry.updated_at);
                if keep_new {
                    group.entries.insert(key, entry);
                }
            }
        }

        // The same file identity under two keys means a rename was recorded
        // both ways; keep the newest entry.
        for shard in groups.values_mut() {
            let mut by_identity: HashMap<String, String> = HashMap::new();
            let mut losers: Vec<String> = Vec::new();
            let keys: Vec<String> = shard.entries.keys().cloned().collect();
            for key in keys {
                let Some(entry) = shard.entries.get(&key) else {
                    continue;
                };
                let Some(identity) = entry.file_id.clone() else {
                    continue;
                };
                let Some(owner) = by_identity.get(&identity).cloned() else {
                    by_identity.insert(identity, key);
                    continue;
                };
                let owner_newer = shard
                    .entries
                    .get(&owner)
                    .is_some_and(|owner| owner.updated_at >= entry.updated_at);
                if owner_newer {
                    losers.push(key);
                } else {
                    losers.push(owner);
                    by_identity.insert(identity, key);
                }
            }
            for key in losers {
                if shard.entries.remove(&key).is_some() {
                    report.dropped_duplicate += 1;
                }
            }
        }

        let mut written: Vec<PathBuf> = Vec::new();
        for (volume, shard) in &groups {
            if shard.entries.is_empty() {
                continue;
            }
            report.entries += shard.entries.len();
            let path = shard_path(&self.config_dir, volume);
            write_shard(&path, shard)?;
            written.push(path);
        }
        // Shards whose entries moved to another volume's file, and the ones
        // left empty, are removed; a quarantined file is already renamed.
        for stale in shard_files {
            if !written.contains(&stale) {
                let _ = fs::remove_file(&stale);
            }
        }
        Ok(report)
    }

    /// Moves index entries after a rename, including everything under a
    /// renamed directory. Called by the rename paths once the backend has
    /// reported success; a path with no index entry costs one shard read and
    /// nothing else. Renames made outside dae are recovered by
    /// [`Self::tags_for`] through the stored file identity instead.
    pub fn migrate(&self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let from_key = index_key(from);
        let to_key = index_key(to);
        if from_key == to_key {
            return;
        }
        let _guard = INDEX_LOCK.lock().expect("tag index poisoned");

        let from_shard_path = shard_path(&self.config_dir, &volume_key(from));
        let to_shard_path = shard_path(&self.config_dir, &volume_key(to));
        let Ok(mut from_shard) = read_shard(&from_shard_path) else {
            return;
        };

        let mut moved: Vec<(String, IndexEntry)> = Vec::new();
        if let Some(entry) = from_shard.entries.remove(&from_key) {
            moved.push((to_key.clone(), entry));
        }
        let separator = std::path::MAIN_SEPARATOR;
        let from_prefix = format!("{from_key}{separator}");
        let to_prefix = format!("{to_key}{separator}");
        let keys: Vec<String> = from_shard.entries.keys().cloned().collect();
        for key in keys {
            if !key.starts_with(&from_prefix) {
                continue;
            }
            let Some(entry) = from_shard.entries.get(&key) else {
                continue;
            };
            // The stored spelling keeps the user's capitals; the folded key is
            // the fallback when the two differ in case on a case-insensitive
            // volume.
            let renamed = match entry.path.strip_prefix(from) {
                Some(suffix) => format!("{to}{suffix}"),
                None => format!("{to}{separator}{}", &key[from_prefix.len()..]),
            };
            let mut entry = from_shard
                .entries
                .remove(&key)
                .expect("the entry was just read");
            entry.path = renamed;
            moved.push((format!("{to_prefix}{}", &key[from_prefix.len()..]), entry));
        }
        if moved.is_empty() {
            return;
        }

        if from_shard_path == to_shard_path {
            for (key, mut entry) in moved {
                entry.file_id = file_identity(&entry.path);
                from_shard.entries.insert(key, entry);
            }
            if let Err(error) = write_shard(&from_shard_path, &from_shard) {
                log::warn!(
                    target: "dae::tags",
                    "could not move tags for {from}: {error}"
                );
            }
            return;
        }

        // A move across volumes: the entry changes shard documents. A leftover
        // entry at the destination (the path was deleted and recreated) keeps
        // both sets of ids.
        let mut to_shard = read_shard(&to_shard_path).unwrap_or_default();
        for (key, mut entry) in moved {
            entry.file_id = file_identity(&entry.path);
            if let Some(existing) = to_shard.entries.remove(&key) {
                for id in existing.tag_ids {
                    if !entry.tag_ids.contains(&id) {
                        entry.tag_ids.push(id);
                    }
                }
            }
            to_shard.entries.insert(key, entry);
        }
        if let Err(error) = write_shard(&to_shard_path, &to_shard) {
            log::warn!(
                target: "dae::tags",
                "could not write the destination shard for {to}: {error}"
            );
        }
        if from_shard.entries.is_empty() {
            let _ = fs::remove_file(&from_shard_path);
        } else if let Err(error) = write_shard(&from_shard_path, &from_shard) {
            log::warn!(
                target: "dae::tags",
                "could not write the source shard for {from}: {error}"
            );
        }
    }
}

/// Process-wide convenience: definitions at the app's config directory.
pub fn definitions() -> Vec<TagDefinition> {
    TagStore::app()
        .map(|store| store.definitions())
        .unwrap_or_default()
}

pub fn tags_for(paths: &[String]) -> Vec<EntryTags> {
    TagStore::app()
        .map(|store| store.tags_for(paths))
        .unwrap_or_else(|| {
            paths
                .iter()
                .map(|path| EntryTags {
                    path: path.clone(),
                    tag_ids: Vec::new(),
                    source: TagSource::Index,
                })
                .collect()
        })
}

pub fn paths_with_tags(tag_ids: &[String]) -> Vec<String> {
    TagStore::app()
        .map(|store| store.paths_with_tags(tag_ids))
        .unwrap_or_default()
}

pub fn counts() -> HashMap<String, u64> {
    TagStore::app()
        .map(|store| store.counts())
        .unwrap_or_default()
}

/// Moves index entries after a rename made through dae. Safe to call for any
/// path: one with no index entry does nothing. Callers are the rename paths in
/// `fs::service` and `fs::undo`, after the backend reported success.
pub fn note_rename(from: &str, to: &str) {
    if let Some(store) = TagStore::app() {
        store.migrate(from, to);
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// `#rrggbb` or the palette colour at `position`. Written as a function rather
/// than a regex so a hand-edited file cannot smuggle anything else into QML's
/// colour parser.
fn normalise_color(value: &str, position: usize) -> String {
    const PALETTE: [&str; 8] = [
        "#4c8dff", "#e0524d", "#e5a13a", "#3fa66b", "#8b6cf0", "#d95fa3", "#2aa9b8", "#8a94a6",
    ];
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() == 6 && hex.chars().all(|character| character.is_ascii_hexdigit()) {
        return format!("#{}", hex.to_ascii_lowercase());
    }
    PALETTE[position % PALETTE.len()].to_owned()
}

/// A slug for the id, falling back to a counter when the name has no ASCII
/// word characters (which is the normal case for a Chinese tag name).
fn unique_id(name: &str, existing: &[TagDefinition]) -> String {
    let slug: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_owned();
    let base = if slug.is_empty() {
        "tag".to_owned()
    } else {
        slug
    };
    if !existing.iter().any(|tag| tag.id == base) {
        return base;
    }
    let mut counter = 2;
    loop {
        let candidate = format!("{base}-{counter}");
        if !existing.iter().any(|tag| tag.id == candidate) {
            return candidate;
        }
        counter += 1;
    }
}

/// The folded key an index entry is stored under: case-insensitive on the
/// platforms whose file systems are (Windows, macOS), verbatim elsewhere.
fn index_key(path: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

/// The shard a path belongs to: one per volume or remote host, so a single
/// document never grows with the whole disk and a corrupt one loses one
/// volume's tags rather than all of them.
fn volume_key(path: &str) -> String {
    if let Ok((scheme, _)) = crate::fs::vfs::split_scheme(path)
        && scheme != crate::fs::vfs::Scheme::Local
    {
        let host = path
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        return format!("{}://{host}", scheme_name(scheme));
    }

    #[cfg(windows)]
    {
        let drive: String = path.chars().take(2).collect();
        return drive.to_lowercase();
    }
    #[cfg(not(windows))]
    {
        "/".to_owned()
    }
}

fn scheme_name(scheme: crate::fs::vfs::Scheme) -> &'static str {
    use crate::fs::vfs::Scheme;
    match scheme {
        Scheme::Local => "local",
        Scheme::Smb => "smb",
        Scheme::Sftp => "sftp",
        Scheme::Ftp => "ftp",
        Scheme::WebDav => "webdav",
        Scheme::Cloud => "cloud",
    }
}

/// A shard file name that is stable across runs and filesystem-safe.
fn shard_path(config_dir: &Path, volume: &str) -> PathBuf {
    let prefix: String = volume
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .take(24)
        .collect();
    config_dir
        .join(INDEX_DIR)
        .join(format!("{prefix}-{:016x}.json", fnv1a(volume.as_bytes())))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn read_shard(path: &Path) -> Result<IndexShard, FileSystemError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| FileSystemError::Internal(format!("tag index corrupt: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(IndexShard::default()),
        Err(error) => Err(error.into()),
    }
}

fn write_shard(path: &Path, shard: &IndexShard) -> Result<(), FileSystemError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(shard)
        .map_err(|error| FileSystemError::Internal(format!("tag index encode: {error}")))?;
    write_atomic(path, &bytes)
}

/// Windows: volume serial number plus the file index the file system assigns,
/// which is stable across a rename within the volume — the same pair
/// `BY_HANDLE_FILE_INFORMATION` exposes. Unix: `dev`+`ino`.
fn file_identity(path: &str) -> Option<String> {
    if !crate::fs::vfs::is_local_path(path) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).ok()?;
        return Some(format!("{}:{}", metadata.dev(), metadata.ino()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
        };

        // `FILE_FLAG_BACKUP_SEMANTICS` is what lets the open name a directory:
        // a renamed folder moves its children's index entries too, so its
        // identity is as useful as a file's.
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
        Some(format!(
            "{}:{:08x}{:08x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn file_store_possible(path: &str) -> bool {
    if !crate::fs::vfs::is_local_path(path) {
        return false;
    }
    let Some(parent) = Path::new(path).parent() else {
        return false;
    };
    stream_supported(parent)
}

fn mark_stream_unsupported(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        let key = volume_key(&parent.to_string_lossy());
        if let Ok(mut cache) = STREAM_SUPPORT.lock() {
            cache.insert(key, false);
        }
    }
}

/// Probes a volume once by creating a temporary file in `directory` and
/// writing the tag stream to it. A directory that is not writable, or a file
/// system without ADS/xattr support, answers `false` and is remembered.
fn stream_supported(directory: &Path) -> bool {
    let key = volume_key(&directory.to_string_lossy());
    if let Ok(cache) = STREAM_SUPPORT.lock()
        && let Some(answer) = cache.get(&key)
    {
        return *answer;
    }

    let probe = directory.join(format!(".dae-tag-probe-{}", std::process::id()));
    let written = fs::write(&probe, b"").is_ok()
        && write_stream(&probe.to_string_lossy(), &["probe".to_owned()]).is_ok();
    let read_back = read_stream(&probe.to_string_lossy()).ok().flatten();
    let supported = written && read_back.is_some_and(|ids| ids == ["probe"]);
    let _ = remove_stream(&probe.to_string_lossy());
    let _ = fs::remove_file(&probe);

    if let Ok(mut cache) = STREAM_SUPPORT.lock() {
        cache.insert(key, supported);
    }
    supported
}

fn stream_source() -> TagSource {
    #[cfg(windows)]
    {
        TagSource::Ads
    }
    #[cfg(not(windows))]
    {
        TagSource::Xattr
    }
}

/// Where an index hit's tags are *also* stored. The index is authoritative,
/// but the UI reports the stronger claim only when the file itself can hold
/// them.
fn stored_source(path: &str) -> TagSource {
    if file_store_possible(path) {
        stream_source()
    } else {
        TagSource::Index
    }
}

/// A stream path is `path + ":name"` on Windows; the colon is the alternate
/// data stream separator and `std::fs` opens it like any other file.
#[cfg(windows)]
fn stream_path(path: &str) -> String {
    format!("{path}:{STREAM_NAME}")
}

fn write_stream(path: &str, ids: &[String]) -> Result<(), FileSystemError> {
    let payload = serde_json::to_vec(ids)
        .map_err(|error| FileSystemError::Internal(format!("tag encode: {error}")))?;
    write_stream_bytes(path, &payload)
}

fn read_stream(path: &str) -> Result<Option<Vec<String>>, FileSystemError> {
    let Some(bytes) = read_stream_bytes(path)? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice(&bytes).ok())
}

fn remove_stream(path: &str) -> Result<(), FileSystemError> {
    remove_stream_bytes(path)
}

#[cfg(windows)]
fn write_stream_bytes(path: &str, bytes: &[u8]) -> Result<(), FileSystemError> {
    fs::write(stream_path(path), bytes)?;
    Ok(())
}

#[cfg(windows)]
fn read_stream_bytes(path: &str) -> Result<Option<Vec<u8>>, FileSystemError> {
    match fs::read(stream_path(path)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn remove_stream_bytes(path: &str) -> Result<(), FileSystemError> {
    match fs::remove_file(stream_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
mod sys {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn name() -> CString {
        CString::new(super::STREAM_NAME).expect("stream name has no NUL")
    }

    pub fn path_cstring(path: &str) -> Option<CString> {
        CString::new(Path::new(path).as_os_str().as_bytes()).ok()
    }

    /// `None` for "the attribute does not exist"; `Err` for anything else.
    pub fn get(path: &CString, name: &CString) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            let size =
                unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
            if size < 0 {
                let error = std::io::Error::last_os_error();
                return match error.raw_os_error() {
                    Some(code) if missing(code) => Ok(None),
                    _ => Err(error),
                };
            }
            let mut buffer = vec![0u8; size as usize];
            let read = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    name.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                // The value grew between the size query and the read.
                if error.raw_os_error() == Some(libc::ERANGE) {
                    continue;
                }
                return match error.raw_os_error() {
                    Some(code) if missing(code) => Ok(None),
                    _ => Err(error),
                };
            }
            buffer.truncate(read as usize);
            return Ok(Some(buffer));
        }
    }

    pub fn set(path: &CString, name: &CString, value: &[u8]) -> std::io::Result<()> {
        let result = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if result < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn remove(path: &CString, name: &CString) -> std::io::Result<()> {
        let result = unsafe { libc::removexattr(path.as_ptr(), name.as_ptr()) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if let Some(code) = error.raw_os_error()
                && missing(code)
            {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn missing(code: i32) -> bool {
        // ENOATTR: the attribute does not exist. macOS does not define
        // ENODATA as a distinct value.
        code == 93
    }

    #[cfg(not(target_os = "macos"))]
    fn missing(code: i32) -> bool {
        code == libc::ENODATA
    }
}

#[cfg(unix)]
fn write_stream_bytes(path: &str, bytes: &[u8]) -> Result<(), FileSystemError> {
    let path = sys::path_cstring(path)
        .ok_or_else(|| FileSystemError::InvalidInput("path is not valid UTF-8".to_owned()))?;
    sys::set(&path, &sys::name(), bytes)?;
    Ok(())
}

#[cfg(unix)]
fn read_stream_bytes(path: &str) -> Result<Option<Vec<u8>>, FileSystemError> {
    let Some(path) = sys::path_cstring(path) else {
        return Ok(None);
    };
    Ok(sys::get(&path, &sys::name())?)
}

#[cfg(unix)]
fn remove_stream_bytes(path: &str) -> Result<(), FileSystemError> {
    let Some(path) = sys::path_cstring(path) else {
        return Ok(());
    };
    sys::remove(&path, &sys::name())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store rooted at a fresh temporary directory, plus one real file to tag.
    /// The global stream-support cache is keyed by volume, so each test also
    /// gets its own volume wording via a distinct config dir; the cache is
    /// intentionally shared with production since a volume's answer does not
    /// change while the process runs.
    fn fixture() -> (tempfile::TempDir, TagStore, PathBuf) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = tempfile::tempdir().expect("temp dir");
        let store = TagStore::new(dir.path().join("config"));
        let file = dir.path().join(format!(
            "note-{}-{}.txt",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&file, b"hello").expect("write file");
        (dir, store, file)
    }

    #[test]
    fn definitions_round_trip_and_reject_duplicates() {
        let (_dir, store, _file) = fixture();
        assert!(store.definitions().is_empty());

        let (work, definitions) = store.create_tag("Work", "#ABCDEF").expect("create");
        assert_eq!(work.id, "work");
        assert_eq!(work.color, "#abcdef");
        assert_eq!(definitions.len(), 1);

        // A Chinese name has no ASCII slug, so the id falls back to `tag`.
        let (important, definitions) = store.create_tag("重要", "").expect("create");
        assert_eq!(important.id, "tag");
        assert_eq!(definitions.len(), 2);

        assert!(
            store.create_tag("work", "#000000").is_err(),
            "case-insensitive"
        );
        let reloaded = TagStore::new(store.config_dir.clone());
        assert_eq!(reloaded.definitions().len(), 2);

        let renamed = reloaded
            .update_tag("work", "Job", "#123456")
            .expect("rename");
        assert_eq!(renamed[0].name, "Job");
        assert_eq!(renamed[0].color, "#123456");
    }

    #[test]
    fn tags_are_written_to_the_file_and_mirrored_into_the_index() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let (home, _) = store.create_tag("Home", "").expect("create");
        let path = file.to_string_lossy().into_owned();

        let report = store
            .set_tags(
                std::slice::from_ref(&path),
                &[work.id.clone(), home.id.clone()],
            )
            .expect("set");
        assert_eq!(report.failed, Vec::<String>::new());
        assert_eq!(
            report.saved_with_file + report.index_only,
            1,
            "one path written somewhere"
        );

        let read = store.tags_for(std::slice::from_ref(&path));
        assert_eq!(read[0].tag_ids, vec![work.id.clone(), home.id.clone()]);

        // The stream itself carries the ids, which is what makes them travel
        // with the file.
        let on_file = read_stream(&path).expect("stream").unwrap_or_default();
        if report.saved_with_file == 1 {
            assert_eq!(on_file, vec![work.id.clone(), home.id.clone()]);
        }

        assert_eq!(
            store.paths_with_tags(&[work.id.clone()]),
            vec![path.clone()]
        );
        assert_eq!(store.counts().get(&work.id), Some(&1));
        assert_eq!(store.counts().get(&home.id), Some(&1));
    }

    #[test]
    fn clearing_the_last_tag_removes_the_stream_and_the_index_entry() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let path = file.to_string_lossy().into_owned();
        store
            .set_tags(std::slice::from_ref(&path), std::slice::from_ref(&work.id))
            .expect("set");
        store.set_tags(&[path.clone()], &[]).expect("clear");

        assert!(store.tags_for(&[path.clone()])[0].tag_ids.is_empty());
        assert!(store.paths_with_tags(&[work.id]).is_empty());
        assert_eq!(read_stream(&path).expect("stream"), None);
    }

    #[test]
    fn a_remote_path_is_index_only_and_says_so() {
        let (_dir, store, _file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let path = "smb://nas.local/share/report.txt".to_owned();

        let report = store
            .set_tags(std::slice::from_ref(&path), &[work.id.clone()])
            .expect("set");
        assert_eq!(report.index_only, 1);
        assert_eq!(report.saved_with_file, 0);
        assert!(report.file_store_unavailable);

        let read = store.tags_for(&[path.clone()]);
        assert_eq!(read[0].source, TagSource::Index);
        assert_eq!(read[0].tag_ids, vec![work.id.clone()]);
    }

    #[test]
    fn an_unknown_tag_id_is_refused() {
        let (_dir, store, file) = fixture();
        let path = file.to_string_lossy().into_owned();
        let error = store
            .set_tags(&[path], &["nope".to_owned()])
            .expect_err("unknown id");
        assert!(matches!(error, FileSystemError::InvalidInput(_)));
    }

    #[test]
    fn removing_a_tag_strips_it_from_the_index() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let path = file.to_string_lossy().into_owned();
        store
            .set_tags(std::slice::from_ref(&path), std::slice::from_ref(&work.id))
            .expect("set");

        store.remove_tag(&work.id).expect("remove");
        assert!(store.definitions().is_empty());
        assert!(store.paths_with_tags(&[work.id]).is_empty());
        // The stream may still name the id; a definition-less id is ignored by
        // the editor, which is the documented behaviour.
        assert!(store.tags_for(&[path])[0].tag_ids.is_empty());
    }

    #[test]
    fn index_keys_fold_case_on_case_insensitive_file_systems() {
        let key = index_key("C:/Users/Me/File.TXT");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(key, key.to_lowercase());
        } else {
            assert_eq!(key, "C:/Users/Me/File.TXT");
        }
    }

    #[test]
    fn shard_names_are_filesystem_safe_and_stable() {
        let config = Path::new("cfg");
        let first = shard_path(config, "smb://nas.local");
        let second = shard_path(config, "smb://nas.local");
        assert_eq!(first, second);
        assert_ne!(first, shard_path(config, "smb://other.local"));
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(".json"));
        assert!(
            name.chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || matches!(character, '-' | '_' | '.'))
        );
    }

    #[test]
    fn colours_are_normalised_or_replaced_from_the_palette() {
        assert_eq!(normalise_color("#A1B2C3", 0), "#a1b2c3");
        assert_eq!(normalise_color("a1b2c3", 0), "#a1b2c3");
        assert_eq!(normalise_color("red", 1), "#e0524d");
        assert_eq!(normalise_color("", 8), "#4c8dff");
    }

    #[test]
    fn export_is_a_tags_document_and_import_merges_it() {
        let (_dir, store, _file) = fixture();
        let (work, _) = store.create_tag("Work", "#112233").expect("create");
        store.create_tag("Home", "").expect("create");

        let document = store.export_document().expect("export");
        let parsed: DefinitionsFile = toml::from_str(&document).expect("export parses");
        assert_eq!(parsed.tags.len(), 2);
        assert!(document.contains("[[tags]]"));

        // `work` is renamed, a new id is appended, `home` tries to take the
        // name the rename just claimed, and an empty id is refused.
        let incoming = format!(
            r##"
[[tags]]
id = "{work_id}"
name = "Job"
color = "#AABBCC"

[[tags]]
id = "urgent"
name = "Urgent"
color = "#123456"

[[tags]]
id = "home"
name = "Job"
color = "#000000"

[[tags]]
id = ""
name = "Nameless"
"##,
            work_id = work.id
        );
        let report = store.import_document(&incoming).expect("import");
        assert_eq!(report.added, 1);
        assert_eq!(report.updated, 1);
        assert_eq!(report.skipped.len(), 2);

        let definitions = store.definitions();
        assert_eq!(definitions.len(), 3);
        let job = definitions
            .iter()
            .find(|tag| tag.id == work.id)
            .expect("known id updated");
        assert_eq!(job.name, "Job");
        assert_eq!(job.color, "#aabbcc");
        assert!(
            definitions
                .iter()
                .any(|tag| tag.id == "urgent" && tag.name == "Urgent"),
            "an unknown id is appended"
        );
        assert!(
            definitions
                .iter()
                .any(|tag| tag.id == "home" && tag.name == "Home"),
            "the refused import did not take the name"
        );
    }

    #[test]
    fn export_and_import_round_trip_through_a_file() {
        let (dir, store, _file) = fixture();
        store.create_tag("Work", "#102030").expect("create");
        let export = dir.path().join("library.tags.toml");
        store.export_to(&export).expect("export");

        let other = TagStore::new(dir.path().join("other-config"));
        let report = other.import_from(&export).expect("import");
        assert_eq!(report.added, 1);
        assert_eq!(report.updated, 0);
        assert!(report.skipped.is_empty());
        assert_eq!(other.definitions().len(), 1);
    }

    #[test]
    fn a_malformed_import_is_refused_without_touching_the_definitions() {
        let (_dir, store, _file) = fixture();
        store.create_tag("Work", "").expect("create");
        let error = store
            .import_document("this is not toml")
            .expect_err("invalid document");
        assert!(matches!(error, FileSystemError::InvalidInput(_)));
        assert_eq!(store.definitions().len(), 1);
    }

    #[test]
    fn an_index_only_tag_follows_a_rename_made_outside_the_app() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let old = file.to_string_lossy().into_owned();
        store
            .set_tags(std::slice::from_ref(&old), std::slice::from_ref(&work.id))
            .expect("set");
        // Drop the stream so the only place the ids can come from is the
        // index — the situation of a volume that cannot carry tags with the
        // file.
        let _ = remove_stream(&old);

        let renamed = file.with_file_name(format!(
            "{}.moved",
            file.file_stem().expect("stem").to_string_lossy()
        ));
        fs::rename(&file, &renamed).expect("rename");
        let new = renamed.to_string_lossy().into_owned();

        let read = store.tags_for(std::slice::from_ref(&new));
        assert_eq!(
            read[0].tag_ids,
            vec![work.id.clone()],
            "identity is what survives the rename"
        );
        // The entry moved rather than being duplicated or left behind.
        assert!(store.tags_for(&[old])[0].tag_ids.is_empty());
        assert_eq!(store.counts().get(&work.id), Some(&1));
    }

    #[test]
    fn a_recreated_path_does_not_inherit_the_old_tags() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let path = file.to_string_lossy().into_owned();
        store
            .set_tags(&[path.clone()], std::slice::from_ref(&work.id))
            .expect("set");
        let before = file_identity(&path);
        fs::remove_file(&file).expect("delete");
        fs::write(&file, b"replacement").expect("recreate");
        if before == file_identity(&path) {
            // The file system handed the same identity to the new file; there
            // is nothing to distinguish and nothing to assert.
            return;
        }
        assert!(
            store.tags_for(&[path])[0].tag_ids.is_empty(),
            "a different file at the same name starts untagged"
        );
    }

    #[test]
    fn repair_drops_missing_files_and_adopts_the_stream() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let (home, _) = store.create_tag("Home", "").expect("create");
        let path = file.to_string_lossy().into_owned();
        let written = store
            .set_tags(&[path.clone()], std::slice::from_ref(&work.id))
            .expect("set");

        if written.saved_with_file > 0 {
            // The index says Work, the file says Home: on rebuild the file
            // wins, because the stream travels with it.
            write_stream(&path, std::slice::from_ref(&home.id)).expect("write stream");
            let report = store.repair_index().expect("repair");
            assert_eq!(report.adopted_from_file, 1);
            assert_eq!(report.entries, 1);
            assert_eq!(
                store.tags_for(&[path.clone()])[0].tag_ids,
                vec![home.id.clone()]
            );
        }

        fs::remove_file(&file).expect("delete");
        let report = store.repair_index().expect("repair");
        assert_eq!(report.dropped_missing, 1);
        assert_eq!(report.entries, 0);
    }

    #[test]
    fn repair_quarantines_a_corrupt_shard_instead_of_deleting_it() {
        let (_dir, store, file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let path = file.to_string_lossy().into_owned();
        store.set_tags(&[path], &[work.id]).expect("set");
        let shard = store
            .shard_paths()
            .into_iter()
            .next()
            .expect("one shard was written");
        fs::write(&shard, b"{ this is not json").expect("corrupt");

        let report = store.repair_index().expect("repair");
        assert_eq!(report.corrupt_shards, 1);
        let quarantined = fs::read_dir(store.config_dir.join(INDEX_DIR))
            .expect("index dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(quarantined, "the corrupt shard is kept for inspection");
        assert!(store.counts().is_empty());
    }

    #[test]
    fn migrating_a_directory_moves_the_entries_beneath_it() {
        let (dir, store, _file) = fixture();
        let (work, _) = store.create_tag("Work", "").expect("create");
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).expect("mkdir");
        let first = folder.join("a.txt");
        let second = folder.join("b.txt");
        fs::write(&first, b"a").expect("write");
        fs::write(&second, b"b").expect("write");
        let paths: Vec<String> = [&first, &second]
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        store
            .set_tags(&paths, std::slice::from_ref(&work.id))
            .expect("set");

        let moved = dir.path().join("moved");
        fs::rename(&folder, &moved).expect("rename");
        store.migrate(&folder.to_string_lossy(), &moved.to_string_lossy());

        for name in ["a.txt", "b.txt"] {
            let path = moved.join(name).to_string_lossy().into_owned();
            assert_eq!(
                store.tags_for(&[path])[0].tag_ids,
                vec![work.id.clone()],
                "{name} kept its tags"
            );
        }
        assert_eq!(store.counts().get(&work.id), Some(&2));
    }
}
