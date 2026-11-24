//! Per-file content-search result cache.
//!
//! [`search_file_contents_sync`] re-reads every file a walk visits, even when
//! the query is one the user ran a minute ago and only one file changed since.
//! This store answers the repeat: a row is keyed by `(path, mtime, size,
//! query, flags)`, so a file whose mtime and size are unchanged provably holds
//! the same bytes and its last matches — including the empty "no match"
//! verdict, which is where most of the reading time goes — are reused without
//! opening the file.
//!
//! Semantics are exact by construction: the key pins the bytes (mtime+size)
//! and the query, so a hit replays precisely what a fresh search of that file
//! produced. Failures are silent and non-fatal — an unopenable store just
//! means searches keep paying the reads, which is the status quo, not a
//! dialog the UI owes anyone (the same contract as `fs::file_index`).
//!
//! Growth is bounded by age, not size: rows carry their insertion time and a
//! row nobody re-created for 30 days is dropped on open. Stale rows (the file
//! changed, so its old mtime no longer matches) age out the same way; they
//! cost one indexed lookup each and nothing else.

use crate::fs::types::{ContentSearchMatch, path_to_string};
use crate::paths;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rows older than this are dropped when the store opens.
const MAX_ROW_AGE_DAYS: i64 = 30;

/// Cap on the rows one search loads at once, mirroring the 250,000-row pending
/// cap `content_search` already applies: a tree of millions of files must not
/// turn the reuse map into the search's memory bill. Rows past the cap are
/// simply not reused — the walk searches those files fresh, which is the cache
/// being absent, not a wrong answer.
const MAX_LOADED_ROWS: usize = 250_000;

/// The query half of a cache key: everything besides the file's identity
/// that decides what the matches look like. Owned, so the search's worker
/// closures can carry one per thread without borrowing the borrowed params.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QueryKey {
    pub query: Box<str>,
    pub is_regex: bool,
    pub case_sensitive: bool,
    /// The raw filter string, so `"rs"` and `".rs"` stay different keys even
    /// though they build the same types.
    pub file_filter: Option<Box<str>>,
}

/// One file's verdict to persist: the key it was searched under and the
/// matches found. `matches` empty is itself the cached answer.
pub struct CacheRow<'a> {
    pub path: &'a str,
    /// Milliseconds since the epoch, as the walker's stat reported it.
    pub mtime_ms: i64,
    pub size: u64,
    pub key: &'a QueryKey,
    pub matches: &'a [ContentSearchMatch],
}

/// The persistent cache. Cheap to clone: every clone shares one connection.
struct Store {
    conn: Arc<Mutex<Connection>>,
}

fn store() -> Option<&'static Store> {
    static STORE: LazyLock<Option<Store>> = LazyLock::new(|| {
        // Tests point the store at a temp dir: caching a fixture into the
        // developer's real cache (and reading it back in a later test) is a
        // side effect no test should be allowed to have.
        #[cfg(test)]
        if let Some(dir) = test_dir() {
            return open(&dir).ok();
        }
        let dir: PathBuf = paths::config_dir().ok()?;
        match open(&dir) {
            Ok(store) => Some(store),
            Err(error) => {
                log::warn!(target: "dae::content_cache", "cache unavailable: {error}");
                None
            }
        }
    });
    STORE.as_ref()
}

/// Where tests want the store, taking precedence over the real config dir.
/// One slot: the store opens once, so the first directory a test sets wins
/// for the whole process; rows are keyed by absolute path, so distinct
/// fixtures under one shared dir cannot see each other's rows.
#[cfg(test)]
static TEST_DIR: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));

#[cfg(test)]
fn test_dir() -> Option<PathBuf> {
    TEST_DIR.lock().ok()?.clone()
}

/// Registers the directory tests want the cache in, before any search opens it.
#[cfg(test)]
pub(crate) fn use_test_dir(dir: PathBuf) {
    if let Ok(mut slot) = TEST_DIR.lock() {
        slot.get_or_insert(dir);
    }
}

fn open(dir: &Path) -> rusqlite::Result<Store> {
    let conn = Connection::open(dir.join("content-search-cache.db"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS matches (
             path       TEXT NOT NULL,
             mtime_ms   INTEGER NOT NULL,
             size       INTEGER NOT NULL,
             query      TEXT NOT NULL,
             is_regex   INTEGER NOT NULL,
             case_sensitive INTEGER NOT NULL,
             file_filter TEXT NOT NULL,
             created_ms INTEGER NOT NULL,
             matches    TEXT NOT NULL,
             PRIMARY KEY (path, mtime_ms, size, query, is_regex, case_sensitive, file_filter)
         );",
    )?;
    let cutoff = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0))
        - MAX_ROW_AGE_DAYS * 24 * 60 * 60 * 1000;
    conn.execute("DELETE FROM matches WHERE created_ms < ?1", params![cutoff])?;
    Ok(Store {
        conn: Arc::new(Mutex::new(conn)),
    })
}

/// One file's verdict as loaded for one search: the identity it was written
/// under, and the matches still encoded. Decoding waits for a hit, so a row
/// this walk does not visit costs a map slot and nothing else.
struct CachedRow {
    mtime_ms: i64,
    size: u64,
    encoded_matches: Box<str>,
}

/// Every verdict one search can reuse, loaded by a single statement. Empty
/// when the store is unavailable, which is the same as a cache that has never
/// seen the tree.
#[derive(Default)]
pub struct Lookups {
    rows: HashMap<Box<str>, CachedRow>,
}

impl Lookups {
    /// The cached answer for one file, or `None` when this (path, bytes,
    /// query) was never stored. `Some(vec![])` is a cached "no matches".
    pub fn matches(&self, path: &str, mtime_ms: i64, size: u64) -> Option<Vec<ContentSearchMatch>> {
        let row = self.rows.get(path)?;
        if row.mtime_ms != mtime_ms || row.size != size {
            return None;
        }
        serde_json::from_str(&row.encoded_matches).ok()
    }
}

/// Loads, in one statement, the verdicts cached for `key` below `root`.
///
/// The `path` range rides the primary key's leading column, so SQLite reads
/// the returned rows and no others; the upper bound is the separator-terminated
/// root with the highest code point appended, which keeps a sibling that only
/// shares the root's spelling out of the map. [`Lookups::matches`] still
/// compares the exact path, so a spelling the range misses is a cache miss,
/// never a wrong answer.
pub fn load(root: &Path, key: &QueryKey) -> Lookups {
    let mut lookups = Lookups::default();
    let Some(store) = store() else {
        return lookups;
    };
    let Ok(conn) = store.conn.lock() else {
        return lookups;
    };
    let Ok(mut statement) = conn.prepare_cached(
        "SELECT path, mtime_ms, size, matches FROM matches
         WHERE query = ?1 AND is_regex = ?2 AND case_sensitive = ?3 AND file_filter = ?4
           AND path >= ?5 AND path < ?6
         LIMIT ?7",
    ) else {
        return lookups;
    };

    let prefix = path_prefix(root);
    let upper = format!("{prefix}\u{10FFFF}");
    let Ok(mut rows) = statement.query(params![
        key.query.as_ref(),
        key.is_regex as i64,
        key.case_sensitive as i64,
        key.file_filter.as_deref().unwrap_or(""),
        prefix,
        upper,
        MAX_LOADED_ROWS as i64,
    ]) else {
        return lookups;
    };

    loop {
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) | Err(_) => break,
        };
        let (Ok(path), Ok(mtime_ms), Ok(size), Ok(encoded_matches)) = (
            row.get::<_, String>(0),
            row.get::<_, i64>(1),
            row.get::<_, i64>(2),
            row.get::<_, String>(3),
        ) else {
            continue;
        };
        lookups.rows.insert(
            path.into_boxed_str(),
            CachedRow {
                mtime_ms,
                size: size as u64,
                encoded_matches: encoded_matches.into_boxed_str(),
            },
        );
    }

    lookups
}

/// The spelling every stored path under `root` starts with, separator
/// included, so `root` and a sibling named `root-other` do not share a range.
fn path_prefix(root: &Path) -> String {
    let mut prefix = path_to_string(root);
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    prefix
}

/// Persists one search's verdicts in a single transaction: either the whole
/// batch lands or none of it does. Inserted-or-ignored, so a re-search under
/// an identical key is a no-op.
pub fn store_rows(rows: &[CacheRow<'_>]) {
    let Some(store) = store() else {
        return;
    };
    let Ok(mut conn) = store.conn.lock() else {
        return;
    };
    let Ok(transaction) = conn.transaction() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    let Ok(mut statement) = transaction.prepare_cached(
        "INSERT OR IGNORE INTO matches
             (path, mtime_ms, size, query, is_regex, case_sensitive, file_filter, created_ms, matches)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    ) else {
        return;
    };
    for row in rows {
        // An unencodable row is skipped, not fatal: it just stays uncached.
        let Ok(encoded) = serde_json::to_string(row.matches) else {
            continue;
        };
        let _ = statement.execute(params![
            row.path,
            row.mtime_ms,
            row.size as i64,
            row.key.query.as_ref(),
            row.key.is_regex as i64,
            row.key.case_sensitive as i64,
            row.key.file_filter.as_deref().unwrap_or(""),
            now,
            encoded,
        ]);
    }
    drop(statement);
    let _ = transaction.commit();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_of(query: &str) -> QueryKey {
        QueryKey {
            query: query.into(),
            is_regex: false,
            case_sensitive: false,
            file_filter: None,
        }
    }

    fn cache_dir() -> PathBuf {
        let dir = tempfile::tempdir().expect("cache temp dir");
        let path = dir.keep();
        use_test_dir(path.clone());
        path
    }

    fn one_match(line: u64) -> ContentSearchMatch {
        ContentSearchMatch {
            line_number: line,
            line_text: "hello world".into(),
            ranges: vec![(0, 5)],
        }
    }

    #[test]
    fn a_row_is_readable_under_exactly_its_key() {
        cache_dir();
        let root = std::env::temp_dir().join("dae-content-cache-key");
        let path = path_to_string(&root.join("a.txt"));
        let sibling_root = root.with_file_name("dae-content-cache-key-other");
        let sibling = path_to_string(&sibling_root.join("a.txt"));
        let matches = vec![one_match(3)];
        let key = key_of("hello");
        for stored in [&path, &sibling] {
            store_rows(&[CacheRow {
                path: stored.as_str(),
                mtime_ms: 1234,
                size: 10,
                key: &key,
                matches: &matches,
            }]);
        }

        let lookups = load(&root, &key);
        assert_eq!(lookups.matches(&path, 1234, 10), Some(matches));
        // A different mtime, size, or query is a different key: no reuse.
        assert_eq!(lookups.matches(&path, 1235, 10), None);
        assert_eq!(lookups.matches(&path, 1234, 11), None);
        assert_eq!(load(&root, &key_of("other")).matches(&path, 1234, 10), None);
        // A sibling that only extends the root's spelling is not this tree.
        assert_eq!(lookups.matches(&sibling, 1234, 10), None);
    }

    #[test]
    fn an_empty_verdict_round_trips_as_a_hit() {
        cache_dir();
        let root = std::env::temp_dir().join("dae-content-cache-empty");
        let path = path_to_string(&root.join("b.txt"));
        let key = key_of("needle");
        store_rows(&[CacheRow {
            path: &path,
            mtime_ms: 5,
            size: 7,
            key: &key,
            matches: &[],
        }]);
        assert_eq!(load(&root, &key).matches(&path, 5, 7), Some(Vec::new()));
    }
}
