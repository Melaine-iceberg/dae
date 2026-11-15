import QtQuick
import QtQuick.Window
import dae

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
    /// The entry's path. Only a folder uses it, and only at 24 dp or more: the
    /// seven well-known destinations get a small sidebar mark inside the
    /// outline (§6.3.2). An empty path keeps the plain folder.
    property string path: ""
    /// The row's icon edge in dp. The provider is asked for a square raster at
    /// `edge * devicePixelRatio` so it stays sharp on fractional scaling.
    property int edge: 18
    /// Folders carry `inkSecondary`, files `inkTertiary`: the outline is a
    /// supporting shape, not a second line of text.
    property color outline: Theme.inkTertiary
    /// The family mark. Defaults to `Theme.fileColor(category)`.
    property color tint: Theme.fileColor(category)

    readonly property int pixelEdge: Math.max(12, Math.round(edge * Screen.devicePixelRatio))

    /// The sidebar mark for one of the seven well-known directories. Below
    /// 24 dp the folder is too small to hold a second shape (§6.3.2).
    readonly property string knownFolder: {
        if (glyph.category !== "folder" || glyph.edge < 24 || glyph.path === "")
            return ""
        const clean = String(glyph.path).replace(/[\\/]+$/, "")
        const base = clean.split(/[\\/]/).pop().toLowerCase()
        const home = String(Workspace.homeDirectory || "")
            .replace(/[\\/]+$/, "").toLowerCase()
        if (home !== "" && clean.toLowerCase() === home)
            return "home"
        switch (base) {
        case "downloads": case "下载": return "downloads"
        case "documents": case "文档": case "我的文档": return "documents"
        case "pictures": case "图片": case "我的图片": return "pictures"
        case "music": case "音乐": case "我的音乐": return "music"
        case "videos": case "视频": case "我的视频": return "videos"
        case "desktop": case "桌面": return "desktop"
        default: return ""
        }
    }

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

    /// The known-folder mark sits inside the paper. `Glyph` is the same
    /// `image://glyph` cache the sidebar uses, so the two never diverge.
    Glyph {
        anchors.centerIn: parent
        width: Math.round(glyph.edge * 0.45)
        height: width
        visible: glyph.knownFolder !== ""
        kind: glyph.knownFolder
        tint: glyph.tint
        wash: 0
    }

    /// The provider takes colours without the leading `#`, which keeps the URL
    /// free of percent-encoding. Alpha is dropped: a glyph is never translucent.
    function hex(color) {
        return color.toString().slice(-6)
    }
}
