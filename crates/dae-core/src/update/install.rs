//! Platform installer scripts and archive extraction.
//!
//! The running process never overwrites its own Qt libraries or executable:
//! those mappings cannot be replaced safely. Instead the scripts wait for this
//! process to exit, swap/copy the staged tree, and relaunch. Paths cross the
//! process boundary as script arguments, never as interpolated text, so spaces
//! and quotes are handled by the OS.

use std::fs::{self, File};
#[cfg(unix)]
use std::io::Read;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AvailableUpdate, UpdateError, download_verified};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
enum LayoutKind {
    Portable,
    MacBundle,
}

/// Where the app lives and where staging can be written without crossing a
/// volume boundary (a macOS app-bundle swap must be a same-volume rename).
#[derive(Debug, Clone)]
pub(super) struct InstallLayout {
    install_dir: PathBuf,
    staging_parent: PathBuf,
    executable: PathBuf,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    launch: PathBuf,
    kind: LayoutKind,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    bundle: Option<PathBuf>,
}

impl InstallLayout {
    pub(super) fn current() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;

        #[cfg(target_os = "macos")]
        {
            let mut cursor = exe.parent();
            while let Some(path) = cursor {
                if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                {
                    let parent = path.parent()?.to_path_buf();
                    return Some(Self {
                        install_dir: parent.clone(),
                        staging_parent: parent,
                        executable: exe.clone(),
                        launch: path.to_path_buf(),
                        kind: LayoutKind::MacBundle,
                        bundle: Some(path.to_path_buf()),
                    });
                }
                cursor = path.parent();
            }
            None
        }

        #[cfg(not(target_os = "macos"))]
        {
            let dir = exe.parent()?.to_path_buf();
            // `windeployqt`/`linuxdeploy` put the platform plugin next to the
            // executable. A `cargo run` from target/ does not, which keeps
            // development builds from updating themselves.
            if !dir.join("platforms").is_dir() {
                return None;
            }
            let staging_parent = dir.parent()?.to_path_buf();
            Some(Self {
                install_dir: dir,
                staging_parent,
                executable: exe.clone(),
                launch: exe,
                kind: LayoutKind::Portable,
                bundle: None,
            })
        }
    }

    fn ensure_writable(&self) -> Result<(), UpdateError> {
        let probe = self
            .install_dir
            .join(format!(".dae-update-probe-{}", std::process::id()));
        let result = File::create(&probe).and_then(|file| {
            let _ = file.sync_all();
            fs::remove_file(&probe)
        });
        result.map_err(|error| {
            UpdateError::Install(format!("the install directory is not writable: {error}"))
        })
    }

    fn new_staging_dir(&self) -> Result<PathBuf, UpdateError> {
        let staging = self.staging_parent.join(format!(
            ".dae-update-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            unique_suffix()
        ));
        fs::create_dir_all(&staging).map_err(|error| UpdateError::Install(error.to_string()))?;
        Ok(staging)
    }

    fn staged_launch(&self, staging: &Path) -> Result<PathBuf, UpdateError> {
        if self.kind == LayoutKind::MacBundle {
            let staged_app = staged_app_bundle(staging)?;
            let launch = staged_app.join("Contents/MacOS").join(
                self.executable
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("dae")),
            );
            if !launch.is_file() {
                return Err(UpdateError::Archive(format!(
                    "the update does not contain {}",
                    launch.display()
                )));
            }
            return Ok(launch);
        }

        let launch = staging.join(
            self.executable
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("dae")),
        );
        if !launch.is_file() {
            return Err(UpdateError::Archive(format!(
                "the update does not contain {}",
                launch.display()
            )));
        }
        Ok(launch)
    }

    fn start_installer(&self, staging: &Path) -> Result<(), UpdateError> {
        #[cfg(target_os = "macos")]
        {
            self.spawn_mac_installer(staging)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.spawn_portable_installer(staging)
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn spawn_portable_installer(&self, staging: &Path) -> Result<(), UpdateError> {
        let script = write_script(
            if cfg!(windows) { "ps1" } else { "sh" },
            if cfg!(windows) {
                PORTABLE_POWERSHELL
            } else {
                PORTABLE_SHELL
            },
        )?;

        let mut command = if cfg!(windows) {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ]);
            command
        } else {
            let mut command = Command::new("/bin/sh");
            command.arg(&script);
            command
        };

        if cfg!(windows) {
            command.arg(&script);
            command
                .arg("-TargetPid")
                .arg(std::process::id().to_string())
                .arg("-Staging")
                .arg(staging)
                .arg("-InstallPath")
                .arg(&self.install_dir)
                .arg("-LaunchPath")
                .arg(&self.launch);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
        } else {
            command
                .arg(std::process::id().to_string())
                .arg(staging)
                .arg(&self.install_dir)
                .arg(&self.launch);
        }

        spawn_detached(command).map_err(|error| {
            let _ = fs::remove_file(&script);
            UpdateError::Install(format!("could not start the installer: {error}"))
        })
    }

    #[cfg(target_os = "macos")]
    fn spawn_mac_installer(&self, staging: &Path) -> Result<(), UpdateError> {
        let staged_app = staged_app_bundle(staging)?;
        let bundle = self
            .bundle
            .as_ref()
            .ok_or_else(|| UpdateError::Install("missing app bundle".to_owned()))?;
        let script = write_script("sh", MAC_BUNDLE_SHELL)?;
        let mut command = Command::new("/bin/sh");
        command
            .arg(&script)
            .arg(std::process::id().to_string())
            .arg(staging)
            .arg(bundle)
            .arg(&staged_app)
            .arg(bundle);
        spawn_detached(command).map_err(|error| {
            let _ = fs::remove_file(&script);
            UpdateError::Install(format!("could not start the installer: {error}"))
        })
    }
}

pub(super) async fn download_and_install<F>(
    update: &AvailableUpdate,
    progress: &F,
) -> Result<(), UpdateError>
where
    F: Fn(u8) + Send + Sync,
{
    let layout = InstallLayout::current().ok_or(UpdateError::NotInstalled)?;
    layout.ensure_writable()?;
    progress(2);

    let (_download, zip_path) = download_verified(update, progress).await?;
    progress(92);

    let staging = layout.new_staging_dir()?;
    progress(94);
    if let Err(error) = extract_zip(&zip_path, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    progress(97);

    // Rejects an archive that does not contain the executable the installer
    // is about to relaunch, before anything outside the staging directory is
    // touched.
    if let Err(error) = layout.staged_launch(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if let Err(error) = layout.start_installer(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    progress(100);
    Ok(())
}

fn staged_app_bundle(staging: &Path) -> Result<PathBuf, UpdateError> {
    let mut bundles = Vec::new();
    let entries = fs::read_dir(staging).map_err(|error| UpdateError::Archive(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| UpdateError::Archive(error.to_string()))?;
        let path = entry.path();
        if path.is_dir()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            bundles.push(path);
        }
    }

    match bundles.len() {
        1 => Ok(bundles.remove(0)),
        0 => Err(UpdateError::Archive(
            "the update does not contain a .app bundle".to_owned(),
        )),
        _ => Err(UpdateError::Archive(
            "the update contains more than one .app bundle".to_owned(),
        )),
    }
}

fn unique_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn write_script(extension: &str, contents: &str) -> Result<PathBuf, UpdateError> {
    let path = std::env::temp_dir().join(format!(
        "dae-update-{}-{}-{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        unique_suffix(),
        extension
    ));
    fs::write(&path, contents).map_err(|error| UpdateError::Install(error.to_string()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| UpdateError::Install(error.to_string()))?;
    }

    Ok(path)
}

fn spawn_detached(mut command: Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Extracts a zip, rejecting absolute and parent-traversing entries. Symlink
/// entries (used by macOS framework bundles) are recreated only when their
/// target stays inside the staging tree.
fn extract_zip(zip_path: &Path, destination: &Path) -> Result<(), UpdateError> {
    let file = File::open(zip_path).map_err(|error| UpdateError::Archive(error.to_string()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| UpdateError::Archive(error.to_string()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| UpdateError::Archive(error.to_string()))?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            UpdateError::Archive("the archive contains an unsafe path".to_owned())
        })?;
        let out_path = destination.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|error| UpdateError::Archive(error.to_string()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|error| UpdateError::Archive(error.to_string()))?;
        }

        #[cfg(unix)]
        {
            if entry.is_symlink() {
                let mut target = String::new();
                entry
                    .read_to_string(&mut target)
                    .map_err(|error| UpdateError::Archive(error.to_string()))?;
                let target_path = Path::new(&target);
                if target_path.is_absolute()
                    || target_path
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                {
                    return Err(UpdateError::Archive(
                        "the archive contains a symlink outside the install tree".to_owned(),
                    ));
                }
                if out_path.exists() {
                    fs::remove_file(&out_path)
                        .map_err(|error| UpdateError::Archive(error.to_string()))?;
                }
                std::os::unix::fs::symlink(target_path, &out_path)
                    .map_err(|error| UpdateError::Archive(error.to_string()))?;
                continue;
            }
        }

        let mut output =
            File::create(&out_path).map_err(|error| UpdateError::Archive(error.to_string()))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| UpdateError::Archive(error.to_string()))?;
        output
            .flush()
            .map_err(|error| UpdateError::Archive(error.to_string()))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keep the executable bit from the archive; when a zip was made on
            // Windows the mode is absent, so fall back to a useful default for
            // the two known launchers.
            let mode = if mode & 0o777 == 0 {
                if out_path.file_name().is_some_and(|name| name == "dae") {
                    0o755
                } else {
                    0o644
                }
            } else {
                mode & 0o777
            };
            fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))
                .map_err(|error| UpdateError::Archive(error.to_string()))?;
        }
    }

    Ok(())
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
const PORTABLE_POWERSHELL: &str = r#"
param(
    [Parameter(Mandatory=$true)][int]$TargetPid,
    [Parameter(Mandatory=$true)][string]$Staging,
    [Parameter(Mandatory=$true)][string]$InstallPath,
    [Parameter(Mandatory=$true)][string]$LaunchPath
)
$ErrorActionPreference = 'Stop'
$log = Join-Path $env:TEMP 'dae-update.log'
try {
    try {
        Wait-Process -Id $TargetPid -Timeout 90 -ErrorAction Stop
    } catch {
        # The process may already have exited.
    }
    Start-Sleep -Milliseconds 300
    Get-ChildItem -LiteralPath $Staging -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $InstallPath -Recurse -Force
    }
    Remove-Item -LiteralPath $Staging -Recurse -Force -ErrorAction SilentlyContinue
    Start-Process -FilePath $LaunchPath -WorkingDirectory $InstallPath
} catch {
    Add-Content -LiteralPath $log -Value ("dae update failed: " + $_.Exception.Message)
    if (Test-Path -LiteralPath $LaunchPath) {
        Start-Process -FilePath $LaunchPath -WorkingDirectory $InstallPath
    }
} finally {
    Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
}
"#;

#[cfg_attr(target_os = "macos", allow(dead_code))]
const PORTABLE_SHELL: &str = r#"#!/bin/sh
# $1 old pid, $2 staging, $3 install directory, $4 executable to relaunch
pid="$1"
staging="$2"
install="$3"
launch="$4"
log="${TMPDIR:-/tmp}/dae-update.log"

i=0
while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 450 ]; do
    sleep 0.2
    i=$((i + 1))
done
sleep 0.3

if cp -R -p "$staging/." "$install/" 2>>"$log"; then
    rm -rf "$staging"
else
    echo "dae update failed while copying files" >>"$log"
fi

( cd "$install" && exec "$launch" ) >/dev/null 2>&1 &
rm -f -- "$0"
exit 0
"#;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAC_BUNDLE_SHELL: &str = r#"#!/bin/sh
# $1 old pid, $2 staging, $3 current .app, $4 staged .app, $5 .app to launch
pid="$1"
staging="$2"
bundle="$3"
staged_app="$4"
launch="$5"
log="${TMPDIR:-/tmp}/dae-update.log"
old="$bundle.old-$pid"

i=0
while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 450 ]; do
    sleep 0.2
    i=$((i + 1))
done
sleep 0.3

if [ -d "$staged_app" ]; then
    rm -rf "$old"
    if mv "$bundle" "$old"; then
        if mv "$staged_app" "$bundle"; then
            rm -rf "$old"
        else
            echo "dae update failed while swapping the app bundle" >>"$log"
            mv "$old" "$bundle"
        fi
    else
        echo "dae update failed while moving the old app bundle" >>"$log"
    fi
else
    echo "dae update failed: staged app bundle is missing" >>"$log"
fi

rm -rf "$staging"
open -n "$launch" >/dev/null 2>&1
rm -f -- "$0"
exit 0
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extraction_keeps_the_tree_and_rejects_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let zip_path = directory.path().join("update.zip");
        write_zip(
            &zip_path,
            &[
                ("platforms/qwindows.dll", b"plugin"),
                ("dae.exe", b"binary"),
            ],
        );
        let destination = directory.path().join("out");
        fs::create_dir(&destination).unwrap();
        extract_zip(&zip_path, &destination).unwrap();

        assert_eq!(
            fs::read(destination.join("platforms/qwindows.dll")).unwrap(),
            b"plugin"
        );
        assert_eq!(fs::read(destination.join("dae.exe")).unwrap(), b"binary");
    }

    #[test]
    fn extraction_rejects_parent_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let zip_path = directory.path().join("evil.zip");
        write_zip(&zip_path, &[("../escaped", b"nope")]);
        let destination = directory.path().join("out");
        fs::create_dir(&destination).unwrap();

        let error = extract_zip(&zip_path, &destination).unwrap_err();
        assert!(matches!(error, UpdateError::Archive(_)));
        assert!(!destination.parent().unwrap().join("escaped").exists());
    }

    #[test]
    fn installer_scripts_use_arguments_instead_of_interpolated_paths() {
        // Paths are passed as argv, so a quote or space must never end up in
        // the script text. This is a regression guard for the old string
        // interpolation approach.
        assert!(!PORTABLE_POWERSHELL.contains("Staging = '"));
        assert!(PORTABLE_POWERSHELL.contains("[string]$Staging"));
        assert!(PORTABLE_SHELL.contains("\"$staging\""));
        assert!(MAC_BUNDLE_SHELL.contains("\"$staged_app\""));
    }

    #[test]
    fn mac_bundle_staging_requires_exactly_one_app() {
        let directory = tempfile::tempdir().unwrap();
        let staging = directory.path().join("staging");
        fs::create_dir(&staging).unwrap();
        fs::create_dir(staging.join("dae.app")).unwrap();

        assert_eq!(
            staged_app_bundle(&staging).unwrap(),
            staging.join("dae.app")
        );

        fs::create_dir(staging.join("other.app")).unwrap();
        assert!(matches!(
            staged_app_bundle(&staging),
            Err(UpdateError::Archive(_))
        ));
    }

    #[test]
    fn a_clean_staging_name_is_unique() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout {
            install_dir: directory.path().join("install"),
            staging_parent: directory.path().to_path_buf(),
            executable: directory.path().join("install/dae"),
            launch: directory.path().join("install/dae"),
            kind: LayoutKind::Portable,
            bundle: None,
        };
        fs::create_dir_all(&layout.install_dir).unwrap();

        let first = layout.new_staging_dir().unwrap();
        let second = layout.new_staging_dir().unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(directory.path()));
        assert!(first.is_dir());
        assert!(second.is_dir());
    }
}
