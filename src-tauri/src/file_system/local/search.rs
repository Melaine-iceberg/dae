use super::directory::{entry_kind, entry_state_flags, modified_at_millis};
use super::ignores;
use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    EntryKind, SearchEntry, SearchResponse, canonical_path, entry_kind_rank, path_to_string,
};
use ignore::{WalkBuilder, WalkState};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// Results the caller is shown.
const MAX_SEARCH_RESULTS: usize = 200;

/// Matches collected before the walk stops.
///
/// Deliberately larger than what is displayed: the walk is parallel, so the
/// order matches arrive in is arbitrary, and collecting exactly
/// [`MAX_SEARCH_RESULTS`] then ranking them — which is what this used to do —
/// means the shown set is "whichever 200 happened to arrive first". A query
/// that matches a dependency tree as well as the project therefore dropped
/// project hits at random. Collecting a bounded multiple first and ranking
/// afterwards costs nothing where matches are sparse (the walk's cost is
/// reaching the matches, not counting them) and is what makes the shown 200
/// the best-ranked ones.
const MAX_SEARCH_CANDIDATES: usize = 5 * MAX_SEARCH_RESULTS;

pub fn search_directory_sync(
    requested_path: PathBuf,
    query: &str,
    is_current: &(dyn Fn() -> bool + Send + Sync),
) -> Result<SearchResponse, FileSystemError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchResponse {
            entries: Vec::new(),
            truncated: false,
        });
    }

    // `canonical_path` rather than `canonicalize`: the walker hands this path to
    // `read_dir` as the base of every entry it yields, and `DirEntry::path()`
    // rebuilds the whole base when it carries the verbatim prefix (see
    // `types::canonical_path`). The walk pays that once per entry it visits.
    let path = canonical_path(&requested_path)?;
    if !fs::metadata(&path)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&path)));
    }

    let folded_query = query.to_lowercase();
    let shared = Arc::new(SearchShared {
        entries: Mutex::new(Vec::new()),
        capped: AtomicBool::new(false),
        is_current,
    });
    let root = Arc::new(path);

    let mut builder = WalkBuilder::new(root.as_path());
    builder.standard_filters(false).follow_links(false);
    // VCS and dependency directories are not what a name search is looking for;
    // see [`ignores`] for the list and why it is a descendant rule.
    builder.overrides(ignores::default_overrides(root.as_path())?);

    {
        let run_shared = Arc::clone(&shared);
        let run_root = Arc::clone(&root);
        let run_query: Arc<str> = Arc::from(folded_query.as_str());

        builder.build_parallel().run(move || {
            let shared = Arc::clone(&run_shared);
            let root = Arc::clone(&run_root);
            let folded_query = Arc::clone(&run_query);

            Box::new(move |result| {
                if shared.capped.load(AtomicOrdering::Relaxed) || !(shared.is_current)() {
                    return WalkState::Quit;
                }

                let entry = match result {
                    Ok(entry) => entry,
                    // Inaccessible descendants should not prevent useful partial results.
                    Err(_) => return WalkState::Continue,
                };

                if entry.depth() == 0 {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.to_lowercase().contains(&*folded_query) {
                    return WalkState::Continue;
                }

                let should_stop = {
                    let mut entries = shared.entries.lock().expect("search results lock poisoned");

                    if entries.len() >= MAX_SEARCH_CANDIDATES {
                        true
                    } else {
                        entries.push(build_search_entry(&entry, &name, &root));
                        false
                    }
                };

                if should_stop {
                    shared.capped.store(true, AtomicOrdering::Relaxed);
                    return WalkState::Quit;
                }

                WalkState::Continue
            })
        });
    }

    let candidates = shared
        .entries
        .lock()
        .expect("search results lock poisoned")
        .drain(..)
        .collect::<Vec<_>>();
    let capped = shared.capped.load(AtomicOrdering::Relaxed);

    // `truncated` says "there are matches you are not being shown", which is
    // true both when the walk was stopped early and when it finished with more
    // matches than the list carries.
    let truncated = capped || candidates.len() > MAX_SEARCH_RESULTS;

    Ok(SearchResponse {
        entries: rank_and_truncate(candidates, &folded_query),
        truncated,
    })
}

struct SearchShared<'a> {
    entries: Mutex<Vec<SearchEntry>>,
    /// Set when the walk stopped because it hit [`MAX_SEARCH_CANDIDATES`].
    capped: AtomicBool,
    is_current: &'a (dyn Fn() -> bool + Send + Sync),
}

fn build_search_entry(entry: &ignore::DirEntry, name: &str, root: &PathBuf) -> SearchEntry {
    let relative_path = entry.path().strip_prefix(root).unwrap_or(entry.path());
    let kind = entry
        .file_type()
        .map(entry_kind)
        .unwrap_or(EntryKind::Other);
    // With `follow_links(false)` this is the link's own metadata for
    // symlinks, unlike directory listings which follow the target.
    let metadata = entry.metadata().ok();
    let modified_at = metadata.as_ref().and_then(modified_at_millis);
    let size = matches!(&kind, EntryKind::File)
        .then(|| metadata.as_ref().map(|metadata| metadata.len()))
        .flatten();
    let (hidden, read_only) = metadata
        .as_ref()
        .map(|metadata| entry_state_flags(metadata, name))
        .unwrap_or_default();

    SearchEntry {
        name: name.to_owned(),
        path: path_to_string(entry.path()),
        relative_path: path_to_string(relative_path),
        kind,
        modified_at,
        size,
        hidden,
        read_only,
    }
}

/// The best [`MAX_SEARCH_RESULTS`] of `candidates`, in display order.
///
/// Each candidate's name is folded once here rather than inside the comparator:
/// the fold is what the *first* key is built from, so comparing computed it once
/// per comparison before — several thousand `to_lowercase` allocations to order
/// a few hundred rows.
fn rank_and_truncate(candidates: Vec<SearchEntry>, query: &str) -> Vec<SearchEntry> {
    let mut ranked: Vec<(String, SearchEntry)> = candidates
        .into_iter()
        .map(|entry| (entry.name.to_lowercase(), entry))
        .collect();

    ranked.sort_by(|(left_name, left), (right_name, right)| {
        match_rank(left_name, query)
            .cmp(&match_rank(right_name, query))
            .then_with(|| entry_kind_rank(&left.kind).cmp(&entry_kind_rank(&right.kind)))
            .then_with(|| left_name.cmp(right_name))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });

    ranked
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .map(|(_, entry)| entry)
        .collect()
}

fn match_rank(name: &str, query: &str) -> u8 {
    if name == query {
        0
    } else if name.starts_with(query) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_nested_files_and_directories_case_insensitively() {
        let directory =
            std::env::temp_dir().join(format!("dae-file-search-test-{}", std::process::id()));
        let matching_directory = directory.join("Reports");
        let matching_file = matching_directory.join("Annual-REPORT.txt");
        let hidden_file = directory.join(".report-draft");

        fs::create_dir_all(&matching_directory).expect("create matching directory");
        fs::write(&matching_file, "report").expect("create matching file");
        fs::write(&hidden_file, "draft").expect("create hidden matching file");

        let response = search_directory_sync(directory.clone(), "report", &|| true)
            .expect("search test directory");
        let names = response
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Reports", ".report-draft", "Annual-REPORT.txt"]);
        assert!(
            response
                .entries
                .iter()
                .any(|entry| entry.relative_path.contains("Annual-REPORT.txt"))
        );
        let file_entry = response
            .entries
            .iter()
            .find(|entry| entry.name == "Annual-REPORT.txt")
            .expect("matching file entry");
        assert_eq!(file_entry.size, Some(6));
        assert!(file_entry.modified_at.is_some());
        assert!(!response.truncated);

        fs::remove_dir_all(directory).expect("remove search test directory");
    }

    #[test]
    fn returns_no_search_results_for_blank_queries() {
        let response = search_directory_sync(PathBuf::from("missing"), "  ", &|| true)
            .expect("blank search should not touch the file system");

        assert!(response.entries.is_empty());
        assert!(!response.truncated);
    }

    #[test]
    fn cancels_search_when_the_generation_is_stale() {
        let directory =
            std::env::temp_dir().join(format!("dae-search-cancel-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        fs::write(directory.join("report.txt"), "content").expect("create matching file");

        let response = search_directory_sync(directory.clone(), "report", &|| false)
            .expect("stale search should return an empty snapshot");

        assert!(response.entries.is_empty());
        assert!(!response.truncated);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn skips_dependency_and_vcs_directories() {
        let directory =
            std::env::temp_dir().join(format!("dae-search-ignores-test-{}", std::process::id()));
        fs::create_dir_all(directory.join("node_modules").join("dep")).expect("dependency tree");
        fs::create_dir_all(directory.join(".git")).expect("git directory");
        fs::create_dir_all(directory.join("target").join("debug")).expect("build directory");
        fs::create_dir_all(directory.join("src")).expect("source directory");
        fs::write(
            directory.join("node_modules").join("dep").join("index.js"),
            "",
        )
        .expect("write");
        fs::write(directory.join(".git").join("index"), "").expect("write");
        fs::write(directory.join("target").join("debug").join("index.d"), "").expect("write");
        fs::write(directory.join("src").join("index.js"), "").expect("write");

        let response = search_directory_sync(directory.clone(), "index", &|| true).expect("search");

        // Compared with `/` separators because `relative_path` is spelled by the
        // platform's path rules.
        let relative_paths: Vec<String> = response
            .entries
            .iter()
            .map(|entry| entry.relative_path.replace('\\', "/"))
            .collect();
        assert_eq!(relative_paths, vec!["src/index.js".to_owned()]);
        assert!(!response.truncated);

        fs::remove_dir_all(directory).expect("remove search test directory");
    }

    /// The rule is relative to the walk root, so a search *started* inside a
    /// skipped directory still finds that directory's files.
    #[test]
    fn searches_inside_a_skipped_directory_when_it_is_the_root() {
        let directory = std::env::temp_dir().join(format!(
            "dae-search-ignored-root-test-{}",
            std::process::id()
        ));
        let root = directory.join("node_modules");
        fs::create_dir_all(root.join("dep")).expect("dependency tree");
        fs::write(root.join("dep").join("index.js"), "").expect("write");

        let response = search_directory_sync(root, "index", &|| true).expect("search");

        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].name, "index.js");

        fs::remove_dir_all(directory).expect("remove search test directory");
    }

    /// The walk is parallel, so the order matches arrive in is arbitrary: which
    /// 200 get shown has to be decided by rank, not by arrival. This pins the
    /// contract `MAX_SEARCH_CANDIDATES` exists for — the exact match is one of
    /// `MAX_SEARCH_RESULTS + 60` rows, all of which match the query.
    #[test]
    fn keeps_the_exact_match_when_more_matches_arrive_than_fit() {
        let directory =
            std::env::temp_dir().join(format!("dae-search-ranking-test-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        for index in 0..MAX_SEARCH_RESULTS + 60 {
            fs::write(directory.join(format!("reporting-{index:03}.txt")), "").expect("write");
        }
        fs::write(directory.join("report.txt"), "").expect("write the exact match");

        let response =
            search_directory_sync(directory.clone(), "report", &|| true).expect("search");

        assert_eq!(response.entries.len(), MAX_SEARCH_RESULTS);
        assert_eq!(response.entries[0].name, "report.txt");
        assert!(response.truncated);

        fs::remove_dir_all(directory).expect("remove search test directory");
    }

    /// Prints what one name search over a real tree costs, and where its
    /// answers came from.
    ///
    /// ```bash
    /// DAE_SEARCH_BENCH=K:/code/dae \
    ///   cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_name_search
    /// ```
    ///
    /// The tree is whatever the variable names; `DAE_SEARCH_BENCH_QUERY`
    /// overrides the query (`explorer` by default). The interesting columns are
    /// the median milliseconds and `sources` — the directories the results
    /// actually came from, which is how a search that walks dependency trees
    /// instead of the project shows up. Compare this test against itself,
    /// before and after a change; absolute numbers are per-machine.
    #[test]
    #[ignore = "prints measurements for the directory in DAE_SEARCH_BENCH"]
    fn reports_the_cost_of_a_name_search() {
        use std::collections::BTreeMap;
        use std::time::Instant;

        let Some(root) = std::env::var_os("DAE_SEARCH_BENCH") else {
            panic!("set DAE_SEARCH_BENCH to a directory to search");
        };
        let root = PathBuf::from(root);
        let query = std::env::var("DAE_SEARCH_BENCH_QUERY").unwrap_or_else(|_| "explorer".into());

        let search = || search_directory_sync(root.clone(), &query, &|| true).expect("search");
        let warmup = search();

        let mut samples: Vec<u128> = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            let response = search();
            samples.push(started.elapsed().as_nanos());
            assert_eq!(response.entries.len(), warmup.entries.len());
        }
        samples.sort_unstable();

        let mut sources: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &warmup.entries {
            let source = entry
                .relative_path
                .split(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_owned();
            *sources.entry(source).or_default() += 1;
        }

        println!("root      {}", root.display());
        println!("query     {query}");
        println!(
            "results   {} truncated={}",
            warmup.entries.len(),
            warmup.truncated
        );
        println!(
            "median    {:.2} ms",
            samples[samples.len() / 2] as f64 / 1_000_000.0
        );
        println!(
            "min/max   {:.2} / {:.2} ms",
            samples[0] as f64 / 1_000_000.0,
            samples[samples.len() - 1] as f64 / 1_000_000.0
        );
        println!("sources   {sources:?}");
    }
}
