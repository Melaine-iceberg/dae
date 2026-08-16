use crate::file_system::error::FileSystemError;
use crate::file_system::types::{
    ContentSearchFile, ContentSearchMatch, ContentSearchResponse, path_to_string,
};
use grep_matcher::Matcher as _;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::overrides::OverrideBuilder;
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

/// Always-skipped directories, mirroring ripgrep's practical defaults for a
/// GUI audience even when the project has no `.gitignore` entries for them.
const ALWAYS_IGNORED: [&str; 4] = ["!.git", "!**/.git", "!**/node_modules", "!**/target"];

pub struct ContentSearchParams<'a> {
    pub query: &'a str,
    /// When false the query is escaped and matched literally.
    pub is_regex: bool,
    pub case_sensitive: bool,
    /// Comma-separated globs or bare extensions, e.g. `"*.ts, .rs"`.
    pub file_filter: Option<&'a str>,
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

    let root = requested_path.canonicalize()?;
    if !fs::metadata(&root)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(&root)));
    }

    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(!params.case_sensitive)
        .fixed_strings(!params.is_regex)
        .build(query)
        .map_err(|error| FileSystemError::InvalidInput(format!("无效的搜索表达式：{error}")))?;

    let shared = Arc::new(ContentSearchShared {
        files: Mutex::new(Vec::new()),
        truncated: AtomicBool::new(false),
        is_current,
    });

    let mut builder = WalkBuilder::new(&root);
    builder.standard_filters(true).follow_links(false);

    let mut overrides = OverrideBuilder::new(&root);
    for pattern in ALWAYS_IGNORED {
        overrides
            .add(pattern)
            .map_err(|error| FileSystemError::Internal(format!("无法注册忽略规则：{error}")))?;
    }
    builder.overrides(
        overrides
            .build()
            .map_err(|error| FileSystemError::Internal(format!("无法构建忽略规则：{error}")))?,
    );

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
                let mut searcher = SearcherBuilder::new()
                    .binary_detection(BinaryDetection::quit(0))
                    .line_number(true)
                    .build();

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
fn build_match(line_number: u64, line: &str, matcher: &grep_regex::RegexMatcher) -> ContentSearchMatch {
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

        builder
            .add("daefilter", &pattern)
            .map_err(|error| FileSystemError::InvalidInput(format!("无效的类型过滤：{error}")))?;
        has_pattern = true;
    }

    if !has_pattern {
        return Ok(None);
    }

    builder.select("daefilter");
    let types = builder
        .build()
        .map_err(|error| FileSystemError::Internal(format!("无法构建类型过滤：{error}")))?;
    Ok(Some(types))
}
