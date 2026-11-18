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

use crate::fs::types::ContentSearchMatch;
use crate::paths;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rows older than this are dropped when the store opens.
const MAX_ROW_AGE_DAYS: i64 = 30;

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

/// The cached matches for one file under one query, or `None` when this
/// (file bytes, query) pair has never been searched. Never fails visibly:
/// an unavailable or corrupt store is an absent cache.
pub fn lookup(
    path: &str,
    mtime_ms: i64,
    size: u64,
    key: &QueryKey,
) -> Option<Vec<ContentSearchMatch>> {
    let store = store()?;
    let Ok(conn) = store.conn.lock() else {
        return None;
    };
    conn.query_row(
        "SELECT matches FROM matches
         WHERE path = ?1 AND mtime_ms = ?2 AND size = ?3
           AND query = ?4 AND is_regex = ?5 AND case_sensitive = ?6 AND file_filter = ?7",
        params![
            path,
            mtime_ms,
            size as i64,
            key.query.as_ref(),
            key.is_regex as i64,
            key.case_sensitive as i64,
            key.file_filter.as_deref().unwrap_or("")
        ],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|encoded| serde_json::from_str(&encoded).ok())
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
        let matches = vec![one_match(3)];
        let key = key_of("hello");
        store_rows(&[CacheRow {
            path: "C:\\tmp\\a.txt",
            mtime_ms: 1234,
            size: 10,
            key: &key,
            matches: &matches,
        }]);

        assert_eq!(lookup("C:\\tmp\\a.txt", 1234, 10, &key), Some(matches));
        // A different mtime, size, or query is a different key: no reuse.
        assert_eq!(lookup("C:\\tmp\\a.txt", 1235, 10, &key), None);
        assert_eq!(lookup("C:\\tmp\\a.txt", 1234, 11, &key), None);
        assert_eq!(lookup("C:\\tmp\\a.txt", 1234, 10, &key_of("other")), None);
    }

    #[test]
    fn an_empty_verdict_round_trips_as_a_hit() {
        cache_dir();
        let key = key_of("needle");
        store_rows(&[CacheRow {
            path: "C:\\tmp\\b.txt",
            mtime_ms: 5,
            size: 7,
            key: &key,
            matches: &[],
        }]);
        assert_eq!(lookup("C:\\tmp\\b.txt", 5, 7, &key), Some(Vec::new()));
    }
}
