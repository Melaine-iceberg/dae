#pragma once

#include <QByteArray>
#include <QString>
#include <cstddef>

#include "rust/cxx.h"

namespace dae_qt {

/// Remembers the engine whose `retranslate()` runs after a language change.
/// The address arrives as `size_t` so the Rust bridge needs no duplicate
/// `QObject` type declaration.
void dae_qt_i18n_attach_engine(std::size_t engine);

/// Installs the translation for `language` (`system`, `zh-CN` or `en`).
/// Returns true when a translator is active; Chinese is the source language,
/// so it needs none.
bool dae_qt_i18n_apply(const QString &language);

/// Applies a settings-panel choice after its Rust slot has returned. Running
/// the translator change and QML retranslation inside the slot re-enters the
/// Appearance property getter while QtBridge still holds its mutable borrow.
void dae_qt_i18n_apply_deferred(const QString &language);

/// The OS UI language reduced to the two languages this shell ships.
QString dae_qt_i18n_system_language();

inline bool dae_qt_i18n_apply_rs(rust::Str language)
{
    return dae_qt_i18n_apply(
        QString::fromUtf8(language.data(), static_cast<qsizetype>(language.size())));
}

inline void dae_qt_i18n_apply_deferred_rs(rust::Str language)
{
    dae_qt_i18n_apply_deferred(
        QString::fromUtf8(language.data(), static_cast<qsizetype>(language.size())));
}

inline rust::String dae_qt_i18n_system_language_rs()
{
    const QByteArray utf8 = dae_qt_i18n_system_language().toUtf8();
    return rust::String(utf8.constData(), static_cast<std::size_t>(utf8.size()));
}

} // namespace dae_qt
