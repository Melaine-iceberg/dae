#include "image_provider.h"

#include "dae-qt/src/icons.cxxqt.h"

#include <QFileInfo>
#include <QColor>
#include <QFont>
#include <QImageReader>
#include <QMutexLocker>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QUrl>
#include <QUrlQuery>
#include <QtQml/QQmlApplicationEngine>

#include <cstdint>
#include <string>

namespace dae_qt {

namespace {

constexpr int kDefaultSize = 256;
constexpr int kMaxSize = 2048;
constexpr qint64 kMaxSourceBytes = 64LL * 1024 * 1024;
constexpr int kCacheEntries = 256;

QString decodedPath(QString id)
{
    int query = id.indexOf(QLatin1Char('?'));
    if (query >= 0)
        id.truncate(query);
    while (id.startsWith(QLatin1Char('/')))
        id.remove(0, 1);
    return QUrl::fromPercentEncoding(id.toUtf8());
}

int requestedDimension(const QString& id, const QSize& requestedSize)
{
    int dimension = requestedSize.isValid()
        ? qMax(requestedSize.width(), requestedSize.height())
        : 0;
    if (dimension <= 0) {
        int query = id.indexOf(QLatin1Char('?'));
        if (query >= 0) {
            QUrlQuery values(id.mid(query + 1));
            dimension = values.queryItemValue(QStringLiteral("size")).toInt();
        }
    }
    if (dimension <= 0)
        dimension = kDefaultSize;
    return qBound(1, dimension, kMaxSize);
}

} // namespace

ThumbnailImageProvider::ThumbnailImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
    m_cache.setMaxCost(kCacheEntries);
}

QImage ThumbnailImageProvider::requestImage(const QString& id, QSize* size,
                                            const QSize& requestedSize)
{
    const QString path = decodedPath(id);
    const QFileInfo info(path);
    if (!info.isFile() || !info.isReadable())
        return {};

    const int dimension = requestedDimension(id, requestedSize);
    const QString key = QStringLiteral("%1|%2|%3|%4")
                            .arg(path)
                            .arg(dimension)
                            .arg(info.lastModified().toMSecsSinceEpoch())
                            .arg(info.size());

    {
        QMutexLocker locker(&m_mutex);
        if (const QImage* cached = m_cache.object(key)) {
            if (size)
                *size = cached->size();
            return *cached;
        }
    }

    QImage image;
    // The in-process decode stays bounded by the byte cap. A desktop-produced
    // thumbnail runs out of process, so the video this check would have
    // rejected is still eligible for a first frame.
    if (info.size() <= kMaxSourceBytes) {
        QImageReader reader(path);
        reader.setAutoTransform(true);
        const QSize original = reader.size();
        if (original.isValid() && original.width() > 0 && original.height() > 0) {
            const QSize target = original.scaled(dimension, dimension, Qt::KeepAspectRatio);
            if (target.isValid())
                reader.setScaledSize(target);
        }
        image = reader.read();

        if (!image.isNull()
            && (image.width() > dimension || image.height() > dimension)) {
            image = image.scaled(dimension, dimension, Qt::KeepAspectRatio,
                                 Qt::SmoothTransformation);
        }
    }

    // PDF first pages, video frames and formats Qt has no image plugin for.
    if (image.isNull()) {
        const rust::Vec<uint8_t> bytes = dae_qt_render_system_thumbnail(
            path.toStdString(), dimension, info.lastModified().toMSecsSinceEpoch());
        if (!bytes.empty()) {
            image = QImage::fromData(reinterpret_cast<const uchar*>(bytes.data()),
                                     int(bytes.size()));
        }
    }

    if (image.isNull())
        return {};

    if (size)
        *size = image.size();

    {
        QMutexLocker locker(&m_mutex);
        m_cache.insert(key, new QImage(image));
    }
    return image;
}

FileIconImageProvider::FileIconImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
}

QImage FileIconImageProvider::requestImage(const QString& id, QSize* size,
                                           const QSize& requestedSize)
{
    constexpr int kDefaultIconSize = 36;
    constexpr int kMinIconSize = 16;
    constexpr int kMaxIconSize = 256;

    const int queryStart = id.indexOf(QLatin1Char('?'));
    const QString body = queryStart >= 0 ? id.left(queryStart) : id;
    const QUrlQuery query(queryStart >= 0 ? id.mid(queryStart + 1) : QString());

    int dimension = requestedSize.isValid()
        ? qMax(requestedSize.width(), requestedSize.height())
        : 0;
    if (dimension <= 0)
        dimension = query.queryItemValue(QStringLiteral("size")).toInt();
    if (dimension <= 0)
        dimension = kDefaultIconSize;
    dimension = qBound(kMinIconSize, dimension, kMaxIconSize);

    rust::Vec<uint8_t> bytes;
    if (body.startsWith(QStringLiteral("type/"))) {
        const QByteArray extension =
            QUrl::fromPercentEncoding(body.mid(5).toUtf8()).toUtf8();
        bytes = dae_qt_render_type_icon(
            std::string(extension.constData(), extension.size()), dimension);
    } else if (body.startsWith(QStringLiteral("name/"))) {
        // A freedesktop icon name, or a colon-separated list of candidates.
        const QByteArray names =
            QUrl::fromPercentEncoding(body.mid(5).toUtf8()).toUtf8();
        bytes = dae_qt_render_named_icon(
            std::string(names.constData(), names.size()), dimension);
    } else if (body.startsWith(QStringLiteral("path/"))) {
        const QByteArray path =
            QUrl::fromPercentEncoding(body.mid(5).toUtf8()).toUtf8();
        const QByteArray version =
            query.queryItemValue(QStringLiteral("v")).toUtf8();
        const bool isDir =
            query.queryItemValue(QStringLiteral("dir")) == QStringLiteral("1");
        bytes = dae_qt_render_file_icon(
            std::string(path.constData(), path.size()), isDir, dimension,
            std::string(version.constData(), version.size()));
    }

    if (bytes.empty())
        return {};

    QImage image = QImage::fromData(reinterpret_cast<const uchar*>(bytes.data()),
                                    int(bytes.size()));
    if (image.isNull())
        return {};

    if (size)
        *size = image.size();
    return image;
}

namespace {

QColor glyphColor(const QUrlQuery& query, const QString& name, const QColor& fallback)
{
    const QColor color(QStringLiteral("#") + query.queryItemValue(name));
    return color.isValid() ? color : fallback;
}

/// The mark inside the page. The geometry is the 20-grid in
/// `docs/ui-redesign/figures/icons-light.svg`; the row paints it at 18 px,
/// where the mark is still the quickest way to tell a QML file from a PDF.
void drawFamilyMark(QPainter& painter, const QString& family, const QColor& tint)
{
    QPen mark(tint);
    mark.setWidthF(1.3);
    mark.setCapStyle(Qt::RoundCap);
    mark.setJoinStyle(Qt::RoundJoin);
    painter.setPen(mark);
    painter.setBrush(Qt::NoBrush);

    if (family == QLatin1String("code")) {
        QPainterPath left;
        left.moveTo(10.4, 9.9);
        left.lineTo(8.7, 12.1);
        left.lineTo(10.4, 14.3);
        painter.drawPath(left);
        QPainterPath right;
        right.moveTo(11.7, 9.9);
        right.lineTo(13.4, 12.1);
        right.lineTo(11.7, 14.3);
        painter.drawPath(right);
    } else if (family == QLatin1String("image")) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawEllipse(QPointF(9.0, 11.0), 1.2, 1.2);
        QPainterPath hills;
        hills.moveTo(6.6, 15.7);
        hills.lineTo(9.7, 12.6);
        hills.lineTo(11.5, 14.3);
        hills.lineTo(12.9, 12.9);
        hills.lineTo(14.1, 15.7);
        painter.setPen(mark);
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(hills);
    } else if (family == QLatin1String("video")) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        QPolygonF play;
        play << QPointF(8.5, 9.6) << QPointF(8.5, 15.2) << QPointF(14.0, 12.4);
        painter.drawPolygon(play);
    } else if (family == QLatin1String("audio")) {
        const qreal heights[] = {1.5, 2.9, 4.0, 2.9, 1.5};
        for (int index = 0; index < 5; ++index) {
            const qreal x = 7.2 + index * 1.475;
            painter.drawLine(QPointF(x, 12.3 - heights[index]),
                             QPointF(x, 12.3 + heights[index]));
        }
    } else if (family == QLatin1String("archive")) {
        painter.drawRoundedRect(QRectF(6.9, 10.0, 7.4, 5.2), 0.6, 0.6);
        painter.drawLine(QPointF(6.9, 11.7), QPointF(14.3, 11.7));
    } else if (family == QLatin1String("spreadsheet")) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawRoundedRect(QRectF(6.8, 9.7, 3.0, 2.9), 0.5, 0.5);
        painter.drawRoundedRect(QRectF(10.5, 9.7, 3.0, 2.9), 0.5, 0.5);
        painter.drawRoundedRect(QRectF(6.8, 13.0, 3.0, 2.9), 0.5, 0.5);
        painter.drawRoundedRect(QRectF(10.5, 13.0, 3.0, 2.9), 0.5, 0.5);
    } else if (family == QLatin1String("font")) {
        QFont glyph = painter.font();
        glyph.setPixelSize(7.6);
        glyph.setBold(true);
        painter.setFont(glyph);
        painter.drawText(QRectF(6.2, 8.2, 8.4, 8.0), Qt::AlignCenter,
                         QStringLiteral("Aa"));
    } else if (family == QLatin1String("executable")) {
        painter.drawEllipse(QPointF(10.2, 12.3), 3.3, 3.3);
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawEllipse(QPointF(10.2, 12.3), 1.1, 1.1);
    } else if (family == QLatin1String("file") || family == QLatin1String("other")
               || family == QLatin1String("link")) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawEllipse(QPointF(10.2, 12.3), 1.1, 1.1);
    } else {
        // Text-bearing families: document, text, pdf, presentation. Three
        // lines is the shape the icon figure uses for all of them; the family
        // colour is what separates one from another.
        for (int index = 0; index < 3; ++index) {
            const qreal y = 10.2 + index * 2.0;
            const qreal right = index == 2 ? 11.4 : 13.9;
            painter.drawLine(QPointF(6.9, y), QPointF(right, y));
        }
    }
}

QImage renderFileGlyph(int edge, const QString& family, const QColor& ink,
                       const QColor& tint)
{
    QImage image(edge, edge, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);

    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing, true);
    const qreal scale = edge / 20.0;
    painter.scale(scale, scale);

    QPen outline(ink);
    outline.setWidthF(1.3);
    outline.setJoinStyle(Qt::RoundJoin);
    outline.setCapStyle(Qt::RoundCap);
    painter.setPen(outline);
    painter.setBrush(Qt::NoBrush);

    if (family == QLatin1String("folder")) {
        QPainterPath folder;
        folder.moveTo(1.6, 5.4);
        folder.lineTo(7.0, 5.4);
        folder.lineTo(8.7, 7.4);
        folder.lineTo(18.4, 7.4);
        folder.lineTo(18.4, 16.6);
        folder.lineTo(1.6, 16.6);
        folder.closeSubpath();
        painter.drawPath(folder);
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawRoundedRect(QRectF(2.4, 7.9, 7.2, 1.6), 0.8, 0.8);
        return image;
    }

    // The page and its folded corner: the one shape every file shares.
    QPainterPath page;
    page.moveTo(3.6, 1.4);
    page.lineTo(13.0, 1.4);
    page.lineTo(16.6, 5.0);
    page.lineTo(16.6, 18.6);
    page.lineTo(3.6, 18.6);
    page.closeSubpath();
    painter.drawPath(page);

    QPainterPath fold;
    fold.moveTo(13.0, 1.4);
    fold.lineTo(13.0, 5.0);
    fold.lineTo(16.6, 5.0);
    painter.drawPath(fold);

    painter.save();
    painter.setClipPath(page);
    drawFamilyMark(painter, family, tint);
    painter.restore();
    return image;
}

} // namespace

FileGlyphImageProvider::FileGlyphImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
    m_cache.setMaxCost(64);
}

QImage FileGlyphImageProvider::requestImage(const QString& id, QSize* size,
                                            const QSize& requestedSize)
{
    constexpr int kMinEdge = 12;
    constexpr int kMaxEdge = 128;

    const int queryStart = id.indexOf(QLatin1Char('?'));
    QString family = queryStart >= 0 ? id.left(queryStart) : id;
    while (family.startsWith(QLatin1Char('/')))
        family.remove(0, 1);
    const QUrlQuery query(queryStart >= 0 ? id.mid(queryStart + 1) : QString());

    int edge = requestedSize.isValid()
        ? qMax(requestedSize.width(), requestedSize.height())
        : 0;
    if (edge <= 0)
        edge = query.queryItemValue(QStringLiteral("size")).toInt();
    edge = qBound(kMinEdge, edge, kMaxEdge);

    const QColor ink = glyphColor(query, QStringLiteral("ink"), QColor(0x6b, 0x6d, 0x72));
    const QColor tint = glyphColor(query, QStringLiteral("fc"), QColor(0x4d, 0x63, 0x8e));
    const QString key = QStringLiteral("%1|%2|%3|%4")
                            .arg(family)
                            .arg(edge)
                            .arg(ink.name(), tint.name());

    {
        QMutexLocker locker(&m_mutex);
        if (const QImage* cached = m_cache.object(key)) {
            if (size)
                *size = cached->size();
            return *cached;
        }
    }

    const QImage image = renderFileGlyph(edge, family, ink, tint);
    {
        QMutexLocker locker(&m_mutex);
        m_cache.insert(key, new QImage(image));
    }
    if (size)
        *size = image.size();
    return image;
}

void dae_qt_register_file_icon_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("fileicon"),
                                 new FileIconImageProvider());
}

void dae_qt_register_file_glyph_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("fileglyph"),
                                 new FileGlyphImageProvider());
}

void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("thumbnail"),
                                 new ThumbnailImageProvider());
}

} // namespace dae_qt
