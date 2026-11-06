//! The Qt shell's language switch.
//!
//! QML marks every user-visible sentence with `qsTr()`. The source strings are
//! Simplified Chinese, matching the shipped shell; `qml/i18n/dae_en.ts` and
//! `qml/i18n/dae_zh_TW.ts` carry the English and Traditional Chinese
//! translations. All of them are embedded as Qt resources by
//! `apps/dae/build.rs`, so a deployed binary needs no loose files.
//!
//! Every non-source language is installed as a chain: English first, then the
//! requested translation on top. Qt searches the most recently installed
//! translator first, so a key the translation lacks reads in English rather
//! than falling back to the Chinese source.
//!
//! Installation is a small C++ helper because `QTranslator` is not part of
//! `cxx-qt-lib` and because switching language has to call
//! `QQmlEngine::retranslate()` for already-evaluated bindings. The helper also
//! parses the `.ts` document directly: this avoids requiring `lrelease` at
//! build time, and the XML file stays the reviewed source of truth.
//!
//! The engine is attached once from `main` before the first QML file loads; a
//! later language change from the settings panel reuses that pointer. The C++
//! declarations live in [`crate::appearance`] so the appearance singleton and
//! startup path share one bridge.

/// Stores the engine that a later language change has to retranslate.
///
/// `main` calls this before `engine.load`, so the very first binding already
/// sees the persisted language. Safe to call again with the same engine.
pub fn attach_engine(engine: usize) {
    crate::appearance::qobject::dae_qt_i18n_attach_engine(engine);
}

/// Applies the language saved in `settings.toml`. `system` resolves to the OS
/// UI language: a Chinese locale keeps either script, everything else loads
/// English.
pub fn apply_stored() {
    let language = crate::appearance::stored_language();
    apply(&language);
}

/// Applies one of `system`, `zh-CN`, `zh-TW` or `en`. Returns whether a
/// translation was installed (`false` means the Simplified Chinese source
/// strings are in use).
pub fn apply(language: &str) -> bool {
    crate::appearance::qobject::dae_qt_i18n_apply(&cxx_qt_lib::QString::from(language))
}

/// The locale name the language menu can show beside "follow system".
pub fn system_language() -> String {
    crate::appearance::qobject::dae_qt_i18n_system_language().to_string()
}
