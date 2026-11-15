// The three seams the Qt Bridge port has to keep.
//
// Every function here exists in the current shell as a `#[cxx_qt::bridge]`
// declaration. Qt Bridge has no per-type C++ glue, so these become plain
// `extern "C"` entry points called from Rust through a raw pointer:
//
//   * `spike_register_providers` — a `QQuickImageProvider`, which Qt Bridge
//     does not model. The engine comes from `QApp::qml_engine()`.
//   * `spike_register_item` — a `QQuickItem` subclass registered as a QML
//     type. The shell's terminal painter is this shape.
//   * `SpikeItem::answer` — a C++ property read from QML.

#pragma once

#include <QImage>
#include <QQuickImageProvider>
#include <QQuickItem>
#include <QString>

class SpikeProvider : public QQuickImageProvider
{
public:
    SpikeProvider();
    QImage requestImage(const QString& id, QSize* size, const QSize& requested) override;
};

class SpikeItem : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(int answer READ answer CONSTANT)
    Q_PROPERTY(QString label READ label CONSTANT)

public:
    explicit SpikeItem(QQuickItem* parent = nullptr);

    int answer() const { return 42; }
    QString label() const { return QStringLiteral("cpp-item"); }
};

extern "C" {
void spike_register_providers(void* engine);
void spike_register_item();
}
