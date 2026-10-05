//! Directory change observation across backends. The local backend uses OS
//! file notifications; every other backend falls back to snapshot polling.

use crate::file_system::local;
use crate::file_system::types::{DirectoryView, entry_kind_rank, path_to_string};
use crate::file_system::vfs::SharedBackend;
use notify::RecommendedWatcher;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;
use tauri::Manager;
use tauri_specta::Event;

/// How long to wait between polls of a remote directory, and the ceiling that
/// wait backs off to while nothing changes.
///
/// A *change* resets the wait to the base, so a folder someone is working in
/// stays exactly as fresh as it is today. An idle folder converges on the
/// ceiling instead of re-enumerating forever: one enumeration of a large share
/// can take longer than the base interval, in which case the poller never stops
/// working and competes with transfers on the same session.
///
/// The price is staleness on an idle directory — up to `POLL_INTERVAL_MAX` before
/// a change made elsewhere shows up (4s while anything is happening, and a local
/// directory is watched by the OS and unaffected).
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const POLL_INTERVAL_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "explorer-directory-changed")]
#[serde(rename_all = "camelCase")]
pub struct DirectoryChanged {
    /// The directory that changed, in the spelling its listing reports.
    pub path: String,
    /// The children that changed, by name.
    ///
    /// Empty means the change is *not attributable to a row* — the watcher hit
    /// an error (whose payload may have been dropped), reported the watched
    /// directory itself, or came from a backend that only knows "something
    /// differs". A caller then has to re-read; a non-empty list is everything it
    /// needs to repair a listing it already holds.
    pub names: Vec<String>,
}

impl DirectoryChanged {
    /// A change that can only be answered by re-reading the directory.
    pub fn resync(path: String) -> Self {
        Self { path, names: Vec::new() }
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
            // Dropping the notify watcher stops it; pollers need the signal.
            WatchHandle::Notify(watcher) => {
                let _ = &*watcher;
            }
            WatchHandle::Poll(stop) => stop.store(true, AtomicOrdering::Relaxed),
        }
    }
}

/// One observer per view, keyed by the id the frontend mints for that view.
///
/// A single global watcher cannot serve a multi-pane, multi-tab, multi-window
/// explorer: the last directory read takes the only watcher, and every other
/// pane — including the visible ones — silently stops seeing changes. Each
/// view therefore holds its own entry: navigation re-targets it, unmounting
/// the view releases it.
#[derive(Default)]
pub struct DirectoryWatcher {
    /// Serializes arms across all views. An arm stores its generation with its
    /// entry, and only the newest generation for a view may install — without
    /// it, a slow arm (creating the OS watcher takes real time) could land
    /// after the view had already moved on and steal the entry from the
    /// directory actually on screen.
    generation: AtomicU64,
    views: Mutex<HashMap<String, ViewWatch>>,
}

struct ViewWatch {
    /// The arm that owns this entry: the newest one begun for the view, which
    /// may still be in flight.
    generation: u64,
    /// The directory that arm targets — the spelling the listing reports, so
    /// events can be matched against the displayed path.
    path: PathBuf,
    /// The observer, once its arm finished installing. `None` while the arm is
    /// in flight; the entry is removed outright when an arm fails.
    handle: Option<WatchHandle>,
}

impl DirectoryWatcher {
    /// Reserves the right for `watcher_id` to watch `path`. `None` means the
    /// view is already committed to exactly this directory — the steady-state
    /// refresh — and arming again would only churn the OS watcher. Otherwise
    /// the previous observation is replaced on the spot: the view navigated
    /// away, and its old directory is nobody's business any more.
    fn claim(&self, watcher_id: &str, path: &Path) -> Option<u64> {
        let mut views = self
            .views
            .lock()
            .expect("directory watcher lock poisoned");

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

    /// Installs the observer an arm produced. A claim newer than this arm's
    /// generation already owns the entry, in which case the handle is dropped
    /// — nothing superseded may keep observing.
    fn install(&self, watcher_id: &str, generation: u64, handle: WatchHandle) {
        let mut views = self
            .views
            .lock()
            .expect("directory watcher lock poisoned");

        if let Some(current) = views.get_mut(watcher_id)
            && current.generation == generation
        {
            current.handle = Some(handle);
        }
    }

    /// Drops the claim an arm failed to fill, so the next read retries instead
    /// of trusting an entry that watches nothing.
    fn abandon(&self, watcher_id: &str, generation: u64) {
        let mut views = self
            .views
            .lock()
            .expect("directory watcher lock poisoned");

        if let Some(current) = views.get(watcher_id)
            && current.generation == generation
        {
            views.remove(watcher_id);
        }
    }

    /// Stops observing for the view. Dropping the handle ends an OS watcher or
    /// signals a poller to stop; ids the registry no longer knows are ignored.
    fn release(&self, watcher_id: &str) {
        self.views
            .lock()
            .expect("directory watcher lock poisoned")
            .remove(watcher_id);
    }
}

/// Stops the observation a view held. Called when the view goes away — its
/// pane unmounted, its tab closed — and must be safe to call more than once.
#[tauri::command]
#[specta::specta]
pub fn unwatch_directory(watcher_id: String, app: tauri::AppHandle) {
    app.state::<DirectoryWatcher>().release(&watcher_id);
}

/// Arms the OS watcher for a local directory under `watcher_id`, re-targeting
/// the view's observation if it pointed elsewhere.
///
/// `canonical_path` must be canonical (see [`crate::file_system::types::canonical_path`])
/// — the listing this accompanies reports that spelling, and the events have
/// to name the same directory.
///
/// Failing to watch is not fatal: the explorer still lists the directory, it
/// just stops seeing live changes, so the error is logged rather than
/// propagated into the read the user is waiting for.
pub fn arm_local_watcher(app: &tauri::AppHandle, canonical_path: PathBuf, watcher_id: &str) {
    let watchers = app.state::<DirectoryWatcher>();
    let Some(generation) = watchers.claim(watcher_id, &canonical_path) else {
        return;
    };

    match local::create_directory_watcher(canonical_path.clone(), app.clone()) {
        Ok(watcher) => {
            watchers.install(watcher_id, generation, WatchHandle::Notify(watcher));
        }
        Err(error) => {
            watchers.abandon(watcher_id, generation);
            log::warn!(
                "Unable to watch {} for changes: {error}",
                path_to_string(&canonical_path)
            );
        }
    }
}

/// Arms a snapshot-polling watcher for a non-local directory under
/// `watcher_id`, re-targeting the view's observation if it pointed elsewhere.
/// Failures are logged, never fatal (see [`arm_local_watcher`]).
pub fn arm_polling_watcher(
    app: &tauri::AppHandle,
    path: &str,
    backend: SharedBackend,
    watcher_id: &str,
) {
    let watchers = app.state::<DirectoryWatcher>();
    let Some(generation) = watchers.claim(watcher_id, Path::new(path)) else {
        return;
    };

    let handle = spawn_polling_watcher(path.to_owned(), backend, app.clone());
    watchers.install(watcher_id, generation, handle);
}

/// Watches `path` on `backend` by diffing directory snapshots until the
/// returned handle is dropped. Transient read errors skip a tick instead of
/// killing the poller, so a briefly unreachable server does not blind the
/// explorer permanently.
pub fn spawn_polling_watcher(
    path: String,
    backend: SharedBackend,
    app: tauri::AppHandle,
) -> WatchHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    thread::spawn(move || {
        let mut snapshot = match backend.read_dir(&path) {
            Ok(view) => fingerprint(&view),
            // The explorer just read this directory; failing here means the
            // session went away between the two calls. Nothing to watch.
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
                        // Nothing new here, so ask again later rather than
                        // spending a full enumeration on an idle directory.
                        continue;
                    }

                    snapshot = current;
                    // Something is happening, so go back to watching closely.
                    // A poll only learns *that* the directory differs, never
                    // which rows, so this stays the re-read form.
                    let _ = DirectoryChanged::resync(view.path.clone()).emit(&app);
                }
                // A transient read error skips a tick instead of killing the
                // poller, so a briefly unreachable server does not blind the
                // explorer permanently. The interval is deliberately *not*
                // grown here: an unreachable server answers with an error
                // immediately, so retrying is cheap, unlike a successful
                // enumeration of a large share.
                Err(_) => continue,
            }
        }
    });

    WatchHandle::Poll(stop)
}

/// The wait before the next poll, given what the last one found.
///
/// Extracted so the policy is one testable expression rather than being spread
/// through the loop: a change goes back to the base interval, an unchanged poll
/// backs off towards the ceiling.
fn next_poll_interval(current: Duration, changed: bool) -> Duration {
    if changed {
        POLL_INTERVAL
    } else {
        (current * 2).min(POLL_INTERVAL_MAX)
    }
}

/// Folds the snapshot into a 64-bit hash instead of building one formatted
/// `String` per entry every poll tick, which keeps polling watchers free of
/// per-entry heap allocations.
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
    use crate::file_system::types::{DirectoryEntry, EntryKind};

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

        assert!(watchers.claim("view", path).is_some(), "first claim arms");
        assert_eq!(
            watchers.claim("view", path),
            None,
            "the steady-state refresh must not churn the OS watcher"
        );
        assert!(
            watchers.claim("view", Path::new("/watched/other")).is_some(),
            "a navigation re-targets the same view"
        );
    }

    #[test]
    fn release_stops_the_view_and_lets_it_arm_again() {
        let watchers = DirectoryWatcher::default();
        let path = Path::new("/watched/dir");
        let stop = Arc::new(AtomicBool::new(false));

        let generation = watchers.claim("view", path).expect("claim");
        watchers.install("view", generation, poll_handle(&stop));
        assert!(!stop.load(AtomicOrdering::Relaxed), "the live observer runs");

        watchers.release("view");
        assert!(
            stop.load(AtomicOrdering::Relaxed),
            "releasing the view must stop its observer"
        );
        assert!(
            watchers.claim("view", path).is_some(),
            "a re-mounted view arms afresh"
        );

        // Ids nobody knows are ignored rather than an error.
        watchers.release("ghost");
    }

    /// Creating an OS watcher takes real time, so two quick navigations can
    /// finish arming out of order. The view must end up observing the
    /// directory it displays — the newer claim — never the one that lost.
    #[test]
    fn a_slow_arm_cannot_unseat_a_newer_navigation() {
        let watchers = DirectoryWatcher::default();
        let superseded_stop = Arc::new(AtomicBool::new(false));
        let current_stop = Arc::new(AtomicBool::new(false));

        let slow = watchers
            .claim("view", Path::new("/dir/a"))
            .expect("claim a");
        let newer = watchers
            .claim("view", Path::new("/dir/b"))
            .expect("claim b");

        // The slow arm for /dir/a lands last.
        watchers.install("view", slow, poll_handle(&superseded_stop));
        watchers.install("view", newer, poll_handle(&current_stop));

        assert!(
            superseded_stop.load(AtomicOrdering::Relaxed),
            "a superseded observer must be stopped, not kept"
        );
        assert!(!current_stop.load(AtomicOrdering::Relaxed));

        let views = watchers.views.lock().expect("lock");
        let entry = views.get("view").expect("the view still has an entry");
        assert_eq!(entry.path, Path::new("/dir/b"));
        assert!(entry.handle.is_some(), "the newer arm is installed");
    }

    #[test]
    fn a_failed_arm_is_retried_by_the_next_read() {
        let watchers = DirectoryWatcher::default();
        let path = Path::new("/watched/dir");

        let generation = watchers.claim("view", path).expect("claim");
        watchers.abandon("view", generation);
        assert!(
            watchers.claim("view", path).is_some(),
            "nothing was installed, so the next read must try again"
        );
    }

    #[test]
    fn backs_off_while_nothing_changes_and_resets_on_a_change() {
        // An idle directory climbs towards the ceiling and then stays there
        // rather than doubling without bound.
        let mut interval = POLL_INTERVAL;
        for _ in 0..12 {
            interval = next_poll_interval(interval, false);
        }
        assert_eq!(interval, POLL_INTERVAL_MAX);

        // Any change goes straight back to the base interval, so a directory
        // someone is working in stays as fresh as it was before the backoff.
        assert_eq!(next_poll_interval(interval, true), POLL_INTERVAL);
        assert_eq!(next_poll_interval(POLL_INTERVAL, true), POLL_INTERVAL);
    }

    /// The fingerprint is the poller's only comparison, so a field missing from
    /// it is a class of change that is never noticed — the poller would sit
    /// there reporting "unchanged" forever. Each assertion below is one field.
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
        assert_ne!(base, fingerprint(&view(vec![touched])), "modified_at");

        let mut retyped = entry("a.txt");
        retyped.kind = EntryKind::Directory;
        assert_ne!(base, fingerprint(&view(vec![retyped])), "kind");

        assert_ne!(
            base,
            fingerprint(&view(vec![entry("a.txt"), entry("c.txt")])),
            "the entry count, so an add or a remove is noticed"
        );

        // Identical content has to fingerprint identically, or every single poll
        // would report a change and the explorer would re-read continuously.
        assert_eq!(base, fingerprint(&view(vec![entry("a.txt")])));
    }
}
