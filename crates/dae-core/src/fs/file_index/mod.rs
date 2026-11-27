//! dae's own filename index — the answer to "searching a big disk walks it",
//! the way Everything answers it: the disk is enumerated once into a small
//! SQLite state file, kept fresh by the file system's own change feed, and the
//! name search reads the table instead of the tree.
//!
//! Two change feeds exist, chosen per platform and per volume:
//!
//! * **Windows** — the USN journal ([`usn`]). One `FSCTL_ENUM_USN_DATA` sweep
//!   enumerates a volume (it walks the MFT, not the directory tree, so a full
//!   disk costs seconds), and `ReadUsnJournal` turns every later change into a
//!   row update. The journal needs a volume handle not every user holds
//!   (typically admin); such volumes fall through to the feeder below and
//!   search on them is no worse than today.
//! * **Everything else, and any volume the journal could not cover** — a
//!   parallel crawl ([`feeder`]) plus a recursive `notify` watcher, the same
//!   library the directory views already trust for live changes.
//!
//! Coverage is per root and *sticky in the store*, not in memory: a root the
//! feeder finished crawling stays covered across restarts, and a search only
//! pays for warming once. [`ensure`] is called from the search itself, so no
//! shell wiring is needed — the first search on a volume starts the workers,
//! and while they are still filling the table the search walks, exactly as it
//! does today.
//!
//! The index answers *name contains* and nothing else. Every candidate row is
//! still stat-ed by `fs::search`'s `index_entry`, which holds the result to the
//! same filters (kind, size, date, depth) the walk applies, so the index can
//! never show an entry the walk would have refused.

pub mod feeder;
pub mod store;
#[cfg(windows)]
pub mod usn;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{LazyLock, Mutex};

use crate::paths;

/// One shared store; the workers below are its only writers.
///
/// A user whose config directory does not resolve gets no index — their
/// searches keep walking, which is the status quo, not a failure the UI owes
/// them a dialog for.
fn store() -> Option<&'static store::Store> {
    static STORE: LazyLock<Option<store::Store>> = LazyLock::new(|| {
        // Tests point the store at a temp dir: touching the developer's real
        // index (and its change workers) from a unit test is a side effect
        // no test should be allowed to have.
        #[cfg(test)]
        if let Some(dir) = test_dir() {
            return store::Store::open(&dir).ok();
        }
        let dir: PathBuf = paths::config_dir().ok()?;
        match store::Store::open(&dir) {
            Ok(store) => Some(store),
            // One warning per process: every search would repeat it, and
            // the walk's results still answer the query.
            Err(error) => {
                log::warn!(target: "dae::file_index", "index unavailable: {error}");
                None
            }
        }
    });
    STORE.as_ref()
}

/// Where tests want the store, taking precedence over the real config dir.
/// One slot: the store opens once, so the first directory a test sets wins.
#[cfg(test)]
static TEST_DIR: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));

#[cfg(test)]
fn test_dir() -> Option<PathBuf> {
    TEST_DIR.lock().ok()?.clone()
}

#[cfg(test)]
pub(crate) fn use_test_dir(dir: PathBuf) {
    if let Ok(mut slot) = TEST_DIR.lock() {
        slot.get_or_insert(dir);
    }
}

/// Roots a worker has already been started for, in canonical spelling.
///
/// `ensure` runs per root per search, so this is the cheap gate that keeps it
/// from spawning a crawler per keystroke.
static STARTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Whether the USN sweep already claimed this process. A plain atomic claim:
/// the losers fall through to the feeder immediately, and a lost claim on an
/// uncovered root still gets that root indexed.
static USN_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Whether the index can answer queries under `root` right now.
///
/// A covered root is one the sweep or a finished crawl populated; case is
/// folded on the platforms whose file systems fold it. Cheap: one `SELECT`
/// over the (tiny) roots table per search call, no refresh thread needed.
pub fn covered(root: &str) -> bool {
    let Some(store) = store() else {
        return false;
    };
    store
        .covered_roots()
        .iter()
        .any(|covered| store::is_under(root, covered))
}

/// Paths under `root` whose name contains `text`, at most `limit` of them.
///
/// Order is the store's; ranking is the caller's. An unavailable store is an
/// empty answer, never an error.
pub fn query(root: &str, text: &str, limit: usize) -> Vec<String> {
    let Some(store) = store() else {
        return Vec::new();
    };
    store.query(text, &[root.to_owned()], limit)
}

/// Starts (once) the workers that turn `root`'s subtree into index rows.
///
/// Never blocking, never failing: the call site is the search the user is
/// waiting on. On Windows the USN sweep is tried first and covers whole
/// volumes; a root that sweep cannot serve — journal unreadable, or a
/// FAT/exFAT/removable drive that has no journal at all — gets the feeder
/// instead. A root the sweep already covered needs no worker at all.
pub fn ensure(root: &str) {
    let Some(store) = store() else {
        return;
    };

    #[cfg(windows)]
    ensure_windows(store, root);
    #[cfg(not(windows))]
    spawn_feeder_if_uncovered(store, root);
}

/// The Windows arm of [`ensure`]: claim the USN sweep once per process, and
/// hand anything the sweep cannot cover to the feeder.
#[cfg(windows)]
fn ensure_windows(store: &store::Store, root: &str) {
    if USN_CLAIMED.load(AtomicOrdering::Relaxed) {
        spawn_feeder_if_uncovered(store, root);
        return;
    }
    if USN_CLAIMED.swap(true, AtomicOrdering::Relaxed) {
        // Lost the claim to another thread: its sweep covers this root, or
        // the next search hands the root to the feeder. Nothing to do.
        return;
    }
    if !usn::capable(store) {
        // Probed at claim time: a journal-less machine would otherwise hold
        // the claim and never hand any root to the feeder.
        return;
    }
    let sweep_store = store.clone();
    let _ = std::thread::Builder::new()
        .name("file-index-usn".into())
        .spawn(move || {
            // The sweep enumerates whole volumes, which outlives any one
            // search — so it runs here, not on the search's thread.
            for volume in usn::start(sweep_store) {
                log::info!(target: "dae::file_index", "USN covered {volume}");
                STARTED.lock().map(|mut roots| roots.insert(volume)).ok();
            }
        });
}
fn spawn_feeder_if_uncovered(store: &store::Store, root: &str) {
    if covered(root) {
        return;
    }
    let Ok(canonical) = dunce::canonicalize(root) else {
        return;
    };
    let root = crate::types::path_to_string(&canonical);
    if !STARTED
        .lock()
        .map(|mut roots| roots.insert(root.clone()))
        .unwrap_or(false)
    {
        return;
    }
    feeder::start(store.clone(), root);
}

pub(crate) fn clear_started(root: &str) {
    if let Ok(mut roots) = STARTED.lock() {
        roots.remove(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_prefix_is_separator_aware() {
        // `is_under` is the coverage test; pin its platform case rule with the
        // spellings a USN sweep (native, `C:\`) and a user-typed root mix.
        assert!(store::is_under("C:\\Users", "c:\\"));
        assert!(!store::is_under("C:\\Users", "C:\\Users2"));
        assert!(store::is_under("/home/user", "/home"));
        assert!(!store::is_under("/home/other", "/home/user"));
    }
}
