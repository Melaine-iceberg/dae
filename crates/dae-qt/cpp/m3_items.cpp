#include "m3_items.h"

#include <QPointF>
#include <QSGGeometry>
#include <QSGGeometryNode>
#include <QSGVertexColorMaterial>
#include <QVector>

#include <QtMath>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace dae_qt {

namespace {

constexpr qreal Feather = 1.0; // one logical pixel of alpha ramp at every edge
constexpr int DiscSegments = 28;
constexpr int BlobSegments = 96;

/// A tiny triangle mesh with per-vertex colour, uploaded once per repaint.
struct Mesh
{
    QVector<QSGGeometry::ColoredPoint2D> verts;
    QVector<quint16> idx;

    int vert(qreal x, qreal y, const QColor &c, qreal alpha = 1.0)
    {
        QSGGeometry::ColoredPoint2D v;
        v.x = float(x);
        v.y = float(y);
        v.r = uchar(c.red());
        v.g = uchar(c.green());
        v.b = uchar(c.blue());
        v.a = uchar(qBound(0.0, alpha * c.alphaF() * 255.0, 255.0));
        verts.append(v);
        return verts.size() - 1;
    }
    void tri(int a, int b, int c) { idx << quint16(a) << quint16(b) << quint16(c); }
    void quad(int a, int b, int c, int d) { tri(a, b, c); tri(a, c, d); }
};

QSGGeometryNode *nodeFromMesh(const Mesh &mesh)
{
    if (mesh.verts.isEmpty() || mesh.idx.isEmpty())
        return nullptr;
    auto *geometry = new QSGGeometry(QSGGeometry::defaultAttributes_ColoredPoint2D(),
                                     mesh.verts.size(), mesh.idx.size(),
                                     QSGGeometry::UnsignedShortType);
    geometry->setDrawingMode(QSGGeometry::DrawTriangles);
    std::memcpy(geometry->vertexDataAsColoredPoint2D(), mesh.verts.constData(),
                size_t(mesh.verts.size()) * sizeof(QSGGeometry::ColoredPoint2D));
    std::memcpy(geometry->indexDataAsUShort(), mesh.idx.constData(),
                size_t(mesh.idx.size()) * sizeof(quint16));
    auto *material = new QSGVertexColorMaterial;
    material->setFlag(QSGMaterial::Blending, true);
    auto *node = new QSGGeometryNode;
    node->setGeometry(geometry);
    node->setMaterial(material);
    node->setFlag(QSGNode::OwnsGeometry, true);
    node->setFlag(QSGNode::OwnsMaterial, true);
    return node;
}

/// A filled disc with a feathered rim: a solid inner fan plus a one-pixel
/// alpha ring, so the edge reads smooth without MSAA.
void featheredDisc(Mesh &mesh, qreal cx, qreal cy, qreal radius, const QColor &color)
{
    if (radius <= 0)
        return;
    const qreal inner = qMax(qreal(0), radius - Feather);
    const int center = mesh.vert(cx, cy, color);
    QVector<int> inRing, outRing;
    for (int i = 0; i < DiscSegments; ++i) {
        const qreal a = 2 * M_PI * i / DiscSegments;
        const qreal ca = std::cos(a), sa = std::sin(a);
        inRing << mesh.vert(cx + inner * ca, cy + inner * sa, color);
        outRing << mesh.vert(cx + radius * ca, cy + radius * sa, color, 0.0);
    }
    for (int i = 0; i < DiscSegments; ++i) {
        const int j = (i + 1) % DiscSegments;
        mesh.tri(center, inRing[i], inRing[j]);
        mesh.quad(inRing[i], outRing[i], outRing[j], inRing[j]);
    }
}

/// A constant-width stroke along a centreline, extruded along the local normal
/// and feathered on both sides, with a round cap (a disc) at each end.
void featheredStroke(Mesh &mesh, const QVector<QPointF> &line, qreal width,
                     const QColor &color)
{
    const int n = line.size();
    if (n < 2 || width <= 0)
        return;
    const qreal half = width / 2;
    const qreal inner = qMax(qreal(0), half - Feather);
    QVector<int> lin, rin, lout, rout;
    for (int i = 0; i < n; ++i) {
        QPointF tangent;
        if (i == 0)
            tangent = line[1] - line[0];
        else if (i == n - 1)
            tangent = line[n - 1] - line[n - 2];
        else
            tangent = line[i + 1] - line[i - 1];
        const qreal len = std::hypot(tangent.x(), tangent.y());
        QPointF normal = len > 1e-6 ? QPointF(-tangent.y() / len, tangent.x() / len)
                                    : QPointF(0, 1);
        const QPointF &p = line[i];
        lin << mesh.vert(p.x() + normal.x() * inner, p.y() + normal.y() * inner, color);
        rin << mesh.vert(p.x() - normal.x() * inner, p.y() - normal.y() * inner, color);
        lout << mesh.vert(p.x() + normal.x() * half, p.y() + normal.y() * half, color, 0.0);
        rout << mesh.vert(p.x() - normal.x() * half, p.y() - normal.y() * half, color, 0.0);
    }
    for (int i = 0; i < n - 1; ++i) {
        mesh.quad(lin[i], rin[i], rin[i + 1], lin[i + 1]);
        mesh.quad(lout[i], lin[i], lin[i + 1], lout[i + 1]);
        mesh.quad(rin[i], rout[i], rout[i + 1], rin[i + 1]);
    }
    featheredDisc(mesh, line.first().x(), line.first().y(), half, color);
    featheredDisc(mesh, line.last().x(), line.last().y(), half, color);
}

qreal easeOutBack(qreal t)
{
    constexpr qreal c1 = 1.70158;
    constexpr qreal c3 = c1 + 1;
    const qreal u = t - 1;
    return 1 + c3 * u * u * u + c1 * u * u;
}

qreal superellipse(qreal theta, qreal a, qreal b, qreal n)
{
    const qreal ct = std::cos(theta) / a;
    const qreal st = std::sin(theta) / b;
    return std::pow(std::pow(std::fabs(ct), n) + std::pow(std::fabs(st), n), -1.0 / n);
}

/// The seven silhouettes of the loading indicator, as polar radius functions
/// sampled at the same angles so any two can be interpolated point for point.
qreal loaderShapeRadius(int shape, qreal theta)
{
    switch (shape) {
    case 0: return 1.0 + 0.15 * std::cos(12 * theta);          // soft burst
    case 1: return 1.0 + 0.11 * std::cos(9 * theta);           // 9-sided cookie
    case 2: {                                                  // pentagon
        const qreal seg = 2 * M_PI / 5;
        const qreal a = std::fmod(theta + seg / 2, seg) - seg / 2;
        const qreal rp = std::cos(M_PI / 5) / std::cos(a);
        return 0.86 * rp + 0.14;
    }
    case 3: return superellipse(theta, 1.22, 0.82, 4.0);       // pill
    case 4: return 1.0 + 0.20 * std::cos(8 * theta);           // sunny
    case 5: return 1.0 + 0.16 * std::cos(4 * theta);           // 4-sided cookie
    default: return superellipse(theta, 1.12, 0.86, 2.0);      // oval
    }
}

} // namespace

// ── M3ProgressItem ──────────────────────────────────────────────────────────

M3ProgressItem::M3ProgressItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_anim.setStartValue(0.0);
    m_anim.setEndValue(1.0);
    m_anim.setDuration(1400);
    m_anim.setLoopCount(-1);
    m_anim.setEasingCurve(QEasingCurve::Linear);
    connect(&m_anim, &QVariantAnimation::valueChanged, this, [this](const QVariant &v) {
        m_clock = v.toReal();
        update();
    });
}

void M3ProgressItem::setValue(qreal value)
{
    value = qBound(qreal(0), value, qreal(1));
    if (qFuzzyCompare(m_value, value))
        return;
    m_value = value;
    emit valueChanged();
    update();
}

void M3ProgressItem::setIndeterminate(bool on)
{
    if (m_indeterminate == on)
        return;
    m_indeterminate = on;
    emit indeterminateChanged();
    syncAnimation();
    update();
}

void M3ProgressItem::setWavy(bool on)
{
    if (m_wavy == on)
        return;
    m_wavy = on;
    emit wavyChanged();
    syncAnimation();
    update();
}

void M3ProgressItem::setReduceMotion(bool on)
{
    if (m_reduceMotion == on)
        return;
    m_reduceMotion = on;
    emit reduceMotionChanged();
    syncAnimation();
    update();
}

void M3ProgressItem::setThickness(qreal thickness)
{
    thickness = qMax(qreal(1), thickness);
    if (qFuzzyCompare(m_thickness, thickness))
        return;
    m_thickness = thickness;
    emit thicknessChanged();
    update();
}

void M3ProgressItem::setActiveColor(const QColor &color)
{
    if (m_activeColor == color)
        return;
    m_activeColor = color;
    emit activeColorChanged();
    update();
}

void M3ProgressItem::setTrackColor(const QColor &color)
{
    if (m_trackColor == color)
        return;
    m_trackColor = color;
    emit trackColorChanged();
    update();
}

void M3ProgressItem::itemChange(ItemChange change, const ItemChangeData &data)
{
    QQuickItem::itemChange(change, data);
    if (change == ItemVisibleHasChanged) {
        syncAnimation();
        update();
    }
}

void M3ProgressItem::componentComplete()
{
    QQuickItem::componentComplete();
    syncAnimation();
    update();
}

void M3ProgressItem::syncAnimation()
{
    // Only the indeterminate wave travels; a determinate bar redraws on value
    // changes alone, so a steady transfer costs no frames.
    const bool animate = isVisible() && m_indeterminate && !m_reduceMotion;
    if (animate && m_anim.state() != QAbstractAnimation::Running)
        m_anim.start();
    else if (!animate && m_anim.state() == QAbstractAnimation::Running)
        m_anim.stop();
}

QSGNode *M3ProgressItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    const qreal w = width();
    const qreal h = height();
    if (w <= 0 || h <= 0 || !isVisible()) {
        delete oldNode;
        return nullptr;
    }

    Mesh mesh;
    const qreal mid = h / 2;
    const qreal stroke = qMin(m_thickness, h);
    const bool wavy = m_wavy && !m_reduceMotion && h >= stroke + 4;
    const qreal amplitude = wavy ? qMin(qreal(3), (h - stroke) / 2 - Feather) : 0;
    const qreal wavelength = 40;
    const qreal gap = 4;
    const qreal dotRadius = 2;

    auto waveLine = [&](qreal x0, qreal x1, qreal phase) {
        QVector<QPointF> line;
        if (x1 <= x0)
            return line;
        const qreal step = 2;
        for (qreal x = x0; x < x1; x += step) {
            const qreal y = mid + amplitude * std::sin(2 * M_PI * (x - phase) / wavelength);
            line << QPointF(x, y);
        }
        const qreal y = mid + amplitude * std::sin(2 * M_PI * (x1 - phase) / wavelength);
        line << QPointF(x1, y);
        return line;
    };

    if (m_indeterminate) {
        // The flat track stays behind a travelling, pulsing wave segment.
        featheredStroke(mesh, {QPointF(0, mid), QPointF(w, mid)}, stroke, m_trackColor);
        const qreal segLen = qMax(qreal(40), w * 0.35);
        qreal start, length;
        if (m_reduceMotion) {
            length = segLen;
            start = (w - length) / 2;
        } else {
            const qreal pulse = 0.6 + 0.4 * std::sin(M_PI * m_clock);
            length = segLen * pulse;
            start = -length + (w + 2 * length) * m_clock;
        }
        const qreal x0 = qMax(qreal(0), start);
        const qreal x1 = qMin(w, start + length);
        const qreal phase = m_reduceMotion ? 0 : m_clock * wavelength * 3;
        featheredStroke(mesh, waveLine(x0, x1, phase), stroke, m_activeColor);
    } else {
        const qreal usable = qMax(qreal(0), w - (wavy ? 8 : 0));
        const qreal activeEnd = m_value * usable;
        const qreal phase = 0;
        if (wavy) {
            featheredStroke(mesh, waveLine(0, activeEnd, phase), stroke, m_activeColor);
            if (m_value < 0.999) {
                const qreal trackStart = qMin(activeEnd + gap, usable);
                featheredStroke(mesh, {QPointF(trackStart, mid), QPointF(usable, mid)},
                                stroke, m_trackColor);
                featheredDisc(mesh, w - dotRadius - 1, mid, dotRadius, m_activeColor);
            }
        } else {
            // Flat fallback: a rounded active bar over the full track.
            featheredStroke(mesh, {QPointF(0, mid), QPointF(w, mid)}, stroke, m_trackColor);
            if (activeEnd > 0)
                featheredStroke(mesh, {QPointF(0, mid), QPointF(activeEnd, mid)},
                                stroke, m_activeColor);
        }
    }

    QSGGeometryNode *node = nodeFromMesh(mesh);
    delete oldNode;
    return node;
}

// ── M3LoaderItem ────────────────────────────────────────────────────────────

M3LoaderItem::M3LoaderItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_anim.setStartValue(0.0);
    m_anim.setEndValue(1.0);
    m_anim.setDuration(4200);
    m_anim.setLoopCount(-1);
    m_anim.setEasingCurve(QEasingCurve::Linear);
    connect(&m_anim, &QVariantAnimation::valueChanged, this, [this](const QVariant &v) {
        m_clock = v.toReal();
        update();
    });
}

void M3LoaderItem::setColor(const QColor &color)
{
    if (m_color == color)
        return;
    m_color = color;
    emit colorChanged();
    update();
}

void M3LoaderItem::setContainerColor(const QColor &color)
{
    if (m_containerColor == color)
        return;
    m_containerColor = color;
    emit containerColorChanged();
    update();
}

void M3LoaderItem::setRunning(bool on)
{
    if (m_running == on)
        return;
    m_running = on;
    emit runningChanged();
    syncAnimation();
    update();
}

void M3LoaderItem::setReduceMotion(bool on)
{
    if (m_reduceMotion == on)
        return;
    m_reduceMotion = on;
    emit reduceMotionChanged();
    syncAnimation();
    update();
}

void M3LoaderItem::itemChange(ItemChange change, const ItemChangeData &data)
{
    QQuickItem::itemChange(change, data);
    if (change == ItemVisibleHasChanged) {
        syncAnimation();
        update();
    }
}

void M3LoaderItem::componentComplete()
{
    QQuickItem::componentComplete();
    syncAnimation();
    update();
}

void M3LoaderItem::syncAnimation()
{
    const bool animate = isVisible() && m_running && !m_reduceMotion;
    if (animate && m_anim.state() != QAbstractAnimation::Running)
        m_anim.start();
    else if (!animate && m_anim.state() == QAbstractAnimation::Running)
        m_anim.stop();
}

QSGNode *M3LoaderItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    const qreal w = width();
    const qreal h = height();
    if (w <= 0 || h <= 0 || !isVisible()) {
        delete oldNode;
        return nullptr;
    }

    Mesh mesh;
    const qreal cx = w / 2;
    const qreal cy = h / 2;
    const qreal full = qMin(w, h) / 2 - Feather;
    const bool contained = m_containerColor.alpha() > 0;
    if (contained)
        featheredDisc(mesh, cx, cy, full, m_containerColor);
    const qreal baseRadius = contained ? full * 0.62 : full;

    constexpr qreal loopSec = 4.2;
    constexpr qreal slotSec = loopSec / 7;
    constexpr qreal morphSec = slotSec * 0.7;
    constexpr qreal baseSpeed = 1.1;
    constexpr qreal boost = 2.6;

    int from = 0, to = 0;
    qreal eased = 0, angle = 0;
    if (m_running && !m_reduceMotion) {
        const qreal pos = m_clock * 7;
        from = int(std::floor(pos)) % 7;
        to = (from + 1) % 7;
        const qreal local = pos - std::floor(pos);
        const qreal mt = qBound(qreal(0), local / 0.7, qreal(1));
        eased = easeOutBack(mt);
        const qreal elapsed = m_clock * loopSec;
        const qreal completed = from * (2.0 / M_PI) * morphSec;
        const qreal current = (1 - std::cos(M_PI * mt)) / M_PI * morphSec;
        angle = baseSpeed * elapsed + boost * (completed + current);
    }

    QVector<int> inRing, outRing;
    const int center = mesh.vert(cx, cy, m_color);
    for (int i = 0; i < BlobSegments; ++i) {
        const qreal theta = 2 * M_PI * i / BlobSegments;
        const qreal r = baseRadius * ((1 - eased) * loaderShapeRadius(from, theta)
                                      + eased * loaderShapeRadius(to, theta));
        const qreal inner = qMax(qreal(0), r - Feather);
        const qreal a = theta + angle;
        const qreal ca = std::cos(a), sa = std::sin(a);
        inRing << mesh.vert(cx + inner * ca, cy + inner * sa, m_color);
        outRing << mesh.vert(cx + r * ca, cy + r * sa, m_color, 0.0);
    }
    for (int i = 0; i < BlobSegments; ++i) {
        const int j = (i + 1) % BlobSegments;
        mesh.tri(center, inRing[i], inRing[j]);
        mesh.quad(inRing[i], outRing[i], outRing[j], inRing[j]);
    }

    QSGGeometryNode *node = nodeFromMesh(mesh);
    delete oldNode;
    return node;
}

} // namespace dae_qt