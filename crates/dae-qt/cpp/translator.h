#pragma once

#include <QString>
#include <cstddef>

namespace dae_qt {

/// Remembers the engine whose `retranslate()` runs after a language change.
/// The address arrives as `size_t` so the Rust bridge needs no duplicate
/// `QObject` type declaration.
void dae_qt_i18n_attach_engine(std::size_t engine);

/// Installs the translation for `language` (`system`, `zh-CN` or `en`).
/// Returns true when a translator is active; Chinese is the source language,
/// so it needs none.
bool dae_qt_i18n_apply(const QString &language);

/// The OS UI language reduced to the two languages this shell ships.
QString dae_qt_i18n_system_language();

} // namespace dae_qt
