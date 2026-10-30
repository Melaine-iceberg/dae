// Cross-platform appearance watcher installed by the Qt shell.
//
// Qt's QStyleHints follows the platform's light/dark preference on every
// supported desktop and emits `colorSchemeChanged`; the shell only has to turn
// that signal into the same Rust update the Windows registry watcher sends.

#ifndef DAE_QT_PLATFORM_WATCH_H
#define DAE_QT_PLATFORM_WATCH_H

namespace dae_qt {

/// Connects `QStyleHints::colorSchemeChanged` to `dae_qt_theme_changed`.
/// Calling it more than once is a no-op.
void dae_qt_install_theme_watcher();

} // namespace dae_qt

#endif // DAE_QT_PLATFORM_WATCH_H
