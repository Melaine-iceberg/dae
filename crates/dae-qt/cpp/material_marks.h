// The Material mark renderer (proposal §12.3).
//
// `qml/material-marks.json` carries the path data — Google's Material Symbols,
// plus the six marks that stay dae's own — and this file turns one entry into a
// raster. It exists so `Glyph.qml` can be a plain `Image`: a `Canvas` per icon
// is one raster buffer per row, and a delegate that owns a `Canvas` repaints
// geometry on the GUI thread every time a tint or a size changes.
//
// Material Symbols are authored on a 960-unit grid (`viewBox="0 -960 960 960"`);
// the path data is kept verbatim and mapped onto the item here. The six legacy
// marks (`_legacy24`) stay on the 24-unit grid. The stroke rule from the old
// canvas is kept because it is what makes the marks legible at 8–16 dp: the
// width is held at 1.2 *device* pixels up to 16 dp and scales above it, rather
// than shrinking with the grid.
//
// Like `image_provider.h`, a macro guard rather than `#pragma once`: cxx-qt
// copies headers into its generated include tree.

#ifndef DAE_QT_MATERIAL_MARKS_H
#define DAE_QT_MATERIAL_MARKS_H

#include <QColor>
#include <QImage>
#include <QPainterPath>
#include <QString>

namespace dae_qt {

/// `kind` rendered into a square `edge`-pixel image with a transparent
/// background.
///
/// `dp` is the size the item will be drawn at, not `edge / dpr`: the stroke
/// correction is optical and is defined against the dp size (`1.2` device
/// pixels at 16 dp and below, proportionally heavier above). `wash` is the
/// silhouette fill, as a fraction of `ink`.
QImage renderMaterialMark(const QString& kind, int edge, int dp, const QColor& ink,
                          qreal wash);

/// The parsed outline of a mark's first part, in its native grid coordinates
/// (960 for Material Symbols, 24 for the legacy marks). The file glyph
/// compositor draws these into the page; `materialMarkGrid` says which grid the
/// path lives on so the caller can scale it.
QPainterPath materialMarkPath(const QString& kind);
qreal materialMarkGrid(const QString& kind);

} // namespace dae_qt

#endif // DAE_QT_MATERIAL_MARKS_H