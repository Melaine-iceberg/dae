//! Disk and WSL discovery for the sidebar.

use dae_core::file_system::types::path_to_string;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use sysinfo::Disks;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskVolume {
    pub mount_point: String,
    pub name: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub is_removable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDistro {
    pub name: String,
    pub path: String,
}

pub fn list_disks() -> Result<Vec<DiskVolume>, String> {
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

    #[cfg(target_os = "linux")]
    collapse_duplicate_mounts(&mut volumes);

    #[cfg(target_os = "linux")]
    replace_devices_with_labels(&mut volumes);

    volumes.sort_by(|a, b| {
        is_system_volume(b).cmp(&is_system_volume(a)).then_with(|| {
            a.mount_point
                .to_lowercase()
                .cmp(&b.mount_point.to_lowercase())
        })
    });

    Ok(volumes)
}

pub fn list_wsl_distros() -> Result<Vec<WslDistro>, String> {
    #[cfg(target_os = "windows")]
    {
        Ok(list_wsl_distros_windows())
    }

    #[cfg(not(target_os = "windows"))]
    Ok(Vec::new())
}

#[cfg(target_os = "windows")]
fn list_wsl_distros_windows() -> Vec<WslDistro> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // Without CREATE_NO_WINDOW a console flashes every time the sidebar refreshes.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = Command::new("wsl.exe")
        .args(["--list", "--quiet"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    decode_wsl_output(&output.stdout)
        .lines()
        .map(|line| line.trim_matches('\0').trim())
        .filter(|line| !line.is_empty())
        .map(|name| WslDistro {
            name: name.to_owned(),
            path: format!(r"\\wsl$\{name}"),
        })
        .collect()
}

/// `wsl.exe` emits UTF-16LE (with or without a BOM) when piped; raw UTF-8 otherwise.
#[cfg(target_os = "windows")]
fn decode_wsl_output(bytes: &[u8]) -> String {
    let has_bom = bytes.starts_with(&[0xFF, 0xFE]);
    let looks_utf16 = has_bom || bytes.iter().skip(1).step_by(2).any(|&byte| byte == 0);
    if !looks_utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    let payload = if has_bom { &bytes[2..] } else { bytes };
    let units: Vec<u16> = payload
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

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
fn is_visible_file_system(file_system: &str) -> bool {
    const HIDDEN: &[&str] = &[
        "cifs", "smbfs", "smb", "smb2", "nfs", "nfs4", "afpfs", "sshfs", "fuse.sshfs",
        "webdav", "davfs", "davfs2", "9p", "proc", "procfs", "sysfs", "tmpfs", "devtmpfs",
        "devpts", "ramfs", "overlay", "squashfs", "efivarfs", "securityfs", "debugfs",
        "tracefs", "cgroup", "cgroup2", "autofs", "mqueue", "hugetlbfs", "pstore",
        "configfs", "fusectl", "binfmt_misc", "nsfs", "fuse.lxcfs", "rpc_pipefs", "none",
    ];

    let lowered = file_system.to_lowercase();
    !HIDDEN.iter().any(|hidden| lowered == *hidden)
}

#[cfg(target_os = "linux")]
fn collapse_duplicate_mounts(volumes: &mut Vec<DiskVolume>) {
    use std::collections::HashSet;

    volumes.sort_by_key(|volume| {
        Path::new(&volume.mount_point)
            .components()
            .count()
    });

    let mut filesystems = HashSet::with_capacity(volumes.len());
    volumes.retain(|volume| filesystems.insert((volume.name.clone(), volume.total_bytes)));
}

#[cfg(target_os = "linux")]
fn replace_devices_with_labels(volumes: &mut [DiskVolume]) {
    let labels = udev_volume_labels();

    for volume in volumes {
        volume.name = label_for_device(Path::new(&volume.name), &labels).unwrap_or_default();
    }
}

#[cfg(target_os = "linux")]
fn udev_volume_labels() -> HashMap<PathBuf, String> {
    let Ok(entries) = fs::read_dir("/dev/disk/by-label") else {
        return HashMap::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let device = fs::canonicalize(entry.path()).ok()?;
            let label = decode_udev_name(entry.file_name().as_os_str().as_bytes());
            Some((device, label))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn label_for_device(device: &Path, labels: &HashMap<PathBuf, String>) -> Option<String> {
    let node = fs::canonicalize(device).ok()?;
    if let Some(label) = labels.get(&node) {
        return Some(label.clone());
    }

    let slaves = Path::new("/sys/class/block")
        .join(node.file_name()?)
        .join("slaves");
    fs::read_dir(slaves).ok()?.flatten().find_map(|slave| {
        let node = fs::canonicalize(Path::new("/dev").join(slave.file_name())).ok()?;
        labels.get(&node).cloned()
    })
}

#[cfg(target_os = "linux")]
fn decode_udev_name(bytes: &[u8]) -> String {
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());

    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 3 < bytes.len() && bytes[index + 1] == b'x' => {
                let hex = std::str::from_utf8(&bytes[index + 2..index + 4]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        decoded.push(byte);
                        index += 4;
                    }
                    None => {
                        decoded.push(b'\\');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_pseudo_and_network_filesystems() {
        assert!(!is_visible_file_system("tmpfs"));
        assert!(!is_visible_file_system("CIFS"));
        assert!(is_visible_file_system("ntfs"));
        assert!(is_visible_file_system("ext4"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn decodes_udev_escapes() {
        assert_eq!(decode_udev_name(b"TUF\\x20C:"), "TUF C:");
        assert_eq!(decode_udev_name(b"plain"), "plain");
        assert_eq!(decode_udev_name(b"back\\slash"), "back\\slash");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn decodes_utf16_wsl_output() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Ubuntu\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wsl_output(&bytes), "Ubuntu\n");
        assert_eq!(decode_wsl_output(b"Ubuntu\n"), "Ubuntu\n");
    }
}
