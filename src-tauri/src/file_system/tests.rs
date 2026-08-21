use super::error::FileSystemError;
use super::local::{
    build_breadcrumbs, copy_entries_with_progress, create_entry_sync,
    delete_entries_with_progress, move_entries_with_progress, read_directory_sync,
    rename_entry_sync, search_directory_sync, search_file_contents_sync, ContentSearchParams,
};
use super::progress::FileOperationProgressReporterTrait;
use super::sidebar::{Favorite, dedupe_favorites, is_visible_file_system};
use super::types::{
    ConflictAction, DirectoryEntry, EntryKind, NewEntryKind, entry_sort_key,
    normalize_path_for_display, path_to_string,
};
use super::vfs::{self, FileSystemBackend};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

struct TestProgress {
    completed: AtomicU64,
    total: AtomicU64,
}

impl TestProgress {
    fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }
}

impl FileOperationProgressReporterTrait for TestProgress {
    fn start(&self, total: u64) {
        self.total.store(total, AtomicOrdering::Relaxed);
    }

    fn begin_entry(&self, _path: &Path) {}

    fn advance_by(&self, units: u64, _path: &Path) {
        self.completed.fetch_add(units, AtomicOrdering::Relaxed);
    }

    fn finish(&self) {
        assert_eq!(
            self.completed.load(AtomicOrdering::Relaxed),
            self.total.load(AtomicOrdering::Relaxed)
        );
    }
}

#[cfg(windows)]
#[test]
fn removes_windows_verbatim_path_prefixes() {
    assert_eq!(
        normalize_path_for_display(r"\\?\C:\Users\test"),
        r"C:\Users\test"
    );
    assert_eq!(
        normalize_path_for_display(r"\\?\UNC\server\share\folder"),
        r"\\server\share\folder"
    );
}

#[test]
fn builds_clickable_breadcrumbs_from_a_path() {
    let path = std::env::temp_dir().join("dae").join("nested");
    let breadcrumbs = build_breadcrumbs(&path);

    assert_eq!(
        breadcrumbs.last().map(|item| item.path.as_str()),
        Some(path.to_string_lossy().as_ref())
    );
    assert_eq!(
        breadcrumbs.last().map(|item| item.name.as_str()),
        Some("nested")
    );
}

#[test]
fn places_directories_before_files_case_insensitively() {
    let mut entries = [
        DirectoryEntry {
            name: "zeta.txt".into(),
            path: "zeta.txt".into(),
            kind: EntryKind::File,
            modified_at: None,
            size: None,
        },
        DirectoryEntry {
            name: "alpha".into(),
            path: "alpha".into(),
            kind: EntryKind::Directory,
            modified_at: None,
            size: None,
        },
        DirectoryEntry {
            name: "Beta".into(),
            path: "Beta".into(),
            kind: EntryKind::Directory,
            modified_at: None,
            size: None,
        },
    ];

    entries.sort_by_cached_key(entry_sort_key);

    assert_eq!(entries[0].name, "alpha");
    assert_eq!(entries[1].name, "Beta");
    assert_eq!(entries[2].name, "zeta.txt");
}

#[test]
fn reads_entries_from_a_directory() {
    let directory =
        std::env::temp_dir().join(format!("dae-file-system-test-{}", std::process::id()));
    let nested_directory = directory.join("folder");
    let file = directory.join("file.txt");

    fs::create_dir_all(&nested_directory).expect("create test directory");
    fs::write(&file, "test").expect("create test file");

    let view = read_directory_sync(directory.clone()).expect("read test directory");
    let names = view
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["folder", "file.txt"]);

    let folder_entry = &view.entries[0];
    let file_entry = &view.entries[1];
    assert!(folder_entry.modified_at.is_some());
    assert_eq!(folder_entry.size, None);
    assert!(file_entry.modified_at.is_some());
    assert_eq!(file_entry.size, Some(4));

    fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn returns_not_directory_for_a_file_path() {
    let file =
        std::env::temp_dir().join(format!("dae-file-system-test-{}.txt", std::process::id()));
    fs::write(&file, "test").expect("create test file");

    let error = read_directory_sync(file.clone()).expect_err("a file is not a directory");

    fs::remove_file(file).expect("remove test file");

    assert!(matches!(error, FileSystemError::NotDirectory(_)));
}

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
    let response = search_directory_sync(Path::new("missing").to_path_buf(), "  ", &|| true)
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

fn write_content_search_fixture(root: &Path) {
    let src = root.join("src");
    fs::create_dir_all(src.join("nested")).expect("create src tree");
    fs::create_dir_all(root.join("node_modules")).expect("create node_modules");
    fs::create_dir_all(root.join("target")).expect("create target");

    fs::write(src.join("notes.txt"), "hello TODO world\ntodo again\nplain line\n")
        .expect("write notes.txt");
    fs::write(src.join("nested").join("main.rs"), "fn main() { let n = 42; }\n")
        .expect("write main.rs");
    fs::write(root.join("node_modules").join("dep.js"), "TODO in deps\n")
        .expect("write dep.js");
    fs::write(root.join("target").join("build.log"), "TODO in build output\n")
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
    let response =
        search_file_contents_sync(root.clone(), &params, &|| true).expect("regex content search");

    let paths = response
        .files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<Vec<_>>();
    let expected_notes = Path::new("src").join("notes.txt").to_string_lossy().into_owned();
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

#[test]
fn blank_content_search_returns_empty_response() {
    let response = search_file_contents_sync(
        Path::new("missing").to_path_buf(),
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

#[test]
fn performs_file_operations_and_reports_entry_progress() {
    let directory =
        std::env::temp_dir().join(format!("dae-file-operation-test-{}", std::process::id()));
    let source = directory.join("source");
    let destination = directory.join("destination");
    let nested_file = source.join("nested.txt");

    fs::create_dir_all(&source).expect("create source directory");
    fs::create_dir_all(&destination).expect("create destination directory");
    fs::write(&nested_file, "copied content").expect("create source file");

    let copy_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![(source.clone(), ConflictAction::Fail)],
        destination.clone(),
        &copy_progress,
    )
    .expect("copy directory");
    assert_eq!(copy_progress.completed.load(AtomicOrdering::Relaxed), 2);
    assert_eq!(copy_progress.total.load(AtomicOrdering::Relaxed), 2);

    let copied_directory = destination.join("source");
    assert_eq!(
        fs::read_to_string(copied_directory.join("nested.txt")).expect("read copied file"),
        "copied content"
    );
    assert!(source.exists());

    let duplicate_progress = TestProgress::new();
    let duplicate_error = copy_entries_with_progress(
        vec![(source.clone(), ConflictAction::Fail)],
        destination.clone(),
        &duplicate_progress,
    )
    .expect_err("copying over an existing entry should fail");
    assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

    let nested_progress = TestProgress::new();
    let nested_error = copy_entries_with_progress(
        vec![(source.clone(), ConflictAction::Fail)],
        source.clone(),
        &nested_progress,
    )
    .expect_err("copying a folder into itself should fail");
    assert!(matches!(nested_error, FileSystemError::InvalidInput(_)));

    rename_entry_sync(nested_file.clone(), "renamed.txt".into()).expect("rename file");
    let renamed_file = source.join("renamed.txt");
    assert!(renamed_file.exists());

    let move_progress = TestProgress::new();
    move_entries_with_progress(
        vec![(renamed_file.clone(), ConflictAction::Fail)],
        destination.clone(),
        &move_progress,
    )
    .expect("move file");
    let moved_file = destination.join("renamed.txt");
    assert!(moved_file.exists());
    assert!(!renamed_file.exists());
    assert_eq!(move_progress.completed.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(move_progress.total.load(AtomicOrdering::Relaxed), 1);

    let delete_progress = TestProgress::new();
    delete_entries_with_progress(vec![moved_file.clone()], &delete_progress).expect("delete file");
    assert!(!moved_file.exists());
    assert_eq!(delete_progress.completed.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(delete_progress.total.load(AtomicOrdering::Relaxed), 1);

    fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn copies_and_deletes_nested_trees_in_parallel() {
    let directory =
        std::env::temp_dir().join(format!("dae-parallel-tree-test-{}", std::process::id()));
    let source = directory.join("source");
    let destination = directory.join("destination");

    for index in 0..3 {
        let nested = source.join(format!("folder-{index}"));
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(nested.join("file.txt"), "parallel").expect("create nested file");
    }
    fs::create_dir_all(&destination).expect("create destination directory");

    let copy_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![(source.clone(), ConflictAction::Fail)],
        destination.clone(),
        &copy_progress,
    )
    .expect("copy nested tree");
    assert_eq!(copy_progress.completed.load(AtomicOrdering::Relaxed), 7);
    assert_eq!(copy_progress.total.load(AtomicOrdering::Relaxed), 7);

    let copied_root = destination.join("source");
    for index in 0..3 {
        assert!(
            copied_root
                .join(format!("folder-{index}"))
                .join("file.txt")
                .is_file()
        );
    }

    let delete_progress = TestProgress::new();
    delete_entries_with_progress(vec![copied_root], &delete_progress).expect("delete nested tree");
    assert_eq!(delete_progress.completed.load(AtomicOrdering::Relaxed), 7);
    assert_eq!(delete_progress.total.load(AtomicOrdering::Relaxed), 7);

    fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn hides_network_and_pseudo_file_systems() {
    assert!(!is_visible_file_system("cifs"));
    assert!(!is_visible_file_system("NFS"));
    assert!(!is_visible_file_system("smbfs"));
    assert!(!is_visible_file_system("fuse.sshfs"));
    assert!(!is_visible_file_system("webdav"));
    assert!(!is_visible_file_system("tmpfs"));
    assert!(!is_visible_file_system("overlay"));
    assert!(!is_visible_file_system("proc"));

    assert!(is_visible_file_system("NTFS"));
    assert!(is_visible_file_system("ext4"));
    assert!(is_visible_file_system("apfs"));
    assert!(is_visible_file_system("btrfs"));
    assert!(is_visible_file_system("FAT32"));
    assert!(is_visible_file_system("exFAT"));
    assert!(is_visible_file_system("iso9660"));
}

#[test]
fn dedupes_favorites_keeping_the_first_entry_per_path() {
    let favorites = vec![
        Favorite {
            path: "/a".into(),
            name: "First".into(),
        },
        Favorite {
            path: "/b".into(),
            name: "B".into(),
        },
        Favorite {
            path: "/a".into(),
            name: "Duplicate".into(),
        },
    ];

    let deduped = dedupe_favorites(favorites);

    assert_eq!(deduped.len(), 2);
    assert_eq!(deduped[0].path, "/a");
    assert_eq!(deduped[0].name, "First");
    assert_eq!(deduped[1].path, "/b");
}

#[test]
fn creates_files_and_directories_with_validated_names() {
    let directory =
        std::env::temp_dir().join(format!("dae-create-entry-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("create test directory");

    let file_path = create_entry_sync(directory.clone(), "notes.txt".into(), NewEntryKind::File)
        .expect("create file");
    assert_eq!(file_path, path_to_string(&directory.join("notes.txt")));
    assert!(directory.join("notes.txt").is_file());

    let directory_path = create_entry_sync(
        directory.clone(),
        "子文件夹".into(),
        NewEntryKind::Directory,
    )
    .expect("create directory");
    assert_eq!(directory_path, path_to_string(&directory.join("子文件夹")));
    assert!(directory.join("子文件夹").is_dir());

    let duplicate_error =
        create_entry_sync(directory.clone(), "notes.txt".into(), NewEntryKind::File)
            .expect_err("creating over an existing entry should fail");
    assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

    let separator_error =
        create_entry_sync(directory.clone(), "a/b.txt".into(), NewEntryKind::File)
            .expect_err("names with path separators should fail");
    assert!(matches!(separator_error, FileSystemError::InvalidInput(_)));

    fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn parses_scheme_prefixes() {
    let (scheme, rest) = vfs::split_scheme("smb://nas/media").expect("smb scheme");
    assert_eq!(scheme, vfs::Scheme::Smb);
    assert_eq!(rest, "nas/media");

    let (scheme, rest) = vfs::split_scheme("sftp://user@nas:22/home").expect("sftp scheme");
    assert_eq!(scheme, vfs::Scheme::Sftp);
    assert_eq!(rest, "user@nas:22/home");

    let (scheme, rest) = vfs::split_scheme("webdavs://cloud.example/dav").expect("webdav scheme");
    assert_eq!(scheme, vfs::Scheme::WebDav);
    assert_eq!(rest, "cloud.example/dav");

    let (scheme, rest) = vfs::split_scheme("file:///home").expect("file maps to local");
    assert_eq!(scheme, vfs::Scheme::Local);
    assert_eq!(rest, "/home");
}

#[test]
fn treats_scheme_less_paths_as_local() {
    for path in [
        r"C:\Users\test",
        "/home/user",
        r"\\nas\share\folder",
        r"relative\nested://segment",
    ] {
        let (scheme, rest) = vfs::split_scheme(path).expect("scheme-less path");
        assert_eq!(scheme, vfs::Scheme::Local, "path should stay local: {path}");
        assert_eq!(rest, path);
    }
}

#[test]
fn rejects_unknown_and_unregistered_schemes() {
    let unknown = vfs::resolve("s3://bucket/data")
        .err()
        .expect("unknown scheme");
    assert!(matches!(unknown, FileSystemError::InvalidInput(_)));

    // SMB and SFTP resolve to connect attempts these days; ftp is a scheme
    // that still has no backend.
    let unregistered = vfs::resolve("ftp://nas/media")
        .err()
        .expect("no ftp backend yet");
    assert!(matches!(unregistered, FileSystemError::InvalidInput(_)));
}

#[test]
fn transfers_trees_between_distinct_backend_instances() {
    use super::local::LocalBackend;
    use super::transfer::{self, TransferSource};
    use std::sync::Arc;

    let root = std::env::temp_dir().join(format!("dae-transfer-test-{}", std::process::id()));
    let source_dir = root.join("source");
    let destination_dir = root.join("destination");
    fs::create_dir_all(source_dir.join("nested")).expect("create source tree");
    fs::write(source_dir.join("root.txt"), "root content").expect("write root file");
    fs::write(source_dir.join("nested/leaf.bin"), vec![7_u8; 600 * 1024])
        .expect("write multi-chunk file");
    fs::create_dir_all(&destination_dir).expect("create destination");

    // Two distinct Arcs force the streaming path instead of any fast path.
    let source_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
    let destination_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
    let source_path = source_dir.to_string_lossy().into_owned();
    let destination_path = destination_dir.to_string_lossy().into_owned();

    let copy_progress = TestProgress::new();
    transfer::copy_entries(
        vec![TransferSource {
            path: source_path.clone(),
            backend: source_backend.clone(),
            on_conflict: ConflictAction::Fail,
        }],
        &destination_path,
        &destination_backend,
        &copy_progress,
    )
    .expect("copy tree across backends");

    let copied_root = destination_dir.join("source");
    assert_eq!(
        fs::read_to_string(copied_root.join("root.txt")).expect("read copied root file"),
        "root content"
    );
    let leaf = fs::read(copied_root.join("nested/leaf.bin")).expect("read copied leaf");
    assert_eq!(leaf.len(), 600 * 1024);
    assert!(leaf.iter().all(|byte| *byte == 7));
    assert_eq!(
        copy_progress.completed.load(AtomicOrdering::Relaxed),
        copy_progress.total.load(AtomicOrdering::Relaxed)
    );

    let duplicate_progress = TestProgress::new();
    let duplicate_error = transfer::copy_entries(
        vec![TransferSource {
            path: source_path.clone(),
            backend: source_backend.clone(),
            on_conflict: ConflictAction::Fail,
        }],
        &destination_path,
        &destination_backend,
        &duplicate_progress,
    )
    .expect_err("overwriting must be blocked");
    assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

    let move_destination_dir = root.join("destination-moved");
    fs::create_dir_all(&move_destination_dir).expect("create move destination");
    let move_progress = TestProgress::new();
    transfer::move_entries(
        vec![TransferSource {
            path: source_path.clone(),
            backend: source_backend.clone(),
            on_conflict: ConflictAction::Fail,
        }],
        &move_destination_dir.to_string_lossy(),
        &destination_backend,
        &move_progress,
    )
    .expect("move tree across backends");
    assert!(!source_dir.exists());
    assert!(
        move_destination_dir
            .join("source/nested/leaf.bin")
            .is_file()
    );
    assert_eq!(
        move_progress.completed.load(AtomicOrdering::Relaxed),
        move_progress.total.load(AtomicOrdering::Relaxed)
    );

    let delete_progress = TestProgress::new();
    transfer::delete_entries(
        vec![TransferSource {
            path: move_destination_dir
                .join("source")
                .to_string_lossy()
                .into_owned(),
            backend: destination_backend.clone(),
            on_conflict: ConflictAction::Fail,
        }],
        &delete_progress,
    )
    .expect("delete tree through engine");
    assert!(!move_destination_dir.join("source").exists());

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn resolves_local_transfer_conflicts_with_replace_skip_and_keep_both() {
    let root =
        std::env::temp_dir().join(format!("dae-conflict-local-test-{}", std::process::id()));
    let source_dir = root.join("source");
    let destination_dir = root.join("destination");
    fs::create_dir_all(&source_dir).expect("create source directory");
    fs::create_dir_all(&destination_dir).expect("create destination directory");

    // Replace: the existing file is deleted, then the source is copied.
    fs::write(source_dir.join("report.txt"), "new content").expect("write source file");
    fs::write(destination_dir.join("report.txt"), "old content").expect("write target file");
    let replace_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![(source_dir.join("report.txt"), ConflictAction::Replace)],
        destination_dir.clone(),
        &replace_progress,
    )
    .expect("copy with replace");
    assert_eq!(
        fs::read_to_string(destination_dir.join("report.txt")).expect("read replaced file"),
        "new content"
    );
    assert_eq!(
        replace_progress.completed.load(AtomicOrdering::Relaxed),
        replace_progress.total.load(AtomicOrdering::Relaxed)
    );
    assert_eq!(replace_progress.total.load(AtomicOrdering::Relaxed), 2);

    // Skip: the destination stays untouched and the source survives.
    let skip_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![(source_dir.join("report.txt"), ConflictAction::Skip)],
        destination_dir.clone(),
        &skip_progress,
    )
    .expect("copy with skip");
    assert_eq!(
        fs::read_to_string(destination_dir.join("report.txt")).expect("skipped file unchanged"),
        "new content"
    );
    assert!(source_dir.join("report.txt").is_file());

    // Keep both: the incoming file lands under a "副本" name.
    let keep_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![(source_dir.join("report.txt"), ConflictAction::KeepBoth)],
        destination_dir.clone(),
        &keep_progress,
    )
    .expect("copy keeping both");
    assert_eq!(
        fs::read_to_string(destination_dir.join("report 副本.txt")).expect("kept copy exists"),
        "new content"
    );
    assert_eq!(
        keep_progress.completed.load(AtomicOrdering::Relaxed),
        keep_progress.total.load(AtomicOrdering::Relaxed)
    );

    // Move with replace removes the source once the target is replaced.
    let move_progress = TestProgress::new();
    move_entries_with_progress(
        vec![(source_dir.join("report.txt"), ConflictAction::Replace)],
        destination_dir.clone(),
        &move_progress,
    )
    .expect("move with replace");
    assert!(!source_dir.join("report.txt").exists());
    assert_eq!(
        fs::read_to_string(destination_dir.join("report.txt")).expect("moved file content"),
        "new content"
    );
    assert_eq!(
        move_progress.completed.load(AtomicOrdering::Relaxed),
        move_progress.total.load(AtomicOrdering::Relaxed)
    );

    // Replacing a directory tree with a file works across kinds.
    fs::create_dir_all(destination_dir.join("bundle")).expect("create target directory");
    fs::write(destination_dir.join("bundle/inner.txt"), "inner").expect("write inner file");
    fs::write(source_dir.join("bundle"), "now a file").expect("write source file");
    copy_entries_with_progress(
        vec![(source_dir.join("bundle"), ConflictAction::Replace)],
        destination_dir.clone(),
        &TestProgress::new(),
    )
    .expect("replace a directory with a file");
    assert_eq!(
        fs::read_to_string(destination_dir.join("bundle")).expect("replaced directory"),
        "now a file"
    );

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn moving_an_entry_onto_itself_is_a_no_op() {
    let root = std::env::temp_dir().join(format!("dae-conflict-self-{}", std::process::id()));
    let directory = root.join("folder");
    fs::create_dir_all(&directory).expect("create directory");
    fs::write(directory.join("file.txt"), "content").expect("write file");

    let move_progress = TestProgress::new();
    move_entries_with_progress(
        vec![(directory.join("file.txt"), ConflictAction::Replace)],
        directory.clone(),
        &move_progress,
    )
    .expect("move onto itself is skipped");
    assert_eq!(
        fs::read_to_string(directory.join("file.txt")).expect("file survives"),
        "content"
    );
    assert_eq!(move_progress.total.load(AtomicOrdering::Relaxed), 0);

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn reports_conflicts_for_the_dialog_and_skips_self_transfers() {
    use super::transfer::{self, TransferSource};
    use std::sync::Arc;

    let root = std::env::temp_dir().join(format!("dae-conflict-report-{}", std::process::id()));
    let source_dir = root.join("source");
    let destination_dir = root.join("destination");
    fs::create_dir_all(&source_dir).expect("create source directory");
    fs::create_dir_all(&destination_dir).expect("create destination directory");
    fs::write(source_dir.join("clashing.txt"), "source bytes").expect("write clashing source");
    fs::write(source_dir.join("fresh.txt"), "fresh bytes").expect("write fresh source");
    // A file already sitting in the destination directory: moving the whole
    // batch into its own parent must not report it as a conflict.
    fs::write(destination_dir.join("clashing.txt"), "target bytes")
        .expect("write clashing target");

    let backend: Arc<dyn FileSystemBackend> = vfs::resolve(&source_dir.to_string_lossy())
        .expect("resolve local backend");

    let source = |name: &str| TransferSource {
        path: source_dir.join(name).to_string_lossy().into_owned(),
        backend: backend.clone(),
        on_conflict: ConflictAction::Fail,
    };

    let conflicts = transfer::find_conflicts(
        vec![source("clashing.txt"), source("fresh.txt")],
        &destination_dir.to_string_lossy(),
        &backend,
    )
    .expect("find conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].name, "clashing.txt");
    assert_eq!(conflicts[0].source_size, Some("source bytes".len() as u64));
    assert_eq!(conflicts[0].target_size, Some("target bytes".len() as u64));
    assert!(conflicts[0].source_modified_at.is_some());
    assert!(conflicts[0].target_modified_at.is_some());

    // A file moved into its own directory lands on itself: no conflict.
    let self_conflicts = transfer::find_conflicts(
        vec![TransferSource {
            path: destination_dir.join("clashing.txt").to_string_lossy().into_owned(),
            backend: backend.clone(),
            on_conflict: ConflictAction::Fail,
        }],
        &destination_dir.to_string_lossy(),
        &backend,
    )
    .expect("find self conflicts");
    assert!(self_conflicts.is_empty());

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn resolves_streaming_transfer_conflicts_across_backends() {
    use super::local::LocalBackend;
    use super::transfer::{self, TransferSource};
    use std::sync::Arc;

    let root = std::env::temp_dir().join(format!("dae-conflict-stream-{}", std::process::id()));
    let source_dir = root.join("source");
    let destination_dir = root.join("destination");
    fs::create_dir_all(&source_dir).expect("create source directory");
    fs::create_dir_all(&destination_dir).expect("create destination directory");
    fs::write(source_dir.join("data.bin"), "streamed").expect("write source file");
    fs::write(destination_dir.join("data.bin"), "existing").expect("write target file");

    // Two distinct Arcs force the streaming engine.
    let source_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);
    let destination_backend: Arc<dyn FileSystemBackend> = Arc::new(LocalBackend);

    let keep_progress = TestProgress::new();
    transfer::copy_entries(
        vec![TransferSource {
            path: source_dir.join("data.bin").to_string_lossy().into_owned(),
            backend: source_backend.clone(),
            on_conflict: ConflictAction::KeepBoth,
        }],
        &destination_dir.to_string_lossy(),
        &destination_backend,
        &keep_progress,
    )
    .expect("stream copy keeping both");
    assert_eq!(
        fs::read_to_string(destination_dir.join("data 副本.bin")).expect("kept streamed copy"),
        "streamed"
    );

    let replace_progress = TestProgress::new();
    transfer::copy_entries(
        vec![TransferSource {
            path: source_dir.join("data.bin").to_string_lossy().into_owned(),
            backend: source_backend.clone(),
            on_conflict: ConflictAction::Replace,
        }],
        &destination_dir.to_string_lossy(),
        &destination_backend,
        &replace_progress,
    )
    .expect("stream copy with replace");
    assert_eq!(
        fs::read_to_string(destination_dir.join("data.bin")).expect("replaced streamed copy"),
        "streamed"
    );
    assert_eq!(
        replace_progress.completed.load(AtomicOrdering::Relaxed),
        replace_progress.total.load(AtomicOrdering::Relaxed)
    );

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn serves_local_paths_through_the_backend_trait() {
    let directory = std::env::temp_dir().join(format!("dae-vfs-trait-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("create test directory");

    let path = directory.to_string_lossy().into_owned();
    let backend = vfs::resolve(&path).expect("local path resolves");

    let created = backend
        .create_entry(&path, "through-trait.txt", NewEntryKind::File)
        .expect("create through trait object");
    assert!(directory.join("through-trait.txt").is_file());

    let view = backend.read_dir(&path).expect("read through trait object");
    assert!(
        view.entries
            .iter()
            .any(|entry| entry.name == "through-trait.txt")
    );

    backend
        .remove(&created)
        .expect("delete through trait object");

    fs::remove_dir_all(directory).expect("remove test directory");
}

/// Exercises the connection store through its public commands. One test
/// function because the registry is a process-global singleton.
#[test]
fn saves_updates_and_deletes_connections() {
    use super::connections::{self, Protocol, SaveConnectionInput};

    let config_dir =
        std::env::temp_dir().join(format!("dae-connections-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&config_dir);
    connections::use_config_dir_for_tests(config_dir.clone()).expect("initialize store");

    let saved = connections::save_connection(SaveConnectionInput {
        protocol: Protocol::Smb,
        host: "  MyServer.Local  ".into(),
        port: Some(445),
        username: Some(" alice ".into()),
        password: Some("session-only".into()),
        // Session memory instead of the OS keychain keeps the test hermetic.
        remember_password: false,
    })
    .expect("save connection");

    assert_eq!(saved.id, "smb://myserver.local:445");
    assert_eq!(saved.host, "myserver.local");
    assert_eq!(saved.username.as_deref(), Some("alice"));

    let listed = connections::list_connections().expect("list connections");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], saved);

    let persisted =
        fs::read_to_string(config_dir.join("connections.json")).expect("connections file exists");
    assert!(
        !persisted.contains("password"),
        "passwords must never be persisted"
    );
    assert!(!persisted.contains("session-only"));

    let (username, password) =
        connections::resolve_credentials(Protocol::Smb, "MyServer.Local", Some(445));
    assert_eq!(username.as_deref(), Some("alice"));
    assert_eq!(password.as_deref(), Some("session-only"));

    // Saving the same server again updates in place and keeps the credential.
    connections::save_connection(SaveConnectionInput {
        protocol: Protocol::Smb,
        host: "myserver.local".into(),
        port: Some(445),
        username: Some("bob".into()),
        password: None,
        remember_password: false,
    })
    .expect("update connection");

    let listed = connections::list_connections().expect("list after update");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].username.as_deref(), Some("bob"));
    let (_, password) =
        connections::resolve_credentials(Protocol::Smb, "myserver.local", Some(445));
    assert_eq!(password.as_deref(), Some("session-only"));

    // Reopening the store reloads from disk.
    connections::use_config_dir_for_tests(config_dir.clone()).expect("reload store");
    let listed = connections::list_connections().expect("list after reload");
    assert_eq!(listed.len(), 1);

    connections::delete_connection("smb://myserver.local:445".into()).expect("delete connection");
    assert!(
        connections::list_connections()
            .expect("list after delete")
            .is_empty()
    );
    let (_, password) =
        connections::resolve_credentials(Protocol::Smb, "myserver.local", Some(445));
    assert_eq!(password, None);

    fs::remove_dir_all(config_dir).expect("remove test directory");
}

#[test]
fn display_name_from_path_handles_separators_and_roots() {
    use super::types::display_name_from_path;

    assert_eq!(display_name_from_path(r"C:\Users\alice\docs"), "docs");
    assert_eq!(display_name_from_path("/home/alice/pictures/"), "pictures");
    assert_eq!(display_name_from_path(r"\wsl$\Ubuntu"), "Ubuntu");
    assert_eq!(display_name_from_path("smb://nas.local/media"), "media");
    // Trailing separators are ignored, so drive roots keep their letter and
    // connection roots show their host.
    assert_eq!(display_name_from_path(r"C:\"), "C:");
    assert_eq!(display_name_from_path("smb://nas.local"), "nas.local");
    assert_eq!(display_name_from_path("/"), "/");
}

#[test]
fn upsert_recent_dedupes_orders_and_caps() {
    use super::recents::{RecentItem, RecentSource, upsert_recent};
    use super::types::EntryKind;

    let make = |path: &str, accessed_at: u64| RecentItem {
        path: path.to_string(),
        name: path.to_string(),
        kind: EntryKind::File,
        source: RecentSource::Opened,
        accessed_at,
    };

    let mut items = vec![make("a", 1), make("b", 2), make("c", 3)];

    // Re-recording an existing path moves it to the front without duplicating.
    upsert_recent(&mut items, make("b", 4), 10);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].path, "b");
    assert_eq!(items[0].accessed_at, 4);

    // The cap evicts the least recently used entries.
    upsert_recent(&mut items, make("d", 5), 2);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].path, "d");
    assert_eq!(items[1].path, "b");
}

#[test]
fn seed_spaces_creates_four_presets() {
    use super::spaces::seed_spaces;

    let spaces = seed_spaces();
    assert_eq!(spaces.len(), 4);
    assert!(spaces.iter().all(|space| space.is_preset));
    assert!(spaces.iter().all(|space| space.items.is_empty()));
    let ids: Vec<&str> = spaces.iter().map(|space| space.id.as_str()).collect();
    assert_eq!(ids, ["work", "personal", "shared", "archive"]);
}

fn make_archive_source_tree(root: &Path) {
    fs::create_dir_all(root.join("资料").join("nested")).expect("create nested directory");
    fs::write(root.join("root.txt"), "root content").expect("write root file");
    fs::write(root.join("资料/nested/leaf.txt"), "leaf content").expect("write leaf file");
}

#[test]
fn compresses_and_extracts_every_archive_format() {
    use super::archive::{ArchiveFormat, compress_sync, extract_sync};

    let root = std::env::temp_dir().join(format!("dae-archive-test-{}", std::process::id()));
    let source = root.join("bundle");
    let output = root.join("output");
    fs::create_dir_all(&source).expect("create source directory");
    fs::create_dir_all(&output).expect("create output directory");
    make_archive_source_tree(&source);

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::SevenZip,
    ] {
        let compress_progress = TestProgress::new();
        let archive_path = compress_sync(
            vec![source.to_string_lossy().into_owned()],
            &output.to_string_lossy(),
            format,
            &compress_progress,
        )
        .expect("compress archive");

        let created = Path::new(&archive_path);
        assert!(created.is_file(), "archive exists: {archive_path}");
        assert_eq!(
            compress_progress.completed.load(AtomicOrdering::Relaxed),
            compress_progress.total.load(AtomicOrdering::Relaxed),
            "compression progress completes for {format:?}"
        );

        let extract_progress = TestProgress::new();
        let destination = extract_sync(&archive_path, None, &extract_progress)
            .expect("extract archive");

        let extracted_root = Path::new(&destination).join("bundle");
        assert_eq!(
            fs::read_to_string(extracted_root.join("root.txt")).expect("read extracted root"),
            "root content",
            "round-trip preserves files for {format:?}"
        );
        assert_eq!(
            fs::read_to_string(extracted_root.join("资料/nested/leaf.txt"))
                .expect("read extracted leaf"),
            "leaf content",
            "round-trip preserves nested unicode paths for {format:?}"
        );
        assert_eq!(
            extract_progress.completed.load(AtomicOrdering::Relaxed),
            extract_progress.total.load(AtomicOrdering::Relaxed),
            "extraction progress completes for {format:?}"
        );

        fs::remove_dir_all(destination).expect("remove extraction folder");
    }

    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn rejects_path_traversal_entries_during_extraction() {
    use super::archive::extract_sync;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    let root = std::env::temp_dir().join(format!("dae-zip-slip-test-{}", std::process::id()));
    fs::create_dir_all(&root).expect("create test directory");

    let archive_path = root.join("malicious.zip");
    let file = fs::File::create(&archive_path).expect("create malicious archive");
    let mut writer = ZipWriter::new(file);
    writer
        .start_file("../escaped.txt", SimpleFileOptions::default())
        .expect("start traversal entry");
    writer.write_all(b"escaped").expect("write traversal entry");
    writer.finish().expect("finish malicious archive");

    let error = extract_sync(&archive_path.to_string_lossy(), None, &TestProgress::new())
        .expect_err("traversal entries must be blocked");

    fs::remove_file(&archive_path).expect("remove malicious archive");
    fs::remove_dir(&root).expect("remove test directory");

    assert!(matches!(error, FileSystemError::InvalidInput(_)));
    assert!(!root.join("../escaped.txt").exists());
}

#[test]
fn derives_extraction_folder_stems_from_archive_names() {
    use super::archive::extraction_stem_of;

    assert_eq!(extraction_stem_of(Path::new("Photos.tar.gz")), "Photos");
    assert_eq!(extraction_stem_of(Path::new("backup.TGZ")), "backup");
    assert_eq!(extraction_stem_of(Path::new("报告.zip")), "报告");
    assert_eq!(extraction_stem_of(Path::new("bundle.7z")), "bundle");
    assert_eq!(extraction_stem_of(Path::new("archive.tar")), "archive");
}
