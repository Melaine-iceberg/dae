//! Cloud storage backend. One `CloudBackend` serves one authorized account;
//! path segments after the account are opaque provider entry ids, so every
//! name-facing operation resolves ids back to real display names.

pub mod accounts;
mod drive;
mod dropbox;
pub mod oauth;
mod onedrive;
pub mod provider;

use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    Breadcrumb, DirectoryEntry, DirectoryView, EntryKind, EntryStat, NewEntryKind, SearchEntry,
    SearchResponse, entry_sort_key,
};
use crate::file_system::vfs::{FileSystemBackend, SharedBackend};
use provider::{CloudMeta, CloudProviderKind};
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Listings stay fresh long enough that the directory watcher's 4-second
/// poll sees the same snapshot instead of hitting the API every tick.
const LIST_TTL: Duration = Duration::from_secs(5);
const MAX_SEARCH_RESULTS: usize = 200;
/// A breadcrumb crumb costs one metadata request on a cold cache; cap the
/// chain so a pathological depth cannot stack requests unboundedly.
const MAX_BREADCRUMB_DEPTH: usize = 64;

static STAGED_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One parsed `cloud://provider:email/segment/...` path.
struct CloudPath {
    /// `cloud://provider:email` — the account root path and registry key.
    account_id: String,
    /// Entry ids below the account root (the last may be a display name in
    /// engine-generated targets).
    segments: Vec<String>,
}

/// Parses `provider:email[/id/...]` (the part after `cloud://`). The split
/// is on the FIRST colon: emails never contain colons, while an rsplit would
/// mistake a domain suffix for a port.
fn parse_cloud_path(rest: &str) -> Result<CloudPath, FileSystemError> {
    let (authority, tail) = match rest.split_once('/') {
        Some((authority, tail)) => (authority, tail),
        None => (rest, ""),
    };
    let (provider, email) = authority.split_once(':').ok_or_else(|| {
        FileSystemError::InvalidInput(format!("Malformed cloud account path: cloud://{rest}"))
    })?;
    let kind = CloudProviderKind::parse(provider)?;
    if email.is_empty() {
        return Err(FileSystemError::InvalidInput(format!(
            "The cloud account has no email: {authority}"
        )));
    }

    Ok(CloudPath {
        account_id: format!("cloud://{}:{email}", kind.as_str()),
        segments: tail
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

fn canonical_path(parsed: &CloudPath) -> String {
    if parsed.segments.is_empty() {
        parsed.account_id.clone()
    } else {
        format!("{}/{}", parsed.account_id, parsed.segments.join("/"))
    }
}

/// The id of the folder holding the path's last segment ("" = account root).
fn parent_of(parsed: &CloudPath) -> String {
    parsed
        .segments
        .iter()
        .rev()
        .nth(1)
        .cloned()
        .unwrap_or_default()
}

/// Opens (or reuses) the backend for `cloud://...` path text. Called by
/// `vfs::resolve`.
pub fn open_backend(rest: &str) -> Result<SharedBackend, FileSystemError> {
    let parsed = parse_cloud_path(rest)?;

    if let Some(existing) = accounts::session_for(&parsed.account_id) {
        return Ok(existing);
    }

    let account = accounts::account_by_id(&parsed.account_id).ok_or_else(|| {
        FileSystemError::NotFound(format!(
            "No cloud account is connected for: {}",
            parsed.account_id
        ))
    })?;
    let material = accounts::token_material(&parsed.account_id)?;

    let backend: SharedBackend = Arc::new(CloudBackend::connect(account, material)?);
    accounts::store_session(&parsed.account_id, Arc::clone(&backend));
    Ok(backend)
}

/// The explorer-facing backend for one cloud account. Cheap to clone: all
/// state lives behind an `Arc`.
pub struct CloudBackend(Arc<CloudInner>);

struct CloudInner {
    account: accounts::StoredCloudAccount,
    provider: Arc<dyn provider::CloudProvider>,
    client: reqwest::Client,
    runtime: Arc<tokio::runtime::Runtime>,
    state: Mutex<TokenState>,
}

struct TokenState {
    access_token: String,
    refresh_token: String,
    client_id: String,
    client_secret: Option<String>,
    /// `None` until the first refresh has minted an access token.
    expires_at: Option<Instant>,
    /// Entry id → metadata, warmed by listings and metadata lookups.
    metas: HashMap<String, CloudMeta>,
    /// Folder id → (fetched at, children), short-lived for the watcher.
    listings: HashMap<String, (Instant, Arc<Vec<CloudMeta>>)>,
}

impl CloudBackend {
    fn connect(
        account: accounts::StoredCloudAccount,
        material: accounts::TokenMaterial,
    ) -> Result<Self, FileSystemError> {
        let kind = account.provider;
        let runtime = Arc::new(
            tokio::runtime::Runtime::new()
                .map_err(|error| FileSystemError::Io(error.to_string()))?,
        );
        Ok(Self(Arc::new(CloudInner {
            account,
            provider: provider::provider_for(kind),
            client: provider::http_client()?,
            runtime,
            state: Mutex::new(TokenState {
                access_token: String::new(),
                refresh_token: material.refresh_token,
                client_id: material.client_id,
                client_secret: material.client_secret,
                expires_at: None,
                metas: HashMap::new(),
                listings: HashMap::new(),
            }),
        })))
    }
}

impl CloudInner {
    fn lock_state(&self) -> MutexGuard<'_, TokenState> {
        self.state.lock().expect("cloud backend state poisoned")
    }

    fn parse_and_verify(&self, path: &str) -> Result<CloudPath, FileSystemError> {
        let parsed = parse_cloud_path(path)?;
        if parsed.account_id != self.account.id {
            return Err(FileSystemError::InvalidInput(format!(
                "The path belongs to a different cloud account: {path}"
            )));
        }
        Ok(parsed)
    }

    /// A valid access token, refreshing (and persisting rotated refresh
    /// tokens) as needed. Holds the state lock over the network call so two
    /// concurrent operations cannot race into a double refresh.
    fn access_token(&self, force: bool) -> Result<String, FileSystemError> {
        let mut state = self.lock_state();
        if !force && state.expires_at.is_some_and(|expires| Instant::now() < expires) {
            return Ok(state.access_token.clone());
        }

        let tokens = self.runtime.block_on(self.provider.refresh_token(
            &self.client,
            &state.client_id,
            state.client_secret.as_deref(),
            &state.refresh_token,
        ))?;

        if let Some(rotated) = tokens.refresh_token {
            state.refresh_token = rotated.clone();
            accounts::save_refresh_token(&self.account.id, &rotated);
        }
        state.access_token = tokens.access_token.clone();
        // Refresh a little before the announced expiry so requests already in
        // flight stay valid.
        state.expires_at = Some(
            Instant::now() + Duration::from_secs(tokens.expires_in_secs.saturating_sub(30)),
        );
        Ok(tokens.access_token)
    }

    /// Runs one provider call with a fresh token, retrying exactly once when
    /// the provider reports an expired token (401).
    fn call<T, F, Fut>(&self, make: F) -> Result<T, FileSystemError>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<T, FileSystemError>>,
    {
        let token = self.access_token(false)?;
        let result = self.runtime.block_on(make(token));
        if let Err(error) = &result
            && provider::is_token_error(error)
        {
            let token = self.access_token(true)?;
            return self.runtime.block_on(make(token));
        }
        result
    }

    fn root_meta(&self) -> CloudMeta {
        CloudMeta {
            id: String::new(),
            name: self.account.display_name.clone(),
            kind: EntryKind::Directory,
            size: 0,
            modified_at: None,
            parent_id: None,
            path: None,
        }
    }

    /// Metadata for one entry id, from cache or the provider. `""` is the
    /// synthesized account root, which providers cannot look up.
    fn metadata(&self, id: &str) -> Result<CloudMeta, FileSystemError> {
        if id.is_empty() {
            return Ok(self.root_meta());
        }
        {
            let state = self.lock_state();
            if let Some(meta) = state.metas.get(id) {
                return Ok(meta.clone());
            }
        }

        let meta = self.call(|token| {
            let provider = Arc::clone(&self.provider);
            let client = self.client.clone();
            let id = id.to_owned();
            async move { provider.metadata(&client, &token, &id).await }
        })?;

        self.lock_state().metas.insert(id.to_owned(), meta.clone());
        Ok(meta)
    }

    /// Children of a folder ("" = account root), cached briefly.
    fn list_folder(&self, folder_id: &str) -> Result<Arc<Vec<CloudMeta>>, FileSystemError> {
        {
            let state = self.lock_state();
            if let Some((fetched, entries)) = state.listings.get(folder_id)
                && fetched.elapsed() < LIST_TTL
            {
                return Ok(Arc::clone(entries));
            }
        }

        let folder = folder_id.to_owned();
        let entries = self.call(|token| {
            let provider = Arc::clone(&self.provider);
            let client = self.client.clone();
            let folder = folder.clone();
            async move { provider.list(&client, &token, &folder).await }
        })?;

        let mut state = self.lock_state();
        for child in &entries {
            state.metas.insert(child.id.clone(), child.clone());
        }
        let shared = Arc::new(entries);
        state
            .listings
            .insert(folder_id.to_owned(), (Instant::now(), Arc::clone(&shared)));
        Ok(shared)
    }

    /// Resolves the final segment of `path`. Ids resolve through the
    /// metadata cache/API; an unknown last segment falls back to a
    /// case-insensitive sibling-name match, because the transfer engine
    /// builds cross-backend destinations as `destination/源文件名`.
    fn resolve_entry(&self, path: &str) -> Result<CloudMeta, FileSystemError> {
        let parsed = self.parse_and_verify(path)?;
        let Some(last) = parsed.segments.last() else {
            return Ok(self.root_meta());
        };

        match self.metadata(last) {
            Ok(meta) => return Ok(meta),
            Err(FileSystemError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }

        let siblings = self.list_folder(&parent_of(&parsed))?;
        if let Some(child) = siblings
            .iter()
            .find(|child| child.name.eq_ignore_ascii_case(last))
        {
            return Ok(child.clone());
        }
        Err(FileSystemError::NotFound(path.to_owned()))
    }

    /// `(parent_id, name)` for the path's last segment; the segment may be
    /// an id of an existing entry or a not-yet-created display name.
    fn split_target(&self, path: &str) -> Result<(String, String), FileSystemError> {
        let parsed = self.parse_and_verify(path)?;
        let Some(last) = parsed.segments.last() else {
            return Err(FileSystemError::InvalidInput(format!(
                "Not an entry path: {path}"
            )));
        };
        let name = match self.metadata(last) {
            Ok(meta) => meta.name,
            Err(FileSystemError::NotFound(_)) => last.clone(),
            Err(error) => return Err(error),
        };
        Ok((parent_of(&parsed), name))
    }

    /// Account root plus one crumb per path segment, each segment name from
    /// the (warm) metadata cache.
    fn breadcrumbs(&self, parsed: &CloudPath) -> Result<Vec<Breadcrumb>, FileSystemError> {
        let mut crumbs = vec![Breadcrumb {
            name: self.account.display_name.clone(),
            path: self.account.id.clone(),
        }];

        let mut built = self.account.id.clone();
        for (depth, segment) in parsed.segments.iter().enumerate() {
            if depth >= MAX_BREADCRUMB_DEPTH {
                break;
            }
            let meta = self.metadata(segment)?;
            built = format!("{built}/{segment}");
            crumbs.push(Breadcrumb {
                name: meta.name,
                path: built.clone(),
            });
        }
        Ok(crumbs)
    }

    /// Records a created/uploaded entry and drops the parent's listing cache
    /// so the next read sees it.
    fn adopt_meta(&self, parent_id: &str, meta: &CloudMeta) {
        let mut state = self.lock_state();
        state.metas.insert(meta.id.clone(), meta.clone());
        state.listings.remove(parent_id);
    }

    fn upload_file(
        &self,
        parent_id: &str,
        name: &str,
        staged: &Path,
        size: u64,
    ) -> Result<CloudMeta, FileSystemError> {
        let meta = self.call(|token| {
            let provider = Arc::clone(&self.provider);
            let client = self.client.clone();
            let parent = parent_id.to_owned();
            let name = name.to_owned();
            let staged = staged.to_path_buf();
            async move { provider.upload(&client, &token, &parent, &name, &staged, size).await }
        })?;
        self.adopt_meta(parent_id, &meta);
        Ok(meta)
    }
}

impl FileSystemBackend for CloudBackend {
    fn read_dir(&self, path: &str) -> Result<DirectoryView, FileSystemError> {
        let parsed = self.0.parse_and_verify(path)?;
        let folder_id = parsed.segments.last().cloned().unwrap_or_default();

        if !folder_id.is_empty() {
            let meta = self.0.metadata(&folder_id)?;
            if meta.kind != EntryKind::Directory {
                return Err(FileSystemError::NotDirectory(canonical_path(&parsed)));
            }
        }

        let children = self.0.list_folder(&folder_id)?;
        let base = canonical_path(&parsed);
        let mut entries: Vec<DirectoryEntry> = children
            .iter()
            .map(|child| DirectoryEntry {
                name: child.name.clone(),
                path: format!("{base}/{}", child.id),
                kind: child.kind,
                modified_at: child.modified_at,
                size: (child.kind == EntryKind::File).then_some(child.size),
                hidden: false,
                read_only: false,
            })
            .collect();
        entries.sort_by_cached_key(entry_sort_key);

        Ok(DirectoryView {
            path: base,
            breadcrumbs: self.0.breadcrumbs(&parsed)?,
            entries,
        })
    }

    fn entry_name(&self, path: &str) -> Result<String, FileSystemError> {
        // The default would return the opaque id segment; resolve the real
        // name so cross-backend transfers name files correctly.
        let parsed = self.0.parse_and_verify(path)?;
        if parsed.segments.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "The root of a volume has no entry name: {path}"
            )));
        }
        Ok(self.0.resolve_entry(path)?.name)
    }

    fn create_entry(
        &self,
        directory: &str,
        name: &str,
        kind: NewEntryKind,
    ) -> Result<String, FileSystemError> {
        validate_entry_name(name)?;
        let parsed = self.0.parse_and_verify(directory)?;
        let parent_id = parsed.segments.last().cloned().unwrap_or_default();

        // Drive tolerates duplicate names, so enforce the never-overwrite
        // contract here instead of per provider.
        let siblings = self.0.list_folder(&parent_id)?;
        if siblings
            .iter()
            .any(|child| child.name.eq_ignore_ascii_case(name))
        {
            return Err(FileSystemError::AlreadyExists(format!(
                "{}/{}",
                canonical_path(&parsed),
                name
            )));
        }

        let created = match kind {
            NewEntryKind::Directory => {
                let parent = parent_id.clone();
                let folder_name = name.to_owned();
                let meta = self.0.call(|token| {
                    let provider = Arc::clone(&self.0.provider);
                    let client = self.0.client.clone();
                    let parent = parent.clone();
                    let folder_name = folder_name.clone();
                    async move {
                        provider
                            .create_folder(&client, &token, &parent, &folder_name)
                            .await
                    }
                })?;
                self.0.adopt_meta(&parent_id, &meta);
                meta
            }
            NewEntryKind::File => {
                let staged = staged_upload_path();
                std::fs::write(&staged, []).map_err(|error| {
                    FileSystemError::Io(format!("Could not stage the new file: {error}"))
                })?;
                let result = self.0.upload_file(&parent_id, name, &staged, 0);
                let _ = std::fs::remove_file(&staged);
                result?
            }
        };

        Ok(format!("{}/{}", canonical_path(&parsed), created.id))
    }

    fn rename_entry(&self, path: &str, new_name: &str) -> Result<(), FileSystemError> {
        validate_entry_name(new_name)?;
        let parsed = self.0.parse_and_verify(path)?;
        let meta = self.0.resolve_entry(path)?;
        if meta.id.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Cannot rename the cloud account root".into(),
            ));
        }

        let meta_id = meta.id.clone();
        let renamed = new_name.to_owned();
        self.0.call(|token| {
            let provider = Arc::clone(&self.0.provider);
            let client = self.0.client.clone();
            let meta_id = meta_id.clone();
            let renamed = renamed.clone();
            async move { provider.rename(&client, &token, &meta_id, &renamed).await }
        })?;

        let parent_id = parent_of(&parsed);
        let mut state = self.0.lock_state();
        if let Some(entry) = state.metas.get_mut(&meta.id) {
            entry.name = new_name.to_owned();
        }
        state.listings.remove(&parent_id);
        Ok(())
    }

    fn search(
        &self,
        root: &str,
        query: &str,
        is_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SearchResponse, FileSystemError> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(SearchResponse {
                entries: Vec::new(),
                truncated: false,
            });
        }

        let parsed = self.0.parse_and_verify(root)?;
        let folder_id = parsed.segments.last().cloned().unwrap_or_default();

        let mut walker = SearchWalker {
            inner: &self.0,
            query,
            entries: Vec::new(),
            truncated: false,
        };
        walker.walk(&folder_id, &canonical_path(&parsed), "", is_current)?;

        Ok(SearchResponse {
            entries: walker.entries,
            truncated: walker.truncated,
        })
    }

    fn stat(&self, path: &str) -> Result<EntryStat, FileSystemError> {
        let meta = self.0.resolve_entry(path)?;
        Ok(EntryStat {
            kind: meta.kind,
            size: meta.size,
            modified_at: meta.modified_at,
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let (parent_id, name) = self.0.split_target(path)?;

        let siblings = self.0.list_folder(&parent_id)?;
        if siblings
            .iter()
            .any(|child| child.name.eq_ignore_ascii_case(&name))
        {
            return Err(FileSystemError::AlreadyExists(path.to_owned()));
        }

        let folder_name = name.clone();
        let meta = self.0.call(|token| {
            let provider = Arc::clone(&self.0.provider);
            let client = self.0.client.clone();
            let parent = parent_id.clone();
            let folder_name = folder_name.clone();
            async move {
                provider
                    .create_folder(&client, &token, &parent, &folder_name)
                    .await
            }
        })?;
        self.0.adopt_meta(&parent_id, &meta);
        Ok(())
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, FileSystemError> {
        let meta = self.0.resolve_entry(path)?;
        if meta.kind == EntryKind::Directory {
            return Err(FileSystemError::NotDirectory(path.to_owned()));
        }

        let meta_id = meta.id.clone();
        let response = self.0.call(|token| {
            let provider = Arc::clone(&self.0.provider);
            let client = self.0.client.clone();
            let meta_id = meta_id.clone();
            async move { provider.download(&client, &token, &meta_id).await }
        })?;

        Ok(Box::new(CloudReadAdapter {
            runtime: Arc::clone(&self.0.runtime),
            response: Some(response),
            buffer: Vec::new(),
            position: 0,
        }))
    }

    fn open_write(&self, path: &str) -> Result<Box<dyn Write + Send>, FileSystemError> {
        let (parent_id, name) = self.0.split_target(path)?;

        // Uploads commit in one provider call at flush time, so bytes spool
        // to a temp file first; constant memory, arbitrary size.
        let staged = staged_upload_path();
        let file = std::fs::File::create(&staged).map_err(|error| {
            FileSystemError::Io(format!("Could not stage the upload file: {error}"))
        })?;

        Ok(Box::new(CloudUploadWriter {
            inner: Arc::clone(&self.0),
            parent_id,
            name,
            staged,
            file: Some(file),
            finished: false,
        }))
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let parsed = self.0.parse_and_verify(path)?;
        let meta = self.0.resolve_entry(path)?;
        if meta.id.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Refusing to delete the cloud account root".into(),
            ));
        }

        let meta_id = meta.id.clone();
        self.0.call(|token| {
            let provider = Arc::clone(&self.0.provider);
            let client = self.0.client.clone();
            let meta_id = meta_id.clone();
            async move { provider.delete(&client, &token, &meta_id).await }
        })?;

        let mut state = self.0.lock_state();
        state.metas.remove(&meta_id);
        state.listings.remove(&parent_of(&parsed));
        Ok(())
    }

    fn rename_to(&self, source: &str, destination: &str) -> Result<(), FileSystemError> {
        let from_parsed = self.0.parse_and_verify(source)?;
        let to_parsed = self.0.parse_and_verify(destination)?;

        let source_meta = self.0.resolve_entry(source)?;
        if source_meta.id.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "Cannot move the cloud account root".into(),
            ));
        }

        let dest_last = to_parsed
            .segments
            .last()
            .cloned()
            .ok_or_else(|| FileSystemError::InvalidInput(format!("Not an entry path: {destination}")))?;
        let dest_name = match self.0.metadata(&dest_last) {
            Ok(meta) => meta.name,
            Err(FileSystemError::NotFound(_)) => dest_last,
            Err(error) => return Err(error),
        };
        let dest_parent = parent_of(&to_parsed);

        let source_id = source_meta.id.clone();
        self.0.call(|token| {
            let provider = Arc::clone(&self.0.provider);
            let client = self.0.client.clone();
            let source_id = source_id.clone();
            let dest_parent = dest_parent.clone();
            let dest_name = dest_name.clone();
            async move {
                provider
                    .move_to(&client, &token, &source_id, &dest_parent, &dest_name)
                    .await
            }
        })?;

        let mut state = self.0.lock_state();
        if let Some(entry) = state.metas.get_mut(&source_meta.id) {
            entry.name = dest_name.clone();
        }
        state.listings.remove(&parent_of(&from_parsed));
        state.listings.remove(&dest_parent);
        Ok(())
    }
}

fn validate_entry_name(name: &str) -> Result<(), FileSystemError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(FileSystemError::InvalidInput(format!(
            "Invalid entry name: {name}"
        )));
    }
    Ok(())
}

fn staged_upload_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "dae-cloud-upload-{}-{}.part",
        std::process::id(),
        STAGED_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

struct SearchWalker<'a> {
    inner: &'a CloudInner,
    query: String,
    entries: Vec<SearchEntry>,
    truncated: bool,
}

impl SearchWalker<'_> {
    fn walk(
        &mut self,
        folder_id: &str,
        path_prefix: &str,
        relative_prefix: &str,
        is_current: &dyn Fn() -> bool,
    ) -> Result<(), FileSystemError> {
        if self.truncated || !is_current() {
            return Ok(());
        }

        let children = self.inner.list_folder(folder_id)?;
        for child in children.iter() {
            if self.entries.len() >= MAX_SEARCH_RESULTS {
                self.truncated = true;
                return Ok(());
            }
            if !is_current() {
                return Ok(());
            }

            let child_path = format!("{path_prefix}/{}", child.id);
            let child_relative = if relative_prefix.is_empty() {
                child.name.clone()
            } else {
                format!("{relative_prefix}/{}", child.name)
            };

            if child.name.to_lowercase().contains(&self.query) {
                self.entries.push(SearchEntry {
                    name: child.name.clone(),
                    path: child_path.clone(),
                    relative_path: child_relative.clone(),
                    kind: child.kind,
                    modified_at: child.modified_at,
                    size: (child.kind == EntryKind::File).then_some(child.size),
                    hidden: false,
                    read_only: false,
                });
            }

            if child.kind == EntryKind::Directory {
                self.walk(&child.id, &child_path, &child_relative, is_current)?;
            }
        }

        Ok(())
    }
}

/// Streams an authenticated download as a blocking `Read`, buffering one
/// network chunk per round trip.
struct CloudReadAdapter {
    runtime: Arc<tokio::runtime::Runtime>,
    response: Option<reqwest::Response>,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for CloudReadAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position >= self.buffer.len() {
            let Some(response) = self.response.as_mut() else {
                return Ok(0);
            };
            match self
                .runtime
                .block_on(response.chunk())
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                Some(bytes) => {
                    self.buffer = bytes.to_vec();
                    self.position = 0;
                }
                None => {
                    self.response = None;
                    return Ok(0);
                }
            }
        }

        let available = &self.buffer[self.position..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count;
        Ok(count)
    }
}

/// Spools the engine's byte stream to a temp file, then commits the whole
/// upload through the provider's session/chunked endpoints on flush.
struct CloudUploadWriter {
    inner: Arc<CloudInner>,
    parent_id: String,
    name: String,
    staged: PathBuf,
    file: Option<std::fs::File>,
    finished: bool,
}

impl Write for CloudUploadWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("The upload already finished"))?;
        file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let Some(file) = self.file.take() else {
            return Err(io::Error::other("The upload already finished"));
        };
        file.sync_all()?;
        drop(file);

        let size = std::fs::metadata(&self.staged)?.len();
        self.inner
            .upload_file(&self.parent_id, &self.name, &self.staged, size)
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for CloudUploadWriter {
    fn drop(&mut self) {
        // Cleans up both committed uploads and abandoned (unflushed) ones; a
        // failed or skipped flush must never leave a half-uploaded file.
        let _ = std::fs::remove_file(&self.staged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cloud_account_paths() {
        let parsed = parse_cloud_path("google_drive:user@example.com").expect("account root");
        assert_eq!(parsed.account_id, "cloud://google_drive:user@example.com");
        assert!(parsed.segments.is_empty());

        let parsed =
            parse_cloud_path("dropbox:a@b.io/folder-1/sub-folder").expect("nested path");
        assert_eq!(parsed.account_id, "cloud://dropbox:a@b.io");
        assert_eq!(parsed.segments, ["folder-1", "sub-folder"]);

        // Emails contain dots and at-signs but no colon; the split must use
        // the FIRST colon so a domain never parses as a port.
        let parsed = parse_cloud_path("onedrive:mail.user+tag@sub.domain.co.uk/id")
            .expect("complex email");
        assert_eq!(parsed.account_id, "cloud://onedrive:mail.user+tag@sub.domain.co.uk");

        // Trailing slashes do not produce empty segments.
        let parsed = parse_cloud_path("google_drive:u@e.com/id/").expect("trailing slash");
        assert_eq!(parsed.segments, ["id"]);
    }

    #[test]
    fn rejects_malformed_cloud_paths() {
        assert!(parse_cloud_path("no-colon").is_err());
        assert!(parse_cloud_path("unknown_provider:u@e.com").is_err());
        assert!(parse_cloud_path("google_drive:").is_err());
    }

    #[test]
    fn rebuilds_canonical_paths() {
        let parsed = parse_cloud_path("google_drive:u@e.com/a/b").expect("path");
        assert_eq!(canonical_path(&parsed), "cloud://google_drive:u@e.com/a/b");
        assert_eq!(parent_of(&parsed), "a");

        let root = parse_cloud_path("google_drive:u@e.com").expect("root");
        assert_eq!(canonical_path(&root), "cloud://google_drive:u@e.com");
        assert_eq!(parent_of(&root), "");

        let one = parse_cloud_path("google_drive:u@e.com/a").expect("one segment");
        assert_eq!(parent_of(&one), "");
    }
}
