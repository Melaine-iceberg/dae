//! Populates the persistent filename index and keeps it current.
//!
//! [`start`] is the whole story for one root: a parallel crawl backfills the
//! store with the same walk a name search of that root would make, then a
//! recursive `notify` watcher applies single events as they arrive. The two
//! halves share one store (cheap clones of one connection, WAL), so queries
//! answer while the crawl's batches commit.
//!
//! Nothing here may fail loudly. The index is an accelerator: every error —
//! a path that will not canonicalize, a batch that does not commit, a stat
//! that raced a delete — is logged and skipped, and a watcher that dies
//! just leaves the index stale until the next crawl rewrites the truth.
//! A removal the OS reports names the *directory*, never each child, so
//! deletes go by subtree (see [`remove_paths`]) rather than by exact path —
//! otherwise deleting one folder would leave every row under it pointing
//! at nothing until the next crawl.

use super::store::{Entry, Store};
use crate::types::{display_name_from_path, path_to_string};
use ignore::overrides::{Override, OverrideBuilder};
use ignore::{WalkBuilder, WalkState};
use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{Event, RecursiveMode, Watcher};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::TryRecvError;
use std::thread::JoinHandle;

/// Rows buffered by a crawl worker before it commits one transaction.
///
/// Thousands of rows per commit is where SQLite stops being the bottleneck
/// of a volume-sized walk; a larger buffer only delays the first commits
/// and holds more rows hostage to one failure.
const UPSERT_BATCH: usize = 5_000;

/// Events drained per wakeup, so a burst (a checkout, a build) costs one
/// thread wakeup plus one store call per event instead of a wakeup per
/// event. The cap keeps a runaway producer from pinning the thread; what is
/// left in the queue is picked up by the next `recv`.
const COALESCE_LIMIT: usize = 1_024;

/// Directory names skipped wherever they appear below a crawl root.
///
/// The same list every search walk in this crate skips — `fs::local::ignores`
/// pins it for name and content search, and the index must cover exactly
/// what a search would visit, no more and no less. That list is private to
/// `fs::local`, so the patterns are mirrored here; keep the two in sync.
/// Exclusion form, relative to the walk root, so a rule never excludes the
/// root itself.
const ALWAYS_IGNORED: [&str; 4] = ["!.git", "!**/.git", "!**/node_modules", "!**/target"];

/// Crawl-then-watch on one thread: the whole index lifetime for `root`.
///
/// The watcher arms after the crawl so the two never race the same tree;
/// events that arrive mid-crawl are answered by the crawl's own upserts.
pub fn start(store: Store, root: String) -> JoinHandle<()> {
    std::thread::spawn(move || {
        crawl(&store, &root);
        watch(&store, &root);
    })
}

/// One-shot parallel crawl of an already-canonicalized `root`.
fn crawl(store: &Store, root: &str) {
    let root_path = match dunce::canonicalize(root) {
        Ok(path) => path,
        Err(error) => {
            log::warn!("Unable to canonicalize {root} for the file index: {error}");
            return;
        }
    };
    let root = path_to_string(&root_path);

    let mut builder = WalkBuilder::new(&root_path);
    // The same walk a name search of this root would make, so the index
    // covers precisely what that search would have visited.
    builder.standard_filters(false).follow_links(false);
    // Unbounded: the index is a volume's memory, not one folder's view.
    if let Some(overrides) = crawl_overrides(&root_path) {
        builder.overrides(overrides);
    }

    // One shared buffer: every worker pushes, and whichever worker's push
    // fills the batch commits it. Locking per entry is the crawl's only
    // cross-thread cost, and it is well under a stat per entry.
    let buffered = Arc::new(Mutex::new(Vec::with_capacity(UPSERT_BATCH)));
    {
        let store = store.clone();
        let buffered = Arc::clone(&buffered);
        let root = root.clone();

        builder.build_parallel().run(move || {
            let store = store.clone();
            let buffered = Arc::clone(&buffered);
            let root = root.clone();

            Box::new(move |result| {
                let entry = match result {
                    Ok(entry) => entry,
                    // An unreadable entry says nothing about its siblings;
                    // the index is better off with the rest of the tree.
                    Err(_) => return WalkState::Continue,
                };
                // The root itself is the `roots` row, not an entry: it has
                // no containing directory, and `covered_roots` already
                // answers "is this root usable".
                if entry.depth() == 0 {
                    return WalkState::Continue;
                }
                let Some(file_type) = entry.file_type() else {
                    return WalkState::Continue;
                };

                let path = path_to_string(entry.path());
                let batch = {
                    let mut buffered = buffered.lock().expect("file index batch lock poisoned");
                    buffered.push(entry_from_path(&path, file_type.is_dir()));
                    if buffered.len() >= UPSERT_BATCH {
                        std::mem::take(&mut *buffered)
                    } else {
                        Vec::new()
                    }
                };
                upsert_batch(&store, &root, batch);

                WalkState::Continue
            })
        });
    }

    let remaining = std::mem::take(&mut *buffered.lock().expect("file index batch lock poisoned"));
    upsert_batch(store, &root, remaining);

    if let Err(error) = store.mark_root(&root, true) {
        log::warn!("Unable to mark {root} covered in the file index: {error}");
    }
}

/// [`ALWAYS_IGNORED`] as a matcher for a walk rooted at `root`, or `None`
/// (logged) when the rules do not build. Without them the crawl simply
/// visits the dependency trees too — slow, not wrong.
fn crawl_overrides(root: &Path) -> Option<Override> {
    let mut overrides = OverrideBuilder::new(root);
    for pattern in ALWAYS_IGNORED {
        if let Err(error) = overrides.add(pattern) {
            log::warn!("Unable to register the crawl rule {pattern}: {error}");
            return None;
        }
    }

    match overrides.build() {
        Ok(overrides) => Some(overrides),
        Err(error) => {
            log::warn!("Unable to build the crawl's rules: {error}");
            None
        }
    }
}

/// Arms the recursive watcher on an already-canonicalized `root` and applies
/// its events until the process ends.
fn watch(store: &Store, root: &str) {
    let root_path = match dunce::canonicalize(root) {
        Ok(path) => path,
        Err(error) => {
            log::warn!("Unable to canonicalize {root} for the file index: {error}");
            return;
        }
    };
    let root = path_to_string(&root_path);

    let (sender, receiver) = std::sync::mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |result| {
        // The send fails only when the reading end is gone, which cannot
        // happen while this watcher is alive; a dropped report is cheaper
        // than a panicking watcher thread.
        let _ = sender.send(result);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("Unable to create a watcher for {root}: {error}");
            return;
        }
    };
    if let Err(error) = watcher.watch(&root_path, RecursiveMode::Recursive) {
        log::warn!("Unable to watch {root} for index updates: {error}");
        return;
    }

    // `watcher` owns the sending end of `receiver`, so it stays armed for
    // exactly as long as this loop — its last use is above, but its drop
    // runs at the end of scope.
    let mut rename_from = None;
    for result in &receiver {
        handle_result(store, &root, result, &mut rename_from);

        // Coalesce: drain whatever queued while the previous events were
        // applied, in arrival order — an upsert deferred past a removal of
        // the same path would resurrect a deleted row.
        for _ in 0..COALESCE_LIMIT {
            match receiver.try_recv() {
                Ok(result) => handle_result(store, &root, result, &mut rename_from),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        // A rename's two halves always arrive adjacently, so a `From` still
        // pending once the burst is drained has no `To` coming: the move
        // left the tree, and its row is dropped rather than left pointing
        // at nothing.
        flush_rename(store, &root, &mut rename_from);
    }
}

/// One watcher report: an event, or a watcher error to log and outlive.
fn handle_result(
    store: &Store,
    root: &str,
    result: Result<Event, notify::Error>,
    rename_from: &mut Option<String>,
) {
    match result {
        Ok(event) => apply_event(store, root, event, rename_from),
        Err(error) => log::warn!("The watcher on {root} reported: {error}"),
    }
}

/// Applies one event to the index.
///
/// A rename arrives as two halves — `From`, then `To` — and only the pair
/// means "moved within the tree": [`Store::rename`] re-paths the moved row
/// *and* its descendants, which is why the halves are paired rather than
/// each translated on its own. A `From` that no `To` follows (this burst's
/// drain, then [`flush_rename`]) left the tree; a lone `To` is an arrival
/// into it. Reads (`Access`) change no path and index nothing.
fn apply_event(store: &Store, root: &str, event: Event, rename_from: &mut Option<String>) {
    match event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                upsert_path(store, root, path);
            }
        }
        EventKind::Remove(_) => {
            let paths = event
                .paths
                .iter()
                .map(|path| path_to_string(path))
                .filter(|path| path != root)
                .collect::<Vec<_>>();
            remove_paths(store, root, &paths);
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            if let Some(from) = event.paths.first() {
                *rename_from = Some(path_to_string(from));
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => match rename_from.take() {
            Some(from) => {
                if let Some(to) = event.paths.first() {
                    rename_path(store, root, &from, &path_to_string(to));
                }
            }
            None => {
                for path in &event.paths {
                    upsert_path(store, root, path);
                }
            }
        },
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if let (Some(from), Some(to)) = (event.paths.first(), event.paths.get(1)) {
                rename_path(store, root, &path_to_string(from), &path_to_string(to));
            }
        }
        EventKind::Modify(_) => {
            for path in &event.paths {
                upsert_path(store, root, path);
            }
        }
        _ => {}
    }
}

/// Resolves a pending rename `From` that nothing followed.
fn flush_rename(store: &Store, root: &str, rename_from: &mut Option<String>) {
    if let Some(from) = rename_from.take() {
        // The watched root itself is the orchestrator's row, never an
        // entry; a `From` naming it is noise, not a move.
        if from != root {
            remove_paths(store, root, &[from]);
        }
    }
}

/// Indexes `path` as it is right now — the stat decides directory-ness, so
/// a create and a modify of the same path need no distinction here.
fn upsert_path(store: &Store, root: &str, path: &Path) {
    let path = path_to_string(path);
    if path == root {
        return;
    }
    match fs::symlink_metadata(&path) {
        Ok(metadata) => upsert_batch(store, root, vec![entry_from_path(&path, metadata.is_dir())]),
        Err(error) => {
            // A create/modify reported for a path that will not stat was
            // outrun by its own deletion; the removal event is behind it.
            log::warn!("Unable to stat {path} for the file index: {error}");
        }
    }
}

/// Drops each path's subtree from the index, logged on failure.
///
/// A removal the OS reports names the directory, not its children, so the
/// delete has to be by subtree: [`Store::forget_root`] is the store's one
/// primitive that drops everything under a path — and its flag half is a
/// no-op for a path that is not an indexed root. For a file, whose subtree
/// is its own row, this is the exact-path delete the contract's `remove`
/// case asks for.
fn remove_paths(store: &Store, root: &str, paths: &[String]) {
    for path in paths {
        if let Err(error) = store.forget_root(path) {
            log::warn!("Unable to drop {path} from the file index under {root}: {error}");
        }
    }
}

/// Re-paths `from`'s subtree to `to`, logged on failure.
fn rename_path(store: &Store, root: &str, from: &str, to: &str) {
    if from == root || to == root {
        return;
    }
    if let Err(error) = store.rename(from, to) {
        log::warn!("Unable to move {from} to {to} in the file index: {error}");
    }
}

/// Commits one batch of rows, logged on failure — a crawl's later batches
/// and the next crawl both cover what one failure loses.
fn upsert_batch(store: &Store, root: &str, batch: Vec<Entry>) {
    if batch.is_empty() {
        return;
    }
    if let Err(error) = store.upsert(&batch) {
        log::warn!("Unable to index a batch under {root}: {error}");
    }
}

/// The row for `path`, whose kind the caller has already established.
///
/// `name` and `parent` are derived here rather than trusted, so the crawl
/// and the watcher — the only two producers of rows — cannot disagree.
/// The derivation matches the store's own splitting convention
/// (`split_last`): the separator belongs to a volume-root parent, so
/// `C:\Users` sits in `C:\`, and a volume root itself (`C:\`, `/`) has
/// no parent — a row the feeder never produces, since roots are `roots`.
fn entry_from_path(path: &str, is_dir: bool) -> Entry {
    Entry {
        path: path.to_owned(),
        name: display_name_from_path(path),
        parent: parent_of(path),
        is_dir,
    }
}

/// The directory containing `path`, or `""` when `path` is itself a volume
/// root (`C:\`, `/`).
///
/// Mirrors the store's `split_last`: the separator belongs to a volume-root
/// parent (`C:\Users` lives in `C:\`, `/home` lives in `/`) and to no other.
fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(index) => {
            let before = &trimmed[..index];
            if before.is_empty() || (cfg!(windows) && before.len() == 2 && before.ends_with(':')) {
                trimmed[..index + 1].to_owned()
            } else {
                before.to_owned()
            }
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, apply_event, crawl, entry_from_path, flush_rename, parent_of};
    use crate::types::path_to_string;
    use notify::Event;
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};
    use std::fs;
    use std::path::Path;

    /// `kind` + one path, the shape every single-path event arrives in.
    fn event(kind: EventKind, path: &Path) -> Event {
        Event::new(kind).add_path(path.to_path_buf())
    }

    #[test]
    fn parent_of_derives_the_containing_directory() {
        assert_eq!(parent_of("C:\\Users\\ada"), "C:\\Users");
        assert_eq!(parent_of("C:\\Users\\ada\\notes.txt"), "C:\\Users\\ada");
        assert_eq!(parent_of("/home/ada"), "/home");
        assert_eq!(parent_of("/home/ada/notes.txt"), "/home/ada");
        // The separator belongs to a volume-root parent only.
        assert_eq!(parent_of("C:\\Users"), "C:\\");
        assert_eq!(parent_of("/home"), "/");
        // A trailing separator is folded away before the split: the path
        // still denotes the directory, so its parent sits one level up.
        assert_eq!(parent_of("C:\\Users\\ada\\"), "C:\\Users");
        assert_eq!(parent_of("/home/ada/"), "/home");
        // A volume root has no containing directory.
        assert_eq!(parent_of("C:\\"), "");
        assert_eq!(parent_of("/"), "");
    }

    #[test]
    fn entry_from_path_derives_name_and_parent() {
        let entry = entry_from_path("C:\\Users\\ada\\notes.txt", false);
        assert_eq!(
            entry,
            Entry {
                path: "C:\\Users\\ada\\notes.txt".to_owned(),
                name: "notes.txt".to_owned(),
                parent: "C:\\Users\\ada".to_owned(),
                is_dir: false,
            }
        );

        let entry = entry_from_path("/home/ada", true);
        assert_eq!(entry.name, "ada");
        assert_eq!(entry.parent, "/home");
        assert!(entry.is_dir);
    }

    #[test]
    fn crawl_populates_entries_and_marks_the_root_covered() {
        let db = tempfile::tempdir().expect("create the db dir");
        let tree = tempfile::tempdir().expect("create the tree");
        let root_path = dunce::canonicalize(tree.path()).expect("canonicalize the tree");
        let root = path_to_string(&root_path);

        fs::write(root_path.join("readme.md"), b"").expect("write readme.md");
        fs::create_dir(root_path.join("sub")).expect("create sub");
        fs::write(root_path.join("sub").join("note.txt"), b"").expect("write note.txt");
        fs::create_dir(root_path.join("node_modules")).expect("create node_modules");
        fs::write(root_path.join("node_modules").join("dep.js"), b"")
            .expect("write node_modules/dep.js");

        let store = super::Store::open(db.path()).expect("open the store");
        let reader = store.clone();
        crawl(&store, &root);

        let note = path_to_string(&root_path.join("sub").join("note.txt"));
        assert_eq!(reader.query("note", &[root.clone()], 10), vec![note]);
        assert_eq!(reader.query("readme", &[root.clone()], 10).len(), 1);
        assert_eq!(reader.query("sub", &[root.clone()], 10).len(), 1);
        // Skipped by the walk, exactly as a name search would skip it.
        assert!(reader.query("dep", &[root.clone()], 10).is_empty());
        // The crawl's whole point: the root is now usable without a walk.
        assert!(reader.covered_roots().contains(&root));
    }

    #[test]
    fn watcher_events_upsert_remove_and_rename() {
        let db = tempfile::tempdir().expect("create the db dir");
        let tree = tempfile::tempdir().expect("create the tree");
        let root_path = dunce::canonicalize(tree.path()).expect("canonicalize the tree");
        let root = path_to_string(&root_path);

        fs::create_dir_all(root_path.join("dir")).expect("create dir");
        fs::write(root_path.join("dir").join("file.txt"), b"").expect("write dir/file.txt");

        let store = super::Store::open(db.path()).expect("open the store");
        crawl(&store, &root);

        let mut rename_from = None;

        // Create: a new file lands, with the components derived from its path.
        let created = root_path.join("new.txt");
        fs::write(&created, b"").expect("write new.txt");
        apply_event(
            &store,
            &root,
            event(EventKind::Create(CreateKind::Any), &created),
            &mut rename_from,
        );
        assert_eq!(
            store.query("new", &[root.clone()], 10),
            vec![path_to_string(&created)]
        );

        // Remove: the row goes with the file.
        fs::remove_file(&created).expect("remove new.txt");
        apply_event(
            &store,
            &root,
            event(EventKind::Remove(RemoveKind::Any), &created),
            &mut rename_from,
        );
        assert!(store.query("new", &[root.clone()], 10).is_empty());

        // Modify: a file the crawl never saw is indexed on first sight.
        let modified = root_path.join("modded.txt");
        fs::write(&modified, b"").expect("write modded.txt");
        apply_event(
            &store,
            &root,
            event(EventKind::Modify(ModifyKind::Any), &modified),
            &mut rename_from,
        );
        assert_eq!(
            store.query("modded", &[root.clone()], 10),
            vec![path_to_string(&modified)]
        );

        // Rename: the From/To pair re-paths the subtree, not just its root.
        let from = root_path.join("dir");
        let to = root_path.join("renamed");
        fs::rename(&from, &to).expect("rename dir");
        apply_event(
            &store,
            &root,
            event(EventKind::Modify(ModifyKind::Name(RenameMode::From)), &from),
            &mut rename_from,
        );
        apply_event(
            &store,
            &root,
            event(EventKind::Modify(ModifyKind::Name(RenameMode::To)), &to),
            &mut rename_from,
        );
        let moved = path_to_string(&to.join("file.txt"));
        assert_eq!(store.query("file", &[root.clone()], 10), vec![moved]);
        assert_eq!(store.query("renamed", &[root.clone()], 10).len(), 1);
        assert!(store.query("dir", &[root.clone()], 10).is_empty());

        // A From that no To follows left the tree: its rows are dropped.
        apply_event(
            &store,
            &root,
            event(EventKind::Modify(ModifyKind::Name(RenameMode::From)), &to),
            &mut rename_from,
        );
        flush_rename(&store, &root, &mut rename_from);
        assert!(store.query("file", &[root.clone()], 10).is_empty());
        assert!(store.query("renamed", &[root.clone()], 10).is_empty());
    }
}
