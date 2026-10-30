import QtQuick

// One side of a transfer conflict: which entry, what kind, how big, when it was
// last written.
//
// The source card is plain and the target card is tinted with the accent,
// because the question the dialog asks is "does the one on the right give way
// to the one on the left" — the eye should land on the incumbent.
Rectangle {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    property bool highlight: false
    property string heading: ""
    property string name: ""
    /// The glyph set's category name (`folder`, `file`, `link`, `other`).
    property string category: "file"
    /// The two caption lines, already formatted by the command surface.
    property string detail: ""

    height: column.implicitHeight + 20
    radius: Theme.radiusLg
    color: root.highlight
        ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, root.dark ? 0.16 : 0.07)
        : (root.dark ? Theme.hover : Theme.chrome)
    border.width: 1
    border.color: root.highlight
        ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.45)
        : Theme.hairline

    Column {
        id: column
        x: 10
        y: 10
        width: parent.width - 20
        spacing: 4

        Text {
            width: parent.width
            text: root.heading
            color: Theme.inkMuted
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Row {
            width: parent.width
            height: 16
            spacing: 6

            ThemedIcon {
                anchors.verticalCenter: parent.verticalCenter
                category: root.category
                isDir: root.category === "folder"
                size: 16
                Accessible.ignored: true
            }

            Text {
                id: nameText
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(implicitWidth, parent.width - 22)
                text: root.name
                color: Theme.ink
                font.pixelSize: 12
                font.weight: Font.Medium
                elide: Text.ElideMiddle
            }
        }

        Text {
            width: parent.width
            text: root.detail
            color: Theme.inkMuted
            font.pixelSize: 11
            lineHeight: 1.3
            wrapMode: Text.WordWrap
        }
    }
}
