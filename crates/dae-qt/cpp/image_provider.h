// The preview thumbnail provider.
//
// QML asks for `image://thumbnail/<percent-encoded path>?size=...`. Decoding
// happens on Qt's image-loading thread, never the GUI thread, and results are
// cached by path + target size + the file's own revision. The revision in the
// URL (`v=...`) makes QML drop its copy when the file changes; the cache key
// re-reads mtime and size so a stale entry cannot be served even if a view
// keeps an old URL.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <QCache>
#include <QImage>
#include <QMutex>
#include <QQuickImageProvider>
#include <QString>

class QQmlApplicationEngine;

namespace dae_qt {

class ThumbnailImageProvider : public QQuickImageProvider
{
public:
    ThumbnailImageProvider();

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override;

private:
    QMutex m_mutex;
    QCache<QString, QImage> m_cache;
};

/// Takes ownership of the provider, as QQmlEngine::addImageProvider does.
void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine);

} // namespace dae_qt
