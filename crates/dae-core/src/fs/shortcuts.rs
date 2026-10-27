//! Windows shortcuts (`.lnk`) for the Alt-drop side of drag and drop.
//!
//! This is the engine-free half of the Tauri shell's `create_shortcuts`. It
//! lives here now because the Qt shell needs the same operation for a link
//! drop, and the shortcut is a property of the file system rather than of a
//! window or a webview.

use super::error::FileSystemError;

/// Creates one shortcut per source inside `destination`, returning their paths.
///
/// The shortcut points at the source path and carries it as the description,
/// which is what Explorer shows on hover. Colliding names grow the same way
/// Explorer's do: `name - Shortcut.lnk`, `name - Shortcut (2).lnk`, ….
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
    #[cfg(not(windows))]
    {
        let _ = destination;
        Err(FileSystemError::Internal(
            "Shortcuts are only implemented on Windows".into(),
        ))
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

/// `{destination}\{stem} - Shortcut.lnk`, appending ` (2)`, ` (3)`… while the
/// name is taken — the same scheme Explorer uses.
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
