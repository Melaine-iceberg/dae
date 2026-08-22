use super::error::FileSystemError;
use super::recents::now_millis;
use super::sidebar::write_atomic;
use super::types::display_name_from_path;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

const SPACES_FILE_NAME: &str = "spaces.json";

/// One pinned location inside a space. Items are pointers, never copies:
/// removing an item from a space never touches the filesystem.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpaceItem {
    pub path: String,
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub added_at: u64,
}

/// A contextual workspace grouping folders, network locations, and files.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    /// Preset spaces ship with the app and cannot be deleted, only renamed.
    pub is_preset: bool,
    pub items: Vec<SpaceItem>,
}

/// Lists all spaces, seeding the preset set (Work/Personal/Shared/Archive)
/// on first launch.
#[tauri::command]
#[specta::specta]
pub fn list_spaces(app: tauri::AppHandle) -> Result<Vec<Space>, FileSystemError> {
    if let Some(spaces) = app.state::<super::prefetch::StartupPrefetch>().take_spaces() {
        return Ok(spaces);
    }
    let path = spaces_path(&app)?;

    match read_spaces(&path)? {
        Some(spaces) => Ok(spaces),
        None => {
            let spaces = seed_spaces();
            write_spaces(&path, &spaces)?;
            Ok(spaces)
        }
    }
}

/// Creates a new empty space and returns it.
#[tauri::command]
#[specta::specta]
pub fn create_space(app: tauri::AppHandle, name: String) -> Result<Space, FileSystemError> {
    let name = validated_name(&name)?;
    let path = spaces_path(&app)?;
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

/// Renames a space. Preset spaces may be renamed but not deleted.
#[tauri::command]
#[specta::specta]
pub fn rename_space(
    app: tauri::AppHandle,
    space_id: String,
    name: String,
) -> Result<Space, FileSystemError> {
    let name = validated_name(&name)?;
    let path = spaces_path(&app)?;
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

/// Deletes a space. The items inside it are pointers, so no files are removed.
/// Preset spaces cannot be deleted.
#[tauri::command]
#[specta::specta]
pub fn delete_space(app: tauri::AppHandle, space_id: String) -> Result<(), FileSystemError> {
    let path = spaces_path(&app)?;
    let mut spaces = read_spaces(&path)?.unwrap_or_else(seed_spaces);

    let Some(space) = spaces.iter().find(|space| space.id == space_id) else {
        return Err(FileSystemError::NotFound(format!(
            "No space with id {space_id}"
        )));
    };
    if space.is_preset {
        return Err(FileSystemError::InvalidInput(
            "Preset spaces cannot be deleted".to_string(),
        ));
    }

    spaces.retain(|space| space.id != space_id);
    write_spaces(&path, &spaces)
}

/// Pins a path inside a space, returning the updated space. Adding the same
/// path twice keeps the original entry.
#[tauri::command]
#[specta::specta]
pub fn add_space_item(
    app: tauri::AppHandle,
    space_id: String,
    path: String,
) -> Result<Space, FileSystemError> {
    update_space_items(&app, &space_id, |items| {
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

/// Removes one path from a space, returning the updated space.
#[tauri::command]
#[specta::specta]
pub fn remove_space_item(
    app: tauri::AppHandle,
    space_id: String,
    path: String,
) -> Result<Space, FileSystemError> {
    update_space_items(&app, &space_id, |items| {
        items.retain(|item| item.path != path);
    })
}

/// Seeds the four preset spaces described by the product's information
/// architecture. Extracted for unit testing.
pub(super) fn seed_spaces() -> Vec<Space> {
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

fn update_space_items(
    app: &tauri::AppHandle,
    space_id: &str,
    update: impl FnOnce(&mut Vec<SpaceItem>),
) -> Result<Space, FileSystemError> {
    let path = spaces_path(app)?;
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

fn validated_name(name: &str) -> Result<String, FileSystemError> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "Space name must not be empty".to_string(),
        ));
    }

    Ok(trimmed.to_string())
}

fn read_spaces(path: &std::path::Path) -> Result<Option<Vec<Space>>, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let spaces: Vec<Space> = serde_json::from_str(&contents)
                .map_err(|error| FileSystemError::Internal(error.to_string()))?;
            Ok(Some(spaces))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_spaces(path: &std::path::Path, spaces: &[Space]) -> Result<(), FileSystemError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(spaces)
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;
    write_atomic(path, contents.as_bytes())
}

fn spaces_path(app: &tauri::AppHandle) -> Result<PathBuf, FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    Ok(config_dir.join(SPACES_FILE_NAME))
}
