import QtQuick

// A drawn 16 px mark, keyed by what it means.
//
// The sidebar's built-in destinations used to draw the first character of their
// own label in the 16 px tile — `主` next to `主目录`, `桌` next to `桌面`. At
// that size it does not read as a monogram, it reads as a rendering collision:
// two copies of the same character, one small and tinted, one normal.
//
// The shell also had three icon sources (this tile, unicode glyphs sprinkled
// through the views, and one Windows-only font). This file is where they
// converge: a mark is a path in a 16-unit box, nothing else. Callers tint it;
// the file itself has no colours, because an icon is structure and the one
// signal colour belongs to what can be acted on.
//
// Canvas rather than rectangles because a roof line and a play triangle are
// diagonals, and stacking 1 px rectangles to fake them is how an icon set turns
// into fifty lines nobody wants to edit. It paints once per size or tint
// change, and the sidebar has at most a few dozen rows.
Item {
    id: root

    /// Navigation: "home", "desktop", "documents", "downloads", "pictures",
    /// "music", "videos", "search", "list", "grid", "column", "more", "branch",
    /// "panel", "terminal".
    /// Sidebar destinations: "drive", "network", "cloud", "trash", "diamond",
    /// "star", "clock", "plus".
    /// Files: "folder", "file", "image", "code", "archive", "media", "link".
    /// Controls: "chevron", "check", "close".
    /// Anything else draws nothing and the caller falls back to its label.
    property string kind: "home"
    property color tint: Theme.inkMuted

    implicitWidth: 16
    implicitHeight: 16

    readonly property bool draws: kind === "home" || kind === "desktop"
        || kind === "documents" || kind === "downloads" || kind === "pictures"
        || kind === "music" || kind === "videos" || kind === "search"
        || kind === "list" || kind === "grid" || kind === "column"
        || kind === "more" || kind === "branch" || kind === "panel"
        || kind === "terminal" || kind === "folder" || kind === "file"
        || kind === "image" || kind === "code" || kind === "archive"
        || kind === "media" || kind === "drive" || kind === "network"
        || kind === "cloud" || kind === "trash" || kind === "diamond"
        || kind === "chevron" || kind === "check" || kind === "close"
        || kind === "star" || kind === "plus" || kind === "clock"
        || kind === "link"

    Canvas {
        id: canvas
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

            // The sheet of paper every file mark is built on: a folded corner,
            // not an envelope. Shared so the file, image, code and archive
            // marks use one silhouette.
            function paperOutline() {
                ctx.beginPath()
                ctx.moveTo(4.2, 2.6)
                ctx.lineTo(8.8, 2.6)
                ctx.lineTo(11.8, 6)
                ctx.lineTo(11.8, 13.4)
                ctx.lineTo(4.2, 13.4)
                ctx.closePath()
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(8.8, 2.6)
                ctx.lineTo(8.8, 6)
                ctx.lineTo(11.8, 6)
                ctx.stroke()
            }

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
                ctx.rect(2.4, 3.2, 11.2, 7.2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(8, 10.4)
                ctx.lineTo(8, 13)
                ctx.moveTo(5.6, 13)
                ctx.lineTo(10.4, 13)
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
            case "image":
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
            case "media":
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
            case "search":
                ctx.beginPath()
                ctx.arc(7, 7, 4.2, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(10.1, 10.1)
                ctx.lineTo(13.6, 13.6)
                ctx.stroke()
                break
            case "list":
                for (let row = 0; row < 3; row += 1) {
                    const y = 3.4 + row * 4.4
                    ctx.beginPath()
                    ctx.arc(3, y + 0.6, 0.9, 0, Math.PI * 2)
                    ctx.fill()
                    ctx.beginPath()
                    ctx.moveTo(6, y + 0.6)
                    ctx.lineTo(13.4, y + 0.6)
                    ctx.stroke()
                }
                break
            case "grid":
                for (let row = 0; row < 2; row += 1) {
                    for (let col = 0; col < 2; col += 1) {
                        ctx.beginPath()
                        ctx.rect(2.4 + col * 6.4, 2.4 + row * 6.4, 4.8, 4.8)
                        ctx.stroke()
                    }
                }
                break
            case "column":
                for (let col = 0; col < 3; col += 1) {
                    ctx.beginPath()
                    ctx.rect(1.9 + col * 4.4, 2.4, 3.4, 11.2)
                    ctx.stroke()
                }
                break
            case "more":
                for (let dot = 0; dot < 3; dot += 1) {
                    ctx.beginPath()
                    ctx.arc(3.4 + dot * 4.6, 8, 1.1, 0, Math.PI * 2)
                    ctx.fill()
                }
                break
            case "branch":
                ctx.beginPath()
                ctx.arc(4.4, 4.2, 1.7, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(4.4, 11.8, 1.7, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(11.6, 4.9, 1.7, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(4.4, 5.9)
                ctx.lineTo(4.4, 10.1)
                ctx.moveTo(11.6, 6.6)
                ctx.lineTo(11.6, 8.4)
                ctx.quadraticCurveTo(11.6, 10.4, 9.6, 10.4)
                ctx.lineTo(6.1, 10.4)
                ctx.stroke()
                break
            case "panel":
                ctx.beginPath()
                ctx.rect(2, 3, 12, 10)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(9.6, 3)
                ctx.lineTo(9.6, 13)
                ctx.stroke()
                break
            case "terminal":
                ctx.beginPath()
                ctx.rect(1.9, 3.1, 12.2, 9.8)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(4.5, 6.2)
                ctx.lineTo(6.6, 8)
                ctx.lineTo(4.5, 9.8)
                ctx.moveTo(8.2, 10)
                ctx.lineTo(11.2, 10)
                ctx.stroke()
                break
            case "folder":
                // One outline, the way Finder and Explorer both draw a folder:
                // a tab on the back plate, then the body.
                ctx.beginPath()
                ctx.moveTo(2.2, 5.2)
                ctx.quadraticCurveTo(2.2, 4.0, 3.2, 4.0)
                ctx.lineTo(6.9, 4.0)
                ctx.lineTo(8.2, 5.5)
                ctx.lineTo(12.8, 5.5)
                ctx.quadraticCurveTo(13.8, 5.5, 13.8, 6.6)
                ctx.lineTo(13.8, 12.3)
                ctx.quadraticCurveTo(13.8, 13.4, 12.8, 13.4)
                ctx.lineTo(3.2, 13.4)
                ctx.quadraticCurveTo(2.2, 13.4, 2.2, 12.3)
                ctx.closePath()
                ctx.stroke()
                break
            case "file":
            case "link":
                paperOutline()
                if (root.kind === "link") {
                    ctx.beginPath()
                    ctx.moveTo(6.4, 9.6)
                    ctx.lineTo(9.2, 6.8)
                    ctx.stroke()
                }
                break
            case "code":
                paperOutline()
                ctx.beginPath()
                ctx.moveTo(6.4, 8.1)
                ctx.lineTo(5.2, 9.4)
                ctx.lineTo(6.4, 10.7)
                ctx.moveTo(9.6, 8.1)
                ctx.lineTo(10.8, 9.4)
                ctx.lineTo(9.6, 10.7)
                ctx.stroke()
                break
            case "archive":
                paperOutline()
                ctx.beginPath()
                ctx.moveTo(6.6, 7.6)
                ctx.lineTo(8.6, 7.6)
                ctx.moveTo(6.6, 9.4)
                ctx.lineTo(8.6, 9.4)
                ctx.moveTo(6.6, 11.2)
                ctx.lineTo(8.6, 11.2)
                ctx.stroke()
                break
            case "drive":
                ctx.beginPath()
                ctx.rect(2.2, 4.8, 11.6, 6.4)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(4.4, 8.9)
                ctx.lineTo(7.4, 8.9)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(11.6, 8.9, 0.65, 0, Math.PI * 2)
                ctx.fill()
                break
            case "network":
                ctx.beginPath()
                ctx.arc(8, 3.4, 1.5, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(3.8, 12, 1.5, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.arc(12.2, 12, 1.5, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(7.2, 4.8)
                ctx.lineTo(4.5, 10.6)
                ctx.moveTo(8.8, 4.8)
                ctx.lineTo(11.5, 10.6)
                ctx.moveTo(5.3, 12)
                ctx.lineTo(10.7, 12)
                ctx.stroke()
                break
            case "cloud":
                ctx.beginPath()
                ctx.moveTo(4.8, 11.8)
                ctx.lineTo(11.2, 11.8)
                ctx.quadraticCurveTo(13.4, 11.8, 13.4, 9.6)
                ctx.quadraticCurveTo(13.4, 7.6, 11.4, 7.4)
                ctx.quadraticCurveTo(11.4, 4.4, 8.4, 4.4)
                ctx.quadraticCurveTo(6.2, 4.4, 5.4, 6.2)
                ctx.quadraticCurveTo(2.8, 6.4, 2.8, 8.9)
                ctx.quadraticCurveTo(2.8, 11.8, 4.8, 11.8)
                ctx.closePath()
                ctx.stroke()
                break
            case "trash":
                ctx.beginPath()
                ctx.moveTo(3.2, 4.8)
                ctx.lineTo(12.8, 4.8)
                ctx.moveTo(6, 4.8)
                ctx.lineTo(6, 3.2)
                ctx.lineTo(10, 3.2)
                ctx.lineTo(10, 4.8)
                ctx.moveTo(4.8, 4.8)
                ctx.lineTo(5.4, 13)
                ctx.lineTo(10.6, 13)
                ctx.lineTo(11.2, 4.8)
                ctx.moveTo(6.8, 7.2)
                ctx.lineTo(6.8, 10.8)
                ctx.moveTo(9.2, 7.2)
                ctx.lineTo(9.2, 10.8)
                ctx.stroke()
                break
            case "diamond":
                ctx.beginPath()
                ctx.moveTo(8, 2.6)
                ctx.lineTo(13.4, 8)
                ctx.lineTo(8, 13.4)
                ctx.lineTo(2.6, 8)
                ctx.closePath()
                ctx.stroke()
                break
            case "chevron":
                ctx.beginPath()
                ctx.moveTo(6, 3.5)
                ctx.lineTo(10.5, 8)
                ctx.lineTo(6, 12.5)
                ctx.stroke()
                break
            case "check":
                ctx.beginPath()
                ctx.moveTo(3, 8.3)
                ctx.lineTo(6.3, 11.6)
                ctx.lineTo(13, 4.8)
                ctx.stroke()
                break
            case "close":
                ctx.beginPath()
                ctx.moveTo(4.2, 4.2)
                ctx.lineTo(11.8, 11.8)
                ctx.moveTo(11.8, 4.2)
                ctx.lineTo(4.2, 11.8)
                ctx.stroke()
                break
            case "plus":
                ctx.beginPath()
                ctx.moveTo(8, 3.2)
                ctx.lineTo(8, 12.8)
                ctx.moveTo(3.2, 8)
                ctx.lineTo(12.8, 8)
                ctx.stroke()
                break
            case "clock":
                ctx.beginPath()
                ctx.arc(8, 8, 5.2, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(8, 5.2)
                ctx.lineTo(8, 8)
                ctx.lineTo(10, 9.2)
                ctx.stroke()
                break
            case "star":
                ctx.beginPath()
                for (let point = 0; point < 10; point += 1) {
                    const radius = point % 2 === 0 ? 5.2 : 2.3
                    const angle = -Math.PI / 2 + point * Math.PI / 5
                    const x = 8 + Math.cos(angle) * radius
                    const y = 8 + Math.sin(angle) * radius
                    if (point === 0)
                        ctx.moveTo(x, y)
                    else
                        ctx.lineTo(x, y)
                }
                ctx.closePath()
                ctx.stroke()
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
