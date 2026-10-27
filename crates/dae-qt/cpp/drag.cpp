#include "drag.h"

#include <QDrag>
#include <QFileInfo>
#include <QGuiApplication>
#include <QIcon>
#include <QImage>
#include <QMimeData>
#include <QPixmap>
#include <QPoint>
#include <QUrl>
#include <QWindow>

#include <string>

#include "dae-qt/src/icons.cxxqt.h"

namespace dae_qt {
namespace {

/// A drag's modal loop dispatches queued events re-entrantly, so a second
/// gesture arriving mid-drag must not start a nested drag.
bool s_drag_in_progress = false;

/// The image the drag cursor carries: the first entry's own shell icon, so a
/// file looks the same leaving this window as it does in Explorer. Falls back
/// to the application icon when the shell has no icon for the entry.
QPixmap drag_preview(const QStringList& paths, int edge)
{
    const QString& first = paths.first();
    const QByteArray path = first.toUtf8();
    const rust::Vec<uint8_t> bytes = dae_qt_render_file_icon(
        std::string(path.constData(), path.size()), QFileInfo(first).isDir(),
        edge, std::string());
    if (!bytes.empty()) {
        const QImage image = QImage::fromData(
            reinterpret_cast<const uchar*>(bytes.data()), int(bytes.size()));
        if (!image.isNull())
            return QPixmap::fromImage(image);
    }

    const QIcon icon = QGuiApplication::windowIcon();
    return icon.isNull() ? QPixmap() : icon.pixmap(edge, edge);
}

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

    // Render at the window's device-pixel ratio so the preview is crisp on a
    // scaled display, and tag the pixmap with the same ratio so Qt draws it at
    // the intended 48 logical pixels.
    constexpr int kPreviewLogicalEdge = 48;
    const qreal ratio = qMax<qreal>(1.0, source->devicePixelRatio());
    const int edge = qMax(16, qRound(kPreviewLogicalEdge * ratio));
    QPixmap preview = drag_preview(paths, edge);
    if (!preview.isNull()) {
        preview.setDevicePixelRatio(ratio);
        drag->setPixmap(preview);
        drag->setHotSpot(preview.rect().center());
    }

    s_drag_in_progress = true;
    drag->exec(Qt::DropActions(actions));
    s_drag_in_progress = false;
    return true;
}

} // namespace dae_qt
