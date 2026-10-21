use super::ignores;
use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    ContentSearchFile, ContentSearchMatch, ContentSearchResponse, canonical_path, path_to_string,
};
use grep_matcher::Matcher as _;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::types::TypesBuilder;
use ignore::{WalkBuilder, WalkState};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// Files larger than this are skipped so a single huge artifact cannot stall
/// the traversal behind the UI's request.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MATCHED_FILES: usize = 100;
const MAX_MATCHES_PER_FILE: usize = 50;
const MAX_RANGES_PER_LINE: usize = 20;
const MAX_LINE_CHARS: usize = 320;

pub struct ContentSearchParams<'a> {
    pub query: &'a str,
    /// When false the query is escaped and matched literally.
    pub is_regex: bool,
    pub case_sensitive: bool,
    /// Comma-separated globs or bare extensions, e.g. `"*.ts, .rs"`.
    pub file_filter: Option<&'a str>,
}

/// The searcher one walk thread reuses across the files it visits.
///
/// A function so the measurement harness prices exactly the searcher the search
/// uses instead of a copy of its configuration.
fn build_searcher() -> grep_searcher::Searcher {
    SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(0))
        .line_number(true)
        .build()
}

/// Searches file contents beneath one local directory using ripgrep's core
/// crates: `ignore` walks in parallel (honoring `.gitignore` plus the forced
/// skips), while `grep-searcher` quits early on binary files.
pub fn search_file_contents_sync(
    requested_path: PathBuf,
    params: &ContentSearchParams<'_>,
    is_current: &(dyn Fn() -> bool + Send + Sync),
) -> Result<ContentSearchResponse, FileSystemError> {
    let query = params.query.trim();
    if query.is_empty() {
        return Ok(ContentSearchResponse {
            files: Vec::new(),
            truncated: false,
        });
    }

    // `canonical_path` rather than `canonicalize`, for the same reason as
    // `search::search_directory_sync`: the walker reads every directory through
    // this base. `OverrideBuilder` takes the same root, so the ignores keep
    // matching relative to the walked tree.
    let root = canonical_path(&requested_path)?;
    if !fs::metadata(&root)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&root)));
    }

    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(!params.case_sensitive)
        .fixed_strings(!params.is_regex)
        .build(query)
        .map_err(|error| {
            FileSystemError::InvalidInput(format!("fs.invalid_search_query: {error}"))
        })?;

    let shared = Arc::new(ContentSearchShared {
        files: Mutex::new(Vec::new()),
        truncated: AtomicBool::new(false),
        is_current,
    });

    let mut builder = WalkBuilder::new(&root);
    builder.standard_filters(true).follow_links(false);
    builder.overrides(ignores::default_overrides(&root)?);
    if let Some(types) = build_file_types(params.file_filter)? {
        builder.types(types);
    }

    builder.build_parallel().run({
        let shared = Arc::clone(&shared);
        let root = Arc::new(root);
        let matcher = Arc::new(matcher);

        move || {
            let shared = Arc::clone(&shared);
            let root = Arc::clone(&root);
            let matcher = Arc::clone(&matcher);

            // One searcher per *thread*, built here rather than per file in the
            // loop below. Everything a `Searcher` owns is a buffer — the read
            // buffer, the line buffer, the transcoding scratch — and the state a
            // per-file answer depends on is either built by each search (the
            // `Core` holding the line counter) or cleared on entry
            // (`LineBufferReader::new` resets the line buffer, binary offset
            // included), so reuse is what the type is for. Building one per file
            // allocated and freed those buffers for every file the walk visited.
            let mut searcher = build_searcher();

            Box::new(move |result| {
                if shared.truncated.load(AtomicOrdering::Relaxed) || !(shared.is_current)() {
                    return WalkState::Quit;
                }

                let entry = match result {
                    Ok(entry) => entry,
                    // Inaccessible descendants should not prevent partial results.
                    Err(_) => return WalkState::Continue,
                };

                if !entry
                    .file_type()
                    .is_some_and(|file_type| file_type.is_file())
                {
                    return WalkState::Continue;
                }
                if entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() > MAX_FILE_BYTES)
                {
                    return WalkState::Continue;
                }

                let mut matches = Vec::new();

                let sink = grep_searcher::sinks::UTF8(|line_number, line| {
                    if matches.len() >= MAX_MATCHES_PER_FILE {
                        return Ok(false);
                    }
                    matches.push(build_match(line_number, line, matcher.as_ref()));
                    Ok(true)
                });

                // Unreadable, binary, or non-UTF-8-heavy files are skipped quietly.
                if searcher
                    .search_path(matcher.as_ref(), entry.path(), sink)
                    .is_err()
                {
                    return WalkState::Continue;
                }

                if matches.is_empty() {
                    return WalkState::Continue;
                }

                let relative_path = entry
                    .path()
                    .strip_prefix(root.as_path())
                    .unwrap_or(entry.path());

                let should_stop = {
                    let mut files = shared.files.lock().expect("content search lock poisoned");
                    if files.len() >= MAX_MATCHED_FILES {
                        true
                    } else {
                        files.push(ContentSearchFile {
                            path: path_to_string(entry.path()),
                            relative_path: path_to_string(relative_path),
                            matches,
                        });
                        false
                    }
                };

                if should_stop {
                    shared.truncated.store(true, AtomicOrdering::Relaxed);
                    return WalkState::Quit;
                }

                WalkState::Continue
            })
        }
    });

    let mut files = shared
        .files
        .lock()
        .expect("content search lock poisoned")
        .drain(..)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(ContentSearchResponse {
        files,
        truncated: shared.truncated.load(AtomicOrdering::Relaxed),
    })
}

struct ContentSearchShared<'a> {
    files: Mutex<Vec<ContentSearchFile>>,
    truncated: AtomicBool,
    is_current: &'a (dyn Fn() -> bool + Send + Sync),
}

/// Builds one trimmed, length-capped match row with character-based ranges so
/// the frontend can highlight without re-decoding bytes.
fn build_match(
    line_number: u64,
    line: &str,
    matcher: &grep_regex::RegexMatcher,
) -> ContentSearchMatch {
    let char_count = line.chars().count();
    let keep_chars = char_count.min(MAX_LINE_CHARS);

    let mut ranges = Vec::new();
    let _ = matcher.find_iter(line.as_bytes(), |found| {
        let start = char_offset(line, found.start());
        if start < keep_chars {
            let end = char_offset(line, found.end()).min(keep_chars);
            ranges.push((start, end));
        }
        ranges.len() < MAX_RANGES_PER_LINE
    });

    let line_text = if keep_chars == char_count {
        line.trim_end().to_owned()
    } else {
        let mut truncated: String = line.chars().take(keep_chars).collect();
        truncated.push('…');
        truncated.trim_end().to_owned()
    };

    ContentSearchMatch {
        line_number,
        line_text,
        ranges,
    }
}

fn char_offset(line: &str, byte_offset: usize) -> usize {
    line.as_bytes()[..byte_offset]
        .iter()
        .filter(|byte| (*byte & 0xC0) != 0x80)
        .count()
}

/// Translates `"*.ts, .rs"` into an `ignore` type filter. Bare tokens are
/// treated as extensions, so `"rs"` and `".rs"` both mean `"*.rs"`.
fn build_file_types(filter: Option<&str>) -> Result<Option<ignore::types::Types>, FileSystemError> {
    let Some(filter) = filter else {
        return Ok(None);
    };

    let mut builder = TypesBuilder::new();
    let mut has_pattern = false;

    for token in filter.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let pattern = if token.contains('*') || token.contains('?') {
            token.to_owned()
        } else {
            format!("*.{}", token.trim_start_matches('.'))
        };

        builder.add("daefilter", &pattern).map_err(|error| {
            FileSystemError::InvalidInput(format!("fs.invalid_type_filter: {error}"))
        })?;
        has_pattern = true;
    }

    if !has_pattern {
        return Ok(None);
    }

    builder.select("daefilter");
    let types = builder.build().map_err(|error| {
        FileSystemError::Internal(format!("fs.type_filter_build_failed: {error}"))
    })?;
    Ok(Some(types))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_content_search_fixture(root: &Path) {
        let src = root.join("src");
        fs::create_dir_all(src.join("nested")).expect("create src tree");
        fs::create_dir_all(root.join("node_modules")).expect("create node_modules");
        fs::create_dir_all(root.join("target")).expect("create target");

        fs::write(
            src.join("notes.txt"),
            "hello TODO world\ntodo again\nplain line\n",
        )
        .expect("write notes.txt");
        fs::write(
            src.join("nested").join("main.rs"),
            "fn main() { let n = 42; }\n",
        )
        .expect("write main.rs");
        fs::write(root.join("node_modules").join("dep.js"), "TODO in deps\n")
            .expect("write dep.js");
        fs::write(
            root.join("target").join("build.log"),
            "TODO in build output\n",
        )
        .expect("write build.log");
    }

    #[test]
    fn searches_contents_with_regex_and_reports_match_ranges() {
        let root =
            std::env::temp_dir().join(format!("dae-content-search-regex-{}", std::process::id()));
        write_content_search_fixture(&root);

        let params = ContentSearchParams {
            query: r"T[OD]DO",
            is_regex: true,
            case_sensitive: false,
            file_filter: None,
        };
        let response = search_file_contents_sync(root.clone(), &params, &|| true)
            .expect("regex content search");

        let paths = response
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>();
        let expected_notes = Path::new("src")
            .join("notes.txt")
            .to_string_lossy()
            .into_owned();
        assert_eq!(paths, vec![expected_notes.as_str()]);
        assert!(!response.truncated);

        let notes = &response.files[0];
        assert_eq!(notes.matches.len(), 2);
        assert_eq!(notes.matches[0].line_number, 1);
        assert_eq!(notes.matches[0].line_text, "hello TODO world");
        assert_eq!(notes.matches[0].ranges, vec![(6, 10)]);
        assert_eq!(notes.matches[1].line_number, 2);

        fs::remove_dir_all(root).expect("remove content search fixture");
    }

    #[test]
    fn fixed_string_search_respects_case_sensitivity() {
        let root =
            std::env::temp_dir().join(format!("dae-content-search-case-{}", std::process::id()));
        write_content_search_fixture(&root);

        let sensitive = ContentSearchParams {
            query: "todo",
            is_regex: false,
            case_sensitive: true,
            file_filter: None,
        };
        let response = search_file_contents_sync(root.clone(), &sensitive, &|| true)
            .expect("case-sensitive content search");
        assert_eq!(response.files.len(), 1);
        assert_eq!(response.files[0].matches.len(), 1);
        assert_eq!(response.files[0].matches[0].line_number, 2);

        let regex_metacharacters = ContentSearchParams {
            query: "main()",
            is_regex: false,
            case_sensitive: true,
            file_filter: None,
        };
        let response = search_file_contents_sync(root.clone(), &regex_metacharacters, &|| true)
            .expect("literal content search");
        assert_eq!(response.files.len(), 1);
        assert!(response.files[0].relative_path.ends_with("main.rs"));

        fs::remove_dir_all(root).expect("remove content search fixture");
    }

    #[test]
    fn content_search_filters_by_file_type() {
        let root =
            std::env::temp_dir().join(format!("dae-content-search-types-{}", std::process::id()));
        write_content_search_fixture(&root);

        let params = ContentSearchParams {
            query: "TODO",
            is_regex: false,
            case_sensitive: true,
            file_filter: Some("*.rs, js"),
        };
        let response = search_file_contents_sync(root.clone(), &params, &|| true)
            .expect("typed content search");

        // node_modules is force-ignored even though dep.js matches the filter.
        assert!(response.files.is_empty());

        let params = ContentSearchParams {
            query: "42",
            is_regex: false,
            case_sensitive: true,
            file_filter: Some("rs"),
        };
        let response = search_file_contents_sync(root.clone(), &params, &|| true)
            .expect("extension-alias content search");
        assert_eq!(response.files.len(), 1);
        assert!(response.files[0].relative_path.ends_with("main.rs"));

        fs::remove_dir_all(root).expect("remove content search fixture");
    }

    /// Counts the files a content search would visit, with the same walker and
    /// the same forced ignores, so a measurement can report the tree's size
    /// beside its cost.
    fn count_searchable_files(root: &Path) -> u64 {
        let mut builder = WalkBuilder::new(root);
        builder.standard_filters(true).follow_links(false);
        builder.overrides(ignores::default_overrides(root).expect("ignore rules"));

        builder
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .count() as u64
    }

    /// Prints what one content search over a real tree costs.
    ///
    /// ```bash
    /// cd src-tauri/crates/dae-core
    /// DAE_CONTENT_SEARCH_BENCH=K:/dae-listing-bench/big \
    ///   cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_content_search
    /// ```
    ///
    /// The tree is whatever the variable names; `DAE_CONTENT_SEARCH_BENCH_QUERY`
    /// overrides the query (`needle` by default). The query has to be rare
    /// enough that the walk is not stopped by `MAX_MATCHED_FILES`, or the number
    /// measures the first hundred matches instead of the tree — the test refuses
    /// to report a truncated one. `files` is how many files the walk visits, so
    /// the per-file fixed cost is the total divided by it; a tree of empty files
    /// is the sharpest instrument for that, since nothing else is happening per
    /// file. Compare this test against itself, before and after a change;
    /// absolute milliseconds are per-machine.
    #[test]
    #[ignore = "prints measurements for the directory in DAE_CONTENT_SEARCH_BENCH"]
    fn reports_the_cost_of_a_content_search() {
        let Some(root) = std::env::var_os("DAE_CONTENT_SEARCH_BENCH") else {
            panic!("set DAE_CONTENT_SEARCH_BENCH to a directory to search");
        };
        let root = PathBuf::from(root);
        let query =
            std::env::var("DAE_CONTENT_SEARCH_BENCH_QUERY").unwrap_or_else(|_| "needle".into());

        let params = ContentSearchParams {
            query: &query,
            is_regex: false,
            case_sensitive: true,
            file_filter: None,
        };
        let search =
            || search_file_contents_sync(root.clone(), &params, &|| true).expect("content search");

        let response = search();
        assert!(
            !response.truncated,
            "the query matched {MAX_MATCHED_FILES} files, so the walk stopped early; pick a rarer \
             one with DAE_CONTENT_SEARCH_BENCH_QUERY"
        );

        let mut samples = Vec::new();
        for _ in 0..7 {
            let started = std::time::Instant::now();
            search();
            samples.push(started.elapsed().as_nanos());
        }
        samples.sort_unstable();

        let files = count_searchable_files(&root);
        println!("root      {}", root.display());
        println!("query     {query}");
        println!("files     {files}");
        println!("matched   {}", response.files.len());
        println!(
            "median    {:.2} ms",
            samples[samples.len() / 2] as f64 / 1_000_000.0
        );
        println!(
            "min/max   {:.2} / {:.2} ms",
            samples[0] as f64 / 1_000_000.0,
            samples[samples.len() - 1] as f64 / 1_000_000.0
        );

        // The per-file fixed cost the reuse removes, with no file read in the
        // way: the shipped searcher built once per file in the tree.
        let mut samples = Vec::new();
        for _ in 0..7 {
            let started = std::time::Instant::now();
            for _ in 0..files {
                std::hint::black_box(build_searcher());
            }
            samples.push(started.elapsed().as_nanos());
        }
        samples.sort_unstable();
        println!(
            "build     {:>8.2} ms  ({files} x build_searcher, no I/O)",
            samples[samples.len() / 2] as f64 / 1_000_000.0
        );
    }

    /// A searcher is reused across files, so each `search_path` has to reset the
    /// state a per-file answer depends on — the line counter first of all. The
    /// same searcher runs both files here, longest first, which is the order the
    /// walk's threads take and the one a carried-over counter would show in.
    #[test]
    fn a_reused_searcher_reports_each_files_own_line_numbers() {
        let root =
            std::env::temp_dir().join(format!("dae-content-search-reuse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture");
        fs::write(root.join("first.txt"), "plain\nTODO first\nplain\n").expect("write first.txt");
        fs::write(root.join("second.txt"), "TODO second\nplain\n").expect("write second.txt");

        let matcher = RegexMatcherBuilder::new()
            .fixed_strings(true)
            .case_insensitive(false)
            .build("TODO")
            .expect("matcher");
        let mut searcher = build_searcher();

        let matches_in = |searcher: &mut grep_searcher::Searcher, name: &str| {
            let mut lines: Vec<(u64, String)> = Vec::new();
            let sink = grep_searcher::sinks::UTF8(|line_number, line| {
                lines.push((line_number, line.trim_end().to_owned()));
                Ok(true)
            });
            searcher
                .search_path(&matcher, root.join(name), sink)
                .expect("search");
            lines
        };

        assert_eq!(
            matches_in(&mut searcher, "first.txt"),
            vec![(2, "TODO first".to_owned())]
        );
        assert_eq!(
            matches_in(&mut searcher, "second.txt"),
            vec![(1, "TODO second".to_owned())]
        );
        // And back to the first file, to catch state that only resets once.
        assert_eq!(
            matches_in(&mut searcher, "first.txt"),
            vec![(2, "TODO first".to_owned())]
        );

        fs::remove_dir_all(root).expect("remove fixture");
    }

    /// The same contract through the shipped entry point, across a whole walk.
    #[test]
    fn a_content_search_reports_each_files_own_line_numbers() {
        let root =
            std::env::temp_dir().join(format!("dae-content-search-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture");
        fs::write(root.join("first.txt"), "plain\nTODO first\nplain\n").expect("write first.txt");
        fs::write(root.join("second.txt"), "TODO second\nplain\n").expect("write second.txt");

        let response = search_file_contents_sync(
            root.clone(),
            &ContentSearchParams {
                query: "TODO",
                is_regex: false,
                case_sensitive: true,
                file_filter: None,
            },
            &|| true,
        )
        .expect("content search");

        let file = |name: &str| {
            response
                .files
                .iter()
                .find(|file| file.relative_path.ends_with(name))
                .unwrap_or_else(|| panic!("{name} should match"))
        };
        assert_eq!(file("first.txt").matches[0].line_number, 2);
        assert_eq!(file("first.txt").matches[0].line_text, "TODO first");
        assert_eq!(file("second.txt").matches[0].line_number, 1);
        assert_eq!(file("second.txt").matches[0].line_text, "TODO second");

        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn blank_content_search_returns_empty_response() {
        let response = search_file_contents_sync(
            PathBuf::from("missing"),
            &ContentSearchParams {
                query: "   ",
                is_regex: true,
                case_sensitive: false,
                file_filter: None,
            },
            &|| true,
        )
        .expect("blank content search should not touch the file system");

        assert!(response.files.is_empty());
        assert!(!response.truncated);
    }
}
