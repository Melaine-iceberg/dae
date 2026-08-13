use super::directory::path_to_string;
use super::error::FileSystemError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use sysinfo::Disks;
use tauri::Manager;

const FAVORITES_FILE_NAME: &str = "sidebar-favorites.json";

/// Well-known directories that are always offered in the sidebar's places section.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SystemPlace {
    pub kind: PlaceKind,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Favorite {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiskVolume {
    pub mount_point: String,
    pub name: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub is_removable: bool,
}

/// Resolves the fixed set of system places, skipping any the OS cannot provide.
#[tauri::command]
#[specta::specta]
pub fn get_system_places(app: tauri::AppHandle) -> Result<Vec<SystemPlace>, FileSystemError> {
    let resolver = app.path();
    let candidates = [
        (PlaceKind::Home, resolver.home_dir()),
        (PlaceKind::Desktop, resolver.desktop_dir()),
        (PlaceKind::Documents, resolver.document_dir()),
        (PlaceKind::Downloads, resolver.download_dir()),
        (PlaceKind::Pictures, resolver.picture_dir()),
        (PlaceKind::Music, resolver.audio_dir()),
        (PlaceKind::Videos, resolver.video_dir()),
    ];

    Ok(candidates
        .into_iter()
        .filter_map(|(kind, resolved)| {
            resolved
                .ok()
                .map(|path| SystemPlace { kind, path: path_to_string(&path) })
        })
        .collect())
}

/// Lists local disk volumes with capacity information for the sidebar's devices section.
#[tauri::command]
#[specta::specta]
pub async fn list_disks() -> Result<Vec<DiskVolume>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(list_disks_sync)
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))
}

/// Loads the user's favorite folders, returning an empty list on first launch.
#[tauri::command]
#[specta::specta]
pub fn load_favorites(app: tauri::AppHandle) -> Result<Vec<Favorite>, FileSystemError> {
    let path = favorites_path(&app)?;

    match fs::read_to_string(&path) {
        Ok(contents) => {
            let favorites: Vec<Favorite> = serde_json::from_str(&contents)
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            Ok(favorites)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

/// Persists the full favorite list, deduplicating entries that share a path.
#[tauri::command]
#[specta::specta]
pub fn save_favorites(
    app: tauri::AppHandle,
    favorites: Vec<Favorite>,
) -> Result<(), FileSystemError> {
    let path = favorites_path(&app)?;
    let favorites = dedupe_favorites(favorites);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(&favorites)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(&path, contents.as_bytes())
}

fn list_disks_sync() -> Vec<DiskVolume> {
    let mut volumes: Vec<DiskVolume> = Disks::new_with_refreshed_list()
        .iter()
        .filter(|disk| {
            let file_system = disk.file_system().to_string_lossy();
            disk.total_space() > 0 && is_visible_file_system(&file_system)
        })
        .map(|disk| DiskVolume {
            mount_point: path_to_string(disk.mount_point()),
            name: disk.name().to_string_lossy().into_owned(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
            is_removable: disk.is_removable(),
        })
        .collect();

    volumes.sort_by(|a, b| {
        is_system_volume(b)
            .cmp(&is_system_volume(a))
            .then_with(|| a.mount_point.to_lowercase().cmp(&b.mount_point.to_lowercase()))
    });

    volumes
}

/// The volume holding the OS: the Windows %SystemDrive% volume, or the root volume elsewhere.
fn is_system_volume(volume: &DiskVolume) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Some(system_drive) = std::env::var("SystemDrive").ok() else {
            return false;
        };

        let prefix = format!("{}:", system_drive.trim_end_matches(':')).to_ascii_uppercase();
        volume.mount_point.to_ascii_uppercase().starts_with(&prefix)
    }

    #[cfg(not(target_os = "windows"))]
    {
        volume.mount_point == "/"
    }
}

/// Hides network shares and pseudo file systems that are not real storage volumes.
pub(super) fn is_visible_file_system(file_system: &str) -> bool {
    const HIDDEN: &[&str] = &[
        // Network file systems (network storage is out of scope).
        "cifs", "smbfs", "smb", "smb2", "nfs", "nfs4", "afpfs", "sshfs", "fuse.sshfs", "webdav",
        "davfs", "davfs2", "9p",
        // Pseudo file systems without meaningful capacity.
        "proc", "procfs", "sysfs", "tmpfs", "devtmpfs", "devpts", "ramfs", "overlay",
        "squashfs", "efivarfs", "securityfs", "debugfs", "tracefs", "cgroup", "cgroup2",
        "autofs", "mqueue", "hugetlbfs", "pstore", "configfs", "fusectl", "binfmt_misc",
        "nsfs", "fuse.lxcfs", "rpc_pipefs", "none",
    ];

    let lowered = file_system.to_lowercase();
    !HIDDEN.iter().any(|hidden| lowered == *hidden)
}

pub(super) fn dedupe_favorites(favorites: Vec<Favorite>) -> Vec<Favorite> {
    let mut seen = std::collections::HashSet::new();

    favorites
        .into_iter()
        .filter(|favorite| seen.insert(favorite.path.clone()))
        .collect()
}

fn favorites_path(app: &tauri::AppHandle) -> Result<PathBuf, FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    Ok(config_dir.join(FAVORITES_FILE_NAME))
}

/// Writes via a temp file + rename so readers never observe a partial document.
fn write_atomic(path: &std::path::Path, contents: &[u8]) -> Result<(), FileSystemError> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, contents)?;
    fs::rename(&temp_path, path)?;
    Ok(())
}
