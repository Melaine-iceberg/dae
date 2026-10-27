#include "drag.h"

#include <QCursor>
#include <QDrag>
#include <QFileInfo>
#include <QFont>
#include <QFontMetrics>
#include <QGuiApplication>
#include <QIcon>
#include <QImage>
#include <QMimeData>
#include <QPainter>
#include <QPen>
#include <QPixmap>
#include <QPoint>
#include <QUrl>
#include <QWindow>

#include <string>

#include "dae-qt/src/icons.cxxqt.h"
#include "lookup.h"

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

/// A tab as the OS drag loop draws it: the same rounded chip, title and
/// colors the strip shows, painted at `ratio` so it stays crisp on a scaled
/// display. The painter works in logical pixels; the pixmap's ratio is what
/// makes those logical pixels map onto the device grid.
QPixmap tab_preview(const QString& title, int width, int height,
                    const QColor& background, const QColor& border,
                    const QColor& text, qreal ratio)
{
    QPixmap pixmap(qMax(1, qRound(width * ratio)),
                   qMax(1, qRound(height * ratio)));
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::transparent);

    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setRenderHint(QPainter::TextAntialiasing);
    // Half-pixel inset keeps the 1 px border on the device grid.
    const QRectF chip(0.5, 0.5, width - 1.0, height - 1.0);
    painter.setPen(QPen(border, 1.0));
    painter.setBrush(background);
    painter.drawRoundedRect(chip, 6.0, 6.0);

    QFont font = QGuiApplication::font();
    font.setPixelSize(12);
    painter.setFont(font);
    painter.setPen(text);
    const QRectF text_rect = chip.adjusted(12.0, 0.0, -12.0, 0.0);
    const QFontMetrics metrics(font);
    const QString elided =
        metrics.elidedText(title, Qt::ElideMiddle, qRound(text_rect.width()));
    painter.drawText(text_rect, Qt::AlignLeft | Qt::AlignVCenter, elided);
    return pixmap;
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
        // QDrag's hot spot is in device-independent pixels; Qt scales it for
        // the platform's drag image (OLE multiplies by the screen factor).
        const QSizeF logical = preview.deviceIndependentSize();
        drag->setHotSpot(QPoint(qRound(logical.width() / 2.0),
                                qRound(logical.height() / 2.0)));
    }

    s_drag_in_progress = true;
    drag->exec(Qt::DropActions(actions));
    s_drag_in_progress = false;
    return true;
}

int dae_qt_start_tab_drag(const QString& source_token, const QString& title,
                          int tab_index, int grab_x, int grab_y, int width,
                          int height, const QColor& background,
                          const QColor& border, const QColor& text)
{
    if (s_drag_in_progress || width <= 0 || height <= 0)
        return Qt::IgnoreAction;

    QWindow* source = find_window_by_token(source_token);
    if (!source)
        return Qt::IgnoreAction;

    auto* mime = new QMimeData;
    // The drop target only needs to say which tab landed; the source window
    // owns the tab object and moves it once the loop returns.
    const QString payload =
        source_token + QLatin1Char('\t') + QString::number(tab_index);
    const QByteArray mime_type(kTabDragMimeType);
    mime->setData(QString::fromLatin1(mime_type), payload.toUtf8());

    auto* drag = new QDrag(source);
    drag->setMimeData(mime);

    const qreal ratio = qMax<qreal>(1.0, source->devicePixelRatio());
    drag->setPixmap(tab_preview(title, width, height, background, border, text,
                                ratio));
    drag->setHotSpot(
        QPoint(qBound(0, grab_x, width - 1), qBound(0, grab_y, height - 1)));

    s_drag_in_progress = true;
    const Qt::DropAction action = drag->exec(Qt::MoveAction);
    s_drag_in_progress = false;
    return int(action);
}

int dae_qt_cursor_logical_x()
{
    return QCursor::pos().x();
}

int dae_qt_cursor_logical_y()
{
    return QCursor::pos().y();
}

} // namespace dae_qt
