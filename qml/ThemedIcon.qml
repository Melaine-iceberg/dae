import QtQuick

// The built-in glyph set, one tile per category.
//
// The "system" icon style asks the desktop for every row; this is the other
// half of that choice, drawn from the same classification the model uses for
// type sorting and the thumbnail decision. A tile is deliberately simple: a
// category colour, a folder silhouette, or the extension in small caps. That
// is enough to make a column of files scannable without shipping a glyph
// library, and it stays legible at the 18px of a list row.
Item {
    id: root

    property string category: "file"
    property string extension: ""
    property bool isDir: false
    property int size: 18
    property bool dark: true
    property color accent: "#2f6fdf"

    implicitWidth: size
    implicitHeight: size

    readonly property color categoryColor: {
        switch (root.category) {
        case "folder": return root.accent
        case "image": return root.dark ? "#5aa9e6" : "#2f7fc1"
        case "video": return root.dark ? "#b48ef0" : "#7d4fc0"
        case "audio": return root.dark ? "#e8a45c" : "#b56d20"
        case "archive": return root.dark ? "#d9b44a" : "#a47c12"
        case "code": return root.dark ? "#63c98c" : "#2f8f56"
        case "text": return root.dark ? "#98a3b3" : "#5f6b7a"
        case "document": return root.dark ? "#6fb6df" : "#2f7c9e"
        case "font": return root.dark ? "#c98fe0" : "#8d4aa6"
        case "executable": return root.dark ? "#ef7b74" : "#b23a32"
        case "link": return root.dark ? "#7fc3e8" : "#2b7d9e"
        default: return root.dark ? "#7f8797" : "#68707d"
        }
    }

    /// A folder is two stacked plates, the way the Finder and Explorer both
    /// draw one; everything else is a rounded tile with a short label.
    Item {
        visible: root.isDir || root.category === "folder"
        anchors.fill: parent

        Rectangle {
            x: Math.round(root.size * 0.10)
            y: Math.round(root.size * 0.24)
            width: Math.round(root.size * 0.46)
            height: Math.round(root.size * 0.22)
            radius: Math.max(1, Math.round(root.size * 0.08))
            color: Qt.rgba(root.categoryColor.r, root.categoryColor.g,
                           root.categoryColor.b, root.dark ? 0.78 : 0.9)
        }

        Rectangle {
            x: Math.round(root.size * 0.08)
            y: Math.round(root.size * 0.36)
            width: Math.round(root.size * 0.84)
            height: Math.round(root.size * 0.46)
            radius: Math.max(1, Math.round(root.size * 0.12))
            color: root.categoryColor
        }
    }

    Rectangle {
        visible: !root.isDir && root.category !== "folder"
        anchors.fill: parent
        radius: Math.max(2, Math.round(root.size * 0.22))
        color: Qt.rgba(root.categoryColor.r, root.categoryColor.g,
                       root.categoryColor.b, root.dark ? 0.24 : 0.16)
        border.width: Math.max(1, Math.round(root.size * 0.05))
        border.color: Qt.rgba(root.categoryColor.r, root.categoryColor.g,
                              root.categoryColor.b, root.dark ? 0.72 : 0.6)

        Text {
            anchors.centerIn: parent
            text: {
                if (root.category === "link")
                    return "\u2197"
                if (root.category === "executable")
                    return "\u25B6"
                const extension = root.extension.toUpperCase()
                return extension.length > 0 ? extension.substring(0, 3) : "\u2022"
            }
            color: root.categoryColor
            font.pixelSize: Math.max(6, Math.round(root.size * 0.34))
            font.weight: Font.DemiBold
            font.letterSpacing: -0.2
        }
    }
}
