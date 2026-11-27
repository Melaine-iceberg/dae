// The preview thumbnail provider.
//
// QML asks for `image://thumbnail/<percent-encoded path>?size=...`. Decoding
// happens on Qt's image-loading thread, never the GUI thread, and results are
// cached by path + target size + the file's own revision. The revision in the
// URL (`v=...`) makes QML drop its copy when the file changes; the cache key
// re-reads mtime and size so a stale entry cannot be served even if a view
// keeps an old URL.
//
// A macro guard rather than `#pragma once`, for the same reason as
// `activation.h`: cxx-qt copies this header into the generated include tree and
// `image_provider.cpp` includes both that copy and the local one.

#ifndef DAE_QT_IMAGE_PROVIDER_H
#define DAE_QT_IMAGE_PROVIDER_H

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <QCache>
#include <QImage>
#include <QMutex>
#include <QQuickImageProvider>
#include <QString>
#include <cstddef>

class QQmlApplicationEngine;

namespace dae_qt {

class ThumbnailImageProvider : public QQuickImageProvider
{
public:
    ThumbnailImageProvider();

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override;

private:
    /// The remote half of `requestImage`: bounded bytes from the vfs layer,
    /// decoded from memory. Split out because the local half's `QFileInfo`
    /// guard and the revision sources differ.
    QImage remoteImage(const QString& id, const QString& path, int dimension,
                       QSize* size);

    QMutex m_mutex;
    QCache<QString, QImage> m_cache;
};

// The OS icon provider.
//
// QML asks for `image://fileicon/path/<percent-encoded path>?dir=...&v=...` for
// an entry with its own icon and `image://fileicon/type/<extension>` for one
// shared by its type. Extraction lives in `dae_core`; the cache lives in the
// Rust bridge, because both hits and misses are worth remembering across the
// image-loading threads Qt calls this from.
class FileIconImageProvider : public QQuickImageProvider
{
public:
    FileIconImageProvider();

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override;
};

// The built-in file glyph provider (proposal §12.3).
//
// QML asks for `image://fileglyph/<family>?size=<px>&ink=RRGGBB&fc=RRGGBB`.
// The 20-grid geometry lives here, and each (family, pixel edge, ink, family
// colour) pair is rasterised once and cached, so a listing row paints a plain
// `Image` instead of owning a `Canvas`. Theme changes land on a new URL and
// therefore a new cache entry; the old one ages out.
class FileGlyphImageProvider : public QQuickImageProvider
{
public:
    FileGlyphImageProvider();

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override;

private:
    QMutex m_mutex;
    QCache<QString, QImage> m_cache;
};

// The interface glyph provider (proposal §12.3).
//
// QML asks for `image://glyph/<kind>?size=<px>&dp=<dp>&ink=AARRGGBB&wash=0.3`.
// The Material path data lives in `qml/material-marks.json` and
// `material_marks.cpp` turns one entry into a raster; each (kind, pixel edge,
// dp, ink, wash) is painted once and cached, so a sidebar row, a menu item and
// a toolbar button share the same texture and no delegate owns a `Canvas`. Tint
// changes (hover, selection) land on a new URL and a new entry; theme-coloured
// tints are a small, fixed set, so the cache does not grow with the listing.
class GlyphImageProvider : public QQuickImageProvider
{
public:
    GlyphImageProvider();

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override;

private:
    QMutex m_mutex;
    QCache<QString, QImage> m_cache;
};

/// Takes ownership of the provider, as QQmlEngine::addImageProvider does.
void dae_qt_register_thumbnail_provider(QQmlApplicationEngine* engine);

/// Takes ownership, as above.
void dae_qt_register_file_icon_provider(QQmlApplicationEngine* engine);

/// Takes ownership, as above.
void dae_qt_register_file_glyph_provider(QQmlApplicationEngine* engine);

/// Takes ownership, as above.
void dae_qt_register_glyph_provider(QQmlApplicationEngine* engine);

void dae_qt_register_thumbnail_provider_ptr(std::size_t enginePtr);
void dae_qt_register_file_icon_provider_ptr(std::size_t enginePtr);
void dae_qt_register_file_glyph_provider_ptr(std::size_t enginePtr);
void dae_qt_register_glyph_provider_ptr(std::size_t enginePtr);

} // namespace dae_qt

#endif // DAE_QT_IMAGE_PROVIDER_H
