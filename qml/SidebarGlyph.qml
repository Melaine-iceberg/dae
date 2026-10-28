import QtQuick

// The 16 px mark inside a sidebar row.
//
// These seven destinations used to draw the first character of their own label
// in the tile — `主` next to `主目录`, `桌` next to `桌面`. At 16 px it does not
// read as a monogram, it reads as a rendering collision: two copies of the same
// character, one small and tinted, one normal. The tile keeps its shape and its
// tint; only the character is replaced by a drawn mark.
//
// Canvas rather than rectangles because a roof line and a play triangle are
// diagonals, and stacking 1 px rectangles to fake them is how an icon set turns
// into fifty lines nobody wants to edit. It paints once per size or tint
// change, and the sidebar has at most a few dozen rows.
Item {
    id: root

    /// "home", "desktop", "documents", "downloads", "pictures", "music",
    /// "videos". Anything else draws nothing and the caller falls back to its
    /// own monogram.
    property string kind: "home"
    property color tint: Theme.inkMuted

    implicitWidth: 16
    implicitHeight: 16

    readonly property bool draws: kind === "home" || kind === "desktop"
        || kind === "documents" || kind === "downloads" || kind === "pictures"
        || kind === "music" || kind === "videos"

    Canvas {
        anchors.fill: parent
        visible: root.draws
        renderStrategy: Canvas.Immediate

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            // Everything below is written in a 16-unit box, so the marks keep
            // their optical weight when the row scale changes.
            ctx.scale(width / 16, height / 16)
            ctx.strokeStyle = root.tint
            ctx.fillStyle = root.tint
            ctx.lineWidth = 1.2
            ctx.lineJoin = "round"
            ctx.lineCap = "round"

            switch (root.kind) {
            case "home":
                ctx.beginPath()
                ctx.moveTo(2.6, 8.1)
                ctx.lineTo(8, 3.4)
                ctx.lineTo(13.4, 8.1)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(4.5, 7.7)
                ctx.lineTo(4.5, 12.6)
                ctx.lineTo(11.5, 12.6)
                ctx.lineTo(11.5, 7.7)
                ctx.stroke()
                break
            case "desktop":
                ctx.beginPath()
                ctx.rect(2, 3.1, 12, 8.2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(8, 11.3)
                ctx.lineTo(8, 13.2)
                ctx.moveTo(5.6, 13.4)
                ctx.lineTo(10.4, 13.4)
                ctx.stroke()
                break
            case "documents":
                ctx.beginPath()
                ctx.rect(3.6, 1.8, 8.8, 12.4)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(5.6, 5.4)
                ctx.lineTo(10.4, 5.4)
                ctx.moveTo(5.6, 8)
                ctx.lineTo(10.4, 8)
                ctx.moveTo(5.6, 10.6)
                ctx.lineTo(8.8, 10.6)
                ctx.stroke()
                break
            case "downloads":
                ctx.beginPath()
                ctx.moveTo(8, 2.6)
                ctx.lineTo(8, 10.2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(4.9, 7.3)
                ctx.lineTo(8, 10.4)
                ctx.lineTo(11.1, 7.3)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(3.4, 12.8)
                ctx.lineTo(12.6, 12.8)
                ctx.stroke()
                break
            case "pictures":
                ctx.beginPath()
                ctx.rect(1.8, 3, 12.4, 10)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(3.6, 11)
                ctx.lineTo(6.8, 7)
                ctx.lineTo(9.2, 9.9)
                ctx.lineTo(10.7, 8.5)
                ctx.lineTo(12.4, 11)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(10.9, 5.6, 0.9, 0, Math.PI * 2)
                ctx.fill()
                break
            case "music":
                ctx.beginPath()
                ctx.moveTo(6, 11.2)
                ctx.lineTo(6, 4.6)
                ctx.lineTo(11.6, 3.4)
                ctx.lineTo(11.6, 10)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(4.8, 11.5, 1.3, 0, Math.PI * 2)
                ctx.fill()
                ctx.beginPath()
                ctx.arc(10.4, 10.3, 1.3, 0, Math.PI * 2)
                ctx.fill()
                break
            case "videos":
                ctx.beginPath()
                ctx.rect(1.8, 3.4, 12.4, 9.2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(6.8, 6.2)
                ctx.lineTo(10.4, 8)
                ctx.lineTo(6.8, 9.8)
                ctx.closePath()
                ctx.fill()
                break
            }
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
    }

    Connections {
        target: root
        function onTintChanged() { canvas.requestPaint() }
        function onKindChanged() { canvas.requestPaint() }
    }
}
