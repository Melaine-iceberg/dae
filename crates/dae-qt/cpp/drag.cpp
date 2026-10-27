#include "drag.h"

#include <QDrag>
#include <QGuiApplication>
#include <QIcon>
#include <QMimeData>
#include <QUrl>
#include <QWindow>

namespace dae_qt {
namespace {

/// A drag's modal loop dispatches queued events re-entrantly, so a second
/// gesture arriving mid-drag must not start a nested drag.
bool s_drag_in_progress = false;

/// The drag's owner. The focused window is the one the gesture started in;
/// falling back to the first visible window keeps the call working while focus
/// is transiently elsewhere (a popup closing, for example).
QWindow* drag_source_window()
{
    if (QWindow* focused = QGuiApplication::focusWindow())
        return focused;

    const auto windows = QGuiApplication::topLevelWindows();
    for (QWindow* window : windows) {
        if (window->isVisible())
            return window;
    }
    return nullptr;
}

} // namespace

bool dae_qt_start_file_drag(const QStringList& paths, int actions)
{
    if (paths.isEmpty() || s_drag_in_progress)
        return false;

    QList<QUrl> urls;
    for (const QString& path : paths)
        urls.append(QUrl::fromLocalFile(path));
    if (urls.isEmpty())
        return false;

    QWindow* source = drag_source_window();
    if (!source)
        return false;

    auto* mime = new QMimeData;
    // `setUrls` is what makes Qt export CF_HDROP on Windows, which is the
    // format Explorer, chat apps and browser upload fields all read.
    mime->setUrls(urls);

    // Qt takes ownership of the QDrag and its mime data at the end of `exec`.
    auto* drag = new QDrag(source);
    drag->setMimeData(mime);
    const QIcon icon = QGuiApplication::windowIcon();
    if (!icon.isNull())
        drag->setPixmap(icon.pixmap(32, 32));

    s_drag_in_progress = true;
    drag->exec(Qt::DropActions(actions));
    s_drag_in_progress = false;
    return true;
}

} // namespace dae_qt
