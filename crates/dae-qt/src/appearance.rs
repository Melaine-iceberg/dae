//! Shell appearance and language preferences, persisted in the shared
//! `settings.toml` document with shortcuts and terminal typography.
//!
//! `theme` is `system` / `light` / `dark`, `accent` is `system` / `brand`, and
//! `language` is `system` / `zh-CN` / `en`. Values are validated on load and on
//! write, so a hand-edited file cannot leave the UI in a state the panel does
//! not offer. The actual language install is done by [`crate::i18n`]; keeping
//! the preference here means the startup path can read it before the QML engine
//! loads its first file.

use std::path::PathBuf;

use cxx_qt_lib::QString;

const FILE_NAME: &str = "appearance-qt.ini";

const THEMES: &[&str] = &["system", "light", "dark"];
const ACCENTS: &[&str] = &["system", "brand"];
const LANGUAGES: &[&str] = &["system", "zh-CN", "en"];

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        // The engine address is exchanged as `size_t`: `QObject` is already
        // implied by the RustQt type below, and an opaque pointer type here
        // would be a second declaration of the same C++ class. The helper
        // casts it back to `QObject*` before the `qobject_cast`.
        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_attach_engine(engine: usize);

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_apply(language: &QString) -> bool;

        include!("translator.h");
        #[namespace = "dae_qt"]
        fn dae_qt_i18n_system_language() -> QString;
    }

    unsafe extern "RustQt" {
        /// QML usage: `Appearance.theme`, `Appearance.setTheme("dark")`.
        ///
        /// Theme, accent and language are separate fields rather than one
        /// "appearance" object so changing one never re-writes the other two.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, theme)]
        #[qproperty(QString, accent)]
        #[qproperty(QString, language)]
        type Appearance = super::AppearanceRust;

        #[qinvokable]
        #[cxx_name = "chooseTheme"]
        fn change_theme(self: Pin<&mut Appearance>, value: &QString);

        #[qinvokable]
        #[cxx_name = "chooseAccent"]
        fn change_accent(self: Pin<&mut Appearance>, value: &QString);

        #[qinvokable]
        #[cxx_name = "chooseLanguage"]
        fn change_language(self: Pin<&mut Appearance>, value: &QString);
    }
}

use std::pin::Pin;

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
        eprintln!("dae: could not migrate appearance preferences: {error}");
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
        settings.accent = legacy.accent.clone();
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
                eprintln!("dae: could not save appearance preferences: {error}");
            }
        }
        Err(error) => eprintln!("dae: could not load appearance preferences: {error}"),
    }
}

pub struct AppearanceRust {
    theme: QString,
    accent: QString,
    language: QString,
}

impl Default for AppearanceRust {
    fn default() -> Self {
        let stored = dae_core::settings::load_settings().unwrap_or_default();
        Self {
            theme: QString::from(stored.theme.as_str()),
            accent: QString::from(stored.accent.as_str()),
            language: QString::from(stored.language.as_str()),
        }
    }
}

impl qobject::Appearance {
    fn change_theme(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        if !THEMES.contains(&value.as_str()) || self.theme.to_string() == value {
            return;
        }
        self.as_mut().set_theme(QString::from(value.as_str()));
        save(|settings| settings.theme = value);
    }

    fn change_accent(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        if !ACCENTS.contains(&value.as_str()) || self.accent.to_string() == value {
            return;
        }
        self.as_mut().set_accent(QString::from(value.as_str()));
        save(|settings| settings.accent = value);
    }

    fn change_language(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        if !LANGUAGES.contains(&value.as_str()) || self.language.to_string() == value {
            return;
        }
        self.as_mut().set_language(QString::from(value.as_str()));
        save(|settings| settings.language = value.clone());
        crate::i18n::apply(&value);
    }
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
}
