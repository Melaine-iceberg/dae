pragma ComponentBehavior: Bound
import QtQuick

// A small custom context menu.
//
// Qt Quick Controls would bring a whole style with it for one popup, and the
// menu has to sit inside the frameless window's own coordinate space anyway.
// The overlay covers the window while open, so clicking anywhere else closes
// it — the same behaviour as a native popup.
Item {
    id: root

    property var actions: []
    property real openX: 0
    property real openY: 0
    property bool dark: true

    readonly property int submenuWidth: 204
    property var submenuActions: []
    property real submenuX: 0
    property real submenuY: 0

    signal actionTriggered(string id)

    visible: false
    focus: visible
    z: 1000

    function openAt(x, y, items) {
        root.actions = items
        root.openX = x
        root.openY = y
        root.closeSubmenu()
        root.visible = true
        root.forceActiveFocus()
    }

    function close() {
        root.visible = false
        root.actions = []
        root.closeSubmenu()
    }

    function closeSubmenu() {
        root.submenuActions = []
    }

    /// Anchors the submenu to `row`'s right edge, flipping to the left when the
    /// window edge would clip it.
    function showSubmenu(row, items) {
        const point = row.mapToItem(root, 0, 0)
        const right = point.x + row.width
        const flip = right + root.submenuWidth > root.width - 8
        root.submenuX = flip ? point.x - root.submenuWidth + 2 : right - 2
        root.submenuY = point.y - 5
        root.submenuActions = items
    }

    Keys.onEscapePressed: {
        if (root.submenuActions.length > 0)
            root.closeSubmenu()
        else
            root.close()
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel
        width: 226
        height: column.implicitHeight + 10
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, Math.min(root.openY, root.height - height - 8))
        radius: 8
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        Rectangle {
            anchors.fill: parent
            anchors.margins: 1
            radius: panel.radius - 1
            color: "transparent"
            border.width: 0
        }

        Column {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 5

            Repeater {
                model: root.actions

                delegate: Item {
                    id: entry

                    required property var modelData

                    width: column.width
                    height: entry.modelData.separator ? 9 : (entry.modelData.enabled === false ? 24 : 27)

                    Rectangle {
                        visible: entry.modelData.separator === true
                        anchors.verticalCenter: parent.verticalCenter
                        x: 10
                        width: parent.width - 20
                        height: 1
                        color: root.dark ? "#1affffff" : "#12000000"
                    }

                    Rectangle {
                        visible: entry.modelData.separator !== true
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 5
                        color: hover.containsMouse && entry.modelData.enabled !== false
                            ? (root.dark ? "#22ffffff" : "#0f000000")
                            : "transparent"
                    }

                    Text {
                        visible: entry.modelData.separator !== true
                        anchors.left: parent.left
                        anchors.leftMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.label || ""
                        color: {
                            if (entry.modelData.enabled === false)
                                return root.dark ? "#5c6370" : "#a5acb6"
                            if (entry.modelData.danger === true)
                                return root.dark ? "#f08a8a" : "#b42323"
                            return root.dark ? "#e3e7ee" : "#24292f"
                        }
                        font.pixelSize: 12
                    }

                    Text {
                        visible: entry.modelData.separator !== true
                            && !entry.modelData.submenu
                            && !!entry.modelData.shortcut
                        anchors.right: parent.right
                        anchors.rightMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.shortcut || ""
                        color: root.dark ? "#6e7684" : "#98a0ab"
                        font.pixelSize: 10
                    }

                    Text {
                        visible: entry.modelData.separator !== true
                            && entry.modelData.submenu !== undefined
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: "\u203a"
                        color: root.dark ? "#8d94a2" : "#657080"
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: hover
                        anchors.fill: parent
                        enabled: entry.modelData.separator !== true
                            && entry.modelData.enabled !== false
                        hoverEnabled: true
                        onEntered: {
                            const items = entry.modelData.submenu
                            if (items && items.length > 0)
                                root.showSubmenu(entry, items)
                            else
                                root.closeSubmenu()
                        }
                        onClicked: {
                            const items = entry.modelData.submenu
                            if (items && items.length > 0) {
                                root.showSubmenu(entry, items)
                                return
                            }
                            const id = entry.modelData.id
                            root.close()
                            root.actionTriggered(id)
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        id: submenuPanel
        visible: root.submenuActions.length > 0
        width: root.submenuWidth
        height: submenuColumn.implicitHeight + 10
        x: Math.max(8, Math.min(root.submenuX, root.width - width - 8))
        y: Math.max(8, Math.min(root.submenuY, root.height - height - 8))
        radius: 8
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        Column {
            id: submenuColumn
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 5

            Repeater {
                model: root.submenuActions

                delegate: Item {
                    id: submenuEntry

                    required property var modelData

                    width: submenuColumn.width
                    height: 27

                    Rectangle {
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 5
                        color: submenuHover.containsMouse
                            ? (root.dark ? "#22ffffff" : "#0f000000")
                            : "transparent"
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: submenuEntry.modelData.label || ""
                        color: root.dark ? "#e3e7ee" : "#24292f"
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: submenuHover
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            const id = submenuEntry.modelData.id
                            root.close()
                            root.actionTriggered(id)
                        }
                    }
                }
            }
        }
    }
}
