#include "image_provider.h"

#include "dae-qt/src/icons.cxxqt.h"
#include "material_marks.h"

#include <QFileInfo>
#include <QColor>
#include <QElapsedTimer>
#include <QBuffer>
#include <QFont>
#include <QImageReader>
#include <QMutexLocker>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QTransform>
#include <QUrl>
#include <QUrlQuery>
#include <QtGlobal>
#include <QtQml/QQmlApplicationEngine>

#include <atomic>
#include <cstdint>
#include <cstdio>
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

/// The `v=...` revision the listing put in the URL. A local file's cache key
/// re-reads mtime and size from disk; a remote source has only this stamp.
QString revisionOf(const QString& id)
{
    const int query = id.indexOf(QLatin1Char('?'));
    if (query < 0)
        return QString();
    const QUrlQuery values(id.mid(query + 1));
    return values.queryItemValue(QStringLiteral("v"));
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
    const int dimension = requestedDimension(id, requestedSize);

    // A network backend is not a file QFileInfo could stat; its bytes arrive
    // bounded from the vfs layer and its revision travels in the URL.
    if (!dae_qt_is_local_path(path.toStdString()))
        return remoteImage(id, path, dimension, size);

    const QFileInfo info(path);
    if (!info.isFile() || !info.isReadable())
        return {};

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

// The remote half of the thumbnail provider: bounded bytes from the vfs
// layer, decoded from memory the same way the local half decodes from disk.
// The revision in the URL (`v=mtime-size`) is the only freshness fact a
// remote source has, so it doubles as the cache key.
QImage ThumbnailImageProvider::remoteImage(const QString& id, const QString& path,
                                           int dimension, QSize* size)
{
    const QString key = QStringLiteral("%1|%2|%3|remote")
                            .arg(path)
                            .arg(dimension)
                            .arg(revisionOf(id));
    {
        QMutexLocker locker(&m_mutex);
        if (const QImage* cached = m_cache.object(key)) {
            if (size)
                *size = cached->size();
            return *cached;
        }
    }

    const rust::Vec<uint8_t> bytes =
        dae_qt_fetch_remote_thumbnail(path.toStdString(), kMaxSourceBytes);
    if (bytes.empty())
        return {};

    QBuffer buffer;
    buffer.setData(reinterpret_cast<const char*>(bytes.data()),
                   qint64(bytes.size()));
    buffer.open(QIODevice::ReadOnly);
    QImageReader reader(&buffer);
    reader.setAutoTransform(true);
    const QSize original = reader.size();
    if (original.isValid() && original.width() > 0 && original.height() > 0) {
        const QSize target = original.scaled(dimension, dimension, Qt::KeepAspectRatio);
        if (target.isValid())
            reader.setScaledSize(target);
    }
    QImage image = reader.read();
    if (!image.isNull()
        && (image.width() > dimension || image.height() > dimension)) {
        image = image.scaled(dimension, dimension, Qt::KeepAspectRatio,
                             Qt::SmoothTransformation);
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

/// A Material mark mapped into `target` (in the painter's current 20-grid
/// space) and filled solid. Material Symbols live on the 960 grid with y running
/// -960..0, so the +960 shift lands them on 0..960 before the scale into the
/// target box. The same mapping the interface glyph renderer uses.
void drawMaterialInto(QPainter& painter, const QString& kind, const QRectF& target,
                      const QColor& color)
{
    const QPainterPath path = materialMarkPath(kind);
    if (path.isEmpty())
        return;
    const qreal grid = materialMarkGrid(kind);
    painter.save();
    painter.translate(target.topLeft());
    painter.scale(target.width() / grid, target.height() / grid);
    if (grid == 960.0)
        painter.translate(0, grid);
    painter.setPen(Qt::NoPen);
    painter.setBrush(color);
    painter.drawPath(path);
    painter.restore();
}

/// The same mapping, as a path the caller can clip to (the paper's body).
QPainterPath materialInto(const QString& kind, const QRectF& target)
{
    const QPainterPath path = materialMarkPath(kind);
    const qreal grid = materialMarkGrid(kind);
    QTransform transform;
    transform.translate(target.x(), target.y());
    transform.scale(target.width() / grid, target.height() / grid);
    if (grid == 960.0)
        transform.translate(0, grid);
    return transform.map(path);
}

/// The mark inside the page. One Material symbol per family, scaled into the
/// paper body; the family colour is what separates the text-bearing families,
/// exactly as the hand-drawn marks did. The caller clips to the paper.
void drawFamilyMark(QPainter& painter, const QString& family, const QColor& tint)
{
    QString kind;
    if (family == QLatin1String("code"))
        kind = QStringLiteral("famCode");
    else if (family == QLatin1String("image"))
        kind = QStringLiteral("famImage");
    else if (family == QLatin1String("video"))
        kind = QStringLiteral("famVideo");
    else if (family == QLatin1String("audio"))
        kind = QStringLiteral("famAudio");
    else if (family == QLatin1String("archive"))
        kind = QStringLiteral("famArchive");
    else if (family == QLatin1String("spreadsheet"))
        kind = QStringLiteral("famSpreadsheet");
    else if (family == QLatin1String("font"))
        kind = QStringLiteral("famFont");
    else if (family == QLatin1String("executable"))
        kind = QStringLiteral("famExecutable");
    else if (family == QLatin1String("link"))
        kind = QStringLiteral("famLink");
    else if (family == QLatin1String("file") || family == QLatin1String("other")) {
        // A plain dot, the shape the instrument keeps for an unknown type.
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawEllipse(QPointF(10.0, 11.6), 1.3, 1.3);
        return;
    } else {
        // Text-bearing families: document, text, pdf, presentation. One lines
        // mark for all of them; the family colour tells them apart.
        kind = QStringLiteral("famText");
    }
    drawMaterialInto(painter, kind, QRectF(5.6, 6.6, 8.8, 8.8), tint);
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

    // The whole 20-grid box the Material page/folder is mapped into.
    const QRectF sheet(0, 0, 20, 20);

    if (family == QLatin1String("folder")) {
        // A silhouette: the family wash carries the shape, the Material outline
        // keeps the edge — the same treatment the interface folder gets.
        QColor wash = tint;
        wash.setAlphaF(tint.alphaF() * 0.3);
        drawMaterialInto(painter, QStringLiteral("fileFolderFill"), sheet, wash);
        drawMaterialInto(painter, QStringLiteral("fileFolder"), sheet, ink);
        painter.setPen(Qt::NoPen);
        painter.setBrush(tint);
        painter.drawRoundedRect(QRectF(3.0, 12.6, 7.0, 1.6), 0.8, 0.8);
        return image;
    }

    // The page: Material's `draft` outline, the one shape every file shares.
    drawMaterialInto(painter, QStringLiteral("filePage"), sheet, ink);

    painter.save();
    painter.setClipPath(materialInto(QStringLiteral("filePageFill"), sheet));
    drawFamilyMark(painter, family, tint);
    painter.restore();

    // §6.3.2: at 20 dp and above the page also carries a 2 device-pixel family
    // bar along its bottom edge. The bar is a size signal, not a second mark,
    // so it is absent from the 18 dp listing glyph.
    if (edge >= 20) {
        QPen bar(tint);
        bar.setWidthF(2.0 / scale);
        bar.setCapStyle(Qt::RoundCap);
        painter.setPen(bar);
        painter.drawLine(QPointF(5.2, 17.6), QPointF(14.8, 17.6));
    }
    return image;
}

} // namespace

FileGlyphImageProvider::FileGlyphImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
    m_cache.setMaxCost(64);
}

namespace {

/// The raster counter from proposal §12.7: every (family, pixel edge, ink,
/// tint) is rasterised once, and the first raster of a cold start stays under
/// 8 ms. `DAE_GLYPH_PERF=1` prints one line per actual rasterisation — a cache
/// hit prints nothing — so the claim is checkable without a profiler. The
/// counter itself is always maintained and is what the sequence number in the
/// line reports.
const bool kFileGlyphPerf = qEnvironmentVariableIsSet("DAE_GLYPH_PERF");
std::atomic<quint64> fileGlyphRasters{ 0 };

} // namespace

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

    QElapsedTimer timer;
    timer.start();
    const QImage image = renderFileGlyph(edge, family, ink, tint);
    const qint64 micros = timer.nsecsElapsed() / 1000;
    const quint64 raster = fileGlyphRasters.fetch_add(1) + 1;
    if (kFileGlyphPerf) {
        fprintf(stderr,
                "[fileglyph] raster #%llu %s edge=%d ink=%s fc=%s %.2f ms\n",
                static_cast<unsigned long long>(raster), qUtf8Printable(family),
                edge, qUtf8Printable(ink.name()), qUtf8Printable(tint.name()),
                double(micros) / 1000.0);
    }
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

GlyphImageProvider::GlyphImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
    m_cache.setMaxCost(128);
}

namespace {

/// The interface glyph counter, on the same terms as the file glyph one: one
/// line per actual rasterisation when `DAE_GLYPH_PERF=1`, nothing on a hit.
const bool kGlyphPerf = qEnvironmentVariableIsSet("DAE_GLYPH_PERF");
std::atomic<quint64> glyphRasters{ 0 };

} // namespace

QImage GlyphImageProvider::requestImage(const QString& id, QSize* size,
                                        const QSize& requestedSize)
{
    constexpr int kMinEdge = 8;
    constexpr int kMaxEdge = 128;

    const int queryStart = id.indexOf(QLatin1Char('?'));
    QString kind = queryStart >= 0 ? id.left(queryStart) : id;
    while (kind.startsWith(QLatin1Char('/')))
        kind.remove(0, 1);
    const QUrlQuery query(queryStart >= 0 ? id.mid(queryStart + 1) : QString());

    int edge = requestedSize.isValid()
        ? qMax(requestedSize.width(), requestedSize.height())
        : 0;
    if (edge <= 0)
        edge = query.queryItemValue(QStringLiteral("size")).toInt();
    edge = qBound(kMinEdge, edge, kMaxEdge);

    // The dp size the item is drawn at, not `edge / dpr`: the stroke
    // correction is defined against it (§6.3.1).
    const int dp = qBound(kMinEdge,
                          query.queryItemValue(QStringLiteral("dp")).toInt(),
                          kMaxEdge);
    const QColor ink = glyphColor(query, QStringLiteral("ink"),
                                  QColor(0x6b, 0x6d, 0x72));
    const qreal wash =
        qBound(0.0, query.queryItemValue(QStringLiteral("wash")).toDouble(), 1.0);
    const QString key = QStringLiteral("%1|%2|%3|%4|%5")
                            .arg(kind)
                            .arg(edge)
                            .arg(dp)
                            .arg(ink.name(QColor::HexArgb))
                            .arg(wash, 0, 'f', 2);

    {
        QMutexLocker locker(&m_mutex);
        if (const QImage* cached = m_cache.object(key)) {
            if (size)
                *size = cached->size();
            return *cached;
        }
    }

    QElapsedTimer timer;
    timer.start();
    const QImage image = renderMaterialMark(kind, edge, dp, ink, wash);
    const qint64 micros = timer.nsecsElapsed() / 1000;
    const quint64 raster = glyphRasters.fetch_add(1) + 1;
    if (kGlyphPerf) {
        fprintf(stderr,
                "[glyph] raster #%llu %s edge=%d dp=%d ink=%s wash=%.2f %.2f ms\n",
                static_cast<unsigned long long>(raster), qUtf8Printable(kind), edge,
                dp, qUtf8Printable(ink.name(QColor::HexArgb)), wash,
                double(micros) / 1000.0);
    }
    {
        QMutexLocker locker(&m_mutex);
        m_cache.insert(key, new QImage(image));
    }
    if (size)
        *size = image.size();
    return image;
}

void dae_qt_register_glyph_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("glyph"),
                                 new GlyphImageProvider());
}

void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("thumbnail"),
                                 new ThumbnailImageProvider());
}

} // namespace dae_qt
