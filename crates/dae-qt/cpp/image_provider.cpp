#include "image_provider.h"

#include "dae-qt/src/icons.cxxqt.h"

#include <QFileInfo>
#include <QImageReader>
#include <QMutexLocker>
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
        const rust::Vec<uint8_t> bytes =
            dae_qt_render_system_thumbnail(path.toStdString(), dimension);
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

void dae_qt_register_file_icon_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("fileicon"),
                                 new FileIconImageProvider());
}

void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("thumbnail"),
                                 new ThumbnailImageProvider());
}

} // namespace dae_qt
