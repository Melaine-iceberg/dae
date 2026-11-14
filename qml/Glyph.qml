import QtQuick
import "SolarMarks.js" as Solar

// A drawn mark, keyed by what it means.
//
// The geometry is Solar Icons' linear set — `SolarMarks.js` carries the path
// data, its licence and the six marks that are still dae's own. This file is
// only the renderer, and it keeps two rules the hand-drawn sprite established:
//
//  * The stroke is held at 1.2 device pixels rather than 1.2 *units*. The shell
//    draws marks at 10, 12 and 14 px as well as 16, and a uniform unit stroke
//    shrinks to 0.75 px at 10 — every edge lands between pixels and the mark
//    reads as grey haze instead of a line. Below 16 px the geometry is scaled
//    up in stroke only, which is the usual optical correction for a small icon.
//  * A silhouette (`s` in the mark table) is filled at `wash` and stroked, so a
//    folder reads as a shape rather than a block.
//
// Canvas rather than rectangles because a roof line and a search handle are
// diagonals, and because Mica/size changes repaint one item instead of
// re-uploading textures. It paints once per size or tint change, and the marks
// are shared by the surface that shows them — a hover retints one row's icon,
// not a page.
Item {
    id: root

    /// Navigation: "home", "desktop", "documents", "downloads", "pictures",
    /// "music", "videos", "search", "list", "grid", "column", "more", "branch",
    /// "panel", "terminal".
    /// Sidebar destinations: "drive", "network", "cloud", "trash", "diamond",
    /// "star", "clock", "plus", "gear".
    /// Files: "folder", "file", "image", "text", "code", "archive", "exe",
    /// "media", "link".
    /// Controls: "chevron" (and its left/up/down forms), "check", "close",
    /// "refresh", "eye", "info", "copy", "cut", "paste", "pencil", "external",
    /// "undo", "sort", "filter", "split", "extract", "arrowLeft", "arrowRight",
    /// "arrowUp", "pause", "play", "windowMin", "windowMax", "windowClose",
    /// "windowFullScreen", "windowExitFullScreen".
    /// Anything else draws nothing and the caller falls back to its label.
    property string kind: "home"
    property color tint: Theme.inkSecondary
    /// Strength of a silhouette's fill, as a fraction of `tint`. At 0.3 the
    /// folder keeps its outline and reads as a shape rather than a block.
    property real wash: 0.3

    implicitWidth: 16
    implicitHeight: 16

    readonly property bool draws: Solar.marks[kind] !== undefined

    Canvas {
        id: canvas
        anchors.fill: parent
        visible: root.draws
        renderStrategy: Canvas.Immediate

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()

            // Solar's grid is 24 units; every mark is drawn in it and scaled to
            // the item, so one table serves 10 px menu marks and 38 px tiles.
            const unit = width / 24
            if (unit <= 0)
                return
            ctx.scale(unit, unit)
            ctx.strokeStyle = root.tint
            ctx.fillStyle = root.tint
            ctx.lineCap = "round"
            ctx.lineJoin = "round"

            // Device pixels at 16 dp and below, proportionally heavier above.
            const boost = Math.max(1, width / 16)
            const parts = Solar.marks[root.kind] || []
            for (let i = 0; i < parts.length; i += 1) {
                const part = parts[i]
                ctx.beginPath()
                trace(ctx, part.d)
                if (part.f) {
                    ctx.fill()
                } else {
                    if (part.s) {
                        // A silhouette: the wash carries the shape, the stroke
                        // keeps the edge. Filled at full ink the folder turns
                        // into a slab that out-weighs the row's text.
                        ctx.globalAlpha = root.wash
                        ctx.fill()
                        ctx.globalAlpha = 1
                    }
                    // `lineWidth` is in grid units, so the device-pixel rule is
                    // divided back out here rather than baked into the table.
                    ctx.lineWidth = (part.w || 1.2) * boost / unit
                    ctx.stroke()
                }
            }
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()

        /// One SVG path, driven through this canvas.
        ///
        /// The marks are library path data now, so the whole grammar has to be
        /// read: `A` arcs (every rounded corner), the relative forms, and the
        /// implicit repetition that lets one command carry many parameter sets.
        /// It stays here instead of pulling in a parser because a mark is
        /// parsed once per paint — a size or tint change — not per frame.
        function trace(ctx, d) {
            const tokens = d.match(/[A-Za-z]|[-+]?(?:\d*\.\d+|\d+)(?:[eE][-+]?\d+)?/g)
            if (!tokens)
                return

            let i = 0
            let command = ""
            let x = 0, y = 0                 // current point
            let startX = 0, startY = 0       // where the subpath began
            let controlX = 0, controlY = 0   // last control point, for S and T
            let previous = ""

            const next = () => Number(tokens[i++])
            const absolute = (relative, value, origin) => relative ? origin + value : value

            while (i < tokens.length) {
                if (/[A-Za-z]/.test(tokens[i])) {
                    command = tokens[i]
                    i += 1
                } else if (command === "") {
                    return  // parameters without a command: nothing sane to draw
                }

                const upper = command.toUpperCase()
                const relative = command !== upper

                switch (upper) {
                case "Z":
                    ctx.closePath()
                    x = startX
                    y = startY
                    command = ""
                    break
                case "M": {
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    startX = x
                    startY = y
                    ctx.moveTo(x, y)
                    // Further pairs after a moveto are linetos, per the grammar.
                    command = relative ? "l" : "L"
                    break
                }
                case "L":
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    ctx.lineTo(x, y)
                    break
                case "H":
                    x = absolute(relative, next(), x)
                    ctx.lineTo(x, y)
                    break
                case "V":
                    y = absolute(relative, next(), y)
                    ctx.lineTo(x, y)
                    break
                case "C": {
                    const x1 = absolute(relative, next(), x)
                    const y1 = absolute(relative, next(), y)
                    const x2 = absolute(relative, next(), x)
                    const y2 = absolute(relative, next(), y)
                    controlX = x2
                    controlY = y2
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    ctx.bezierCurveTo(x1, y1, x2, y2, x, y)
                    break
                }
                case "S": {
                    // The first control point mirrors the previous curve's last
                    // one; with no curve behind it, it collapses onto `x`.
                    const mirrored = /[CS]/.test(previous.toUpperCase())
                    const x1 = mirrored ? 2 * x - controlX : x
                    const y1 = mirrored ? 2 * y - controlY : y
                    const x2 = absolute(relative, next(), x)
                    const y2 = absolute(relative, next(), y)
                    controlX = x2
                    controlY = y2
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    ctx.bezierCurveTo(x1, y1, x2, y2, x, y)
                    break
                }
                case "Q": {
                    const x1 = absolute(relative, next(), x)
                    const y1 = absolute(relative, next(), y)
                    controlX = x1
                    controlY = y1
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    ctx.quadraticCurveTo(x1, y1, x, y)
                    break
                }
                case "T": {
                    const mirrored = /[QT]/.test(previous.toUpperCase())
                    const x1 = mirrored ? 2 * x - controlX : x
                    const y1 = mirrored ? 2 * y - controlY : y
                    controlX = x1
                    controlY = y1
                    x = absolute(relative, next(), x)
                    y = absolute(relative, next(), y)
                    ctx.quadraticCurveTo(x1, y1, x, y)
                    break
                }
                case "A": {
                    const rx = next()
                    const ry = next()
                    const rotation = next()
                    const largeArc = next() !== 0
                    const sweep = next() !== 0
                    const toX = absolute(relative, next(), x)
                    const toY = absolute(relative, next(), y)
                    arc(ctx, x, y, rx, ry, rotation, largeArc, sweep, toX, toY)
                    x = toX
                    y = toY
                    break
                }
                default:
                    return  // an unknown command would spin the loop
                }
                previous = command
            }
        }

        /// SVG's endpoint arc, as the beziers the canvas can draw.
        ///
        /// Endpoint-to-centre conversion is the spec's own (F.6.5); the sweep
        /// is then cut into quarters and each quarter approximated by a cubic,
        /// which is within a thousandth of a pixel at every size this shell
        /// draws. Rounded corners arrive here — every Solar mark is built from
        /// them.
        function arc(ctx, fromX, fromY, rx, ry, rotation, largeArc, sweep, toX, toY) {
            if (rx === 0 || ry === 0) {
                ctx.lineTo(toX, toY)
                return
            }
            rx = Math.abs(rx)
            ry = Math.abs(ry)
            const phi = rotation * Math.PI / 180
            const cosPhi = Math.cos(phi)
            const sinPhi = Math.sin(phi)

            const dx = (fromX - toX) / 2
            const dy = (fromY - toY) / 2
            let x1 = cosPhi * dx + sinPhi * dy
            let y1 = -sinPhi * dx + cosPhi * dy

            // A radius too small for the chord is scaled up, per the spec.
            const scale = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
            if (scale > 1) {
                const grow = Math.sqrt(scale)
                rx *= grow
                ry *= grow
            }

            const numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
            const denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1
            const factor = (largeArc === sweep ? -1 : 1)
                * Math.sqrt(Math.max(0, numerator / denominator))
            const centerX1 = factor * rx * y1 / ry
            const centerY1 = -factor * ry * x1 / rx
            const centerX = cosPhi * centerX1 - sinPhi * centerY1 + (fromX + toX) / 2
            const centerY = sinPhi * centerX1 + cosPhi * centerY1 + (fromY + toY) / 2

            const angle = (ux, uy, vx, vy) => {
                const dot = ux * vx + uy * vy
                const length = Math.sqrt((ux * ux + uy * uy) * (vx * vx + vy * vy))
                const value = Math.acos(Math.max(-1, Math.min(1, dot / length)))
                return ux * vy - uy * vx < 0 ? -value : value
            }

            const ux = (x1 - centerX1) / rx
            const uy = (y1 - centerY1) / ry
            const start = angle(1, 0, ux, uy)
            let sweepAngle = angle(ux, uy, (-x1 - centerX1) / rx, (-y1 - centerY1) / ry)
            if (!sweep && sweepAngle > 0)
                sweepAngle -= 2 * Math.PI
            else if (sweep && sweepAngle < 0)
                sweepAngle += 2 * Math.PI

            const steps = Math.max(1, Math.ceil(Math.abs(sweepAngle) / (Math.PI / 2)))
            const step = sweepAngle / steps
            const handle = 4 / 3 * Math.tan(step / 4)
            let theta = start
            for (let s = 0; s < steps; s += 1) {
                const next = theta + step
                const cos1 = Math.cos(theta), sin1 = Math.sin(theta)
                const cos2 = Math.cos(next), sin2 = Math.sin(next)
                const endX = centerX + rx * cos2 * cosPhi - ry * sin2 * sinPhi
                const endY = centerY + rx * cos2 * sinPhi + ry * sin2 * cosPhi
                ctx.bezierCurveTo(
                    centerX + rx * cos1 * cosPhi - ry * sin1 * sinPhi
                        + handle * (-rx * sin1 * cosPhi - ry * cos1 * sinPhi),
                    centerY + rx * cos1 * sinPhi + ry * sin1 * cosPhi
                        + handle * (-rx * sin1 * sinPhi + ry * cos1 * cosPhi),
                    endX - handle * (-rx * sin2 * cosPhi - ry * cos2 * sinPhi),
                    endY - handle * (-rx * sin2 * sinPhi + ry * cos2 * cosPhi),
                    endX, endY)
                theta = next
            }
        }
    }

    Connections {
        target: root
        function onTintChanged() { canvas.requestPaint() }
        function onKindChanged() { canvas.requestPaint() }
    }
}
