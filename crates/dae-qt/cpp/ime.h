// The focus-change guard from gate 2 of docs/decisions/engine-qt-quick.md §5.
//
// Composing text and then moving the focus away is the case every platform
// gets wrong in a different place, so the rule the shell enforces is stated
// once, here, and the cancel primitive is picked per platform:
//
//   **A composition is cancelled, never committed, when the focus leaves the
//   item that owns it.**
//
// The raw composition (`shi'jie`) must not become the field's text: the
// converted candidate was never chosen, so there is nothing to commit.
//
// ## Why Windows needs its own primitive
//
// Qt 6.12's `QWindowsInputContext::reset()` sends the stored composition to the
// *old* focus object as a commit when the focus changes:
//
//     if (m_compositionContext.isComposing && !focusObject.isNull()) {
//         QInputMethodEvent event;
//         if (!m_compositionContext.composition.isEmpty())
//             event.setCommitString(m_compositionContext.composition);
//         QCoreApplication::sendEvent(m_compositionContext.focusObject, &event);
//     }
//
// The string is the raw composition, not the converted candidate. A press that
// would move focus away (or Tab/Backtab, which moves it without a press) is
// therefore cancelled at the IMM level *before* Qt dispatches it, so reset()
// has nothing left to commit. A press inside the composing item is left alone.
//
// This is control-agnostic: a QTextEdit would receive the same event, so
// swapping the text layer does not fix it.
//
// ## Why macOS and Linux can use `QInputMethod::reset()`
//
// Both of their input contexts implement reset as *cancel*, the opposite of the
// Windows one, so the same interception point can call straight into Qt and get
// the semantics this guard wants:
//
//  - **macOS** (`qcocoainputcontext.mm`, Qt 6.10 and 6.12):
//
//        void QCocoaInputContext::reset() { …
//            if (NSTextInputContext *ctxt = [NSTextInputContext currentInputContext]) {
//                [ctxt discardMarkedText];
//                [view unmarkText];      // sends an empty preedit to the item
//            }
//        }
//
//    `setFocusObject()` discards the marked text the same way, which is why
//    macOS does not show the Windows defect even without this guard.
//
//  - **Linux, ibus** (`qibusplatforminputcontext.cpp`, Qt 6.10 and 6.12):
//
//        void QIBusPlatformInputContext::reset() { …
//            d->context->Reset();        // ibus drops the preedit
//            d->predit = QString();
//        }
//
//    Only `commit()` sends `d->predit` as a commit string, and it is reached
//    from `invokeAction(Click)` — never from `setFocusObject()`, which sends
//    `FocusOut()` (cancel) instead.
//
// So on those two platforms the guard is belt and braces: it makes the shell's
// behaviour independent of the backend's focus policy (XIM, fcitx5 through
// ibus, a future Qt change), and it costs nothing while no composition is up,
// because the cancel is skipped entirely unless `preeditText` is non-empty.
//
// Gate 2 measured only the Windows one; docs/qt-platform-parity.md §4 lists the
// macOS/Linux manual pass this behaviour is written against.

#pragma once

#include <QtGlobal>

#if defined(Q_OS_WIN)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <imm.h>
#endif

#include <QGuiApplication>
#include <QHash>
#include <QInputMethod>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QString>
#include <QVariant>

#include "lookup.h"

namespace dae_qt {

/// Whether the focused item is mid-composition.
///
/// `QQuickTextInput` exposes the composition as the read-only `preeditText`
/// property, so this asks the item itself instead of the platform input
/// context — the platform's own idea of "composing" is exactly what cannot be
/// trusted on Windows.
inline bool focus_has_composition()
{
    QObject* focused = QGuiApplication::focusObject();
    if (!focused)
        return false;
    const QVariant preedit = focused->property("preeditText");
    return preedit.isValid() && !preedit.toString().isEmpty();
}

/// Cancels the composition of one window, if it has one.
///
/// Returns whether anything was cancelled. Only called when
/// `focus_has_composition()` answered true, so the platform round trip (an IMM
/// call on Windows, a D-Bus `Reset` on ibus) happens at most once per
/// composition.
inline bool cancel_composition(QQuickWindow* window)
{
#if defined(Q_OS_WIN)
    if (!window)
        return false;
    HWND hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd)
        return false;
    HIMC context = ImmGetContext(hwnd);
    if (!context)
        return false;
    const BOOL cancelled = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
    ImmReleaseContext(hwnd, context);
    return cancelled != FALSE;
#else
    // `window` is unused: Qt's input method is application-wide, and on both
    // platforms `reset()` acts on the focus object's own composition.
    Q_UNUSED(window);
    QInputMethod* inputMethod = QGuiApplication::inputMethod();
    if (!inputMethod)
        return false;
    inputMethod->reset();
    return true;
#endif
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
            // The property read is cheap and runs on every press; the platform
            // cancel it guards is not, so ask first.
            if (!focus_has_composition())
                return false;
            auto* mouse = static_cast<QMouseEvent*>(event);
            QQuickItem* target = m_window->contentItem()->childAt(
                mouse->position().x(), mouse->position().y());
            QQuickItem* focused = qobject_cast<QQuickItem*>(QGuiApplication::focusObject());
            for (QQuickItem* item = target; item; item = item->parentItem()) {
                if (item == focused)
                    return false; // press inside the composing item
            }
            cancel_composition(m_window);
        } else if (event->type() == QEvent::KeyPress) {
            // Tab moves the focus without a press, so it needs the same guard.
            // Escape is deliberately absent: the input method owns that key and
            // cancelling here would take it away from the IME.
            auto* key = static_cast<QKeyEvent*>(event);
            if ((key->key() == Qt::Key_Tab || key->key() == Qt::Key_Backtab)
                && focus_has_composition()) {
                cancel_composition(m_window);
            }
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
