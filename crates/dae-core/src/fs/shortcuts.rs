//! Shortcuts for the Alt-drop side of drag and drop: `.lnk` on Windows,
//! symbolic links on Unix.
//!
//! This is the engine-free half of the Tauri shell's `create_shortcuts`. It
//! lives here now because the Qt shell needs the same operation for a link
//! drop, and the shortcut is a property of the file system rather than of a
//! window or a webview.

use super::error::FileSystemError;

/// Creates one shortcut per source inside `destination`, returning their paths.
///
/// Windows writes `.lnk` files that carry the source as path and description,
/// named `name - Shortcut.lnk`, `name - Shortcut (2).lnk`, … the way Explorer
/// does. Unix creates symbolic links named `Link to name`, the desktop
/// convention; the collision counter goes before the extension
/// (`Link to name (2).txt`) so the file type stays readable.
pub fn create_shortcuts(
    sources: &[String],
    destination: &str,
) -> Result<Vec<String>, FileSystemError> {
    if sources.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Choose at least one entry to link".into(),
        ));
    }

    #[cfg(windows)]
    {
        create_shortcuts_windows(sources, destination)
    }
    #[cfg(unix)]
    {
        create_shortcuts_unix(sources, destination)
    }
}

#[cfg(windows)]
fn create_shortcuts_windows(
    sources: &[String],
    destination: &str,
) -> Result<Vec<String>, FileSystemError> {
    use std::iter::once;

    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HRESULT, Interface, PCWSTR};

    /// RAII COM apartment. `owned` records whether this call initialized COM:
    /// S_FALSE (0x1) means the thread already had an apartment, so dropping
    /// must not uninitialize it.
    struct CoApartment {
        owned: bool,
    }
    impl CoApartment {
        fn enter() -> Result<Self, FileSystemError> {
            let result: HRESULT =
                unsafe { CoInitializeEx(Some(std::ptr::null()), COINIT_APARTMENTTHREADED) };
            match result.0 {
                0 => Ok(Self { owned: true }),
                1 => Ok(Self { owned: false }),
                _ => Err(FileSystemError::Internal(format!(
                    "CoInitializeEx failed with HRESULT {:#010x}",
                    result.0
                ))),
            }
        }
    }
    impl Drop for CoApartment {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn wide_null(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(once(0)).collect()
    }

    let _apartment = CoApartment::enter()?;

    let mut created = Vec::new();
    for source in sources {
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| FileSystemError::Internal(error.to_string()))?;

        let source_wide = wide_null(source);
        unsafe {
            link.SetPath(PCWSTR::from_raw(source_wide.as_ptr()))
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            // Explorer-style description so the shortcut's tooltip mirrors
            // what the shell itself would have written.
            link.SetDescription(PCWSTR::from_raw(source_wide.as_ptr()))
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
        }

        let stem = std::path::Path::new(source)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                FileSystemError::InvalidInput(format!("Cannot derive a name from {source}"))
            })?;
        let link_path = unique_shortcut_path(destination, stem);

        let persist: IPersistFile = link
            .cast::<IPersistFile>()
            .map_err(|error| FileSystemError::Internal(error.to_string()))?;
        let link_wide = wide_null(&link_path);
        unsafe {
            persist
                .Save(PCWSTR::from_raw(link_wide.as_ptr()), true)
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
        }

        created.push(link_path);
    }

    Ok(created)
}

/// `{destination}/Link to {name}`, appending ` (2)`, ` (3)`… while the name
/// is taken. The counter goes before the extension so type detection still
/// sees it; `symlink_metadata` also counts broken links as taken names.
#[cfg(unix)]
fn unique_link_path(destination: &str, name: &str) -> String {
    let base = if destination.ends_with('/') {
        destination.to_string()
    } else {
        format!("{destination}/")
    };

    let mut candidate = format!("{base}Link to {name}");
    let mut counter = 2u32;
    while std::fs::symlink_metadata(&candidate).is_ok() {
        candidate = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => {
                format!("{base}Link to {stem} ({counter}).{extension}")
            }
            _ => format!("{base}Link to {name} ({counter})"),
        };
        counter += 1;
    }
    candidate
}

#[cfg(unix)]
fn create_shortcuts_unix(
    sources: &[String],
    destination: &str,
) -> Result<Vec<String>, FileSystemError> {
    let mut created = Vec::new();
    for source in sources {
        let name = std::path::Path::new(source)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                FileSystemError::InvalidInput(format!("Cannot derive a name from {source}"))
            })?;
        let link_path = unique_link_path(destination, name);
        std::os::unix::fs::symlink(source, &link_path)?;
        created.push(link_path);
    }
    Ok(created)
}

/// `{destination}\{stem} - Shortcut.lnk`, appending ` (2)`, ` (3)`… while the
/// name is taken — the same scheme Explorer uses.
#[cfg(windows)]
fn unique_shortcut_path(destination: &str, stem: &str) -> String {
    let separator = if destination.contains('\\') && !destination.contains('/') {
        '\\'
    } else {
        '/'
    };
    let base = if destination.ends_with('\\') || destination.ends_with('/') {
        destination.to_string()
    } else {
        format!("{destination}{separator}")
    };

    let mut candidate = format!("{base}{stem} - Shortcut.lnk");
    let mut counter = 2u32;
    while std::path::Path::new(&candidate).exists() {
        candidate = format!("{base}{stem} - Shortcut ({counter}).lnk");
        counter += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn shortcut_names_grow_until_they_are_free() {
        let directory =
            std::env::temp_dir().join(format!("dae-shortcut-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create test directory");
        let destination = directory.to_string_lossy().into_owned();

        let first = unique_shortcut_path(&destination, "report.txt");
        assert!(first.ends_with("report.txt - Shortcut.lnk"));
        std::fs::write(&first, b"").expect("occupy the first name");

        let second = unique_shortcut_path(&destination, "report.txt");
        assert!(second.ends_with("report.txt - Shortcut (2).lnk"));

        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn link_names_grow_until_they_are_free() {
        let directory =
            std::env::temp_dir().join(format!("dae-link-name-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create test directory");
        let destination = directory.to_string_lossy().into_owned();

        let first = unique_link_path(&destination, "report.txt");
        assert!(first.ends_with("Link to report.txt"));
        std::fs::write(&first, b"").expect("occupy the first name");

        let second = unique_link_path(&destination, "report.txt");
        assert!(second.ends_with("Link to report (2).txt"));
        std::fs::write(&second, b"").expect("occupy the second name");

        // A broken symlink still occupies its name.
        let broken = directory.join("Link to report (3).txt");
        std::os::unix::fs::symlink(directory.join("missing"), &broken).expect("broken link");
        let fourth = unique_link_path(&destination, "report.txt");
        assert!(fourth.ends_with("Link to report (4).txt"));

        // Extensionless names take the counter at the end.
        let plain = unique_link_path(&destination, "README");
        assert!(plain.ends_with("Link to README"));
        std::fs::write(&plain, b"").expect("occupy the plain name");
        let plain_second = unique_link_path(&destination, "README");
        assert!(plain_second.ends_with("Link to README (2)"));

        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn creates_real_symbolic_links() {
        let root = std::env::temp_dir().join(format!("dae-link-test-{}", std::process::id()));
        let file = root.join("source.txt");
        let folder = root.join("folder");
        let destination = root.join("links");
        std::fs::create_dir_all(&destination).expect("create destination");
        std::fs::create_dir_all(&folder).expect("create source folder");
        std::fs::write(&file, b"payload").expect("write source");

        let sources = vec![
            file.to_string_lossy().into_owned(),
            folder.to_string_lossy().into_owned(),
        ];
        let destination = destination.to_string_lossy().into_owned();
        let created = create_shortcuts(&sources, &destination).expect("create links");

        assert_eq!(created.len(), 2);
        assert!(created[0].ends_with("Link to source.txt"));
        assert!(created[1].ends_with("Link to folder"));
        for (path, target) in created.iter().zip([&file, &folder]) {
            let metadata = std::fs::symlink_metadata(path).expect("link exists");
            assert!(metadata.file_type().is_symlink());
            assert_eq!(std::fs::read_link(path).expect("read link"), *target);
        }
        assert_eq!(std::fs::read(&created[0]).expect("read through link"), b"payload");

        // Linking the same source again grows the name instead of failing.
        let again = create_shortcuts(&sources[..1], &destination).expect("create second link");
        assert!(again[0].ends_with("Link to source (2).txt"));

        std::fs::remove_dir_all(root).expect("remove test directory");
    }

    #[cfg(windows)]
    #[test]
    fn writes_a_real_windows_shortcut() {
        let root = std::env::temp_dir().join(format!("dae-shortcut-com-{}", std::process::id()));
        let target = root.join("source.txt");
        let destination = root.join("links");
        std::fs::create_dir_all(&destination).expect("create destination");
        std::fs::write(&target, b"payload").expect("write source");

        let created = create_shortcuts(
            &[target.to_string_lossy().into_owned()],
            &destination.to_string_lossy(),
        )
        .expect("create shortcut");

        assert_eq!(created.len(), 1);
        assert!(created[0].ends_with("source - Shortcut.lnk"));
        let metadata = std::fs::metadata(&created[0]).expect("shortcut exists");
        assert!(metadata.len() > 0);

        std::fs::remove_dir_all(root).expect("remove test directory");
    }
}
