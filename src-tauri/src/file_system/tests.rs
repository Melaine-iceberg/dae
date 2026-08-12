use super::directory::{
    DirectoryEntry, EntryKind, build_breadcrumbs, compare_entries, normalize_path_for_display,
    read_directory_sync,
};
use super::error::FileSystemError;
use super::operations::{
    copy_entries_with_progress, delete_entries_with_progress, move_entries_with_progress,
    rename_entry_sync,
};
use super::progress::FileOperationProgressReporterTrait;
use std::fs;
use std::path::Path;

struct TestProgress {
    completed: u64,
    total: u64,
}

impl TestProgress {
    fn new() -> Self {
        Self {
            completed: 0,
            total: 0,
        }
    }
}

impl FileOperationProgressReporterTrait for TestProgress {
    fn start(&mut self, total: u64) {
        self.total = total;
    }

    fn begin_entry(&mut self, _path: &Path) {}

    fn advance_by(&mut self, units: u64, _path: &Path) {
        self.completed += units;
    }

    fn finish(&mut self) {
        assert_eq!(self.completed, self.total);
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

    entries.sort_by(compare_entries);

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
fn performs_file_operations_and_reports_entry_progress() {
    let directory =
        std::env::temp_dir().join(format!("dae-file-operation-test-{}", std::process::id()));
    let source = directory.join("source");
    let destination = directory.join("destination");
    let nested_file = source.join("nested.txt");

    fs::create_dir_all(&source).expect("create source directory");
    fs::create_dir_all(&destination).expect("create destination directory");
    fs::write(&nested_file, "copied content").expect("create source file");

    let mut copy_progress = TestProgress::new();
    copy_entries_with_progress(
        vec![source.clone()],
        destination.clone(),
        &mut copy_progress,
    )
    .expect("copy directory");
    assert_eq!(copy_progress.completed, 2);
    assert_eq!(copy_progress.total, 2);

    let copied_directory = destination.join("source");
    assert_eq!(
        fs::read_to_string(copied_directory.join("nested.txt")).expect("read copied file"),
        "copied content"
    );
    assert!(source.exists());

    let mut duplicate_progress = TestProgress::new();
    let duplicate_error = copy_entries_with_progress(
        vec![source.clone()],
        destination.clone(),
        &mut duplicate_progress,
    )
    .expect_err("copying over an existing entry should fail");
    assert!(matches!(duplicate_error, FileSystemError::AlreadyExists(_)));

    let mut nested_progress = TestProgress::new();
    let nested_error =
        copy_entries_with_progress(vec![source.clone()], source.clone(), &mut nested_progress)
            .expect_err("copying a folder into itself should fail");
    assert!(matches!(nested_error, FileSystemError::InvalidInput(_)));

    rename_entry_sync(nested_file.clone(), "renamed.txt".into()).expect("rename file");
    let renamed_file = source.join("renamed.txt");
    assert!(renamed_file.exists());

    let mut move_progress = TestProgress::new();
    move_entries_with_progress(
        vec![renamed_file.clone()],
        destination.clone(),
        &mut move_progress,
    )
    .expect("move file");
    let moved_file = destination.join("renamed.txt");
    assert!(moved_file.exists());
    assert!(!renamed_file.exists());
    assert_eq!(move_progress.completed, 1);
    assert_eq!(move_progress.total, 1);

    let mut delete_progress = TestProgress::new();
    delete_entries_with_progress(vec![moved_file.clone()], &mut delete_progress)
        .expect("delete file");
    assert!(!moved_file.exists());
    assert_eq!(delete_progress.completed, 1);
    assert_eq!(delete_progress.total, 1);

    fs::remove_dir_all(directory).expect("remove test directory");
}
