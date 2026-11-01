#![cfg(any(target_os = "linux", test))]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

//! Linux desktop registration for a portable build.
//!
//! A `dae.desktop` inside the release zip is inert: the scheme handler is only
//! found when the file is in `~/.local/share/applications` and `xdg-mime`
//! points `x-scheme-handler/dae` at it. Windows registers its URL scheme from
//! the registry on every launch; this is the same contract for Linux, without
//! requiring the user to run an installer.
//!
//! A package-installed build gets its entry from the package instead. That file
//! is in `$XDG_DATA_DIRS` and carries `MimeType=inode/directory`, which the file
//! written here does not; because user data directories are searched first, a
//! copy in `~/.local/share/applications` would shadow it, drop dae from "Open
//! With" and survive `dnf remove`. So the entry is only written when no
//! packaged one exists — the association is repaired either way.
//!
//! Only the scheme is registered here. Making dae the folder handler is a
//! separate, explicit action in the settings panel (`default_manager`), and an
//! app that silently took over folders would be a different product decision.
//!
//! `xdg-mime` is queried before it is invoked: a launch is not worth one
//! subprocess when the association is already correct. A missing tool is
//! logged and does not fail startup; the file is still written, so a desktop
//! that indexes applications without `xdg-mime` can use it.

use std::path::Path;
use std::path::PathBuf;

const DESKTOP_ID: &str = "dae.desktop";
const SCHEME: &str = "x-scheme-handler/dae";

pub(crate) fn ensure_registered() {
    if let Some(entry) = installed_entry() {
        // The package owns the entry; writing another one here would hide it.
        log::debug!(target: "dae::linux", "desktop entry provided by {}", entry.display());
        set_default_handler();
        return;
    }

    let Some(exe) = std::env::current_exe().ok() else {
        log::warn!(target: "dae::linux", "could not resolve the executable for scheme registration");
        return;
    };
    let Some(applications) = applications_dir() else {
        log::warn!(target: "dae::linux", "could not locate ~/.local/share/applications");
        return;
    };

    let entry = applications.join(DESKTOP_ID);
    let desired = contents(&exe);
    let changed = std::fs::read_to_string(&entry)
        .map(|current| current != desired)
        .unwrap_or(true);
    if changed {
        if let Err(error) = std::fs::write(&entry, desired) {
            log::warn!(target: "dae::linux", "could not write {}: {error}", entry.display());
            return;
        }
        // The MIME cache is what desktop shells read when they resolve the
        // handler; a missing tool is not a failed registration.
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&applications)
            .status();
    }

    set_default_handler();
}

/// The `dae.desktop` an installer put in one of the system data directories.
///
/// `XDG_DATA_DIRS` defaults to `/usr/local/share:/usr/share` when unset, which
/// is where both the `.deb` and the `.rpm` install theirs.
fn installed_entry() -> Option<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    installed_entry_in(&dirs)
}

fn installed_entry_in(dirs: &str) -> Option<PathBuf> {
    dirs.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join("applications").join(DESKTOP_ID))
        .find(|entry| entry.is_file())
}

fn set_default_handler() {
    if current_handler().as_deref() == Some(DESKTOP_ID) {
        return;
    }

    match std::process::Command::new("xdg-mime")
        .args(["default", DESKTOP_ID, SCHEME])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!(target: "dae::linux", "xdg-mime exited with {status}"),
        Err(error) => log::warn!(target: "dae::linux", "could not run xdg-mime: {error}"),
    }
}

fn applications_dir() -> Option<PathBuf> {
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME")
        && !data_home.is_empty()
    {
        let path = PathBuf::from(data_home).join("applications");
        std::fs::create_dir_all(&path).ok()?;
        return Some(path);
    }

    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    let path = PathBuf::from(home).join(".local/share/applications");
    std::fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn current_handler() -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", SCHEME])
        .output()
        .ok()?;
    let handler = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!handler.is_empty()).then_some(handler)
}

/// The desktop entry written for the running executable.
///
/// `Icon` is the PNG beside the portable executable when there is one, so a
/// desktop that never installed an icon theme still shows the application
/// icon; otherwise the conventional `dae` theme name is used.
fn contents(exe: &Path) -> String {
    let icon = exe
        .parent()
        .map(|parent| parent.join("dae.png"))
        .filter(|icon| icon.is_file())
        .map(|icon| icon.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dae".to_owned());

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=dae\n\
         GenericName=File Manager\n\
         Comment=Browse local, network and cloud files\n\
         Exec={}\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=System;FileManager;Utility;\n\
         MimeType={SCHEME};\n\
         StartupWMClass=dae\n",
        exec_line(exe)
    )
}

/// The `Exec=` value. `%u` is the URL or path the desktop passes to the
/// handler; the executable is always quoted, which the Desktop Entry spec
/// accepts even when the path has no spaces.
fn exec_line(exe: &Path) -> String {
    let escaped = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\" %u")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_the_executable_and_keeps_the_url_placeholder() {
        assert_eq!(exec_line(Path::new("/opt/dae/dae")), "\"/opt/dae/dae\" %u");
        assert_eq!(
            exec_line(Path::new("/home/me/My Apps/dae")),
            "\"/home/me/My Apps/dae\" %u"
        );
    }

    #[test]
    fn escapes_quotes_and_backslashes_for_the_desktop_parser() {
        assert_eq!(
            exec_line(Path::new("/tmp/a\"b/dae")),
            "\"/tmp/a\\\"b/dae\" %u"
        );
        assert_eq!(
            exec_line(Path::new("/tmp/a\\b/dae")),
            "\"/tmp/a\\\\b/dae\" %u"
        );
        // A literal percent has to become %% or the desktop parser reads it
        // as a field code.
        assert_eq!(exec_line(Path::new("/tmp/a%b/dae")), "\"/tmp/a%%b/dae\" %u");
    }

    #[test]
    fn the_entry_declares_the_scheme_and_the_portable_icon() {
        let entry = contents(Path::new("/opt/dae/dae"));
        assert!(entry.contains("MimeType=x-scheme-handler/dae;"));
        assert!(entry.contains("Exec=\"/opt/dae/dae\" %u"));
        assert!(entry.contains("StartupWMClass=dae"));
    }

    /// The `.deb`/`.rpm` entry lives under `/usr/share`; finding it is what
    /// keeps the portable path from shadowing it.
    ///
    /// Unix only, and not because the code is: the search path is separated by
    /// `:`, which an absolute Windows path also contains (`C:\…`), so the split
    /// would break a path the function is never asked to read on Linux.
    #[test]
    #[cfg(unix)]
    fn finds_a_packaged_entry_under_xdg_data_dirs() {
        let root = std::env::temp_dir().join(format!("dae-desktop-{}", std::process::id()));
        let applications = root.join("share/applications");
        std::fs::create_dir_all(&applications).unwrap();
        let entry = applications.join(DESKTOP_ID);
        std::fs::write(&entry, "[Desktop Entry]\nName=dae\n").unwrap();

        let dirs = format!("/nonexistent/dir:{}", root.join("share").display());
        assert_eq!(installed_entry_in(&dirs).as_deref(), Some(entry.as_path()));

        std::fs::remove_file(&entry).unwrap();
        assert!(installed_entry_in(&dirs).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// An empty variable means "unset" per the XDG spec, so the default list
    /// has to be used rather than an empty search path.
    #[test]
    fn ignores_empty_entries_in_the_search_path() {
        let dirs = "/nonexistent/dir::/also/missing";
        assert!(installed_entry_in(dirs).is_none());
    }
}
