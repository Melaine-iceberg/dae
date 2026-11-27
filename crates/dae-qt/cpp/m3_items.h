// The two M3 Expressive indicators the shell cannot draw with plain QML:
// the wavy linear progress indicator and the seven-shape morphing loading
// indicator (m3.material.io/components/progress-indicators, /loading-indicator).
//
// Both are QQuickItems that build a QSGGeometryNode with per-vertex colour in
// `updatePaintNode`, so they render through the scene graph's RHI backend
// (Direct3D 11 on Windows) as one draw call each. No custom shader and no
// `.qsb` is involved: `QSGVertexColorMaterial` is a built-in material, which is
// why this path works on a Qt install without Qt Shader Tools. Edges are
// feathered with a one-pixel alpha ring rather than relying on MSAA, so the
// wave and the blob stay smooth at every device pixel ratio.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <QColor>
#include <QElapsedTimer>
#include <QPointer>
#include <QQuickItem>
#include <QVariantAnimation>

#include <QtQml/qqml.h>

namespace dae_qt {

/// The wavy linear progress indicator. Determinate: a sine-wave active track
/// (3 dp amplitude, 40 dp wavelength, 4 dp stroke) grows left to right, a 4 dp
/// gap, then the flat remaining track and a 4 dp stop dot. Indeterminate: a
/// fixed wave segment travels the track while the flat track stays behind it.
/// `wavy: false` or `reduceMotion: true` falls back to the flat M3 indicator.
class M3ProgressItem : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(qreal value READ value WRITE setValue NOTIFY valueChanged)
    Q_PROPERTY(bool indeterminate READ indeterminate WRITE setIndeterminate NOTIFY indeterminateChanged)
    Q_PROPERTY(bool wavy READ wavy WRITE setWavy NOTIFY wavyChanged)
    Q_PROPERTY(bool reduceMotion READ reduceMotion WRITE setReduceMotion NOTIFY reduceMotionChanged)
    Q_PROPERTY(qreal thickness READ thickness WRITE setThickness NOTIFY thicknessChanged)
    Q_PROPERTY(QColor activeColor READ activeColor WRITE setActiveColor NOTIFY activeColorChanged)
    Q_PROPERTY(QColor trackColor READ trackColor WRITE setTrackColor NOTIFY trackColorChanged)

public:
    explicit M3ProgressItem(QQuickItem *parent = nullptr);

    qreal value() const { return m_value; }
    void setValue(qreal value);
    bool indeterminate() const { return m_indeterminate; }
    void setIndeterminate(bool on);
    bool wavy() const { return m_wavy; }
    void setWavy(bool on);
    bool reduceMotion() const { return m_reduceMotion; }
    void setReduceMotion(bool on);
    qreal thickness() const { return m_thickness; }
    void setThickness(qreal thickness);
    QColor activeColor() const { return m_activeColor; }
    void setActiveColor(const QColor &color);
    QColor trackColor() const { return m_trackColor; }
    void setTrackColor(const QColor &color);

signals:
    void valueChanged();
    void indeterminateChanged();
    void wavyChanged();
    void reduceMotionChanged();
    void thicknessChanged();
    void activeColorChanged();
    void trackColorChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;
    void componentComplete() override;

private:
    void syncAnimation();

    qreal m_value = 0.0;
    bool m_indeterminate = false;
    bool m_wavy = true;
    bool m_reduceMotion = false;
    qreal m_thickness = 4.0;
    QColor m_activeColor = QColor("#6750a4");
    QColor m_trackColor = QColor("#e8def8");
    /// Loop clock in [0, 1); drives the travelling segment and the wave scroll.
    qreal m_clock = 0.0;
    QVariantAnimation m_anim;
};

/// The loading indicator: a single blob that morphs through seven Material
/// shapes on a loop while it rotates, speeding up mid-morph and overshooting
/// each silhouette with a spatial spring. Not a spinner. `reduceMotion` holds
/// one static shape; `containerColor` with alpha draws the contained variant.
class M3LoaderItem : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(QColor color READ color WRITE setColor NOTIFY colorChanged)
    Q_PROPERTY(QColor containerColor READ containerColor WRITE setContainerColor NOTIFY containerColorChanged)
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(bool reduceMotion READ reduceMotion WRITE setReduceMotion NOTIFY reduceMotionChanged)

public:
    explicit M3LoaderItem(QQuickItem *parent = nullptr);

    QColor color() const { return m_color; }
    void setColor(const QColor &color);
    QColor containerColor() const { return m_containerColor; }
    void setContainerColor(const QColor &color);
    bool running() const { return m_running; }
    void setRunning(bool on);
    bool reduceMotion() const { return m_reduceMotion; }
    void setReduceMotion(bool on);

signals:
    void colorChanged();
    void containerColorChanged();
    void runningChanged();
    void reduceMotionChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;
    void componentComplete() override;

private:
    void syncAnimation();

    QColor m_color = QColor("#6750a4");
    QColor m_containerColor = QColor(0, 0, 0, 0);
    bool m_running = true;
    bool m_reduceMotion = false;
    qreal m_clock = 0.0;
    QVariantAnimation m_anim;
};

/// Called from `dae_qt::init` before the QML engine loads, beside the terminal
/// item registration. The module is separate from `dae` for the same reason:
/// these are plain C++ QQuickItems, not cxx-qt objects.
///
/// ChromeTabShape: draws Chrome Desktop's connected active-tab silhouette
/// (rounded top corners + concave outward bottom fillets) on the RHI in a
/// single draw call with 1px feathered anti-aliasing.
class ChromeTabShape : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(QColor color READ color WRITE setColor NOTIFY colorChanged)
    Q_PROPERTY(QColor borderColor READ borderColor WRITE setBorderColor NOTIFY borderColorChanged)
    Q_PROPERTY(qreal borderWidth READ borderWidth WRITE setBorderWidth NOTIFY borderWidthChanged)
    Q_PROPERTY(qreal topRadius READ topRadius WRITE setTopRadius NOTIFY topRadiusChanged)
    Q_PROPERTY(qreal bottomRadius READ bottomRadius WRITE setBottomRadius NOTIFY bottomRadiusChanged)
    Q_PROPERTY(bool connected READ connected WRITE setConnected NOTIFY connectedChanged)

public:
    explicit ChromeTabShape(QQuickItem *parent = nullptr);

    QColor color() const { return m_color; }
    void setColor(const QColor &color);
    QColor borderColor() const { return m_borderColor; }
    void setBorderColor(const QColor &color);
    qreal borderWidth() const { return m_borderWidth; }
    void setBorderWidth(qreal width);
    qreal topRadius() const { return m_topRadius; }
    void setTopRadius(qreal radius);
    qreal bottomRadius() const { return m_bottomRadius; }
    void setBottomRadius(qreal radius);
    bool connected() const { return m_connected; }
    void setConnected(bool on);

signals:
    void colorChanged();
    void borderColorChanged();
    void borderWidthChanged();
    void topRadiusChanged();
    void bottomRadiusChanged();
    void connectedChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;

private:
    QColor m_color = QColor("#ffffff");
    QColor m_borderColor = QColor(0, 0, 0, 0);
    qreal m_borderWidth = 0.0;
    qreal m_topRadius = 10.0;
    qreal m_bottomRadius = 10.0;
    bool m_connected = true;
};

/// Pointer-originated Material 3 / Chrome ink ripple rendered on the RHI.
/// The expanding radial wave is analytically clamped to the item's rounded
/// rectangle so it never bleeds past pill or card corners and costs zero draw
/// calls at rest.
class InkRippleItem : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(QColor color READ color WRITE setColor NOTIFY colorChanged)
    Q_PROPERTY(qreal radius READ radius WRITE setRadius NOTIFY radiusChanged)
    Q_PROPERTY(bool reduceMotion READ reduceMotion WRITE setReduceMotion NOTIFY reduceMotionChanged)

public:
    explicit InkRippleItem(QQuickItem *parent = nullptr);

    QColor color() const { return m_color; }
    void setColor(const QColor &color);
    qreal radius() const { return m_radius; }
    void setRadius(qreal radius);
    bool reduceMotion() const { return m_reduceMotion; }
    void setReduceMotion(bool on);

    Q_INVOKABLE void burst(qreal x, qreal y);
    Q_INVOKABLE void release();
    Q_INVOKABLE void pulse();

signals:
    void colorChanged();
    void radiusChanged();
    void reduceMotionChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;

private:
    void stopAnimations();

    QColor m_color = QColor(0, 0, 0, 32);
    qreal m_radius = 6.0;
    bool m_reduceMotion = false;
    qreal m_originX = -1.0;
    qreal m_originY = -1.0;
    qreal m_progress = 0.0;
    qreal m_fade = 0.0;
    bool m_pressed = false;
    bool m_active = false;
    QVariantAnimation m_expandAnim;
    QVariantAnimation m_fadeAnim;
};

/// Chrome-style circular arc spinner rendered on the RHI with feathered stroke
/// and smooth variable-sweep animation.
class ChromeSpinnerItem : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(QColor color READ color WRITE setColor NOTIFY colorChanged)
    Q_PROPERTY(qreal strokeWidth READ strokeWidth WRITE setStrokeWidth NOTIFY strokeWidthChanged)
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(bool reduceMotion READ reduceMotion WRITE setReduceMotion NOTIFY reduceMotionChanged)

public:
    explicit ChromeSpinnerItem(QQuickItem *parent = nullptr);

    QColor color() const { return m_color; }
    void setColor(const QColor &color);
    qreal strokeWidth() const { return m_strokeWidth; }
    void setStrokeWidth(qreal width);
    bool running() const { return m_running; }
    void setRunning(bool on);
    bool reduceMotion() const { return m_reduceMotion; }
    void setReduceMotion(bool on);

signals:
    void colorChanged();
    void strokeWidthChanged();
    void runningChanged();
    void reduceMotionChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;
    void componentComplete() override;

private:
    void syncAnimation();

    QColor m_color = QColor("#007a7b");
    qreal m_strokeWidth = 1.8;
    bool m_running = true;
    bool m_reduceMotion = false;
    qreal m_clock = 0.0;
    QVariantAnimation m_anim;
};

inline void dae_qt_register_m3_items()
{
    qmlRegisterType<M3ProgressItem>("dae.m3", 1, 0, "ProgressIndicator");
    qmlRegisterType<M3LoaderItem>("dae.m3", 1, 0, "LoadingIndicator");
    qmlRegisterType<ChromeTabShape>("dae.m3", 1, 0, "ChromeTabShape");
    qmlRegisterType<InkRippleItem>("dae.m3", 1, 0, "InkRippleEffect");
    qmlRegisterType<ChromeSpinnerItem>("dae.m3", 1, 0, "ChromeSpinner");
}

} // namespace dae_qt