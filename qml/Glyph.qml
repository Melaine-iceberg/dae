import QtQuick
import QtQuick.Window

// A drawn mark, keyed by what it means (proposal §6.3.1, §12.3).
//
// The geometry is Google's Material Symbols — `material-marks.json` carries the
// path data, its provenance and the six marks that are still dae's own. This
// file is only the seam: the mark is rasterised by the `glyph://` provider and
// painted as a plain `Image`, so nothing in the shell owns a `Canvas` and a
// listing row or a column-view delegate never re-parses a path.
//
// Three rules the renderer keeps, because they are what the marks look like:
//
//  * Material's outlined glyphs are filled outlines; the silhouette marks
//    (`s` in the mark table — folder, tag) and the six legacy marks are stroked
//    at 1.2 device pixels rather than 1.2 *units*. The shell draws marks at 8,
//    12 and 14 dp as well as 16, and a uniform unit stroke shrinks to 0.75 px at
//    10 — every edge lands between pixels and the mark reads as grey haze
//    instead of a line. Below 16 dp the geometry is scaled up in stroke only,
//    which is the usual optical correction for a small icon.
//  * A silhouette is filled at `wash` and stroked, so a folder reads as a shape
//    rather than a block.
//  * The cache key is (kind, edge, dp, ink, wash): a tinted mark is a different
//    raster, not a repaint, and twenty rows of the same mark share one texture.
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
    /// The stroke and silhouette colour. Alpha is part of the key: the title
    /// bar's traffic lights are drawn at 45 % over the plate.
    property color tint: Theme.inkSecondary
    /// Strength of a silhouette's fill, as a fraction of `tint`. At 0.3 the
    /// folder keeps its outline and reads as a shape rather than a block.
    property real wash: 0.3

    implicitWidth: 16
    implicitHeight: 16

    /// The raster is asked for at the item's own size times the ratio the
    /// screen is running at, so a 12 dp chevron on a 150 % display is 18
    /// device pixels rather than a stretched 12.
    readonly property int pixelEdge: Math.max(8,
        Math.round(Math.max(1, width) * Screen.devicePixelRatio))

    Image {
        anchors.fill: parent
        /// A row with no mark passes an empty kind (the command palette's file
        /// results, a chip without one). Nothing is asked of the provider then:
        /// the old canvas simply did not draw, and a transparent raster would
        /// still cost a cache entry.
        source: root.kind === "" ? ""
                : "image://glyph/" + root.kind
                  + "?size=" + root.pixelEdge
                  + "&dp=" + Math.max(8, Math.round(root.width))
                  + "&ink=" + root.hex(root.tint)
                  + "&wash=" + root.wash.toFixed(2)
        /// Stretch, not Pad: the raster is `width * dpr` pixels, and Pad would
        /// draw it at that many device-independent pixels. Stretch maps it
        /// exactly onto the item.
        sourceSize.width: root.pixelEdge
        sourceSize.height: root.pixelEdge
        fillMode: Image.Stretch
        smooth: true
        cache: true
        asynchronous: false
        Accessible.ignored: true
    }

    /// The provider takes the colour without the leading `#`: a `#` in the URL
    /// would start a fragment and swallow `ink`. Eight hex digits are kept
    /// (AARRGGBB) because the title bar's traffic lights are drawn at 45 %.
    function hex(color) {
        return color.toString().slice(1)
    }
}
