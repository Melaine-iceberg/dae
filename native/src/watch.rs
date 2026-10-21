//! Directory change observation for the Electron core process.
//!
//! The local backend uses OS file notifications; every other backend falls back
//! to snapshot polling. This is the same policy as the Tauri shell's watcher,
//! minus the Tauri app handle: reports go to the core process's event pump and
//! invalidate the shared Git status cache.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::types::{DirectoryView, entry_kind_rank, path_to_string};
use dae_core::file_system::vfs::SharedBackend;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use crate::emit_event;

const EVENT_NAME: &str = "explorer-directory-changed";

/// How long to wait between polls of a remote directory, and the ceiling that
/// wait backs off to while nothing changes.
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const POLL_INTERVAL_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryChanged {
    /// The directory that changed, in the spelling its listing reports.
    pub path: String,
    /// The children that changed, by name. Empty means the change is not
    /// attributable to a row and the caller has to re-read.
    pub names: Vec<String>,
}

impl DirectoryChanged {
    pub fn resync(path: String) -> Self {
        Self {
            path,
            names: Vec::new(),
        }
    }

    fn emit(self) {
        if let Ok(payload) = serde_json::to_string(&self) {
            emit_event(EVENT_NAME, payload);
        }
    }
}

/// The active observation. Dropping a `Notify` handle stops its OS watcher;
/// a `Poll` handle owns a stop flag that its poller thread checks each tick.
pub enum WatchHandle {
    Notify(RecommendedWatcher),
    Poll(Arc<AtomicBool>),
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        match self {
            WatchHandle::Notify(watcher) => {
                let _ = &*watcher;
            }
            WatchHandle::Poll(stop) => stop.store(true, AtomicOrdering::Relaxed),
        }
    }
}

/// One observer per view, keyed by the id the frontend mints for that view.
#[derive(Default)]
pub struct DirectoryWatcher {
    /// Serializes arms across all views. An arm stores its generation with its
    /// entry, and only the newest generation for a view may install — without
    /// it, a slow arm could land after the view had already moved on.
    generation: AtomicU64,
    views: Mutex<HashMap<String, ViewWatch>>,
}

struct ViewWatch {
    generation: u64,
    path: PathBuf,
    handle: Option<WatchHandle>,
}

impl DirectoryWatcher {
    fn claim(&self, watcher_id: &str, path: &Path) -> Option<u64> {
        let mut views = self.views.lock().expect("directory watcher lock poisoned");

        if let Some(current) = views.get(watcher_id)
            && current.path == path
        {
            return None;
        }

        let generation = self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1;
        views.insert(
            watcher_id.to_owned(),
            ViewWatch {
                generation,
                path: path.to_owned(),
                handle: None,
            },
        );

        Some(generation)
    }

    fn install(&self, watcher_id: &str, generation: u64, handle: WatchHandle) {
        let mut views = self.views.lock().expect("directory watcher lock poisoned");

        if let Some(current) = views.get_mut(watcher_id)
            && current.generation == generation
        {
            current.handle = Some(handle);
        }
    }

    fn abandon(&self, watcher_id: &str, generation: u64) {
        let mut views = self.views.lock().expect("directory watcher lock poisoned");

        if let Some(current) = views.get(watcher_id)
            && current.generation == generation
        {
            views.remove(watcher_id);
        }
    }

    fn release(&self, watcher_id: &str) {
        self.views
            .lock()
            .expect("directory watcher lock poisoned")
            .remove(watcher_id);
    }

    fn release_all(&self) {
        self.views
            .lock()
            .expect("directory watcher lock poisoned")
            .clear();
    }
}

static WATCHERS: LazyLock<DirectoryWatcher> = LazyLock::new(DirectoryWatcher::default);

/// Stops the observation a view held. Safe to call more than once.
pub fn unwatch(watcher_id: &str) {
    WATCHERS.release(watcher_id);
}

/// Stops every observer; called while the core process shuts down.
pub fn release_all() {
    WATCHERS.release_all();
}

/// Installs an OS watcher on `path`, which must already be canonical.
fn create_directory_watcher(path: PathBuf) -> Result<RecommendedWatcher, FileSystemError> {
    let event_path = path_to_string(&path);
    let watched = path.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            let names = match result {
                // An access event is a read bumping an atime; nothing a listing
                // shows has changed.
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
                // A watcher error means changes were dropped, so what is left is
                // "possibly dirty" — re-read rather than patch a stale row.
                Err(_) => Vec::new(),
                Ok(event) => changed_children(&watched, &event),
            };

            // A change on disk can move a Git badge; invalidate the scan before
            // the renderer's refresh reaches the cache.
            crate::git_cache().note_change();

            DirectoryChanged {
                path: event_path.clone(),
                names,
            }
            .emit();
        })
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    Ok(watcher)
}

/// The children `event` accounts for, or an empty list when it accounts for
/// nothing that maps to a single row.
fn changed_children(directory: &Path, event: &notify::Event) -> Vec<String> {
    if matches!(event.kind, EventKind::Any | EventKind::Other) {
        return Vec::new();
    }

    let mut names = Vec::with_capacity(event.paths.len());

    for path in &event.paths {
        if path.parent() != Some(directory) {
            return Vec::new();
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };

        names.push(name.to_owned());
    }

    names
}

/// Arms the OS watcher for a canonical local directory under `watcher_id`.
/// Failing to watch is logged, never fatal: the listing still works, it just
/// stops seeing live changes.
pub fn arm_local_watcher(canonical_path: PathBuf, watcher_id: &str) {
    let Some(generation) = WATCHERS.claim(watcher_id, &canonical_path) else {
        return;
    };

    match create_directory_watcher(canonical_path.clone()) {
        Ok(watcher) => {
            WATCHERS.install(watcher_id, generation, WatchHandle::Notify(watcher));
        }
        Err(error) => {
            WATCHERS.abandon(watcher_id, generation);
            eprintln!(
                "[dae-native] unable to watch {} for changes: {error}",
                path_to_string(&canonical_path)
            );
        }
    }
}

/// Arms a snapshot-polling watcher for a non-local directory.
pub fn arm_polling_watcher(path: &str, backend: SharedBackend, watcher_id: &str) {
    let Some(generation) = WATCHERS.claim(watcher_id, Path::new(path)) else {
        return;
    };

    let handle = spawn_polling_watcher(path.to_owned(), backend);
    WATCHERS.install(watcher_id, generation, handle);
}

/// Watches `path` on `backend` by diffing directory snapshots until the
/// returned handle is dropped.
fn spawn_polling_watcher(path: String, backend: SharedBackend) -> WatchHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    thread::Builder::new()
        .name("dae-directory-poll".into())
        .spawn(move || {
            let mut snapshot = match backend.read_dir(&path) {
                Ok(view) => fingerprint(&view),
                Err(_) => return,
            };
            let mut interval = POLL_INTERVAL;

            loop {
                thread::sleep(interval);

                if stop_flag.load(AtomicOrdering::Relaxed) {
                    return;
                }

                match backend.read_dir(&path) {
                    Ok(view) => {
                        let current = fingerprint(&view);
                        let changed = current != snapshot;
                        interval = next_poll_interval(interval, changed);
                        if !changed {
                            continue;
                        }

                        snapshot = current;
                        DirectoryChanged::resync(view.path.clone()).emit();
                    }
                    // A transient read error skips a tick instead of killing
                    // the poller; retrying is cheap.
                    Err(_) => continue,
                }
            }
        })
        .ok();

    WatchHandle::Poll(stop)
}

fn next_poll_interval(current: Duration, changed: bool) -> Duration {
    if changed {
        POLL_INTERVAL
    } else {
        (current * 2).min(POLL_INTERVAL_MAX)
    }
}

fn fingerprint(view: &DirectoryView) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    view.entries.len().hash(&mut hasher);
    for entry in &view.entries {
        entry.name.hash(&mut hasher);
        entry_kind_rank(&entry.kind).hash(&mut hasher);
        entry.size.hash(&mut hasher);
        entry.modified_at.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_core::file_system::types::{DirectoryEntry, EntryKind};

    fn event(kind: EventKind, paths: Vec<PathBuf>) -> notify::Event {
        notify::Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    #[test]
    fn attributes_a_change_to_the_children_it_names() {
        let directory = Path::new("/watched/dir");

        let created = event(
            EventKind::Create(notify::event::CreateKind::File),
            vec![directory.join("new.txt")],
        );
        assert_eq!(
            changed_children(directory, &created),
            vec!["new.txt".to_owned()]
        );

        let moved = event(
            EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::From,
            )),
            vec![directory.join("old.txt"), directory.join("new.txt")],
        );
        assert_eq!(
            changed_children(directory, &moved),
            vec!["old.txt".to_owned(), "new.txt".to_owned()]
        );
    }

    #[test]
    fn refuses_to_attribute_what_is_not_plainly_a_child() {
        let directory = Path::new("/watched/dir");

        let self_deleted = event(
            EventKind::Remove(notify::event::RemoveKind::Folder),
            vec![directory.to_path_buf()],
        );
        assert!(changed_children(directory, &self_deleted).is_empty());

        let nested = event(
            EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            vec![directory.join("sub").join("deep.txt")],
        );
        assert!(changed_children(directory, &nested).is_empty());

        for kind in [EventKind::Any, EventKind::Other] {
            let vague = event(kind.clone(), vec![directory.join("a.txt")]);
            assert!(changed_children(directory, &vague).is_empty(), "{kind:?}");
        }

        let nameless = event(
            EventKind::Create(notify::event::CreateKind::File),
            Vec::new(),
        );
        assert!(changed_children(directory, &nameless).is_empty());
    }

    fn entry(name: &str) -> DirectoryEntry {
        DirectoryEntry {
            name: name.to_owned(),
            path: format!("remote:/dir/{name}"),
            kind: EntryKind::File,
            modified_at: Some(1),
            size: Some(10),
            hidden: false,
            read_only: false,
        }
    }

    fn view(entries: Vec<DirectoryEntry>) -> DirectoryView {
        DirectoryView {
            path: "remote:/dir".to_owned(),
            breadcrumbs: Vec::new(),
            entries,
            stream_id: None,
        }
    }

    fn poll_handle(stop: &Arc<AtomicBool>) -> WatchHandle {
        WatchHandle::Poll(Arc::clone(stop))
    }

    #[test]
    fn re_claiming_the_same_directory_is_a_no_op() {
        let watchers = DirectoryWatcher::default();
        let path = Path::new("/watched/dir");

        assert!(watchers.claim("view", path).is_some());
        assert_eq!(watchers.claim("view", path), None);
        assert!(watchers.claim("view", Path::new("/watched/other")).is_some());
    }

    #[test]
    fn release_stops_the_view_and_lets_it_arm_again() {
        let watchers = DirectoryWatcher::default();
        let path = Path::new("/watched/dir");
        let stop = Arc::new(AtomicBool::new(false));

        let generation = watchers.claim("view", path).expect("claim");
        watchers.install("view", generation, poll_handle(&stop));
        assert!(!stop.load(AtomicOrdering::Relaxed));

        watchers.release("view");
        assert!(stop.load(AtomicOrdering::Relaxed));
        assert!(watchers.claim("view", path).is_some());

        watchers.release("ghost");
    }

    #[test]
    fn a_slow_arm_cannot_unseat_a_newer_navigation() {
        let watchers = DirectoryWatcher::default();
        let superseded_stop = Arc::new(AtomicBool::new(false));
        let current_stop = Arc::new(AtomicBool::new(false));

        let slow = watchers.claim("view", Path::new("/dir/a")).expect("claim a");
        let newer = watchers.claim("view", Path::new("/dir/b")).expect("claim b");

        watchers.install("view", slow, poll_handle(&superseded_stop));
        watchers.install("view", newer, poll_handle(&current_stop));

        assert!(superseded_stop.load(AtomicOrdering::Relaxed));
        assert!(!current_stop.load(AtomicOrdering::Relaxed));

        let views = watchers.views.lock().expect("lock");
        let entry = views.get("view").expect("the view still has an entry");
        assert_eq!(entry.path, Path::new("/dir/b"));
        assert!(entry.handle.is_some());
    }

    #[test]
    fn a_failed_arm_is_retried_by_the_next_read() {
        let watchers = DirectoryWatcher::default();
        let path = Path::new("/watched/dir");

        let generation = watchers.claim("view", path).expect("claim");
        watchers.abandon("view", generation);
        assert!(watchers.claim("view", path).is_some());
    }

    #[test]
    fn backs_off_while_nothing_changes_and_resets_on_a_change() {
        let mut interval = POLL_INTERVAL;
        for _ in 0..12 {
            interval = next_poll_interval(interval, false);
        }
        assert_eq!(interval, POLL_INTERVAL_MAX);
        assert_eq!(next_poll_interval(interval, true), POLL_INTERVAL);
        assert_eq!(next_poll_interval(POLL_INTERVAL, true), POLL_INTERVAL);
    }

    #[test]
    fn fingerprint_notices_every_field_it_folds_in() {
        let base = fingerprint(&view(vec![entry("a.txt")]));

        let mut renamed = entry("a.txt");
        renamed.name = "b.txt".to_owned();
        assert_ne!(base, fingerprint(&view(vec![renamed])), "name");

        let mut resized = entry("a.txt");
        resized.size = Some(11);
        assert_ne!(base, fingerprint(&view(vec![resized])), "size");

        let mut touched = entry("a.txt");
        touched.modified_at = Some(2);
        assert_ne!(base, fingerprint(&view(vec![touched])), "modified");

        let mut retyped = entry("a.txt");
        retyped.kind = EntryKind::Directory;
        assert_ne!(base, fingerprint(&view(vec![retyped])), "kind");

        assert_ne!(base, fingerprint(&view(vec![entry("a.txt"), entry("b.txt")])), "rows");
    }
}
