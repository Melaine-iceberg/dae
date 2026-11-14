pragma ComponentBehavior: Bound
import QtQuick

// Space, without giving up the listing.
//
// The docked preview pane is a fourth column: opening it reflows the window.
// This overlay is the other gesture — the one Files trains — a plate over the
// content that closes on Escape or an outside click and leaves the selection
// exactly where it was. It reuses `PreviewPanel` rather than a second reader,
// so a file that previews in the pane previews here with the same limits
// (text peeks; thumbnails are local-only).
Item {
    id: root

    property var paths: []
    property bool dark: true
    property color accent: Theme.accent

    visible: false
    focus: visible
    z: 1004

    readonly property bool open: visible

    /// The property `open` is the state; this is the verb.
    function show(next) {
        root.paths = next && next.length > 0 ? next : []
        if (root.paths.length === 0)
            return
        root.visible = true
        panel.forceActiveFocus()
    }

    function close() {
        root.visible = false
        root.paths = []
    }

    /// The overlay owns the arrow keys, not the file list: scrolling the
    /// preview must not move the selection behind it.
    Keys.onEscapePressed: root.close()
    Keys.onUpPressed: panelContent.scrollBy(-40)
    Keys.onDownPressed: panelContent.scrollBy(40)
    Keys.onLeftPressed: root.close()
    Keys.onRightPressed: root.close()
    Keys.onPressed: function (event) {
        if (event.key === Qt.Key_PageUp) {
            panelContent.scrollBy(-panelContent.height * 0.8)
            event.accepted = true
        } else if (event.key === Qt.Key_PageDown) {
            panelContent.scrollBy(panelContent.height * 0.8)
            event.accepted = true
        } else if (event.key === Qt.Key_Space) {
            root.close()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            onClicked: root.close()
        }
    }

    Rectangle {
        id: panel

        anchors.centerIn: parent
        width: Math.min(760, parent.width - 96)
        height: Math.min(parent.height - 96, 680)
        radius: Theme.radiusMd
        color: Theme.canvas
        border.width: 1
        border.color: Theme.line
        focus: true

        // A click inside the plate must not close it; the scrim is behind.
        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
        }

        PreviewPanel {
            id: panelContent
            anchors.fill: parent
            paths: root.paths
            dark: root.dark
            accent: root.accent
            onCloseRequested: root.close()
        }
    }
}
