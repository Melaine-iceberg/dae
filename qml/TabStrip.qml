pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Shell.js" as ShellState

// The tab strip, including the tear-off gesture.
//
// A tab drag is polled rather than driven by mouse events. QML stops receiving
// coordinates at the window edge, which is the exact moment the gesture becomes
// interesting: the release may land on another window or on the desktop. The
// poll reads the physical cursor, the window under it and the button state from
// `Shell`, so one code path covers reorder, cross-window merge and tear-off.
Item {
    id: root

    property var tabs: []
    property int activeIndex: 0
    property var hostWindow: null
    property bool dark: true
    property string material: "none"

    property int pressedIndex: -1
    property bool dragging: false
    property int dragIndex: -1
    property int dragTargetIndex: -1
    property string dragTargetToken: ""
    property bool dragInside: false
    property real dragLocalX: -1
    property real dragGlobalX: 0
    property real dragGlobalY: 0
    property real dragStartX: 0

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

    function beginDrag(index) {
        dragging = true
        dragIndex = index
        dragTargetIndex = -1
        dragTargetToken = ""
        dragInside = false
        dragLocalX = -1
    }

    function pollDrag() {
        const physicalX = Shell.cursorPhysicalX()
        const physicalY = Shell.cursorPhysicalY()
        dragGlobalX = Shell.cursorLogicalX()
        dragGlobalY = Shell.cursorLogicalY()

        const token = Shell.windowAt(physicalX, physicalY)
        dragTargetToken = token
        dragInside = hostWindow !== null && token === hostWindow.objectName
        if (dragInside) {
            dragLocalX = dragGlobalX - hostWindow.x
            dragTargetIndex = indexAt(dragLocalX)
        } else {
            dragLocalX = -1
            dragTargetIndex = -1
        }

        // The release may have happened over the desktop, where QML hears
        // nothing. The physical button is the only reliable end-of-gesture.
        if (!Shell.mouseDown())
            finishDrag()
    }

    function finishDrag() {
        if (!dragging)
            return

        const from = dragIndex
        const token = dragTargetToken
        const inside = dragInside
        const localX = dragLocalX
        const globalX = dragGlobalX
        const globalY = dragGlobalY

        dragging = false
        dragIndex = -1
        dragTargetIndex = -1
        dragTargetToken = ""
        dragInside = false
        dragLocalX = -1

        if (inside) {
            let to = indexAt(localX)
            if (to > from)
                to -= 1
            if (to !== from && to >= 0)
                tabMoved(from, to)
        } else if (token !== "") {
            const target = ShellState.windowFor(token)
            const targetX = target ? (globalX - target.x) : 0
            tabDroppedOnWindow(from, token, targetX)
        } else {
            tabTornOff(from, globalX, globalY)
        }
    }

    Timer {
        interval: 16
        repeat: true
        running: root.dragging
        onTriggered: root.pollDrag()
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

                    onPressed: {
                        root.pressedIndex = tabItem.index
                        root.dragStartX = mouse.x
                    }

                    onPositionChanged: {
                        if (pressed && !root.dragging
                                && Math.abs(mouse.x - root.dragStartX) > 5)
                            root.beginDrag(tabItem.index)
                    }

                    onReleased: {
                        if (!root.dragging)
                            root.tabActivated(tabItem.index)
                        // A drag is finished by the poll timer: the release
                        // that matters may be outside this window entirely.
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

    // Where the tab would land if released inside the strip.
    Rectangle {
        visible: root.dragging && root.dragInside
        x: root.indicatorX(root.dragTargetIndex)
        y: 5
        width: 2
        height: 24
        radius: 1
        color: root.accent
    }

    // The dragging tab cannot leave the window as a real item, so the ghost is
    // drawn only while the pointer is still inside. Outside, the window under
    // the cursor (or its absence) is the feedback.
    Rectangle {
        visible: root.dragging && root.dragInside && root.dragIndex >= 0
        x: root.dragLocalX - root.tabWidthValue / 2
        y: 2
        width: root.tabWidthValue
        height: 30
        radius: 6
        color: root.activeColor
        opacity: 0.78
        border.width: 1
        border.color: root.accent

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.right: parent.right
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            text: root.dragIndex >= 0 && root.dragIndex < root.tabs.length
                ? root.tabs[root.dragIndex].title : ""
            color: root.textColor
            font.pixelSize: 12
            elide: Text.ElideMiddle
        }
    }
}
