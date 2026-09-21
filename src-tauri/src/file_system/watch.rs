//! Directory change observation across backends. The local backend uses OS
//! file notifications; every other backend falls back to snapshot polling.

use crate::file_system::error::FileSystemError;
use crate::file_system::local;
use crate::file_system::types::{DirectoryView, entry_kind_rank, path_to_string};
use crate::file_system::vfs::SharedBackend;
use notify::RecommendedWatcher;
use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
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
pub struct DirectoryChanged(pub String);

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

#[derive(Default)]
pub struct DirectoryWatcher {
    generation: AtomicU64,
    watcher: Mutex<Option<WatchHandle>>,
}

impl DirectoryWatcher {
    pub fn begin_update(&self) -> u64 {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel) + 1
    }

    pub fn replace(&self, generation: u64, handle: WatchHandle) -> Result<(), FileSystemError> {
        let mut active_watcher = self.watcher.lock().map_err(|_| {
            FileSystemError::Internal("The directory watcher lock was poisoned".into())
        })?;

        if self.generation.load(AtomicOrdering::Acquire) == generation {
            *active_watcher = Some(handle);
        }

        Ok(())
    }
}

/// Arms the OS watcher for a local directory, replacing whatever was watched
/// before.
///
/// `canonical_path` must be canonical (see [`crate::file_system::types::canonical_path`])
/// — the listing this accompanies reports that spelling, and the events have
/// to name the same directory.
///
/// Failing to watch is not fatal: the explorer still lists the directory, it
/// just stops seeing live changes, so the error is logged rather than
/// propagated into the read the user is waiting for.
pub fn arm_local_watcher(app: &tauri::AppHandle, canonical_path: PathBuf) {
    let generation = app.state::<DirectoryWatcher>().begin_update();

    match local::create_directory_watcher(canonical_path.clone(), app.clone()) {
        Ok(watcher) => {
            if let Err(error) = app
                .state::<DirectoryWatcher>()
                .replace(generation, WatchHandle::Notify(watcher))
            {
                log::warn!(
                    "Unable to store the directory watcher for {}: {error}",
                    path_to_string(&canonical_path)
                );
            }
        }
        Err(error) => log::warn!(
            "Unable to watch {} for changes: {error}",
            path_to_string(&canonical_path)
        ),
    }
}

/// Arms a snapshot-polling watcher for a non-local directory, replacing
/// whatever was watched before. Failures are logged, never fatal (see
/// [`arm_local_watcher`]).
pub fn arm_polling_watcher(app: &tauri::AppHandle, path: &str, backend: SharedBackend) {
    let generation = app.state::<DirectoryWatcher>().begin_update();
    let handle = spawn_polling_watcher(path.to_owned(), backend, app.clone());

    if let Err(error) = app.state::<DirectoryWatcher>().replace(generation, handle) {
        log::warn!("Unable to store the polling watcher for {path}: {error}");
    }
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
                    let _ = DirectoryChanged(view.path.clone()).emit(&app);
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
