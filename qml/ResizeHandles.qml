import QtQuick

// Frameless windows have no native resize border. On Windows the hit test in
// `crates/dae-qt/cpp/window.h` returns the HT* edge codes. On X11 and Wayland
// `startSystemResize` asks the window manager to run its own resize loop. Qt's
// Cocoa backend does not implement `startSystemResize` (it returns false), so
// macOS falls back to moving the window geometry here. That fallback is the
// reason this item owns the press/move/release state instead of a single call.
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

    /// Eight anonymous strips are the frame, not content. Keyboard users resize
    /// a window through the window manager's own commands; announcing the
    /// handles would only pad the tree with targets that have no keyboard path.
    Accessible.ignored: true

    property bool manualActive: false
    property int manualEdges: 0
    property real startWindowX: 0
    property real startWindowY: 0
    property real startWindowWidth: 0
    property real startWindowHeight: 0
    property real startGlobalX: 0
    property real startGlobalY: 0

    /// The pointer in screen coordinates. `mouse.x` is relative to the handle,
    /// which sits on the window edge, so the handle's offset inside the window
    /// has to be added before the window origin.
    function pointerGlobal(edges, mouse) {
        let x = mouse.x
        let y = mouse.y
        if (edges & Qt.RightEdge)
            x += root.width - root.thickness
        if (edges & Qt.BottomEdge)
            y += root.height - root.thickness
        return Qt.point(root.window.x + x, root.window.y + y)
    }

    function begin(edges, mouse) {
        if (root.window === null)
            return false

        if (root.window.startSystemResize(edges))
            return true

        // macOS: no platform resize loop. Remember the geometry and the press
        // point; `move` converts the pointer into a window-geometry delta.
        root.manualActive = true
        root.manualEdges = edges
        root.startWindowX = root.window.x
        root.startWindowY = root.window.y
        root.startWindowWidth = root.window.width
        root.startWindowHeight = root.window.height
        const start = root.pointerGlobal(edges, mouse)
        root.startGlobalX = start.x
        root.startGlobalY = start.y
        return true
    }

    function move(mouse) {
        if (!root.manualActive || root.window === null)
            return

        // The pointer is reported relative to the handle, and a left/top drag
        // moves that handle (and the window) under the pointer. The global
        // reading keeps the delta in screen space instead of letting it feed
        // back into itself.
        const point = root.pointerGlobal(root.manualEdges, mouse)
        const dx = point.x - root.startGlobalX
        const dy = point.y - root.startGlobalY

        let x = root.startWindowX
        let y = root.startWindowY
        let width = root.startWindowWidth
        let height = root.startWindowHeight

        // The design minimum is 900x600. Never let a window that is already
        // smaller than that grow just because an edge is dragged outward.
        let minWidth = root.window.minimumWidth > 0 ? root.window.minimumWidth : 900
        let minHeight = root.window.minimumHeight > 0 ? root.window.minimumHeight : 600
        minWidth = Math.min(minWidth, root.startWindowWidth)
        minHeight = Math.min(minHeight, root.startWindowHeight)

        if (root.manualEdges & Qt.LeftEdge) {
            const delta = Math.min(dx, root.startWindowWidth - minWidth)
            x = root.startWindowX + delta
            width = root.startWindowWidth - delta
        } else if (root.manualEdges & Qt.RightEdge) {
            width = Math.max(minWidth, root.startWindowWidth + dx)
        }

        if (root.manualEdges & Qt.TopEdge) {
            const delta = Math.min(dy, root.startWindowHeight - minHeight)
            y = root.startWindowY + delta
            height = root.startWindowHeight - delta
        } else if (root.manualEdges & Qt.BottomEdge) {
            height = Math.max(minHeight, root.startWindowHeight + dy)
        }

        root.window.x = Math.round(x)
        root.window.y = Math.round(y)
        root.window.width = Math.round(width)
        root.window.height = Math.round(height)
    }

    function finish() {
        root.manualActive = false
    }

    MouseArea {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeHorCursor
        onPressed: (mouse) => root.begin(Qt.LeftEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeHorCursor
        onPressed: (mouse) => root.begin(Qt.RightEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeVerCursor
        onPressed: (mouse) => root.begin(Qt.TopEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeVerCursor
        onPressed: (mouse) => root.begin(Qt.BottomEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    // Declared after the edges: overlapping corners belong to the diagonal
    // resize, not to whichever edge happens to be on top.
    MouseArea {
        anchors.left: parent.left
        anchors.top: parent.top
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeFDiagCursor
        onPressed: (mouse) => root.begin(Qt.LeftEdge | Qt.TopEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.right: parent.right
        anchors.top: parent.top
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeBDiagCursor
        onPressed: (mouse) => root.begin(Qt.RightEdge | Qt.TopEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeBDiagCursor
        onPressed: (mouse) => root.begin(Qt.LeftEdge | Qt.BottomEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }

    MouseArea {
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: root.thickness
        height: root.thickness
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.SizeFDiagCursor
        onPressed: (mouse) => root.begin(Qt.RightEdge | Qt.BottomEdge, mouse)
        onPositionChanged: (mouse) => root.move(mouse)
        onReleased: root.finish()
        onCanceled: root.finish()
    }
}
