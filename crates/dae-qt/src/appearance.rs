//! Shell appearance and language preferences.
//!
//! The Tauri shell keeps these in `localStorage` (see `src/lib/theme.ts` and
//! `src/i18n/index.ts`); the Qt shell has no web storage, so the same choices
//! live in a small document next to the other dae profile files. The document
//! belongs to this shell only: it is named after the shell, and the shared
//! `settings.toml` still holds the shortcuts and terminal typography both
//! shells read.
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

    fn document(&self) -> String {
        format!(
            "# dae Qt shell appearance preferences\ntheme={}\naccent={}\nlanguage={}\n",
            self.theme, self.accent, self.language
        )
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
    preferences_path()
        .map(|path| StoredAppearance::read(&path).language)
        .unwrap_or_else(|| "system".to_owned())
}

fn save(stored: &StoredAppearance) {
    let Some(path) = preferences_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        eprintln!("dae: could not create the appearance directory: {error}");
        return;
    }
    if let Err(error) = std::fs::write(&path, stored.document()) {
        eprintln!("dae: could not save appearance preferences: {error}");
    }
}

pub struct AppearanceRust {
    theme: QString,
    accent: QString,
    language: QString,
}

impl Default for AppearanceRust {
    fn default() -> Self {
        let stored = preferences_path()
            .map(|path| StoredAppearance::read(&path))
            .unwrap_or_default();
        Self {
            theme: QString::from(stored.theme.as_str()),
            accent: QString::from(stored.accent.as_str()),
            language: QString::from(stored.language.as_str()),
        }
    }
}

impl AppearanceRust {
    fn stored(&self) -> StoredAppearance {
        StoredAppearance {
            theme: self.theme.to_string(),
            accent: self.accent.to_string(),
            language: self.language.to_string(),
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
        save(&self.stored());
    }

    fn change_accent(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        if !ACCENTS.contains(&value.as_str()) || self.accent.to_string() == value {
            return;
        }
        self.as_mut().set_accent(QString::from(value.as_str()));
        save(&self.stored());
    }

    fn change_language(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        if !LANGUAGES.contains(&value.as_str()) || self.language.to_string() == value {
            return;
        }
        self.as_mut().set_language(QString::from(value.as_str()));
        save(&self.stored());
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
    fn a_hand_edited_document_round_trips() {
        let stored = StoredAppearance::from_document(
            "theme=dark\naccent=brand\nlanguage=en\n",
        );
        assert_eq!(stored.theme, "dark");
        assert_eq!(stored.accent, "brand");
        assert_eq!(stored.language, "en");

        let reparsed = StoredAppearance::from_document(&stored.document());
        assert_eq!(reparsed, stored);
    }

    #[test]
    fn unknown_values_keep_the_default() {
        let stored = StoredAppearance::from_document(
            "theme=sepia\naccent=rainbow\nlanguage=fr\nnonsense=1\n",
        );
        assert_eq!(stored, StoredAppearance::default());
    }
}
