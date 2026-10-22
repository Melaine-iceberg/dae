//! Sidebar favorites and contextual spaces.
//!
//! Both stores are plain JSON documents under the app config directory, using
//! the file names the Tauri shell wrote (`sidebar-favorites.json`,
//! `spaces.json`), so a migrated install keeps its pinned entries and spaces.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::fs_atomic::write_atomic;
use dae_core::file_system::types::display_name_from_path;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const FAVORITES_FILE_NAME: &str = "sidebar-favorites.json";
const SPACES_FILE_NAME: &str = "spaces.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Favorite {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceItem {
    pub path: String,
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    pub is_preset: bool,
    pub items: Vec<SpaceItem>,
}

fn config_path(file_name: &str) -> PathBuf {
    crate::config_dir().join(file_name)
}

// ---------------------------------------------------------------------------
// Favorites
// ---------------------------------------------------------------------------

pub fn load_favorites() -> Result<Vec<Favorite>, FileSystemError> {
    let path = config_path(FAVORITES_FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| FileSystemError::Internal(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_favorites(favorites: Vec<Favorite>) -> Result<(), FileSystemError> {
    let path = config_path(FAVORITES_FILE_NAME);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(&dedupe_favorites(favorites))
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(&path, contents.as_bytes())
}

fn dedupe_favorites(favorites: Vec<Favorite>) -> Vec<Favorite> {
    let mut seen = HashSet::new();
    favorites
        .into_iter()
        .filter(|favorite| seen.insert(favorite.path.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Spaces
// ---------------------------------------------------------------------------

/// Lists all spaces, seeding the preset set on first launch.
pub fn list_spaces() -> Result<Vec<Space>, FileSystemError> {
    let path = config_path(SPACES_FILE_NAME);
    match read_spaces(&path)? {
        Some(spaces) => Ok(spaces),
        None => {
            let spaces = seed_spaces();
            write_spaces(&path, &spaces)?;
            Ok(spaces)
        }
    }
}

pub fn create_space(name: String) -> Result<Space, FileSystemError> {
    let name = validated_name(&name)?;
    let path = config_path(SPACES_FILE_NAME);
    let mut spaces = read_spaces(&path)?.unwrap_or_else(seed_spaces);

    let space = Space {
        id: format!("space-{:x}", now_millis()),
        name,
        is_preset: false,
        items: Vec::new(),
    };
    spaces.push(space.clone());
    write_spaces(&path, &spaces)?;
    Ok(space)
}

pub fn rename_space(space_id: String, name: String) -> Result<Space, FileSystemError> {
    let name = validated_name(&name)?;
    let path = config_path(SPACES_FILE_NAME);
    let mut spaces = read_spaces(&path)?.unwrap_or_else(seed_spaces);

    let space = spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| FileSystemError::NotFound(format!("No space with id {space_id}")))?;
    space.name = name;
    let updated = space.clone();
    write_spaces(&path, &spaces)?;
    Ok(updated)
}

pub fn delete_space(space_id: String) -> Result<(), FileSystemError> {
    let path = config_path(SPACES_FILE_NAME);
    let mut spaces = read_spaces(&path)?.unwrap_or_else(seed_spaces);

    let space = spaces
        .iter()
        .find(|space| space.id == space_id)
        .ok_or_else(|| FileSystemError::NotFound(format!("No space with id {space_id}")))?;
    if space.is_preset {
        return Err(FileSystemError::InvalidInput(
            "Preset spaces cannot be deleted".to_string(),
        ));
    }

    spaces.retain(|space| space.id != space_id);
    write_spaces(&path, &spaces)
}

pub fn add_space_item(space_id: String, path: String) -> Result<Space, FileSystemError> {
    update_space_items(&space_id, |items| {
        if items.iter().any(|item| item.path == path) {
            return;
        }
        items.push(SpaceItem {
            name: display_name_from_path(&path),
            path,
            added_at: now_millis(),
        });
    })
}

pub fn remove_space_item(space_id: String, path: String) -> Result<Space, FileSystemError> {
    update_space_items(&space_id, |items| {
        items.retain(|item| item.path != path);
    })
}

fn update_space_items(
    space_id: &str,
    update: impl FnOnce(&mut Vec<SpaceItem>),
) -> Result<Space, FileSystemError> {
    let path = config_path(SPACES_FILE_NAME);
    let mut spaces = read_spaces(&path)?.unwrap_or_else(seed_spaces);

    let space = spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| FileSystemError::NotFound(format!("No space with id {space_id}")))?;
    update(&mut space.items);
    let updated = space.clone();
    write_spaces(&path, &spaces)?;
    Ok(updated)
}

fn seed_spaces() -> Vec<Space> {
    [
        ("work", "工作"),
        ("personal", "个人"),
        ("shared", "共享"),
        ("archive", "归档"),
    ]
    .into_iter()
    .map(|(id, name)| Space {
        id: id.to_string(),
        name: name.to_string(),
        is_preset: true,
        items: Vec::new(),
    })
    .collect()
}

fn validated_name(name: &str) -> Result<String, FileSystemError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Space name must not be empty".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn read_spaces(path: &Path) -> Result<Option<Vec<Space>>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| FileSystemError::Internal(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_spaces(path: &Path, spaces: &[Space]) -> Result<(), FileSystemError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(spaces)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(path, contents.as_bytes())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_the_four_preset_spaces() {
        let spaces = seed_spaces();
        assert_eq!(spaces.len(), 4);
        assert!(spaces.iter().all(|space| space.is_preset));
        assert_eq!(spaces[0].id, "work");
    }

    #[test]
    fn dedupes_favorites_by_path() {
        let favorites = vec![
            Favorite {
                path: "/a".into(),
                name: "a".into(),
            },
            Favorite {
                path: "/a".into(),
                name: "again".into(),
            },
            Favorite {
                path: "/b".into(),
                name: "b".into(),
            },
        ];
        let deduped = dedupe_favorites(favorites);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].name, "a");
    }
}
