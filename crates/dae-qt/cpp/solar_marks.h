// The Solar mark renderer (proposal §12.3).
//
// `qml/solar-marks.json` carries the path data — Solar Icons' linear set, plus
// the six marks that stay dae's own — and this file turns one entry into a
// raster. It exists so `Glyph.qml` can be a plain `Image`: a `Canvas` per icon
// is one raster buffer per row, and a delegate that owns a `Canvas` repaints
// geometry on the GUI thread every time a tint or a size changes.
//
// The stroke rule from the old canvas is kept because it is what makes the
// marks legible at 8–16 dp: the width is held at 1.2 *device* pixels up to
// 16 dp and scales above it, rather than shrinking with the 24-unit grid.
//
// Like `image_provider.h`, a macro guard rather than `#pragma once`: cxx-qt
// copies headers into its generated include tree.

#ifndef DAE_QT_SOLAR_MARKS_H
#define DAE_QT_SOLAR_MARKS_H

#include <QColor>
#include <QImage>
#include <QString>

namespace dae_qt {

/// `kind` rendered into a square `edge`-pixel image with a transparent
/// background.
///
/// `dp` is the size the item will be drawn at, not `edge / dpr`: the stroke
/// correction is optical and is defined against the dp size (`1.2` device
/// pixels at 16 dp and below, proportionally heavier above). `wash` is the
/// silhouette fill, as a fraction of `ink`.
QImage renderSolarMark(const QString& kind, int edge, int dp, const QColor& ink,
                       qreal wash);

} // namespace dae_qt

#endif // DAE_QT_SOLAR_MARKS_H
