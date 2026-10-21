//! User settings persisted as a human-editable `settings.toml` in the app
//! config directory.
//!
//! The document is the same one the Tauri shell wrote, field for field, and the
//! directory is derived the same way (`DAE_CONFIG_DIR`, or the platform config
//! root plus the bundle identifier), so an existing install keeps its settings.

use dae_core::file_system::error::FileSystemError;
use dae_core::file_system::fs_atomic::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const SETTINGS_FILE_NAME: &str = "settings.toml";

/// Bounds applied when normalizing terminal typography so a hand-edited file
/// cannot produce an unusable surface.
const MIN_FONT_SIZE: u8 = 8;
const MAX_FONT_SIZE: u8 = 24;
const MIN_LINE_HEIGHT: f32 = 0.8;
const MAX_LINE_HEIGHT: f32 = 3.0;
const ANSI_PALETTE_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Action id -> binding string. On load this is overlaid onto
    /// [`default_shortcuts`] so newly added actions appear with their defaults
    /// and stale ids are dropped.
    pub shortcuts: HashMap<String, String>,
    pub terminal: TerminalSettings,
    pub default_file_manager: DefaultFileManagerState,
    /// Kept for file-format compatibility with the Tauri shell.
    pub warm_tab_pool: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalSettings {
    pub font_size: u8,
    pub line_height: f32,
    pub font_family: Option<String>,
    pub ansi_colors: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct DefaultFileManagerState {
    pub is_default: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: default_shortcuts(),
            terminal: TerminalSettings::default(),
            default_file_manager: DefaultFileManagerState::default(),
            warm_tab_pool: true,
        }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_size: 12,
            line_height: 1.75,
            font_family: None,
            ansi_colors: None,
        }
    }
}

/// The single source of truth for default key bindings, mirrored by
/// `src/features/settings/shortcut-registry.ts`.
pub fn default_shortcuts() -> HashMap<String, String> {
    let entries: [(&str, &str); 20] = [
        ("app.commandBar", "Mod+K"),
        ("app.pathJump", "Mod+P"),
        ("app.toggleTerminal", "Control+`"),
        ("app.openSettings", "Mod+,"),
        ("explorer.clearSelection", "Escape"),
        ("explorer.copy", "Mod+C"),
        ("explorer.cut", "Mod+X"),
        ("explorer.paste", "Mod+V"),
        ("explorer.selectAll", "Mod+A"),
        ("explorer.toggleHidden", "Mod+H"),
        ("explorer.rename", "F2"),
        ("explorer.preview", "Space"),
        ("explorer.undo", "Mod+Z"),
        ("explorer.redo", "Mod+Shift+Z"),
        ("explorer.redoAlt", "Mod+Y"),
        ("explorer.trash", "Delete"),
        ("explorer.deletePermanent", "Shift+Delete"),
        ("explorer.openSystemTerminal", "Mod+`"),
        ("explorer.focusSearch", "Mod+F"),
        ("explorer.switchPane", "F6"),
    ];
    entries
        .into_iter()
        .map(|(id, binding)| (id.to_owned(), binding.to_owned()))
        .collect()
}

#[derive(Default)]
struct RegistryInner {
    config_dir: Option<PathBuf>,
    settings: AppSettings,
}

static REGISTRY: LazyLock<Mutex<RegistryInner>> =
    LazyLock::new(|| Mutex::new(RegistryInner::default()));

/// Loads persisted settings. Safe to call more than once; a corrupt file is
/// reported but still falls back to defaults so a bad hand-edit cannot brick
/// startup.
pub fn init() -> Result<(), FileSystemError> {
    init_at(crate::config_dir())
}

fn init_at(config_dir: PathBuf) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.settings = read_settings_file(&settings_path(&config_dir)).unwrap_or_default();
    Ok(())
}

/// Loads on first use when the shell did not call `initialize`.
pub fn ensure_loaded() {
    let ready = REGISTRY
        .lock()
        .map(|registry| registry.config_dir.is_some())
        .unwrap_or(false);
    if !ready {
        let _ = init();
    }
}

pub fn load() -> Result<AppSettings, FileSystemError> {
    ensure_loaded();
    let registry = REGISTRY.lock().expect("settings registry poisoned");
    Ok(registry.settings.clone())
}

/// Normalizes, stores, and persists the given settings, returning the
/// normalized document so the frontend converges on exactly what is on disk.
pub fn save(settings: &AppSettings) -> Result<AppSettings, FileSystemError> {
    ensure_loaded();
    let normalized = normalize(settings.clone());
    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.settings = normalized.clone();
    persist_locked(&mut registry)?;
    Ok(normalized)
}

fn persist_locked(registry: &mut RegistryInner) -> Result<(), FileSystemError> {
    let Some(config_dir) = registry.config_dir.clone() else {
        return Err(FileSystemError::Internal(
            "settings.not_initialized".to_string(),
        ));
    };

    fs::create_dir_all(&config_dir)?;
    let contents = toml::to_string_pretty(&registry.settings)
        .map_err(|error| FileSystemError::Internal(format!("settings.save_failed: {error}")))?;
    write_atomic(&settings_path(&config_dir), contents.as_bytes())
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

fn read_settings_file(path: &Path) -> Result<AppSettings, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let parsed: AppSettings = toml::from_str(&contents).map_err(|error| {
                FileSystemError::Internal(format!("settings.parse_failed: {error}"))
            })?;
            Ok(normalize(parsed))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(error.into()),
    }
}

/// Fills defaults for absent shortcut ids, drops unknown ids, and clamps
/// terminal typography into usable ranges.
fn normalize(mut settings: AppSettings) -> AppSettings {
    settings.shortcuts = normalize_shortcuts(settings.shortcuts);
    settings.terminal = normalize_terminal(settings.terminal);
    settings
}

fn normalize_shortcuts(raw: HashMap<String, String>) -> HashMap<String, String> {
    let mut normalized = default_shortcuts();
    for (id, binding) in raw {
        if normalized.contains_key(&id) {
            normalized.insert(id, binding);
        }
    }
    normalized
}

fn normalize_terminal(terminal: TerminalSettings) -> TerminalSettings {
    let font_size = terminal.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    let line_height = terminal.line_height.clamp(MIN_LINE_HEIGHT, MAX_LINE_HEIGHT);
    let ansi_colors = terminal
        .ansi_colors
        .and_then(|colors| (colors.len() == ANSI_PALETTE_LEN).then_some(colors));
    TerminalSettings {
        font_size,
        line_height,
        font_family: terminal.font_family,
        ansi_colors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn temp_config_dir() -> PathBuf {
        let unique = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("dae-settings-test-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp config dir");
        dir
    }

    #[test]
    fn default_shortcut_ids_are_stable() {
        let ids = default_shortcuts();
        let expected = [
            "app.commandBar",
            "app.pathJump",
            "app.toggleTerminal",
            "app.openSettings",
            "explorer.clearSelection",
            "explorer.copy",
            "explorer.cut",
            "explorer.paste",
            "explorer.selectAll",
            "explorer.toggleHidden",
            "explorer.rename",
            "explorer.preview",
            "explorer.undo",
            "explorer.redo",
            "explorer.redoAlt",
            "explorer.trash",
            "explorer.deletePermanent",
            "explorer.openSystemTerminal",
            "explorer.focusSearch",
            "explorer.switchPane",
        ];
        assert_eq!(ids.len(), expected.len(), "shortcut count drifted");
        for id in expected {
            assert!(ids.contains_key(id), "missing shortcut id: {id}");
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let _guard = lock();
        let dir = temp_config_dir();
        init_at(dir.clone()).expect("init");

        let mut settings = AppSettings::default();
        settings
            .shortcuts
            .insert("explorer.copy".to_string(), "Mod+Shift+C".to_string());
        settings.terminal.font_size = 16;
        settings.terminal.font_family = Some("JetBrains Mono".to_string());

        let saved = save(&settings).expect("save");
        assert_eq!(saved.terminal.font_size, 16);
        assert!(settings_path(&dir).exists(), "settings.toml written");

        init_at(dir.clone()).expect("reload");
        let loaded = load().expect("load");
        assert_eq!(loaded, saved);
        assert_eq!(loaded.shortcuts["explorer.copy"], "Mod+Shift+C");
        assert_eq!(
            loaded.terminal.font_family.as_deref(),
            Some("JetBrains Mono")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn partial_file_fills_defaults_and_drops_unknown_ids() {
        let _guard = lock();
        let dir = temp_config_dir();
        fs::write(
            settings_path(&dir),
            "[terminal]\nfontSize = 15\n\n[shortcuts]\n\"explorer.copy\" = \"\"\n\"bogus.id\" = \"Mod+Q\"\n",
        )
        .expect("write partial file");

        init_at(dir.clone()).expect("init");
        let loaded = load().expect("load");

        assert_eq!(loaded.shortcuts["explorer.copy"], "");
        assert!(!loaded.shortcuts.contains_key("bogus.id"));
        assert_eq!(loaded.shortcuts["app.commandBar"], "Mod+K");
        assert_eq!(loaded.terminal.font_size, 15);
        assert_eq!(loaded.terminal.line_height, 1.75);
        assert_eq!(loaded.terminal.font_family, None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_absent_pool_setting_reads_as_on() {
        let _guard = lock();
        let dir = temp_config_dir();

        fs::write(settings_path(&dir), "[terminal]\nfontSize = 12\n").expect("write partial");
        init_at(dir.clone()).expect("init");
        assert!(load().expect("load").warm_tab_pool);

        fs::write(settings_path(&dir), "warmTabPool = false\n").expect("write off");
        init_at(dir.clone()).expect("init");
        assert!(!load().expect("load").warm_tab_pool);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clamps_out_of_range_terminal_values() {
        let _guard = lock();
        let dir = temp_config_dir();
        init_at(dir.clone()).expect("init");

        let mut settings = AppSettings::default();
        settings.terminal.font_size = 200;
        settings.terminal.line_height = 99.0;
        settings.terminal.ansi_colors = Some(vec!["#000000".to_string(); 3]);

        let saved = save(&settings).expect("save");
        assert_eq!(saved.terminal.font_size, MAX_FONT_SIZE);
        assert_eq!(saved.terminal.line_height, MAX_LINE_HEIGHT);
        assert_eq!(saved.terminal.ansi_colors, None);

        fs::remove_dir_all(&dir).ok();
    }
}
