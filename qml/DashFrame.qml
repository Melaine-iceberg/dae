import QtQuick
import QtQuick.Shapes

// A dashed rule around a rectangle: the frame that marks a drop target.
//
// `Rectangle` has no border style, and the alternative — a `Canvas` per row —
// rasterises one texture per visible cell. A stroked `Shape` is the cheap way
// to draw an edge that follows the item's size: the geometry is rebuilt only
// when the path changes, and the dash pattern is expressed in pen widths, so
// it stays proportional when the frame is stretched across a wide row.
//
// The frame is inset by half the stroke so it lands on the device grid instead
// of straddling the item's edge, the same correction the tab chip's border
// makes in `drag.cpp`.
Shape {
    id: root

    /// The rule's ink. Drop targets take the signal colour, like every other
    /// "you can act on this" mark in the shell.
    property color color: Theme.signal
    /// Stroke weight, in device-independent pixels.
    property real weight: Theme.edgeWidth
    /// Dash then gap, in multiples of `weight`: 4/3 is a dash you can read at
    /// 1 px without the edge turning into a dotted line.
    property var pattern: [4, 3]

    ShapePath {
        strokeColor: root.color
        strokeWidth: root.weight
        strokeStyle: ShapePath.DashLine
        dashPattern: root.pattern
        capStyle: ShapePath.FlatCap
        fillColor: "transparent"

        startX: root.weight / 2
        startY: root.weight / 2
        PathLine { x: root.width - root.weight / 2; y: root.weight / 2 }
        PathLine { x: root.width - root.weight / 2; y: root.height - root.weight / 2 }
        PathLine { x: root.weight / 2; y: root.height - root.weight / 2 }
        PathLine { x: root.weight / 2; y: root.weight / 2 }
    }
}
