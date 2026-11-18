//! SQLite persistence for the filename index.
//!
//! Everything-style name search needs one thing from storage: answer "which
//! paths under these roots have a name containing this text" without walking
//! the disk. The store keeps a flat `entries` table (path, name, parent,
//! is_dir) with the path as primary key and answers queries with a plain
//! SQL `LIKE` scan — no FTS, no extra tables. A full-volume index is a few
//! hundred thousand rows; a covered scan of that size is faster than even a
//! shallow directory walk, and the alternative (FTS5) buys ranking we do not
//! need because `fs::local::search` already ranks candidates.
//!
//! One connection lives behind a `Mutex`, and every mutation is a single
//! transaction: a crawl batch of thousands of entries either lands whole or
//! not at all, so a killed dae never leaves a half-populated index. WAL is
//! enabled so reads never block behind a crawl's write transaction. Cheap
//! queries run on the caller's blocking thread; nothing here spawns one.
//!
//! Paths are the canonical form pinned by the module contract: absolute,
//! native-separator, no trailing separator on folders (volume roots keep
//! theirs: `C:\`, `/`). Comparison case follows the platform's file system —
//! folded on Windows and macOS, verbatim on Linux — via [`fold`], so the
//! behaviour is identical whether a match is made in SQL or in [`is_under`].

use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// One indexed filesystem object.
///
/// `name` is the last path component; `parent` is the directory containing
/// it, `""` for a volume-root entry. Callers build these from a canonicalized
/// path — the store stores what it is given verbatim and never re-derives
/// the components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub parent: String,
    pub is_dir: bool,
}

/// The persistent index. Cheap to clone: every clone shares one connection.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Opens/creates `<dir>/file-index.db`, creating the tables and enabling
    /// WAL. The directory is created if it does not exist yet.
    pub fn open(dir: &Path) -> rusqlite::Result<Store> {
        // Connection::open reports the failure itself if creating the
        // directory did not succeed.
        let _ = std::fs::create_dir_all(dir);
        let conn = Connection::open(dir.join("file-index.db"))?;
        // WAL keeps name queries answering while a crawl's batches commit.
        // The pragma returns the mode it selected; anything but "wal" means
        // the volume cannot use it (a network share), which is fine.
        let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // SQLite's LIKE folds ASCII case on every platform by default; on
        // Linux the file system does not, so pin the operator to match it.
        #[cfg(not(any(windows, target_os = "macos")))]
        conn.pragma_update(None, "case_sensitive_like", true)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries(
                 path   TEXT PRIMARY KEY,
                 name   TEXT NOT NULL,
                 parent TEXT NOT NULL,
                 is_dir INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS roots(
                 root    TEXT PRIMARY KEY,
                 covered INTEGER NOT NULL,
                 touched INTEGER NOT NULL
             );",
        )?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Atomic batch upsert. An empty batch commits nothing and succeeds.
    pub fn upsert(&self, batch: &[Entry]) -> rusqlite::Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO entries VALUES(?1, ?2, ?3, ?4)")?;
            for entry in batch {
                stmt.execute(params![
                    entry.path,
                    entry.name,
                    entry.parent,
                    i64::from(entry.is_dir)
                ])?;
            }
        }
        tx.commit()
    }

    /// Atomic batch delete by exact path. Paths that are not indexed are
    /// skipped, not an error — a watcher may report a removal for something
    /// the crawl never reached.
    pub fn remove(&self, paths: &[String]) -> rusqlite::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM entries WHERE path = ?1")?;
            for path in paths {
                stmt.execute(params![path])?;
            }
        }
        tx.commit()
    }

    /// Moves a subtree: `from` itself and everything under it is re-pathed to
    /// `to`. The moved entry's own `name`/`parent` are fixed up, direct
    /// children get `parent = to`, and deeper descendants keep their relative
    /// shape. Renaming to itself is a no-op. Entries not under `from` are
    /// left alone.
    pub fn rename(&self, from: &str, to: &str) -> rusqlite::Result<()> {
        if fold(from) == fold(to) {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let (pred, exact, pattern) = subtree_predicates(from);
        let rows: Vec<(String, String, bool)> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT path, parent, is_dir FROM entries WHERE {pred}"
            ))?;
            let found = stmt
                .query_map(params![exact, pattern], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            found
        };
        if rows.is_empty() {
            return tx.commit();
        }
        // The moved entry's own name and parent come from the destination,
        // not from the source spelling.
        let (to_parent, to_name) = split_last(to);
        let mut updates: Vec<Entry> = Vec::with_capacity(rows.len());
        // Descendants are stripped against `from` plus its separator, so a
        // suffix never starts with one and [`join_under` glues exactly one.
        let from_sep = if from.ends_with(|c| is_sep(c)) {
            from.to_owned()
        } else {
            format!("{from}{}", std::path::MAIN_SEPARATOR)
        };
        let from_folded = fold(from);
        for (path, parent, is_dir) in rows {
            let suffix = match strip_prefix_folded(&path, &from_sep) {
                Some(suffix) => suffix,
                // The moved entry itself: `from` verbatim, no separator.
                None if fold(&path) == from_folded => "",
                // Matched the subtree predicate but not the prefix strip:
                // only possible if the caller passed a non-canonical `from`.
                None => continue,
            };
            if suffix.is_empty() {
                updates.push(Entry {
                    path: to.to_owned(),
                    name: to_name.clone(),
                    parent: to_parent.clone(),
                    is_dir,
                });
                continue;
            }
            updates.push(Entry {
                path: join_under(to, suffix),
                name: path
                    .rsplit(|c| is_sep(c))
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                parent: if fold(&parent) == from_folded {
                    // A direct child's parent is the destination itself.
                    to.to_owned()
                } else {
                    match strip_prefix_folded(&parent, &from_sep) {
                        Some(parent_suffix) => join_under(to, parent_suffix),
                        // Parent outside the subtree cannot happen for a
                        // path inside it; keep the row's shape untouched.
                        None => continue,
                    }
                },
                is_dir,
            });
        }
        {
            let mut stmt = tx.prepare(&format!("DELETE FROM entries WHERE {pred}"))?;
            stmt.execute(params![exact, pattern])?;
            let mut upsert = tx.prepare("INSERT OR REPLACE INTO entries VALUES(?1, ?2, ?3, ?4)")?;
            for entry in &updates {
                upsert.execute(params![
                    entry.path,
                    entry.name,
                    entry.parent,
                    i64::from(entry.is_dir)
                ])?;
            }
        }
        tx.commit()
    }

    /// Paths whose *name* contains `text` (case per platform), under one of
    /// `roots`, at most `limit` of them. Scanning stops as soon as the limit
    /// is reached — on a volume-wide index the cost is reaching the matches,
    /// not counting past them. Errors are logged, never propagated into the
    /// search hot path; a failing query answers with whatever it collected.
    pub fn query(&self, text: &str, roots: &[String], limit: usize) -> Vec<String> {
        if limit == 0 || roots.is_empty() || text.is_empty() {
            return Vec::new();
        }
        let pattern = format!("%{}%", escape_like(&fold(text)));
        let conn = self.lock();
        let mut out = Vec::new();
        let mut scan = match conn.prepare(&name_scan_sql()) {
            Ok(scan) => scan,
            Err(error) => {
                log::warn!("file index query prepare failed: {error}");
                return out;
            }
        };
        let rows = match scan.query_map(params![pattern], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("file index query failed: {error}");
                return out;
            }
        };
        for row in rows {
            match row {
                Ok(path) => {
                    if roots.iter().any(|root| is_under(&path, root)) {
                        out.push(path);
                        if out.len() == limit {
                            break;
                        }
                    }
                }
                Err(error) => {
                    log::warn!("file index row read failed: {error}");
                    break;
                }
            }
        }
        out
    }

    /// Records whether a root's initial population finished. Uncovering does
    /// not delete entries — the re-crawl's upserts replace them.
    pub fn mark_root(&self, root: &str, covered: bool) -> rusqlite::Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO roots(root, covered, touched) VALUES(?1, ?2, ?3)
             ON CONFLICT(root) DO UPDATE
                 SET covered = excluded.covered, touched = excluded.touched",
            params![root, i64::from(covered), now_millis()],
        )?;
        Ok(())
    }

    /// Every covered root. Whether a root is *usable* for a query path is
    /// the caller's call — that is exactly what [`is_under`] answers.
    pub fn covered_roots(&self) -> Vec<String> {
        let conn = self.lock();
        match conn
            .prepare("SELECT root FROM roots WHERE covered != 0 ORDER BY root")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            }) {
            Ok(roots) => roots,
            Err(error) => {
                log::warn!("file index root read failed: {error}");
                Vec::new()
            }
        }
    }

    /// Deletes a root's flag *and* every entry under it. The next crawl
    /// starts from a blank slate for that root.
    pub fn forget_root(&self, root: &str) -> rusqlite::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let (pred, exact, pattern) = subtree_predicates(root);
        tx.execute(
            &format!("DELETE FROM entries WHERE {pred}"),
            params![exact, pattern],
        )?;
        tx.execute("DELETE FROM roots WHERE root = ?1", params![root])?;
        tx.commit()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the connection would poison it for every
        // clone; rusqlite errors are values, so the only realistic poison
        // is a bug, and carrying on beats refusing every future query.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Whether `path` is `root` itself or something inside it: `root` followed by
/// a separator, then the rest. Volume roots (`C:\`, `/`) keep their trailing
/// separator, so "under the root" is the plain prefix test there. Comparison
/// follows the platform's case rules, because the file system's do.
pub(crate) fn is_under(path: &str, root: &str) -> bool {
    let path = fold(path);
    let root = fold(root);
    let root = root.trim_end_matches(|c| is_sep(c));
    if root.is_empty() {
        // Root is a bare separator: every absolute path is under it.
        return path.starts_with(|c: char| is_sep(c));
    }
    if path == root {
        return true;
    }
    match path.get(..root.len()) {
        Some(prefix) if prefix == root => path[root.len()..].starts_with(|c: char| is_sep(c)),
        _ => false,
    }
}

/// Folds `text` for comparison: case-insensitive on the platforms whose file
/// systems are (Windows, macOS), verbatim elsewhere. `#[cfg]`-selected so
/// the behaviour is pinned per platform rather than by a runtime check.
#[cfg(any(windows, target_os = "macos"))]
fn fold(text: &str) -> String {
    text.to_lowercase()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn fold(text: &str) -> String {
    text.to_owned()
}

/// Whether `c` is a path separator. Native-separator canonical paths never
/// mix separators, but the incoming query side (roots, `from`/`to`) is
/// user-adjacent, and on Windows accepting both spellings costs nothing.
fn is_sep(c: char) -> bool {
    if cfg!(windows) {
        c == '\\' || c == '/'
    } else {
        c == '/'
    }
}

/// Splits a canonical path into its containing directory and last component.
/// A volume root (`C:\`, `/`) has no parent: the answer is `("", path)`.
fn split_last(path: &str) -> (String, String) {
    match path.rfind(|c| is_sep(c)) {
        None => (String::new(), path.to_owned()),
        Some(idx) => {
            let name = &path[idx + 1..];
            if name.is_empty() {
                return (String::new(), path.to_owned());
            }
            let before = &path[..idx];
            let parent = if before.is_empty() || is_drive_prefix(before) {
                // The separator belongs to the parent: `/a` sits in `/`,
                // `C:\Users` sits in `C:\`.
                path[..idx + 1].to_owned()
            } else {
                before.to_owned()
            };
            (parent, name.to_owned())
        }
    }
}

/// `C:` — the tail of a Windows volume root, whose separator is part of it.
fn is_drive_prefix(text: &str) -> bool {
    cfg!(windows) && text.len() == 2 && text.ends_with(':')
}

/// Glues `suffix` (no leading separator) onto `to` exactly once.
fn join_under(to: &str, suffix: &str) -> String {
    if to.ends_with(|c: char| is_sep(c)) {
        format!("{to}{suffix}")
    } else {
        format!("{to}{}{suffix}", std::path::MAIN_SEPARATOR)
    }
}

/// `path` minus the `prefix` characters, compared folded; `None` when `path`
/// does not start with `prefix`. Only used by [`Store::rename`], where the
/// source spelling of a path can differ in case from what the watcher
/// reported.
fn strip_prefix_folded<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let mut end = 0usize;
    for want in prefix.chars() {
        let got = path[end..].chars().next()?;
        end += got.len_utf8();
        if fold(&path[end - got.len_utf8()..end]) != fold(&want.to_string()) {
            return None;
        }
    }
    Some(&path[end..])
}

/// The SQL predicate + bindings matching `root` itself and everything under
/// it, with the platform's case rules. The pattern is built from the folded
/// spelling on Windows/macOS (the scan folds the column with `lower()` to
/// meet it) and from the verbatim one on Linux.
fn subtree_predicates(root: &str) -> (&'static str, String, String) {
    let ends_with_sep = root.ends_with(|c: char| is_sep(c));
    let pattern = if ends_with_sep {
        format!("{}%", escape_like(&fold(root)))
    } else {
        let sep = if cfg!(windows) { "\\" } else { "/" };
        format!("{}{}%", escape_like(&fold(root)), escape_like(sep))
    };
    #[cfg(any(windows, target_os = "macos"))]
    let sql = "lower(path) = ?1 OR lower(path) LIKE ?2 ESCAPE '\\'";
    #[cfg(not(any(windows, target_os = "macos")))]
    let sql = "path = ?1 OR path LIKE ?2 ESCAPE '\\'";
    (sql, fold(root), pattern)
}

/// The name-contains scan, with the platform's case rules on the column.
fn name_scan_sql() -> String {
    #[cfg(any(windows, target_os = "macos"))]
    let column = "lower(name)";
    #[cfg(not(any(windows, target_os = "macos")))]
    let column = "name";
    format!("SELECT path FROM entries WHERE {column} LIKE ?1 ESCAPE '\\'")
}

/// Escapes LIKE's wildcard and escape characters so user text matches
/// literally. `%`, `_` and the escape character `\` itself.
fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Entry, Store, fold, is_under, split_last};

    /// Case rules are a platform property, so the expectations are selected
    /// the same way the code is.
    fn folds_case() -> bool {
        cfg!(any(windows, target_os = "macos"))
    }

    fn sep() -> char {
        if cfg!(windows) { '\\' } else { '/' }
    }

    /// A volume root in canonical form, per platform.
    fn root() -> String {
        if cfg!(windows) {
            "C:\\".to_owned()
        } else {
            "/".to_owned()
        }
    }

    /// `root()` + components, native separators: the canonical form of the
    /// paths every caller stores.
    fn path(parts: &[&str]) -> String {
        let mut path = root();
        for part in parts {
            path.push_str(part);
            path.push(sep());
        }
        path.trim_end_matches(sep()).to_owned()
    }

    /// An entry for `path`, with the components derived the way the feeder
    /// derives them.
    fn entry(parts: &[&str], is_dir: bool) -> Entry {
        let path = path(parts);
        let (parent, name) = split_last(&path);
        Entry {
            path,
            name,
            parent,
            is_dir,
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::TempDir::new().expect("create a temp dir");
        let store = Store::open(dir.path()).expect("open the store");
        (dir, store)
    }

    #[test]
    fn upsert_then_query_round_trips() {
        let (_dir, store) = store();
        store
            .upsert(&[
                entry(&["proj"], true),
                entry(&["proj", "readme.md"], false),
                entry(&["proj", "src"], true),
                entry(&["proj", "src", "main.rs"], false),
            ])
            .expect("upsert the batch");
        let found = store.query("readme", &[root()], 100);
        assert_eq!(found, vec![path(&["proj", "readme.md"])]);
        // The query is on the *name*, not the whole path: only the
        // directory itself is called "proj".
        assert_eq!(store.query("proj", &[root()], 100).len(), 1);
        // A re-upsert replaces rather than duplicates.
        let mut file = entry(&["proj", "readme.md"], false);
        file.name = "Readme.md".to_owned();
        store.upsert(&[file]).expect("re-upsert");
        assert_eq!(store.query("readme", &[root()], 100).len(), 1);
    }

    #[test]
    fn empty_batches_are_ok() {
        let (_dir, store) = store();
        store.upsert(&[]).expect("an empty upsert commits nothing");
        store.remove(&[]).expect("an empty removal commits nothing");
        assert!(store.query("anything", &[root()], 10).is_empty());
    }

    #[test]
    fn batch_atomicity_lands_whole() {
        let (_dir, store) = store();
        let batch: Vec<Entry> = (0..50)
            .map(|i| {
                let mut path = root();
                path.push_str(&format!("dir-{i}.txt"));
                let (parent, name) = split_last(&path);
                Entry {
                    path,
                    name,
                    parent,
                    is_dir: false,
                }
            })
            .collect();
        store.upsert(&batch).expect("commit the batch");
        // Every row of the batch is visible after the single call.
        for entry in &batch {
            let found = store.query(&entry.name, &[root()], 100);
            assert_eq!(found, vec![entry.path.clone()], "batch row landed");
        }
    }

    #[test]
    fn remove_deletes_exact_paths_only() {
        let (_dir, store) = store();
        store
            .upsert(&[entry(&["a.txt"], false), entry(&["b.txt"], false)])
            .expect("upsert");
        store.remove(&[path(&["a.txt"])]).expect("remove one path");
        assert!(store.query("a.txt", &[root()], 10).is_empty());
        assert_eq!(store.query("b.txt", &[root()], 10).len(), 1);
        // Removing an unknown path is not an error.
        store.remove(&[path(&["nope.txt"])]).expect("remove a miss");
    }

    #[test]
    fn open_creates_the_database_file() {
        let dir = tempfile::TempDir::new().expect("create a temp dir");
        let nested = dir.path().join("deep");
        let store = Store::open(&nested).expect("open in a missing directory");
        assert!(nested.join("file-index.db").exists());
        // And it is usable immediately.
        store
            .upsert(&[entry(&["x.txt"], false)])
            .expect("upsert into the fresh store");
        assert_eq!(store.query("x.txt", &[root()], 10).len(), 1);
    }

    #[test]
    fn rename_reparents_the_subtree() {
        let (_dir, store) = store();
        let (from, to) = (path(&["from"]), path(&["to"]));
        store
            .upsert(&[
                entry(&["from"], true),
                entry(&["from", "child.txt"], false),
                entry(&["from", "sub"], true),
                entry(&["from", "sub", "grand.txt"], false),
                entry(&["outside.txt"], false),
            ])
            .expect("upsert the tree");
        store.rename(&from, &to).expect("rename the subtree");

        let moved = store.query("child", &[root()], 100);
        assert_eq!(moved, vec![path(&["to", "child.txt"])]);
        let grand = store.query("grand", &[root()], 100);
        assert_eq!(grand, vec![path(&["to", "sub", "grand.txt"])]);
        // Nothing under the old name is left, and the untouched row stays.
        assert!(store.query("child", &[from], 100).is_empty());
        assert_eq!(store.query("outside", &[root()], 100).len(), 1);

        // The moved directory's own row carries the destination's name and
        // parent; the direct child's parent is the destination itself.
        let (parent_of_to, name_of_to) = split_last(&to);
        let conn = store.lock();
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT name, parent, is_dir FROM entries WHERE path = ?1",
                [&to],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the moved directory row exists");
        assert_eq!(row.0, name_of_to);
        assert_eq!(row.1, parent_of_to);
        assert_eq!(row.2, 1, "a renamed directory stays a directory");
        let child_parent: String = conn
            .query_row(
                "SELECT parent FROM entries WHERE path = ?1",
                [path(&["to", "child.txt"])],
                |row| row.get(0),
            )
            .expect("the direct child row exists");
        assert_eq!(child_parent, to);
        drop(conn);
    }

    #[test]
    fn rename_of_a_volume_root_repaths_descendants() {
        let (_dir, store) = store();
        let to = format!("{}moved{}", root(), sep());
        store
            .upsert(&[entry(&["sub"], true), entry(&["sub", "file.txt"], false)])
            .expect("upsert");
        store
            .rename(&root(), &to)
            .expect("rename from the volume root");
        // The descendants keep their relative shape under the new root,
        // with exactly one separator glued on.
        let found = store.query("file", &[to.clone()], 100);
        assert_eq!(found, vec![format!("{to}sub{}file.txt", sep())]);
        let direct = store.query("sub", &[to.clone()], 100);
        assert_eq!(direct, vec![format!("{to}sub")]);
    }

    #[test]
    fn rename_to_itself_is_a_no_op() {
        let (_dir, store) = store();
        let dir = entry(&["dir"], true);
        store.upsert(&[dir.clone()]).expect("upsert");
        store
            .rename(&dir.path, &dir.path)
            .expect("rename to itself");
        assert_eq!(store.query("dir", &[root()], 10).len(), 1);
    }

    #[test]
    fn query_filters_by_roots_and_stops_at_the_limit() {
        let (_dir, store) = store();
        let mut batch = Vec::new();
        for i in 0..10 {
            let mut p = root();
            p.push_str(&format!("match-{i}.txt"));
            let (parent, name) = split_last(&p);
            batch.push(Entry {
                path: p,
                name,
                parent,
                is_dir: false,
            });
        }
        batch.push(entry(&["elsewhere", "match-other.txt"], false));
        store.upsert(&batch).expect("upsert");

        let nowhere = path(&["nowhere"]);
        assert_eq!(store.query("match-", &[nowhere], 100).len(), 0);
        assert_eq!(store.query("match-", &[root()], 3).len(), 3);
        assert_eq!(store.query("match-", &[root()], 0).len(), 0);
        // No roots answers nothing, whatever the text matches.
        assert!(store.query("match-", &[], 10).is_empty());
    }

    #[test]
    fn covered_roots_mark_and_forget() {
        let (_dir, store) = store();
        let a = path(&["a"]);
        let b = path(&["b"]);
        store
            .upsert(&[entry(&["a", "x.txt"], false), entry(&["b", "y.txt"], false)])
            .expect("upsert");
        assert!(store.covered_roots().is_empty());

        store.mark_root(&a, true).expect("cover a");
        store.mark_root(&b, false).expect("uncover b");
        assert_eq!(store.covered_roots(), vec![a.clone()]);

        store.mark_root(&a, false).expect("uncover a");
        assert!(store.covered_roots().is_empty());
        // Uncovering keeps the entries; the re-crawl replaces them.
        assert_eq!(store.query("x.txt", &[root()], 10).len(), 1);

        store.mark_root(&b, true).expect("cover b");
        store.forget_root(&b).expect("forget b");
        assert!(store.covered_roots().is_empty());
        assert!(store.query("y.txt", &[root()], 10).is_empty());
        // Forgetting one root leaves the other's entries alone.
        assert_eq!(store.query("x.txt", &[root()], 10).len(), 1);
    }

    #[test]
    fn case_folding_is_pinned_per_platform() {
        if folds_case() {
            assert_eq!(fold("ReadMe.TXT"), "readme.txt");
        } else {
            assert_eq!(fold("ReadMe.TXT"), "ReadMe.TXT");
        }

        let (_dir, store) = store();
        store
            .upsert(&[entry(&["Report.PDF"], false)])
            .expect("upsert");
        let hits = store.query("report", &[root()], 10);
        if folds_case() {
            assert_eq!(hits, vec![path(&["Report.PDF"])]);
        } else {
            assert!(hits.is_empty());
        }
    }

    #[test]
    fn like_wildcards_in_names_match_literally() {
        let (_dir, store) = store();
        store
            .upsert(&[
                entry(&["weird%name.txt"], false),
                entry(&["plain.txt"], false),
                entry(&["list_a.txt"], false),
                entry(&["listba.txt"], false),
            ])
            .expect("upsert");
        // A query of "%" must mean a literal percent sign, not every row.
        assert_eq!(store.query("%", &[root()], 10).len(), 1);
        assert_eq!(store.query("weird%na", &[root()], 10).len(), 1);
        assert_eq!(store.query("plain", &[root()], 10).len(), 1);
        // `_` is a wildcard too; escaped, it matches only itself.
        assert_eq!(store.query("list_a", &[root()], 10).len(), 1);
    }

    #[test]
    fn is_under_tests_separator_boundaries() {
        // Slash-form paths are valid on both platforms for this helper.
        assert!(is_under("/home/user", "/home"));
        assert!(is_under("/home/user", "/home/"));
        assert!(is_under("/home", "/home"));
        assert!(is_under("/home/user/docs", "/home/user"));
        // A sibling that merely shares a prefix is not inside.
        assert!(!is_under("/homework", "/home"));
        assert!(!is_under("/home/user", "/home/usr"));
        // The bare-separator root contains everything absolute.
        assert!(is_under("/home", "/"));
        assert!(!is_under("relative", "/"));
    }

    #[test]
    fn is_under_follows_platform_case() {
        if folds_case() {
            assert!(is_under("/Home/User", "/home"));
        } else {
            assert!(!is_under("/Home/User", "/home"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn is_under_handles_windows_roots() {
        assert!(is_under("C:\\Users", "C:\\"));
        assert!(is_under("C:\\Users\\me", "C:\\Users"));
        assert!(is_under("C:\\", "C:\\"));
        assert!(!is_under("D:\\Users", "C:\\"));
        assert!(!is_under("/Users", "C:\\"));
        assert!(is_under("C:/Users", "C:\\"));
    }

    #[test]
    fn split_last_derives_parent_and_name() {
        let (parent, name) = split_last(&path(&["home", "user", "file.txt"]));
        assert_eq!(parent, path(&["home", "user"]));
        assert_eq!(name, "file.txt");

        let (parent, name) = split_last(&path(&["file.txt"]));
        assert_eq!(parent, root());
        assert_eq!(name, "file.txt");

        let (parent, name) = split_last(&root());
        assert_eq!(parent, "");
        assert_eq!(name, root());
    }
}
