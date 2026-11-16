//! Shell appearance and language preferences, persisted in the shared
//! `settings.toml` document with shortcuts and terminal typography.
//!
//! `theme` is `system` / `light` / `dark`, `accent` is `system` / `brand`, and
//! `language` is `system` / `zh-CN` / `zh-TW` / `en`. Values are validated on
//! load and on write, so a hand-edited file cannot leave the UI in a state the
//! panel does not offer. The actual language install is done by
//! [`crate::i18n`]; keeping the preference here means the startup path can read
//! it before the QML engine loads its first file.

use std::path::PathBuf;

use qtbridge::{QmlElement, qobject};

const FILE_NAME: &str = "appearance-qt.ini";

const THEMES: &[&str] = &["system", "light", "dark"];
const ACCENTS: &[&str] = &["system", "brand"];
const LANGUAGES: &[&str] = dae_core::settings::LANGUAGES;
const MATERIALS: &[&str] = &["mica", "acrylic", "none"];

/// The C++ half of the language install. Only the `extern "C++"` block is
/// left here after the Qt Bridge port; [`crate::i18n`] still calls these three
/// helpers. The engine address is exchanged as `size_t` because an opaque
/// `QObject` type would be a second declaration of the same C++ class; the
/// helper casts it back to `QObject*` before the `qobject_cast`.
#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_attach_engine(engine: usize);

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_apply(language: &QString) -> bool;

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_apply_deferred(language: &QString);

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_system_language() -> QString;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAppearance {
    theme: String,
    accent: String,
    language: String,
}

impl Default for StoredAppearance {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            accent: "system".to_owned(),
            language: "system".to_owned(),
        }
    }
}

impl StoredAppearance {
    #[cfg(test)]
    fn read(path: &PathBuf) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self::from_document(&contents)
    }

    fn from_document(contents: &str) -> Self {
        let mut stored = Self::default();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" if THEMES.contains(&value) => stored.theme = value.to_owned(),
                "accent" if ACCENTS.contains(&value) => stored.accent = value.to_owned(),
                "language" if LANGUAGES.contains(&value) => stored.language = value.to_owned(),
                _ => {}
            }
        }
        stored
    }
}

fn preferences_path() -> Option<PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|dir| dir.join(FILE_NAME))
}

/// The persisted language before QML exists. `system` is returned verbatim;
/// [`crate::i18n`] resolves it against the OS locale.
pub fn stored_language() -> String {
    dae_core::settings::load_settings()
        .map(|settings| settings.language)
        .unwrap_or_else(|_| "system".to_owned())
}

/// Whether this launch should present the stand-in window.
///
/// Read before the QML engine exists, which is why it comes from the stored
/// document rather than from QML state. `DAE_SHELL_FIRST` overrides it for
/// measurement (`0` off, anything else on).
pub fn stored_shell_first() -> bool {
    match std::env::var("DAE_SHELL_FIRST") {
        Ok(value) => !matches!(value.trim(), "0" | "false" | "off"),
        Err(_) => dae_core::settings::load_settings()
            .map(|settings| settings.shell_first)
            .unwrap_or(true),
    }
}

pub(crate) fn migrate_legacy() {
    let Some(legacy_path) = preferences_path() else {
        return;
    };
    let Ok(legacy_document) = std::fs::read_to_string(legacy_path) else {
        return;
    };
    let legacy = StoredAppearance::from_document(&legacy_document);
    let Ok(settings_path) = dae_core::settings::settings_file() else {
        return;
    };
    let settings_document = std::fs::read_to_string(settings_path).unwrap_or_default();
    let Ok(existing) = toml::from_str::<toml::Value>(&settings_document) else {
        return;
    };
    let Ok(mut settings) = dae_core::settings::load_settings() else {
        return;
    };
    if import_legacy(&mut settings, &legacy, &existing)
        && let Err(error) = dae_core::settings::save_settings(settings)
    {
        log::warn!(target: "dae::appearance", "could not migrate appearance preferences: {error}");
    }
}

fn import_legacy(
    settings: &mut dae_core::settings::AppSettings,
    legacy: &StoredAppearance,
    existing: &toml::Value,
) -> bool {
    let mut changed = false;
    if existing.get("theme").is_none() {
        settings.theme = legacy.theme.clone();
        changed = true;
    }
    if existing.get("accent").is_none() {
        // The legacy shell defaulted to the OS accent. Decision 1 makes the
        // brand teal the out-of-the-box choice, and a user who never picked
        // "system" should not be migrated into it.
        settings.accent = if legacy.accent == "system" {
            "brand".to_owned()
        } else {
            legacy.accent.clone()
        };
        changed = true;
    }
    if existing.get("language").is_none() {
        settings.language = legacy.language.clone();
        changed = true;
    }
    changed
}

fn save(change: impl FnOnce(&mut dae_core::settings::AppSettings)) {
    match dae_core::settings::load_settings() {
        Ok(mut settings) => {
            change(&mut settings);
            if let Err(error) = dae_core::settings::save_settings(settings) {
                log::warn!(target: "dae::appearance", "could not save appearance preferences: {error}");
            }
        }
        Err(error) => {
            log::warn!(target: "dae::appearance", "could not load appearance preferences: {error}")
        }
    }
}

/// QML usage: `Appearance.theme`, `Appearance.chooseTheme("dark")`.
///
/// Theme, accent and language are separate fields rather than one
/// "appearance" object so changing one never re-writes the other two.
pub struct Appearance {
    theme: String,
    accent: String,
    language: String,
    blur_enabled: bool,
    window_material: String,
    /// Whether the next launch presents a text-free window while the QML
    /// tree is still being built (`docs/qml-aot.md` §16). Read by `main`
    /// before the engine exists, so it cannot live in QML state.
    shell_first: bool,
    /// The UI family chosen in settings. Empty keeps the bundled Noto Sans SC.
    /// This is the saved preference, not necessarily the family this process
    /// started with.
    ui_font_family: String,
    /// The fixed-pitch family chosen in settings. Empty keeps the bundled
    /// GoogleSansCode NFM. QML binds this live, so no restart is needed.
    mono_font_family: String,
    /// The UI family this process applied before QML started. `Theme` reads
    /// this instead of `ui_font_family`: `QQuickText` copies the application
    /// font when it is created, so a new preference cannot repaint the open
    /// window and must wait for the next launch.
    applied_ui_font: String,
}

impl Default for Appearance {
    fn default() -> Self {
        let stored = dae_core::settings::load_settings().unwrap_or_default();
        let ui_font_family = stored.fonts.ui_family.unwrap_or_default();
        Self {
            theme: stored.theme,
            accent: stored.accent,
            language: stored.language,
            blur_enabled: stored.blur,
            window_material: stored.window_material,
            shell_first: stored.shell_first,
            applied_ui_font: ui_font_family.clone(),
            ui_font_family,
            mono_font_family: stored.fonts.mono_family.unwrap_or_default(),
        }
    }
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
impl Appearance {
    qproperty!("theme", Member = theme, Notify = theme_changed);
    qproperty!("accent", Member = accent, Notify = accent_changed);
    qproperty!("language", Member = language, Notify = language_changed);
    qproperty!("blurEnabled", Member = blur_enabled, Notify = blur_enabled_changed);
    qproperty!(
        "windowMaterial",
        Member = window_material,
        Notify = window_material_changed
    );
    qproperty!("shellFirst", Member = shell_first, Notify = shell_first_changed);
    qproperty!("uiFontFamily", Member = ui_font_family, Notify = ui_font_family_changed);
    qproperty!("monoFontFamily", Member = mono_font_family, Notify = mono_font_family_changed);
    qproperty!("appliedUiFont", Member = applied_ui_font, Notify = applied_ui_font_changed);

    #[qsignal]
    fn theme_changed(&mut self);
    #[qsignal]
    fn accent_changed(&mut self);
    #[qsignal]
    fn language_changed(&mut self);
    #[qsignal]
    fn blur_enabled_changed(&mut self);
    #[qsignal]
    fn window_material_changed(&mut self);
    #[qsignal]
    fn shell_first_changed(&mut self);
    #[qsignal]
    fn ui_font_family_changed(&mut self);
    #[qsignal]
    fn mono_font_family_changed(&mut self);
    #[qsignal]
    fn applied_ui_font_changed(&mut self);

    // `SettingsPanel` calls these `chooseTheme`, `chooseAccent`, … — the names
    // the CXX-Qt bridge exported through `cxx_name`. They stay, so no `.qml`
    // file changed; `ConvertToCamelCase` reproduces them from these idents.
    #[qslot]
    fn choose_theme(&mut self, value: String) {
        if !THEMES.contains(&value.as_str()) || self.theme == value {
            return;
        }
        self.theme.clone_from(&value);
        self.theme_changed();
        save(|settings| settings.theme = value);
    }

    #[qslot]
    fn choose_accent(&mut self, value: String) {
        if !ACCENTS.contains(&value.as_str()) || self.accent == value {
            return;
        }
        self.accent.clone_from(&value);
        self.accent_changed();
        save(|settings| settings.accent = value);
    }

    #[qslot]
    fn choose_language(&mut self, value: String) {
        if !LANGUAGES.contains(&value.as_str()) || self.language == value {
            return;
        }
        self.language.clone_from(&value);
        self.language_changed();
        save(|settings| settings.language = value.clone());
        crate::i18n::apply_deferred(&value);
    }

    #[qslot]
    fn choose_blur(&mut self, enabled: bool) {
        if self.blur_enabled == enabled {
            return;
        }
        self.blur_enabled = enabled;
        self.blur_enabled_changed();
        save(|settings| settings.blur = enabled);
    }

    #[qslot]
    fn choose_window_material(&mut self, value: String) {
        if !MATERIALS.contains(&value.as_str()) || self.window_material == value {
            return;
        }
        self.window_material.clone_from(&value);
        self.window_material_changed();
        save(|settings| settings.window_material = value);
    }

    /// Stores the two families. The mono side binds live in QML; the UI side
    /// is applied before the next launch, because existing `Text` items copy
    /// the application font when they are created.
    #[qslot]
    fn choose_fonts(&mut self, ui: String, mono: String) {
        let ui = ui.trim().to_owned();
        let mono = mono.trim().to_owned();
        if self.ui_font_family == ui && self.mono_font_family == mono {
            return;
        }
        if self.ui_font_family != ui {
            self.ui_font_family.clone_from(&ui);
            self.ui_font_family_changed();
        }
        if self.mono_font_family != mono {
            self.mono_font_family.clone_from(&mono);
            self.mono_font_family_changed();
        }
        save(|settings| {
            settings.fonts.ui_family = (!ui.is_empty()).then_some(ui.clone());
            settings.fonts.mono_family = (!mono.is_empty()).then_some(mono.clone());
        });
    }

    /// Takes effect on the next launch: the current one has already decided
    /// whether to show the stand-in window.
    #[qslot]
    fn choose_shell_first(&mut self, enabled: bool) {
        if self.shell_first == enabled {
            return;
        }
        self.shell_first = enabled;
        self.shell_first_changed();
        save(|settings| settings.shell_first = enabled);
    }
}

impl QmlElement for Appearance {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Appearance";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_document_reads_the_defaults() {
        assert_eq!(
            StoredAppearance::read(&PathBuf::from("definitely/missing.ini")),
            StoredAppearance::default()
        );
    }

    #[test]
    fn a_hand_edited_document_is_read() {
        let stored = StoredAppearance::from_document("theme=dark\naccent=brand\nlanguage=en\n");
        assert_eq!(stored.theme, "dark");
        assert_eq!(stored.accent, "brand");
        assert_eq!(stored.language, "en");
    }

    #[test]
    fn unknown_values_keep_the_default() {
        let stored = StoredAppearance::from_document(
            "theme=sepia\naccent=rainbow\nlanguage=fr\nnonsense=1\n",
        );
        assert_eq!(stored, StoredAppearance::default());
    }

    #[test]
    fn legacy_values_only_fill_missing_shared_settings() {
        let legacy = StoredAppearance::from_document("theme=dark\naccent=brand\nlanguage=en\n");
        let existing = toml::from_str("theme = \"system\"\n").expect("valid TOML");
        let mut settings = dae_core::settings::AppSettings::default();
        settings.terminal.font_size = 16;

        assert!(import_legacy(&mut settings, &legacy, &existing));
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.accent, "brand");
        assert_eq!(settings.language, "en");
        assert_eq!(settings.terminal.font_size, 16);
    }

    #[test]
    fn the_legacy_system_accent_migrates_to_brand() {
        // The old shell defaulted to the OS accent. Decision 1 makes the brand
        // teal the default, so a user who never picked "system" must not be
        // carried into it by the migration.
        let legacy = StoredAppearance::from_document("theme=system\naccent=system\n");
        let existing = toml::from_str("").expect("valid TOML");
        let mut settings = dae_core::settings::AppSettings::default();

        assert!(import_legacy(&mut settings, &legacy, &existing));
        assert_eq!(settings.accent, "brand");
    }
}
