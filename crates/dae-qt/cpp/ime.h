// The CPS_CANCEL guard from gate 2 of docs/decisions/engine-qt-quick.md §5.
//
// Qt 6.12's QWindowsInputContext::reset() sends the stored composition string
// to the old focus object as a commit when the focus changes:
//
//     if (m_compositionContext.isComposing && !focusObject.isNull()) {
//         QInputMethodEvent event;
//         if (!m_compositionContext.composition.isEmpty())
//             event.setCommitString(m_compositionContext.composition);
//         QCoreApplication::sendEvent(m_compositionContext.focusObject, &event);
//
// The string is the raw composition, not the converted candidate. A press that
// would move focus away (or Tab/Backtab, which moves it without a press) is
// therefore cancelled at the IMM level *before* Qt dispatches it, so reset()
// has nothing left to commit. A press inside the composing item is left alone.
//
// This is control-agnostic: a QTextEdit would receive the same event, so
// swapping the text layer does not fix it.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <imm.h>

#include <QGuiApplication>
#include <QHash>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>

#include "lookup.h"

namespace dae_qt {

/// Cancels the IME composition of one window, if it has one.
inline bool cancel_composition(HWND hwnd)
{
    if (!hwnd)
        return false;
    HIMC context = ImmGetContext(hwnd);
    if (!context)
        return false;
    const BOOL cancelled = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
    ImmReleaseContext(hwnd, context);
    return cancelled != FALSE;
}

class FocusCancelFilter final : public QObject
{
public:
    explicit FocusCancelFilter(QQuickWindow* window)
        : QObject(window)
        , m_window(window)
    {
    }

    bool eventFilter(QObject*, QEvent* event) override
    {
        if (!m_window)
            return false;

        if (event->type() == QEvent::MouseButtonPress) {
            auto* mouse = static_cast<QMouseEvent*>(event);
            QQuickItem* target = m_window->contentItem()->childAt(
                mouse->position().x(), mouse->position().y());
            QQuickItem* focused = qobject_cast<QQuickItem*>(QGuiApplication::focusObject());
            for (QQuickItem* item = target; item; item = item->parentItem()) {
                if (item == focused)
                    return false; // press inside the composing item
            }
            cancel_composition(reinterpret_cast<HWND>(m_window->winId()));
        } else if (event->type() == QEvent::KeyPress) {
            auto* key = static_cast<QKeyEvent*>(event);
            if (key->key() == Qt::Key_Tab || key->key() == Qt::Key_Backtab)
                cancel_composition(reinterpret_cast<HWND>(m_window->winId()));
        }
        return false;
    }

private:
    QPointer<QQuickWindow> m_window;
};

/// Installs the guard once per window. The filter is parented to the window,
/// so it dies with it; the entry is removed from the registry at that point.
inline bool install_ime_guard(QQuickWindow* window)
{
    if (!window)
        return false;

    static QHash<QQuickWindow*, QPointer<FocusCancelFilter>> guards;
    if (guards.contains(window))
        return true;

    auto* filter = new FocusCancelFilter(window);
    window->installEventFilter(filter);
    guards.insert(window, filter);
    QObject::connect(window, &QObject::destroyed, [window](QObject*) {
        guards.remove(window);
    });
    return true;
}

/// The token-named entry point used by the Rust window controller.
inline bool dae_qt_install_focus_cancel_guard(const QString& token)
{
    return install_ime_guard(find_window_by_token(token));
}

} // namespace dae_qt
