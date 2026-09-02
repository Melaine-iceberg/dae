//! User settings persisted as a human-editable `settings.toml` in the app
//! config directory.
//!
//! Only the settings introduced by the settings panel live here: keyboard
//! shortcuts, integrated-terminal appearance, and the cached "default file
//! manager" state. The explorer view preferences (sort/density/filters),
//! theme, and locale intentionally stay in the frontend's localStorage and
//! are *not* mirrored into this file.
//!
//! The module follows the same registry pattern as
//! [`crate::file_system::cloud::accounts`]: a process-wide `LazyLock<Mutex<_>>`
//! initialized once from app setup, with atomic writes so a crash mid-save
//! never leaves a partial document. Every field is `#[serde(default)]`, so a
//! hand-edited or older/partial file still loads and unknown keys are ignored.

use crate::file_system::error::FileSystemError;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tauri::Manager;

const SETTINGS_FILE_NAME: &str = "settings.toml";

/// Bounds applied when normalizing terminal typography so a hand-edited file
/// cannot produce an unusable surface. The frontend Select offers 10..=20.
const MIN_FONT_SIZE: u8 = 8;
const MAX_FONT_SIZE: u8 = 24;
const MIN_LINE_HEIGHT: f32 = 0.8;
const MAX_LINE_HEIGHT: f32 = 3.0;
/// A complete xterm ANSI palette is exactly 16 entries (8 normal + 8 bright).
const ANSI_PALETTE_LEN: usize = 16;

/// The full settings document, as persisted and exposed to the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Action id -> binding string (e.g. `"Mod+K"`, `"Shift+Delete"`, `""` for
    /// unbound). On load this is overlaid onto [`default_shortcuts`] so newly
    /// added actions appear with their defaults and stale ids are dropped.
    pub shortcuts: HashMap<String, String>,
    pub terminal: TerminalSettings,
    pub default_file_manager: DefaultFileManagerState,
}

/// Integrated-terminal appearance. `None` fields fall back to the frontend's
/// built-in defaults (font stack, curated theme-aware ANSI palette).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalSettings {
    pub font_size: u8,
    pub line_height: f32,
    /// CSS font-family override; `None` keeps the app's monospace stack.
    pub font_family: Option<String>,
    /// Optional 16-entry hex palette override (black..white, brightBlack..
    /// brightWhite). `None` derives colors from the active light/dark theme.
    pub ansi_colors: Option<Vec<String>>,
}

/// Cached "is dae the OS default file manager" flag. The authoritative state
/// is always re-queried from the OS by `default_manager`; this is only a hint
/// for the first paint before that query resolves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct DefaultFileManagerState {
    pub is_default: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: default_shortcuts(),
            terminal: TerminalSettings::default(),
            default_file_manager: DefaultFileManagerState::default(),
        }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
            line_height: default_line_height(),
            font_family: None,
            ansi_colors: None,
        }
    }
}

impl Default for DefaultFileManagerState {
    fn default() -> Self {
        Self { is_default: false }
    }
}

fn default_font_size() -> u8 {
    13
}

fn default_line_height() -> f32 {
    1.2
}

/// The single source of truth for default key bindings.
///
/// These ids and bindings are mirrored exactly by
/// `src/features/settings/shortcut-registry.ts` on the frontend; a Rust test
/// below asserts the id set so the two cannot silently drift. Binding strings
/// use TanStack Hotkeys' canonical form (`Mod` = Cmd on macOS, Ctrl elsewhere).
pub fn default_shortcuts() -> HashMap<String, String> {
    let entries: [(&str, &str); 20] = [
        // Global (App.tsx)
        ("app.commandBar", "Mod+K"),
        ("app.pathJump", "Mod+P"),
        ("app.toggleTerminal", "Control+`"),
        ("app.openSettings", "Mod+,"),
        // Explorer file operations (file-list.tsx)
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
        // View / navigation
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

/// Loads persisted settings. Called once from app setup; safe to call again.
/// A missing file yields defaults; a corrupt file is reported but still falls
/// back to defaults so a bad hand-edit cannot brick startup.
pub fn init(app: &tauri::AppHandle) -> Result<(), FileSystemError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.settings = read_settings_file(&settings_path(&config_dir)).unwrap_or_default();
    Ok(())
}

/// Returns the in-memory settings (normalized on load).
#[tauri::command]
#[specta::specta]
pub fn load_settings() -> Result<AppSettings, FileSystemError> {
    let registry = REGISTRY.lock().expect("settings registry poisoned");
    Ok(registry.settings.clone())
}

/// Normalizes, stores, and persists the given settings, returning the
/// normalized document so the frontend converges on exactly what is on disk.
#[tauri::command]
#[specta::specta]
pub fn save_settings(settings: AppSettings) -> Result<AppSettings, FileSystemError> {
    let normalized = normalize(settings);
    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.settings = normalized.clone();
    persist_locked(&mut registry)?;
    Ok(normalized)
}

/// Updates only the cached default-file-manager flag (used by
/// [`crate::default_manager`] after a successful set/unset) and persists it.
pub(crate) fn set_default_file_manager_cache(is_default: bool) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.settings.default_file_manager.is_default = is_default;
    persist_locked(&mut registry)
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
    crate::file_system::sidebar::write_atomic(&settings_path(&config_dir), contents.as_bytes())
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

fn read_settings_file(path: &Path) -> Result<AppSettings, FileSystemError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let parsed: AppSettings = toml::from_str(&contents)
                .map_err(|error| FileSystemError::Internal(format!("settings.parse_failed: {error}")))?;
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
        // Only known action ids are honored; an empty string is a valid
        // "unbound" value and is preserved as-is.
        if normalized.contains_key(&id) {
            normalized.insert(id, binding);
        }
    }
    normalized
}

fn normalize_terminal(terminal: TerminalSettings) -> TerminalSettings {
    let font_size = terminal.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    let line_height = terminal.line_height.clamp(MIN_LINE_HEIGHT, MAX_LINE_HEIGHT);
    // A partial palette would mis-color the terminal; drop it unless complete.
    let ansi_colors = terminal.ansi_colors.and_then(|colors| {
        (colors.len() == ANSI_PALETTE_LEN).then_some(colors)
    });
    TerminalSettings {
        font_size,
        line_height,
        font_family: terminal.font_family,
        ansi_colors,
    }
}

/// Points the store at `config_dir` and reloads from disk, without an app
/// handle. Test-only.
#[cfg(test)]
pub(crate) fn use_config_dir_for_tests(config_dir: PathBuf) -> Result<(), FileSystemError> {
    let mut registry = REGISTRY.lock().expect("settings registry poisoned");
    registry.config_dir = Some(config_dir.clone());
    registry.settings = read_settings_file(&settings_path(&config_dir)).unwrap_or_default();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// The registry is a process-wide static, so tests that point it at a temp
    /// dir and read/write the file must not overlap. Hold this guard for the
    /// whole body of any such test. Poisoning is recovered (a panicking test
    /// must not cascade into the others).
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn temp_config_dir() -> PathBuf {
        let unique = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("dae-settings-test-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp config dir");
        dir
    }

    /// The default shortcut ids must match the frontend registry exactly.
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
        use_config_dir_for_tests(dir.clone()).expect("init");

        let mut settings = AppSettings::default();
        settings
            .shortcuts
            .insert("explorer.copy".to_string(), "Mod+Shift+C".to_string());
        settings.terminal.font_size = 16;
        settings.terminal.font_family = Some("JetBrains Mono".to_string());

        let saved = save_settings(settings.clone()).expect("save");
        assert_eq!(saved.terminal.font_size, 16);
        assert!(settings_path(&dir).exists(), "settings.toml written");

        // Reload from disk in a fresh registry view.
        use_config_dir_for_tests(dir.clone()).expect("reload");
        let loaded = load_settings().expect("load");
        assert_eq!(loaded, saved);
        assert_eq!(loaded.shortcuts["explorer.copy"], "Mod+Shift+C");
        assert_eq!(loaded.terminal.font_family.as_deref(), Some("JetBrains Mono"));

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

        use_config_dir_for_tests(dir.clone()).expect("init");
        let loaded = load_settings().expect("load");

        // Known id honors the user value (empty = unbound).
        assert_eq!(loaded.shortcuts["explorer.copy"], "");
        // Unknown id is dropped, and absent ids keep their defaults.
        assert!(!loaded.shortcuts.contains_key("bogus.id"));
        assert_eq!(loaded.shortcuts["app.commandBar"], "Mod+K");
        // Partial terminal section fills the rest from defaults.
        assert_eq!(loaded.terminal.font_size, 15);
        assert_eq!(loaded.terminal.line_height, 1.2);
        assert_eq!(loaded.terminal.font_family, None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clamps_out_of_range_terminal_values() {
        let _guard = lock();
        let dir = temp_config_dir();
        use_config_dir_for_tests(dir.clone()).expect("init");

        let mut settings = AppSettings::default();
        settings.terminal.font_size = 200;
        settings.terminal.line_height = 99.0;
        settings.terminal.ansi_colors = Some(vec!["#000000".to_string(); 3]); // wrong length

        let saved = save_settings(settings).expect("save");
        assert_eq!(saved.terminal.font_size, MAX_FONT_SIZE);
        assert_eq!(saved.terminal.line_height, MAX_LINE_HEIGHT);
        assert_eq!(saved.terminal.ansi_colors, None);

        fs::remove_dir_all(&dir).ok();
    }
}
