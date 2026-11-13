import QtQuick

// The paper sheet a listing row or grid tile is drawn on.
//
// drift has no icon set: a file is a piece of printed stock — a card with a
// folded corner, the extension set in mono on a black tab, and the content
// sketched inside (ruled lines for text, a ruled grid for a sheet, waveform
// bars for audio, a still frame for video). This reimplements that drawing in
// Canvas so one component serves the 76 dp grid tile, the 18 dp list row and
// the 104 dp plate in the preview pane.
//
// It replaces the category glyph on every surface that shows a file: the grid,
// the detail list, the column view, the conflict dialog, the preview's folder
// rows, the breadcrumbs and the media preview's empty state. `Glyph` stays for
// chrome that is not a file — chevrons, close marks, the bin, menu arrows;
// a sheet is not a glyph: it has a body, a fold and a tab, and at 18 dp those
// three would collapse into one black shape, which is why the small sizes draw
// the silhouette instead.
//
// The drawing is authored in the prototype's own coordinate space (90 × 120
// for a sheet, 120 × 96 for a folder) and scaled on paint, so every size is
// the same drawing rather than a second set of proportions.
Item {
    id: root

    /// The model's category: folder, link, image, video, audio, document,
    /// text, code, archive, font, executable, other, file.
    property string category: "file"
    /// The suffix shown on the tab, without the dot. Empty draws no tab.
    property string extension: ""
    /// Directories draw the manila folder instead of the card.
    property bool isDir: false
    /// How many entries a folder holds, set on the folder's face. Negative
    /// draws no number, which is the honest mark when the listing does not know
    /// the count — a model row carries no child count, and "00" on a folder
    /// that holds six files is a lie the prototype's mock data never had to
    /// tell.
    property int count: -1
    /// False draws the silhouette only — the flat mark a dense list needs.
    property bool detail: true
    /// The file name the sheet is drawn for. The drawing needs a number that
    /// picks the image hues and the audio waveform; deriving it here from the
    /// whole name is what makes two photographs side by side two colours —
    /// a hash of the length and the first character gave every `Windows-…jpg`
    /// in a folder the same red.
    property string name: ""
    readonly property int seed: {
        let hash = 7
        for (let index = 0; index < name.length; index += 1)
            hash = (hash * 31 + name.charCodeAt(index)) >>> 0
        return hash
    }
    /// Callers that want the sheet to react to selection (the preview plate)
    /// set this; the ink stroke then follows the signal colour.
    property bool accented: false
    /// Draw the silhouette in paper instead of ink. A list row selects by
    /// inverting to an ink band, and an ink sheet on that band is invisible.
    property bool inverted: false
    /// Paint a hard ink shadow 5 dp down and right of the sheet. drift casts
    /// one under the card on the preview plate; there is no blur, the shadow
    /// is the sheet's own outline in ink, displaced.
    property bool shadowed: false

    readonly property bool folder: isDir || category === "folder"
    /// A row in a listing is 28–40 dp tall, which leaves the sheet about 25 dp
    /// wide. Below 40 the tab and the hairline rules cannot be drawn — the tab
    /// would be two pixels of type — so the same drawing carries its content as
    /// three chunky bars instead of six thin ones, and drops the tab. It is
    /// still the same card, the same fold and the same colours.
    readonly property bool compact: width < 40
    implicitWidth: 90
    implicitHeight: folder ? Math.round(width * 96 / 120) : Math.round(width * 120 / 90)

    Canvas {
        id: canvas
        anchors.fill: parent

        onPaint: {
            const ctx = getContext("2d")
            ctx.setTransform(1, 0, 0, 1, 0, 0)
            const theme = Theme
            const ink = root.accented ? theme.signal : theme.ink
            const paper = theme.canvasAlt
            const edge = Math.max(1, root.width / 90 * 1.6)

            ctx.save()
            if (root.folder) {
                ctx.scale(root.width / 120, root.height / 96)
                root.paintFolder(ctx, theme, ink, edge)
            } else {
                ctx.scale(root.width / 90, root.height / 120)
                root.paintSheet(ctx, theme, ink, paper, edge)
            }
            ctx.restore()
        }

        Connections {
            target: root
            function onCategoryChanged() { canvas.requestPaint() }
            function onExtensionChanged() { canvas.requestPaint() }
            function onCountChanged() { canvas.requestPaint() }
            function onDetailChanged() { canvas.requestPaint() }
            function onSeedChanged() { canvas.requestPaint() }
            function onAccentedChanged() { canvas.requestPaint() }
            function onInvertedChanged() { canvas.requestPaint() }
            function onShadowedChanged() { canvas.requestPaint() }
        }
        Connections {
            target: Theme
            function onDarkChanged() { canvas.requestPaint() }
        }
    }

    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()

    function stroke(ctx, ink, edge) {
        ctx.strokeStyle = ink
        ctx.lineWidth = edge
        ctx.lineJoin = "round"
    }

    /// The sheet's own outline in ink, displaced. Drawn before the body so the
    /// body covers the part of the shadow it sits on.
    function paintShadow(ctx, theme) {
        ctx.save()
        ctx.translate(5, 5)
        ctx.fillStyle = theme.ink
        ctx.beginPath()
        if (root.folder) {
            ctx.moveTo(7, 20)
            ctx.lineTo(113, 20)
            ctx.lineTo(115, 22)
            ctx.lineTo(115, 89)
            ctx.lineTo(113, 91)
            ctx.lineTo(7, 91)
            ctx.lineTo(5, 89)
            ctx.lineTo(5, 22)
        } else {
            ctx.moveTo(6, 4)
            ctx.lineTo(62, 4)
            ctx.lineTo(84, 26)
            ctx.lineTo(84, 116)
            ctx.lineTo(6, 116)
        }
        ctx.closePath()
        ctx.fill()
        ctx.restore()
    }

    /// 120 × 96: tab behind, body in front, the entry count on the face.
    function paintFolder(ctx, theme, ink, edge) {
        if (root.shadowed)
            paintShadow(ctx, theme)

        ctx.fillStyle = theme.folderTab
        ctx.beginPath()
        ctx.rect(5, 7, 46, 20)
        ctx.fill()
        stroke(ctx, ink, edge)
        ctx.stroke()

        ctx.fillStyle = root.inverted ? theme.canvas
            : root.detail ? theme.folderTab : theme.folderBody
        ctx.beginPath()
        ctx.moveTo(7, 20)
        ctx.lineTo(113, 20)
        ctx.quadraticCurveTo(115, 20, 115, 22)
        ctx.lineTo(115, 89)
        ctx.quadraticCurveTo(115, 91, 113, 91)
        ctx.lineTo(7, 91)
        ctx.quadraticCurveTo(5, 91, 5, 89)
        ctx.lineTo(5, 22)
        ctx.quadraticCurveTo(5, 20, 7, 20)
        ctx.closePath()
        ctx.fill()
        ctx.stroke()

        if (!root.detail || root.count < 0)
            return
        ctx.fillStyle = theme.ink
        ctx.font = "700 12px " + theme.monoFamily
        ctx.textAlign = "left"
        ctx.textBaseline = "alphabetic"
        ctx.fillText(String(root.count).padStart(2, "0"), 14, 80)
    }

    /// 90 × 120: body, fold, content, extension tab.
    function paintSheet(ctx, theme, ink, paper, edge) {
        if (root.shadowed)
            paintShadow(ctx, theme)

        ctx.save()
        ctx.beginPath()
        ctx.moveTo(6, 4)
        ctx.lineTo(62, 4)
        ctx.lineTo(84, 26)
        ctx.lineTo(84, 116)
        ctx.lineTo(6, 116)
        ctx.closePath()
        ctx.clip()
        /// The card's own stock. Without this the content is drawn straight
        /// onto whatever is behind the sheet — a document with a red bar and
        /// four rules over an ink shadow reads as a black card.
        ctx.fillStyle = paper
        ctx.fillRect(0, 0, 90, 120)
        if (root.detail)
            paintContent(ctx, theme, ink, paper)
        else {
            ctx.fillStyle = root.inverted ? theme.canvas : silhouette(theme)
            ctx.fillRect(0, 0, 90, 120)
        }
        ctx.restore()

        ctx.fillStyle = paper
        ctx.beginPath()
        ctx.moveTo(62, 4)
        ctx.lineTo(84, 26)
        ctx.lineTo(62, 26)
        ctx.closePath()
        ctx.fill()

        stroke(ctx, ink, edge)
        ctx.beginPath()
        ctx.moveTo(6, 4)
        ctx.lineTo(62, 4)
        ctx.lineTo(84, 26)
        ctx.lineTo(84, 116)
        ctx.lineTo(6, 116)
        ctx.closePath()
        ctx.stroke()
        ctx.beginPath()
        ctx.moveTo(62, 4)
        ctx.lineTo(62, 26)
        ctx.lineTo(84, 26)
        ctx.stroke()

        if (!root.detail || root.extension === "" || root.compact)
            return
        const tab = String(root.extension).slice(0, 4).toUpperCase()
        const width = 12 + tab.length * 7
        ctx.fillStyle = theme.ink
        ctx.fillRect(6, 100, width, 16)
        ctx.fillStyle = theme.canvas
        ctx.font = "700 10px " + theme.monoFamily
        ctx.textAlign = "left"
        ctx.fillText(tab, 11, 112)
    }

    function silhouette(theme) {
        switch (root.category) {
        case "image": return hsl(theme, root.seed % 360, 75, 58)
        case "video":
        case "code": return theme.ink
        case "archive": return theme.folderTab
        case "executable": return theme.folderBody
        default: return theme.canvasAlt
        }
    }

    function paintContent(ctx, theme, ink, paper) {
        /// `thickness` exists for the compact sheet: at 25 dp wide a 3-unit bar
        /// is one device pixel, which reads as a smudge rather than a line.
        const rule = function (y, width, colour, alpha, thickness) {
            ctx.globalAlpha = alpha === undefined ? 0.75 : alpha
            ctx.fillStyle = colour || ink
            ctx.fillRect(14, y, width, thickness === undefined ? 3 : thickness)
            ctx.globalAlpha = 1
        }

        switch (root.category) {
        case "image": {
            const a = root.seed % 360
            const b = (a + 70 + (root.seed % 50)) % 360
            ctx.fillStyle = hsl(theme, a, 75, 58)
            ctx.fillRect(0, 0, 90, 120)
            ctx.fillStyle = hsl(theme, b, 85, 62)
            ctx.beginPath()
            ctx.arc(20 + (root.seed % 40), 40, 46, 0, Math.PI * 2)
            ctx.fill()
            ctx.fillStyle = hsl(theme, (a + 200) % 360, 70, 40)
            ctx.beginPath()
            ctx.arc(70 - (root.seed % 30), 90, 50, 0, Math.PI * 2)
            ctx.fill()
            ctx.fillStyle = hsl(theme, (a + 30) % 360, 95, 70)
            ctx.globalAlpha = 0.85
            ctx.beginPath()
            ctx.arc(55, 60, 26, 0, Math.PI * 2)
            ctx.fill()
            ctx.globalAlpha = 1
            break
        }
        case "document":
            ctx.fillStyle = theme.signal
            ctx.fillRect(14, 34, 26, 6)
            if (root.compact) {
                ;[50, 64, 78].forEach(function (y, i) {
                    rule(y, [58, 50, 58][i], ink, 0.75, 6)
                })
            } else {
                ;[48, 58, 68, 78].forEach(function (y, i) {
                    rule(y, [58, 52, 58, 40][i])
                })
            }
            break
        case "text":
            if (root.compact) {
                ;[48, 64, 80].forEach(function (y, i) {
                    rule(y, [58, 46, 58][i], ink, 0.55, 6)
                })
            } else {
                ;[44, 52, 60, 68, 76, 84].forEach(function (y, i) {
                    rule(y, [58, 52, 58, 40, 56, 30][i], ink, 0.55)
                })
            }
            break
        case "code":
            ctx.fillStyle = theme.ink
            ctx.fillRect(0, 0, 90, 120)
            ;[38, 48, 58, 68, 78, 88].forEach(function (y, i) {
                ctx.fillStyle = i % 2 ? "#7be0a0" : theme.canvas
                ctx.globalAlpha = 0.9
                ctx.fillRect(14 + (i % 3) * 6, y, [30, 44, 24, 38, 18, 34][i], 3)
                ctx.globalAlpha = 1
            })
            break
        case "audio":
            for (let i = 0; i < 13; i++) {
                const v = 12 + ((root.seed >> i) & 7) * 6 + ((i * 17) % 14)
                ctx.fillStyle = i % 4 === 0 ? theme.signal : theme.ink
                ctx.fillRect(13 + i * 5, 66 - v / 2, 3, v)
            }
            break
        case "video":
            ctx.fillStyle = theme.ink
            ctx.fillRect(0, 0, 90, 120)
            ctx.fillStyle = theme.canvas
            ctx.beginPath()
            ctx.moveTo(34, 46)
            ctx.lineTo(58, 60)
            ctx.lineTo(34, 74)
            ctx.closePath()
            ctx.fill()
            ctx.globalAlpha = 0.3
            ctx.fillRect(14, 88, 62, 2)
            ctx.globalAlpha = 1
            ctx.fillStyle = theme.signal
            ctx.fillRect(14, 88, 20 + (root.seed % 30), 2)
            break
        case "archive":
            ctx.fillStyle = theme.folderTab
            ctx.fillRect(0, 0, 90, 120)
            ctx.fillStyle = theme.ink
            for (let i = 0; i < 9; i++)
                ctx.fillRect(i % 2 ? 41 : 47, 30 + i * 8, 8, 5)
            stroke(ctx, theme.ink, 2)
            ctx.strokeRect(38, 82, 22, 14)
            break
        case "executable":
            ctx.fillStyle = theme.folderBody
            ctx.fillRect(0, 0, 90, 120)
            ctx.fillStyle = theme.ink
            ctx.beginPath()
            ctx.moveTo(36, 46)
            ctx.lineTo(58, 60)
            ctx.lineTo(36, 74)
            ctx.closePath()
            ctx.fill()
            break
        case "font":
            ctx.fillStyle = theme.ink
            ctx.font = "italic 44px " + theme.serifFamily
            ctx.textAlign = "center"
            ctx.fillText("Aa", 45, 76)
            break
        case "link":
            rule(44, 50, theme.link, 0.9)
            rule(56, 34, theme.link, 0.9)
            break
        default:
            ;[48, 58, 68].forEach(function (y, i) {
                rule(y, [50, 36, 44][i], ink, 0.4)
            })
            break
        }
    }

    /// A hue from the theme's own lightness, so a light theme's image sheet
    /// and a dark theme's read as the same photograph.
    function hsl(theme, hue, saturation, lightness) {
        return Qt.hsla(hue / 360, saturation / 100,
                       (theme.dark ? lightness * 0.72 : lightness) / 100, 1)
    }
}
