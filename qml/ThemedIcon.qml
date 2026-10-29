import QtQuick

// The built-in glyph for a listing row, keyed by the model's category.
//
// This used to be a tile per type: a coloured plate with the extension in small
// caps, and a different hue for every category. A column of them turned the
// listing into a colour chart, which competes with the two colours the shell
// reserves for state (the signal orange and the git badges). The mark is now
// the same line art as the rest of the shell — one silhouette per category,
// drawn in `Glyph` — so a row's colour means state, not file type.
//
// The API stays `category` / `extension` / `isDir` / `size`: the model already
// classifies rows, and the extension is kept for callers that want to render
// it beside the name later. Real thumbnails still come from the backend; this
// only stands in when there is no thumbnail to show.
Item {
    id: root

    property string category: "file"
    property string extension: ""
    property bool isDir: false
    property int size: 18
    /// The ink. Defaults to the muted step so a listing reads as text first;
    /// a caller may pass a fainter tint (the breadcrumb trail does).
    property color tint: Theme.inkMuted

    implicitWidth: size
    implicitHeight: size

    readonly property string glyphKind: {
        if (root.isDir || root.category === "folder")
            return "folder"
        switch (root.category) {
        case "image": return "image"
        case "video":
        case "audio": return "media"
        case "archive": return "archive"
        case "code": return "code"
        case "link": return "link"
        default: return "file"
        }
    }

    Glyph {
        anchors.fill: parent
        kind: root.glyphKind
        tint: root.tint
    }
}
