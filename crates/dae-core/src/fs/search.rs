//! Search commands for the explorer: recursive name search and ripgrep-backed
//! content search. Both are local-only; a network backend has no search index
//! and walking it would issue unbounded round trips. A generation counter in
//! [`FileSearchState`] cancels stale traversals when a newer query arrives.

use super::error::FileSystemError;
use super::local;
use super::local::search::rank_and_truncate;
use super::tags;
use super::types::{ContentSearchResponse, EntryKind, SearchEntry, SearchResponse};
use super::vfs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Default)]
pub struct FileSearchState {
    generation: AtomicU64,
}

/// The process-wide generation counter. One app per process, so this is the
/// lifetime the managed state had; tests keep constructing their own.
fn state() -> &'static FileSearchState {
    static STATE: OnceLock<FileSearchState> = OnceLock::new();
    STATE.get_or_init(FileSearchState::default)
}

impl FileSearchState {
    fn begin(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(AtomicOrdering::Acquire) == generation
    }
}

/// The visibility filter a name search applies while it walks.
///
/// The shell translates its user-facing buckets into the two bounds once, so
/// "today" is the local midnight the user means rather than a 24-hour window
/// computed in a different timezone.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchFilters {
    /// `all`, `folders`, `files` or `images` — the same vocabulary the
    /// listing filter uses.
    pub kind: String,
    /// Entries modified before this are skipped. `None` disables the test.
    pub modified_after: Option<u64>,
    /// Inclusive lower bound on file size, in bytes.
    pub size_min: Option<u64>,
    /// Exclusive upper bound on file size, in bytes.
    pub size_max: Option<u64>,
}

impl SearchFilters {
    /// Whether `entry` survives every active bucket. Directories have no size,
    /// so the size bounds only apply to files, exactly like the listing filter.
    pub fn keeps(&self, entry: &SearchEntry) -> bool {
        if self
            .modified_after
            .is_some_and(|cutoff| cutoff > 0 && entry.modified_at.unwrap_or(0) < cutoff)
        {
            return false;
        }

        match self.kind.as_str() {
            "folders" if entry.kind != EntryKind::Directory => return false,
            "files" if entry.kind != EntryKind::File => return false,
            "images" if entry.kind != EntryKind::File || !is_image_name(&entry.name) => {
                return false;
            }
            _ => {}
        }

        if entry.kind == EntryKind::File {
            let size = entry.size.unwrap_or(0);
            if self.size_min.is_some_and(|min| size < min) {
                return false;
            }
            if self.size_max.is_some_and(|max| size >= max) {
                return false;
            }
        }

        true
    }
}

/// One search across one or more roots, optionally narrowed to tags.
///
/// `text` may carry `tag:` tokens (`tag:work`, `tag:work,home`); they are
/// stripped from the name match and resolved against the tag definitions.
/// `tags` is the same filter supplied by a click rather than by typing, so the
/// sidebar and the search field reach one implementation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchRequest {
    pub text: String,
    /// Absolute paths to walk. Every one must be local; a remote root is
    /// rejected rather than silently skipped, because a result set that
    /// quietly lacks a whole volume is worse than an error.
    pub roots: Vec<String>,
    /// Tag ids to require. `tag:` tokens in `text` are added to this.
    pub tags: Vec<String>,
    pub filters: SearchFilters,
}

/// See `commands::search_directory` for the contract; this half is engine-free.
pub async fn search(request: SearchRequest) -> Result<SearchResponse, FileSystemError> {
    let generation = state().begin();

    tokio::task::spawn_blocking(move || {
        let is_current = || state().is_current(generation);
        search_local(request, &is_current)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn search_local(
    request: SearchRequest,
    is_current: &(dyn Fn() -> bool + Send + Sync),
) -> Result<SearchResponse, FileSystemError> {
    for root in &request.roots {
        if !vfs::is_local_path(root) {
            return Err(FileSystemError::InvalidInput("fs.search_local_only".into()));
        }
    }

    let (text, typed_tags) = split_tag_tokens(&request.text);
    let Some(tag_ids) = merge_tag_ids(&request.tags, &typed_tags, &tags::definitions()) else {
        // A tag nobody defined matches nothing; that is an answer, not an
        // error the user has to dismiss.
        return Ok(SearchResponse {
            entries: Vec::new(),
            truncated: false,
        });
    };

    if text.is_empty() && tag_ids.is_empty() {
        return Ok(SearchResponse {
            entries: Vec::new(),
            truncated: false,
        });
    }

    if text.is_empty() {
        return tagged_search(&tag_ids, &request);
    }

    let mut candidates: Vec<SearchEntry> = Vec::new();
    let mut truncated = false;
    for root in &request.roots {
        let backend = vfs::resolve(root)?;
        let response = backend.search(root, &text, &request.filters, is_current)?;
        truncated |= response.truncated;
        candidates.extend(response.entries);
        if !is_current() {
            return Ok(SearchResponse {
                entries: Vec::new(),
                truncated: false,
            });
        }
    }

    if !tag_ids.is_empty() {
        let paths: Vec<String> = candidates.iter().map(|entry| entry.path.clone()).collect();
        let tagged: Vec<Vec<String>> = tags::tags_for(&paths)
            .into_iter()
            .map(|entry| entry.tag_ids)
            .collect();
        let mut tagged = tagged.into_iter();
        candidates.retain(|_| {
            tagged
                .next()
                .is_some_and(|ids| tag_ids.iter().all(|id| ids.contains(id)))
        });
    }

    truncated |= candidates.len() > MAX_RESULTS;
    Ok(SearchResponse {
        entries: rank_and_truncate(candidates, &text.to_lowercase()),
        truncated,
    })
}

/// The results ceiling the shell shows. `local::search` ranks against the same
/// number internally; the merged set is cut here again so several roots cannot
/// multiply it.
const MAX_RESULTS: usize = 200;

/// Tag-only results: the index answers which paths carry the tags, and each
/// path is stat-ed once to fill the columns the result list paints.
fn tagged_search(
    tag_ids: &[String],
    request: &SearchRequest,
) -> Result<SearchResponse, FileSystemError> {
    let mut entries = Vec::new();
    for path in tags::paths_with_tags(tag_ids) {
        let Some(root) = request.roots.iter().find(|root| path_is_under(&path, root)) else {
            continue;
        };
        let backend = vfs::resolve(&path)?;
        let Ok(stat) = backend.stat(&path) else {
            // The index outlived the file; a tag search reports what exists.
            continue;
        };
        let name = super::types::display_name_from_path(&path);
        let relative = path
            .strip_prefix(root.as_str())
            .map(|rest| rest.trim_start_matches(['/', '\\']))
            .unwrap_or(path.as_str())
            .to_owned();
        let entry = SearchEntry {
            name,
            path: path.clone(),
            relative_path: relative,
            kind: stat.kind,
            modified_at: stat.modified_at,
            size: (stat.kind == EntryKind::File).then_some(stat.size),
            hidden: false,
            read_only: false,
        };
        if request.filters.keeps(&entry) {
            entries.push(entry);
        }
    }

    let truncated = entries.len() > MAX_RESULTS;
    Ok(SearchResponse {
        entries: rank_and_truncate(entries, ""),
        truncated,
    })
}

fn path_is_under(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches(['/', '\\']);
    path.strip_prefix(root)
        .is_some_and(|rest| rest.starts_with(['/', '\\']))
        || path == root
}

/// Splits `tag:` tokens out of the query. Both colons are accepted: a CJK
/// keyboard's full-width colon is what a user typing Chinese actually enters.
/// Combines the ids a click supplied with the names the query typed. `None`
/// means a typed name names no definition; every requested id has to exist for
/// the filter to mean what the user asked.
fn merge_tag_ids(
    requested: &[String],
    typed_names: &[String],
    definitions: &[tags::TagDefinition],
) -> Option<Vec<String>> {
    let mut ids = requested.to_vec();
    for name in typed_names {
        let definition = definitions.iter().find(|definition| {
            definition.id == *name || definition.name.eq_ignore_ascii_case(name)
        })?;
        if !ids.contains(&definition.id) {
            ids.push(definition.id.clone());
        }
    }
    Some(ids)
}

fn split_tag_tokens(text: &str) -> (String, Vec<String>) {
    let mut words: Vec<&str> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let stripped = token
            .strip_prefix("tag:")
            .or_else(|| token.strip_prefix("tag："));
        match stripped {
            Some(list) => {
                for name in list.split(',') {
                    let name = name.trim();
                    if !name.is_empty() && !tags.iter().any(|tag| tag == name) {
                        tags.push(name.to_owned());
                    }
                }
            }
            None => words.push(token),
        }
    }
    (words.join(" "), tags)
}

fn is_image_name(name: &str) -> bool {
    const IMAGE: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "heic", "avif",
    ];
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    extension.is_some_and(|extension| {
        IMAGE
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    })
}

/// See `commands::cancel_search` for the contract; this half is engine-free.
pub fn cancel_search() {
    state().cancel();
}

/// See `commands::search_file_contents` for the contract; this half is engine-free.
pub async fn search_file_contents(
    path: String,
    query: String,
    is_regex: bool,
    case_sensitive: bool,
    file_filter: Option<String>,
) -> Result<ContentSearchResponse, FileSystemError> {
    if !vfs::is_local_path(&path) {
        return Err(FileSystemError::InvalidInput(
            "fs.content_search_local_only".into(),
        ));
    }

    let generation = state().begin();

    tokio::task::spawn_blocking(move || {
        let is_current = move || state().is_current(generation);
        local::search_file_contents_sync(
            PathBuf::from(path),
            &local::ContentSearchParams {
                query: &query,
                is_regex,
                case_sensitive,
                file_filter: file_filter.as_deref(),
            },
            &is_current,
        )
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn name_search_rejects_network_paths() {
        for path in [
            "smb://nas/media",
            "sftp://user@nas:22/home",
            "cloud://google_drive:me@example.com/folder-id",
            "ftp://nas/share",
            "webdavs://cloud.example/dav",
        ] {
            let error = search(SearchRequest {
                text: "resize".to_owned(),
                roots: vec![path.to_owned()],
                ..SearchRequest::default()
            })
            .await
            .expect_err("remote name search must be rejected");
            assert!(
                matches!(
                    &error,
                    FileSystemError::InvalidInput(message)
                        if message == "fs.search_local_only"
                ),
                "{path}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn content_search_keeps_its_own_rejection() {
        let error = search_file_contents(
            "smb://nas/media".to_owned(),
            "resize".to_owned(),
            false,
            false,
            None,
        )
        .await
        .expect_err("remote content search must be rejected");
        assert!(
            matches!(
                &error,
                FileSystemError::InvalidInput(message)
                    if message == "fs.content_search_local_only"
            ),
            "{error}"
        );
    }

    use crate::fs::tags::TagDefinition;

    fn definition(id: &str, name: &str) -> TagDefinition {
        TagDefinition {
            id: id.to_owned(),
            name: name.to_owned(),
            color: "#4c8dff".to_owned(),
        }
    }

    fn entry(kind: EntryKind, name: &str, size: Option<u64>, modified: Option<u64>) -> SearchEntry {
        SearchEntry {
            name: name.to_owned(),
            path: format!("C:/root/{name}"),
            relative_path: name.to_owned(),
            kind,
            modified_at: modified,
            size,
            hidden: false,
            read_only: false,
        }
    }

    #[test]
    fn tag_tokens_are_split_from_the_name_and_accept_both_colons() {
        let (text, tags) = split_tag_tokens("report tag:work,home tag：重要");
        assert_eq!(text, "report");
        assert_eq!(tags, vec!["work", "home", "重要"]);

        let (text, tags) = split_tag_tokens("tag:work");
        assert_eq!(text, "");
        assert_eq!(tags, vec!["work"]);

        let (text, tags) = split_tag_tokens("  quarterly report ");
        assert_eq!(text, "quarterly report");
        assert!(tags.is_empty());
    }

    #[test]
    fn typed_tag_names_resolve_to_ids_and_unknown_names_match_nothing() {
        let definitions = [definition("work", "Work"), definition("home", "Home")];
        let merged = merge_tag_ids(&[], &["Home".to_owned()], &definitions).expect("resolved");
        assert_eq!(merged, vec!["home"]);

        let merged = merge_tag_ids(&["work".to_owned()], &["work".to_owned()], &definitions)
            .expect("resolved");
        assert_eq!(merged, vec!["work"], "an id supplied twice is one filter");

        assert!(merge_tag_ids(&[], &["nope".to_owned()], &definitions).is_none());
    }

    #[test]
    fn filters_apply_kind_date_and_size_the_way_the_listing_does() {
        let filters = SearchFilters {
            kind: "images".to_owned(),
            modified_after: Some(1_000),
            size_min: Some(10),
            size_max: Some(100),
            ..SearchFilters::default()
        };
        assert!(filters.keeps(&entry(EntryKind::File, "a.PNG", Some(50), Some(2_000))));
        assert!(!filters.keeps(&entry(EntryKind::File, "a.txt", Some(50), Some(2_000))));
        assert!(!filters.keeps(&entry(EntryKind::File, "a.png", Some(5), Some(2_000))));
        assert!(!filters.keeps(&entry(EntryKind::File, "a.png", Some(500), Some(2_000))));
        assert!(!filters.keeps(&entry(EntryKind::File, "a.png", Some(50), Some(999))));

        let folders = SearchFilters {
            kind: "folders".to_owned(),
            ..SearchFilters::default()
        };
        assert!(folders.keeps(&entry(EntryKind::Directory, "src", None, None)));
        assert!(!folders.keeps(&entry(EntryKind::File, "src", None, None)));
    }

    #[test]
    fn tag_only_roots_match_at_a_path_boundary() {
        assert!(path_is_under(r"C:\code\dae\a.txt", r"C:\code\dae"));
        assert!(path_is_under("smb://nas/share/a.txt", "smb://nas/share"));
        assert!(!path_is_under(r"C:\code\dae-other\a.txt", r"C:\code\dae"));
    }
}
