pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Shell.js" as ShellState

// The tab strip, including the tear-off gesture.
//
// A tab drag is handed to the platform's own drag loop (`Shell.startTabDrag`).
// That loop draws the chip with the grab point under the cursor on every
// platform (OLE, NSDraggingSession, X11/Wayland DnD) and keeps reporting after
// the pointer leaves the window, which a MouseArea cannot. Each window carries
// a `DropArea` matching the tab MIME type; a drop only records where it landed
// in `ShellState`, and this strip applies the reorder or merge once the loop
// returns. A release that no drop target took — the desktop or another app —
// tears the tab off at the final cursor position.
Item {
    id: root

    property var tabs: []
    property int activeIndex: 0
    property var hostWindow: null
    property bool dark: true
    property string material: "none"

    /// A pressed tab that could still become a drag. Cleared before the native
    /// loop starts, so a stray move after the loop cannot begin a second one.
    property bool dragArmed: false
    property bool dragging: false
    property int dragIndex: -1
    property real dragStartX: 0
    property real dragStartY: 0
    /// A native drag ran and ended somewhere; the release it consumed must not
    /// activate the tab it left behind.
    property bool dragSuppressClick: false
    /// Where a hovering tab drag would insert, in strip coordinates; -1 while
    /// no tab drag is over this strip. Set by the window's `DropArea`.
    property real dropX: -1

    signal tabActivated(int index)
    signal tabClosed(int index)
    signal newTabRequested()
    signal newWindowRequested()
    signal tabMoved(int from, int to)
    signal tabDroppedOnWindow(int index, string token, real localX)
    signal tabTornOff(int index, real globalX, real globalY)

    implicitHeight: 34
    height: 34
    clip: true

    readonly property int tabWidthValue: 184
    readonly property int tabGap: 2
    readonly property int tabLeft: 8

    readonly property color chromeColor: material === "none"
        ? (dark ? "#14161a" : "#f4f6f8")
        : (dark ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
                : Qt.rgba(0.98, 0.985, 0.99, 0.46))
    readonly property color activeColor: dark ? "#24272d" : "#ffffff"
    readonly property color hoverColor: dark ? "#1affffff" : "#0d000000"
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"
    readonly property color accent: dark ? "#6ea8fe" : "#2f6fdf"

    function tabX(index) {
        return tabLeft + index * (tabWidthValue + tabGap)
    }

    function indexAt(x) {
        for (let i = 0; i < tabs.length; i += 1) {
            const middle = tabX(i) + tabWidthValue / 2
            if (x < middle)
                return i
        }
        return tabs.length
    }

    function indicatorX(index) {
        if (index <= 0)
            return tabLeft
        const column = Math.min(index, tabs.length) - 1
        return tabX(column) + tabWidthValue + tabGap / 2
    }

    /// Whether a logical screen point is inside the window this strip belongs
    /// to. `hostWindow` is frameless, so its position/size are the window rect.
    function insideWindow(x, y) {
        return hostWindow !== null
            && x >= hostWindow.x && x < hostWindow.x + hostWindow.width
            && y >= hostWindow.y && y < hostWindow.y + hostWindow.height
    }

    /// Marks where a tab drag hovering this strip would insert. The window's
    /// `DropArea` drives this while the native loop runs.
    function showDrop(x) {
        dropX = x
    }

    function clearDrop() {
        dropX = -1
    }

    function beginDrag(index, grabX, grabY) {
        if (hostWindow === null || index < 0 || index >= tabs.length)
            return

        const title = tabs[index] ? tabs[index].title : ""
        const token = hostWindow.objectName
        dragArmed = false
        dragging = true
        dragIndex = index
        dragSuppressClick = true
        dropX = -1
        ShellState.clearTabDrop()

        // Blocks in the platform's modal loop. The drag image is the tab chip
        // with `grabX/grabY` as its hot spot, so the preview keeps the cursor
        // on the point the user grabbed. Drops land in the window DropAreas
        // while this runs, so the record is complete when it returns.
        const action = Shell.startTabDrag(token, title, index,
                                          Math.round(grabX), Math.round(grabY),
                                          tabWidthValue, 30,
                                          activeColor, accent, textColor)

        const drop = ShellState.takeTabDrop()
        const globalX = Shell.cursorLogicalX()
        const globalY = Shell.cursorLogicalY()

        dragging = false
        dragIndex = -1
        dropX = -1

        if (drop !== null && drop.sourceToken === token
                && drop.sourceIndex === index) {
            if (drop.targetToken === token) {
                if (drop.onStrip) {
                    // The dragged tab is still in the list; the insertion index
                    // is measured among the tabs it would land between.
                    let to = indexAt(drop.x)
                    if (to > index)
                        to -= 1
                    if (to !== index && to >= 0)
                        tabMoved(index, to)
                }
            } else if (drop.targetToken !== "") {
                tabDroppedOnWindow(index, drop.targetToken, drop.x)
            }
            return
        }

        // Nothing took the drop. Outside this window that is the platform
        // reporting a release over the desktop or another app; inside it, the
        // loop was cancelled (Escape) and the tab stays where it was.
        if (action === Qt.IgnoreAction && !insideWindow(globalX, globalY))
            tabTornOff(index, globalX, globalY)
    }

    Rectangle {
        anchors.fill: parent
        color: root.chromeColor
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: root.hairline
    }

    Row {
        x: root.tabLeft
        y: 2
        height: 30
        spacing: root.tabGap

        Repeater {
            model: root.tabs

            Rectangle {
                id: tabItem
                required property int index
                required property var modelData

                width: root.tabWidthValue
                height: 30
                radius: 6
                color: root.activeIndex === index
                    ? root.activeColor
                    : (tabMouse.containsMouse ? root.hoverColor : "transparent")
                opacity: root.dragging && root.dragIndex === index ? 0.45 : 1.0

                Text {
                    id: tabTitle
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    anchors.right: closeButton.left
                    anchors.rightMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    text: tabItem.modelData ? tabItem.modelData.title : ""
                    color: root.activeIndex === tabItem.index
                        ? root.textColor : root.mutedColor
                    font.pixelSize: 12
                    elide: Text.ElideMiddle
                }

                Item {
                    id: closeButton
                    anchors.right: parent.right
                    anchors.rightMargin: 5
                    anchors.verticalCenter: parent.verticalCenter
                    width: 20
                    height: 20

                    Rectangle {
                        anchors.fill: parent
                        radius: 4
                        color: closeMouse.containsMouse
                            ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: "\u00d7"
                        color: closeMouse.containsMouse ? root.textColor : root.mutedColor
                        font.pixelSize: 14
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.tabClosed(tabItem.index)
                    }
                }

                MouseArea {
                    id: tabMouse
                    anchors.fill: parent
                    anchors.rightMargin: 24
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton

                    onPressed: function (mouse) {
                        root.dragArmed = true
                        root.dragSuppressClick = false
                        root.dragStartX = mouse.x
                        root.dragStartY = mouse.y
                    }

                    onPositionChanged: function (mouse) {
                        if (!root.dragArmed || root.dragging
                                || (mouse.buttons & Qt.LeftButton) === 0)
                            return
                        const dx = mouse.x - root.dragStartX
                        const dy = mouse.y - root.dragStartY
                        if (Math.sqrt(dx * dx + dy * dy) > 6)
                            root.beginDrag(tabItem.index, root.dragStartX,
                                           root.dragStartY)
                    }

                    onReleased: {
                        root.dragArmed = false
                        // The release that ended a native drag may arrive as a
                        // click after the loop; it must not activate the tab.
                        if (root.dragSuppressClick) {
                            root.dragSuppressClick = false
                            return
                        }
                        if (!root.dragging)
                            root.tabActivated(tabItem.index)
                    }

                    onCanceled: {
                        root.dragArmed = false
                    }
                }
            }
        }

        Item {
            width: 30
            height: 30

            Rectangle {
                anchors.fill: parent
                radius: 6
                color: plusMouse.containsMouse ? root.hoverColor : "transparent"
            }

            Text {
                anchors.centerIn: parent
                text: "+"
                color: root.mutedColor
                font.pixelSize: 16
            }

            MouseArea {
                id: plusMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.newTabRequested()
            }
        }
    }

    Rectangle {
        id: newWindowButton
        anchors.right: parent.right
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        width: 84
        height: 24
        radius: 6
        color: newWindowMouse.containsMouse
            ? (root.dark ? "#26ffffff" : "#12000000")
            : (root.dark ? "#1affffff" : "#0d000000")

        Text {
            anchors.centerIn: parent
            text: "新窗口"
            color: root.mutedColor
            font.pixelSize: 11
        }

        MouseArea {
            id: newWindowMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.newWindowRequested()
        }
    }

    // Where the tab would land. The platform draws the dragged chip itself, so
    // this is the only in-window feedback while a drag is over the strip; it
    // also shows for a tab dragged in from another window.
    Rectangle {
        visible: root.dropX >= 0
        x: root.indicatorX(root.indexAt(root.dropX))
        y: 5
        width: 2
        height: 24
        radius: 1
        color: root.accent
    }
}
