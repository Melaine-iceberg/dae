//! "New from template": one file or folder per entry in a templates folder.
//!
//! The model is deliberately the file system itself. A template is a real
//! file or folder in one directory, so creating or editing a template needs
//! no editor here — "打开模板文件夹" opens that directory in the OS, and the
//! next menu reads whatever landed. Folders sort before files, both by name,
//! which is what makes a numbered prefix useful for ordering.
//!
//! Only direct children of the templates directory are templates. The
//! instantiation path re-checks that, so a stale menu row cannot turn into an
//! arbitrary copy from anywhere on disk.

use super::error::FileSystemError;
use super::local::{copy_entry_unreported, validate_entry_name};
use super::types::{NewEntryKind, path_to_string};
use crate::settings;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// One entry in the templates folder, as the menu needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// The configured templates directory, or `config_dir()/templates` when the
/// document leaves it empty.
pub fn directory() -> Result<PathBuf, FileSystemError> {
    let configured = settings::load_settings()?
        .templates
        .directory
        .filter(|directory| !directory.trim().is_empty());

    match configured {
        Some(directory) => Ok(PathBuf::from(directory)),
        None => Ok(crate::paths::config_dir()?.join("templates")),
    }
}

/// The templates directory, created when it does not exist yet.
///
/// Listing does not create it — an empty folder and a missing one both mean
/// "no templates" — but opening the folder should hand the user somewhere to
/// put the first one.
pub fn ensure_directory() -> Result<PathBuf, FileSystemError> {
    let directory = directory()?;
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// Templates in the configured directory, folders first, then case-insensitive
/// by name. A missing directory is an empty collection, not an error.
pub fn list() -> Result<Vec<Template>, FileSystemError> {
    list_in(&directory()?)
}

fn list_in(directory: &Path) -> Result<Vec<Template>, FileSystemError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut templates = Vec::new();
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() && !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        templates.push(Template {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
            path: path_to_string(&path),
        });
    }

    templates.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(templates)
}

/// Instantiates `template` as `directory/name`, returning the new path.
pub fn create_from_template(
    directory: &str,
    template: &str,
    name: &str,
) -> Result<String, FileSystemError> {
    create_from_template_in(
        &PathBuf::from(directory),
        &self::directory()?,
        &PathBuf::from(template),
        name,
    )
}

fn create_from_template_in(
    directory: &Path,
    templates_root: &Path,
    template: &Path,
    name: &str,
) -> Result<String, FileSystemError> {
    validate_entry_name(name)?;

    if !fs::metadata(directory)?.is_dir() {
        return Err(FileSystemError::NotDirectory(path_to_string(directory)));
    }

    ensure_template_child(template, templates_root)?;

    let target = directory.join(name);
    if target.try_exists()? {
        return Err(FileSystemError::AlreadyExists(path_to_string(&target)));
    }

    copy_template(template, &target)?;
    Ok(path_to_string(&target))
}

/// Recreates `target` from `template` without a destination directory or a
/// name. Used by redo, where both were already fixed by the first creation.
pub fn create_from_template_at(target: &str, template: &str) -> Result<(), FileSystemError> {
    let template = PathBuf::from(template);
    ensure_template_child(&template, &directory()?)?;

    let target = PathBuf::from(target);
    if target.try_exists()? {
        return Err(FileSystemError::AlreadyExists(path_to_string(&target)));
    }

    copy_template(&template, &target)
}

/// The kind undo needs to restore the entry after a delete.
pub fn template_kind(template: &str) -> Result<NewEntryKind, FileSystemError> {
    let metadata = fs::symlink_metadata(template)?;
    Ok(if metadata.is_dir() {
        NewEntryKind::Directory
    } else {
        NewEntryKind::File
    })
}

fn copy_template(template: &Path, target: &Path) -> Result<(), FileSystemError> {
    match copy_entry_unreported(template, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            // A half-written tree would look like a usable template output
            // on the next attempt. The target did not exist before, so
            // removing whatever landed is exact.
            remove_created(target);
            Err(error)
        }
    }
}

fn remove_created(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// True when `template` is a direct child of the configured templates folder.
///
/// The parent is canonicalized, not the entry: a template may itself be a
/// symlink, and resolving it would reject a template whose target lives
/// elsewhere. What must stay put is the *menu's* namespace.
fn ensure_template_child(template: &Path, templates_root: &Path) -> Result<(), FileSystemError> {
    let parent = template.parent().ok_or_else(|| {
        FileSystemError::InvalidInput("A template must be a file or folder".to_owned())
    })?;
    let parent = dunce::canonicalize(parent)?;
    let root = dunce::canonicalize(templates_root)?;

    if parent != root {
        return Err(FileSystemError::InvalidInput(
            "A template must live directly in the templates folder".to_owned(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn unique_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "dae-templates-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&directory).expect("create the test directory");
        directory
    }

    fn write_file(path: &Path, contents: &[u8]) {
        let mut file = fs::File::create(path).expect("create the test file");
        file.write_all(contents).expect("write the test file");
    }

    #[test]
    fn folders_sort_before_files_and_names_ignore_case() {
        let directory = unique_directory("order");
        fs::create_dir(directory.join("zebra")).unwrap();
        fs::create_dir(directory.join("Alpha")).unwrap();
        write_file(&directory.join("beta.txt"), b"b");
        write_file(&directory.join("Gamma.md"), b"g");

        let listed = list_in(&directory).expect("list templates");
        let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zebra", "beta.txt", "Gamma.md"]);
        assert!(listed[0].is_dir);
        assert!(!listed[2].is_dir);

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_file_template_copies_its_contents() {
        let templates = unique_directory("file");
        let destination = unique_directory("file-output");
        let template = templates.join("note.txt");
        write_file(&template, b"template contents");

        let created = create_from_template_in(&destination, &templates, &template, "renamed.txt")
            .expect("instantiate the template");

        assert_eq!(fs::read(&created).unwrap(), b"template contents");
        assert!(template.exists(), "the template itself must survive");

        fs::remove_dir_all(&templates).unwrap();
        fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn a_folder_template_copies_the_tree() {
        let templates = unique_directory("folder");
        let destination = unique_directory("folder-output");
        let template = templates.join("project");
        fs::create_dir_all(template.join("src")).unwrap();
        write_file(&template.join("README.md"), b"readme");

        let created = create_from_template_in(&destination, &templates, &template, "new-project")
            .expect("instantiate the folder template");

        assert_eq!(
            fs::read(Path::new(&created).join("README.md")).unwrap(),
            b"readme"
        );
        assert!(template.join("src").is_dir());

        // The source tree is untouched, and the copy is an independent tree.
        write_file(&template.join("README.md"), b"changed");
        assert_eq!(
            fs::read(Path::new(&created).join("README.md")).unwrap(),
            b"readme"
        );

        fs::remove_dir_all(&templates).unwrap();
        fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn an_existing_destination_is_not_overwritten() {
        let templates = unique_directory("conflict");
        let destination = unique_directory("conflict-output");
        let template = templates.join("note.txt");
        write_file(&template, b"template");
        let existing = destination.join("note.txt");
        write_file(&existing, b"keep me");

        let error = create_from_template_in(&destination, &templates, &template, "note.txt")
            .expect_err("the existing entry must stop the copy");

        assert!(matches!(error, FileSystemError::AlreadyExists(_)));
        assert_eq!(fs::read(&existing).unwrap(), b"keep me");

        fs::remove_dir_all(&templates).unwrap();
        fs::remove_dir_all(&destination).unwrap();
    }

    #[test]
    fn only_direct_children_of_the_templates_folder_are_accepted() {
        let root = unique_directory("root");
        let outside = unique_directory("outside");
        let inside = root.join("note.txt");
        write_file(&inside, b"template");
        let escaped = outside.join("note.txt");
        write_file(&escaped, b"outside");

        ensure_template_child(&inside, &root).expect("a direct child is a template");
        assert!(matches!(
            ensure_template_child(&escaped, &root),
            Err(FileSystemError::InvalidInput(_))
        ));

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn a_separator_in_the_new_name_is_refused() {
        let templates = unique_directory("name");
        let destination = unique_directory("name-output");
        let template = templates.join("note.txt");
        write_file(&template, b"template");

        let error = create_from_template_in(&destination, &templates, &template, "../escape.txt")
            .expect_err("a path separator is not a name");

        assert!(matches!(error, FileSystemError::InvalidInput(_)));

        fs::remove_dir_all(&templates).unwrap();
        fs::remove_dir_all(&destination).unwrap();
    }
}
