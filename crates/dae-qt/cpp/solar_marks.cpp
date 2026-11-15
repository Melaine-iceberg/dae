#include "solar_marks.h"

#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QRegularExpression>
#include <QStringList>
#include <QtMath>

namespace dae_qt {

namespace {

/// Solar's grid. Every path in the table is drawn in a 24-unit square and
/// scaled to the item, so one table serves an 8 px menu mark and a 38 px tile.
constexpr qreal kGrid = 24.0;

/// Where the mark table lives inside the binary. `apps/dae/build.rs` mounts
/// `qml/solar-marks.json` under this prefix.
constexpr auto kMarkResource = ":/qt/qml/dae/qml/solar-marks.json";

/// The parsed table. Read once, on whichever image-loading thread asks first;
/// a function-local static is initialised exactly once and the `QJsonObject`
/// is read-only afterwards, so no lock is needed.
const QJsonObject& markTable()
{
    static const QJsonObject table = [] {
        QFile file(QString::fromUtf8(kMarkResource));
        if (!file.open(QIODevice::ReadOnly)) {
            qWarning("solar marks: cannot open %s", kMarkResource);
            return QJsonObject();
        }
        return QJsonDocument::fromJson(file.readAll()).object();
    }();
    return table;
}

/// SVG's endpoint arc, as the cubics `QPainterPath` can hold.
///
/// Endpoint-to-centre conversion is the spec's own (F.6.5); the sweep is then
/// cut into quarters and each quarter approximated by a cubic, which is within
/// a thousandth of a pixel at every size the shell draws. Rounded corners
/// arrive here — every Solar mark is built from them.
void appendArc(QPainterPath& path, qreal fromX, qreal fromY, qreal rx, qreal ry,
               qreal rotation, bool largeArc, bool sweep, qreal toX, qreal toY)
{
    if (rx == 0 || ry == 0) {
        path.lineTo(toX, toY);
        return;
    }
    rx = qAbs(rx);
    ry = qAbs(ry);

    const qreal phi = qDegreesToRadians(rotation);
    const qreal cosPhi = qCos(phi);
    const qreal sinPhi = qSin(phi);

    const qreal dx = (fromX - toX) / 2;
    const qreal dy = (fromY - toY) / 2;
    const qreal x1 = cosPhi * dx + sinPhi * dy;
    const qreal y1 = -sinPhi * dx + cosPhi * dy;

    // A radius too small for the chord is scaled up, per the spec.
    const qreal scale = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if (scale > 1) {
        const qreal grow = qSqrt(scale);
        rx *= grow;
        ry *= grow;
    }

    const qreal numerator =
        rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    const qreal denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    const qreal factor = (largeArc == sweep ? -1 : 1)
        * qSqrt(qMax<qreal>(0, numerator / denominator));
    const qreal centerX1 = factor * rx * y1 / ry;
    const qreal centerY1 = -factor * ry * x1 / rx;
    const qreal centerX = cosPhi * centerX1 - sinPhi * centerY1 + (fromX + toX) / 2;
    const qreal centerY = sinPhi * centerX1 + cosPhi * centerY1 + (fromY + toY) / 2;

    const auto angle = [](qreal ux, qreal uy, qreal vx, qreal vy) {
        const qreal dot = ux * vx + uy * vy;
        const qreal length = qSqrt((ux * ux + uy * uy) * (vx * vx + vy * vy));
        const qreal value = qAcos(qBound<qreal>(-1, dot / length, 1));
        return ux * vy - uy * vx < 0 ? -value : value;
    };

    const qreal ux = (x1 - centerX1) / rx;
    const qreal uy = (y1 - centerY1) / ry;
    const qreal start = angle(1, 0, ux, uy);
    qreal sweepAngle = angle(ux, uy, (-x1 - centerX1) / rx, (-y1 - centerY1) / ry);
    if (!sweep && sweepAngle > 0)
        sweepAngle -= 2 * M_PI;
    else if (sweep && sweepAngle < 0)
        sweepAngle += 2 * M_PI;

    const int steps = qMax(1, int(qCeil(qAbs(sweepAngle) / (M_PI / 2))));
    const qreal step = sweepAngle / steps;
    const qreal handle = 4.0 / 3.0 * qTan(step / 4);
    qreal theta = start;
    for (int index = 0; index < steps; ++index) {
        const qreal next = theta + step;
        const qreal cos1 = qCos(theta);
        const qreal sin1 = qSin(theta);
        const qreal cos2 = qCos(next);
        const qreal sin2 = qSin(next);
        const qreal endX = centerX + rx * cos2 * cosPhi - ry * sin2 * sinPhi;
        const qreal endY = centerY + rx * cos2 * sinPhi + ry * sin2 * cosPhi;
        path.cubicTo(centerX + rx * cos1 * cosPhi - ry * sin1 * sinPhi
                         + handle * (-rx * sin1 * cosPhi - ry * cos1 * sinPhi),
                     centerY + rx * cos1 * sinPhi + ry * sin1 * cosPhi
                         + handle * (-rx * sin1 * sinPhi + ry * cos1 * cosPhi),
                     endX - handle * (-rx * sin2 * cosPhi - ry * cos2 * sinPhi),
                     endY - handle * (-rx * sin2 * sinPhi + ry * cos2 * sinPhi),
                     endX, endY);
        theta = next;
    }
}

/// One SVG path string, as a `QPainterPath`.
///
/// The whole grammar has to be read: `A` arcs (every rounded corner), the
/// relative forms, and the implicit repetition that lets one command carry
/// many parameter sets. A mark is parsed once per raster, not per frame, so a
/// hand-written tokenizer beats pulling in a parser dependency.
QPainterPath svgPath(const QString& data)
{
    static const QRegularExpression tokenPattern(QStringLiteral(
        "[A-Za-z]|[-+]?(?:\\d*\\.\\d+|\\d+)(?:[eE][-+]?\\d+)?"));

    QPainterPath path;

    QStringList tokens;
    for (auto it = tokenPattern.globalMatch(data); it.hasNext();)
        tokens.append(it.next().captured());
    if (tokens.isEmpty())
        return path;

    int index = 0;
    QString command;
    QString previous;
    qreal x = 0, y = 0;               // current point
    qreal startX = 0, startY = 0;     // where the subpath began
    qreal controlX = 0, controlY = 0; // last control point, for S and T

    const auto number = [&] { return tokens.at(index++).toDouble(); };
    const auto absolute = [](bool relative, qreal value, qreal origin) {
        return relative ? origin + value : value;
    };

    while (index < tokens.size()) {
        const QString token = tokens.at(index);
        // A command is a single letter. `1e5` is one number token that
        // contains an `e`, so the test is anchored rather than "contains a
        // letter".
        if (token.size() == 1 && token.at(0).isLetter()) {
            command = token;
            ++index;
        } else if (command.isEmpty()) {
            break; // parameters without a command: nothing sane to draw
        }

        const QString upper = command.toUpper();
        const bool relative = command != upper;

        if (upper == QLatin1String("Z")) {
            path.closeSubpath();
            x = startX;
            y = startY;
            command.clear();
        } else if (upper == QLatin1String("M")) {
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            startX = x;
            startY = y;
            path.moveTo(x, y);
            // Further pairs after a moveto are linetos, per the grammar.
            command = relative ? QStringLiteral("l") : QStringLiteral("L");
        } else if (upper == QLatin1String("L")) {
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            path.lineTo(x, y);
        } else if (upper == QLatin1String("H")) {
            x = absolute(relative, number(), x);
            path.lineTo(x, y);
        } else if (upper == QLatin1String("V")) {
            y = absolute(relative, number(), y);
            path.lineTo(x, y);
        } else if (upper == QLatin1String("C")) {
            const qreal x1 = absolute(relative, number(), x);
            const qreal y1 = absolute(relative, number(), y);
            const qreal x2 = absolute(relative, number(), x);
            const qreal y2 = absolute(relative, number(), y);
            controlX = x2;
            controlY = y2;
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            path.cubicTo(x1, y1, x2, y2, x, y);
        } else if (upper == QLatin1String("S")) {
            // The first control point mirrors the previous curve's last one;
            // with no curve behind it, it collapses onto the current point.
            const bool mirrored = previous.compare(QLatin1String("C"), Qt::CaseInsensitive) == 0
                || previous.compare(QLatin1String("S"), Qt::CaseInsensitive) == 0;
            const qreal x1 = mirrored ? 2 * x - controlX : x;
            const qreal y1 = mirrored ? 2 * y - controlY : y;
            const qreal x2 = absolute(relative, number(), x);
            const qreal y2 = absolute(relative, number(), y);
            controlX = x2;
            controlY = y2;
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            path.cubicTo(x1, y1, x2, y2, x, y);
        } else if (upper == QLatin1String("Q")) {
            const qreal x1 = absolute(relative, number(), x);
            const qreal y1 = absolute(relative, number(), y);
            controlX = x1;
            controlY = y1;
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            path.quadTo(x1, y1, x, y);
        } else if (upper == QLatin1String("T")) {
            const bool mirrored = previous.compare(QLatin1String("Q"), Qt::CaseInsensitive) == 0
                || previous.compare(QLatin1String("T"), Qt::CaseInsensitive) == 0;
            const qreal x1 = mirrored ? 2 * x - controlX : x;
            const qreal y1 = mirrored ? 2 * y - controlY : y;
            controlX = x1;
            controlY = y1;
            x = absolute(relative, number(), x);
            y = absolute(relative, number(), y);
            path.quadTo(x1, y1, x, y);
        } else if (upper == QLatin1String("A")) {
            const qreal rx = number();
            const qreal ry = number();
            const qreal rotation = number();
            const bool largeArc = number() != 0;
            const bool sweep = number() != 0;
            const qreal toX = absolute(relative, number(), x);
            const qreal toY = absolute(relative, number(), y);
            appendArc(path, x, y, rx, ry, rotation, largeArc, sweep, toX, toY);
            x = toX;
            y = toY;
        } else {
            break; // an unknown command would spin the loop
        }
        previous = command;
    }

    return path;
}

} // namespace

QImage renderSolarMark(const QString& kind, int edge, int dp, const QColor& ink,
                       qreal wash)
{
    QImage image(edge, edge, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);

    const QJsonArray parts = markTable().value(kind).toArray();
    if (parts.isEmpty())
        return image;

    // One grid unit in dp, so the pen can be stated in the units the optical
    // correction was designed in and still land on the device pixels the
    // painter writes.
    const qreal unit = qMax(1, dp) / kGrid;
    const qreal boost = qMax(1.0, qreal(qMax(1, dp)) / 16.0);

    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.scale(edge / kGrid, edge / kGrid);

    for (const QJsonValue& entry : parts) {
        const QJsonObject part = entry.toObject();
        const QPainterPath path = svgPath(part.value(QStringLiteral("d")).toString());
        if (path.isEmpty())
            continue;

        if (part.value(QStringLiteral("f")).toBool()) {
            painter.setPen(Qt::NoPen);
            painter.setBrush(ink);
            painter.drawPath(path);
            continue;
        }

        if (part.value(QStringLiteral("s")).toBool()) {
            // A silhouette: the wash carries the shape, the stroke keeps the
            // edge. Filled at full ink the folder turns into a slab that
            // out-weighs the row's text.
            QColor fill = ink;
            fill.setAlphaF(ink.alphaF() * wash);
            painter.setPen(Qt::NoPen);
            painter.setBrush(fill);
            painter.drawPath(path);
        }

        const qreal width =
            part.value(QStringLiteral("w")).toDouble(1.2) * boost / unit;
        QPen pen(ink);
        pen.setWidthF(width);
        pen.setCapStyle(Qt::RoundCap);
        pen.setJoinStyle(Qt::RoundJoin);
        painter.setPen(pen);
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(path);
    }

    return image;
}

} // namespace dae_qt
