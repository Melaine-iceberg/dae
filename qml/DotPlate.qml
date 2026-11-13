import QtQuick

// The dotted plate behind the preview's card.
//
// drift sets the pane's illustration on a field of dots — the inside cover of a
// notebook rather than a tinted panel. It is drawn as a Canvas and not as a
// tiled image because there is no asset to ship: the pattern is one square per
// 12 dp, its size is the caller's, and a raster would blur at every scale
// factor the build runs at.
//
// The dots sit at 28 % ink, the weight the prototype uses. They are structure,
// not state, so they never take the signal colour.
Item {
    id: root

    /// Distance between dot centres.
    property int spacing: 12
    /// Edge length of one dot.
    property int dot: 2

    Canvas {
        id: canvas
        anchors.fill: parent

        onPaint: {
            const ctx = getContext("2d")
            ctx.setTransform(1, 0, 0, 1, 0, 0)
            ctx.clearRect(0, 0, width, height)
            ctx.fillStyle = Qt.rgba(Theme.ink.r, Theme.ink.g, Theme.ink.b, 0.28)
            const step = Math.max(4, root.spacing)
            for (let y = step / 2; y < height; y += step) {
                for (let x = step / 2; x < width; x += step)
                    ctx.fillRect(Math.round(x), Math.round(y), root.dot, root.dot)
            }
        }

        Connections {
            target: Theme
            function onDarkChanged() { canvas.requestPaint() }
        }
    }

    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
}
