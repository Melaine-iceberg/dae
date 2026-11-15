#include "spike.h"

#include <QQmlEngine>
#include <QtQml/qqml.h>

#include <QColor>

SpikeProvider::SpikeProvider()
    : QQuickImageProvider(QQuickImageProvider::Image)
{
}

QImage SpikeProvider::requestImage(const QString&, QSize* size, const QSize& requested)
{
    const int edge = requested.width() > 0 ? requested.width() : 4;
    if (size)
        *size = QSize(edge, edge);
    QImage image(edge, edge, QImage::Format_ARGB32);
    image.fill(QColor(0xff, 0x33, 0x66));
    return image;
}

SpikeItem::SpikeItem(QQuickItem* parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, false);
}

void spike_register_providers(void* engine)
{
    static_cast<QQmlEngine*>(engine)->addImageProvider(QStringLiteral("spike"), new SpikeProvider);
}

void spike_register_item()
{
    qmlRegisterType<SpikeItem>("qtbridge_spike.cpp", 1, 0, "SpikeItem");
}
