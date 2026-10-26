#include "image_provider.h"

#include <QFileInfo>
#include <QImageReader>
#include <QMutexLocker>
#include <QUrl>
#include <QUrlQuery>
#include <QtQml/QQmlApplicationEngine>

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
    if (!info.isFile() || !info.isReadable() || info.size() > kMaxSourceBytes)
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

    QImageReader reader(path);
    reader.setAutoTransform(true);
    const QSize original = reader.size();
    if (original.isValid() && original.width() > 0 && original.height() > 0) {
        const QSize target = original.scaled(dimension, dimension, Qt::KeepAspectRatio);
        if (target.isValid())
            reader.setScaledSize(target);
    }

    QImage image = reader.read();
    if (image.isNull())
        return {};

    if (image.width() > dimension || image.height() > dimension) {
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

void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine)
{
    if (engine)
        engine->addImageProvider(QStringLiteral("thumbnail"),
                                 new ThumbnailImageProvider());
}

} // namespace dae_qt
