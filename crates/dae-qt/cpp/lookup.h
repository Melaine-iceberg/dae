// Shared window lookup by the QML `objectName`.
//
// The Rust bridge cannot name a QQuickWindow (cxx-qt-lib exposes no such
// type), so QML gives every window a stable objectName and the native half
// resolves it. Titles are not used: they are user-visible and may repeat.

#pragma once

#include <QGuiApplication>
#include <QQuickWindow>
#include <QString>
#include <QWindow>

namespace dae_qt {

inline QQuickWindow* find_window_by_token(const QString& token)
{
    if (token.isEmpty())
        return nullptr;
    for (QWindow* candidate : QGuiApplication::topLevelWindows()) {
        auto* quick = qobject_cast<QQuickWindow*>(candidate);
        if (quick && quick->objectName() == token)
            return quick;
    }
    return nullptr;
}

} // namespace dae_qt
