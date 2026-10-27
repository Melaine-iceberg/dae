#include "clipboard.h"

#include <QClipboard>
#include <QGuiApplication>

namespace dae_qt {

void dae_qt_set_clipboard_text(const QString& text)
{
    QClipboard* clipboard = QGuiApplication::clipboard();
    if (clipboard == nullptr) {
        return;
    }

    clipboard->setText(text, QClipboard::Clipboard);
    // X11 keeps the selection buffer next to the clipboard; a middle-click
    // paste reads the former. Everywhere else `supportsSelection()` is false.
    if (clipboard->supportsSelection()) {
        clipboard->setText(text, QClipboard::Selection);
    }
}

} // namespace dae_qt
