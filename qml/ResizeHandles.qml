import QtQuick

// Frameless windows have no native resize border. On Windows the hit test in
// `crates/dae-qt/cpp/window.h` returns the HT* edge codes; elsewhere these thin
// handles ask the window manager for the same edge drag, so a window is
// resizable without reimplementing the resize loop in QML.
//
// The item itself only holds the handles: the centre is empty and passes input
// through, which keeps scrollbars and sidebars behind the frame clickable.
Item {
    id: root

    property var window: null
    /// How close to the edge a press must land. Wide enough for a pointer,
    /// narrow enough not to swallow clicks on content that sits at the frame.
    property int thickness: 6

    visible: Qt.platform.os !== "windows"

    function begin(edges) {
        if (root.window !== null)
            root.window.startSystemResize(edges)
    }

    MouseArea {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.LeftEdge)
    }

    MouseArea {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.RightEdge)
    }

    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.TopEdge)
    }

    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.BottomEdge)
    }

    // Declared after the edges: overlapping corners belong to the diagonal
    // resize, not to whichever edge happens to be on top.
    MouseArea {
        anchors.left: parent.left
        anchors.top: parent.top
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.LeftEdge | Qt.TopEdge)
    }

    MouseArea {
        anchors.right: parent.right
        anchors.top: parent.top
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.RightEdge | Qt.TopEdge)
    }

    MouseArea {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.LeftEdge | Qt.BottomEdge)
    }

    MouseArea {
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        onPressed: root.begin(Qt.RightEdge | Qt.BottomEdge)
    }
}
