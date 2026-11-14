import QtQuick
import QtQuick.Window

/// The built-in file glyph (proposal §6.3.2, §12.3): one page shape for every
/// file, a folder for directories, and a family-coloured mark inside.
///
/// The 20-grid geometry is rasterised by the `fileglyph://` provider and cached
/// per (family, pixel edge, ink, family colour), so a listing row owns a plain
/// `Image` rather than a `Canvas`; 20 rows of the same extension share one
/// texture, and scrolling never paints geometry.
Item {
    id: glyph

    /// The family `dae-qt::model::category_of` reports: folder, code, image,
    /// video, audio, archive, spreadsheet, presentation, pdf, document, text,
    /// font, executable, link, file, other.
    property string category: "file"
    /// The row's icon edge in dp. The provider is asked for a square raster at
    /// `edge * devicePixelRatio` so it stays sharp on fractional scaling.
    property int edge: 18
    /// Folders carry `inkSecondary`, files `inkTertiary`: the outline is a
    /// supporting shape, not a second line of text.
    property color outline: Theme.inkTertiary
    /// The family mark. Defaults to `Theme.fileColor(category)`.
    property color tint: Theme.fileColor(category)

    readonly property int pixelEdge: Math.max(12, Math.round(edge * Screen.devicePixelRatio))

    implicitWidth: edge
    implicitHeight: edge

    Image {
        anchors.fill: parent
        source: "image://fileglyph/" + glyph.category
                + "?size=" + glyph.pixelEdge
                + "&ink=" + glyph.hex(glyph.outline)
                + "&fc=" + glyph.hex(glyph.tint)
        sourceSize.width: glyph.pixelEdge
        sourceSize.height: glyph.pixelEdge
        /// Stretch, not Pad: the raster is `edge * dpr` pixels, and Pad would
        /// draw it at that many device-independent pixels, clipped to the
        /// 18 dp box. Stretch maps the raster exactly onto the item.
        fillMode: Image.Stretch
        smooth: true
        cache: true
        asynchronous: false
    }

    /// The provider takes colours without the leading `#`, which keeps the URL
    /// free of percent-encoding. Alpha is dropped: a glyph is never translucent.
    function hex(color) {
        return color.toString().slice(-6)
    }
}
