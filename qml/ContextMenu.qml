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
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        NumberAnimation { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
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

    /// Swaps the rows without moving the panel: the installed apps' commands
    /// arrive asynchronously, and the menu they belong to is already open.
    function replaceActions(items) {
        root.actions = items
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
        radius: 12
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

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
                    height: {
                        if (entry.modelData.separator === true)
                            return 9
                        if (entry.modelData.header === true)
                            return 20
                        return entry.modelData.enabled === false ? 24 : 27
                    }

                    Rectangle {
                        visible: entry.modelData.separator === true
                        anchors.verticalCenter: parent.verticalCenter
                        x: 10
                        width: parent.width - 20
                        height: 1
                        color: Theme.fill
                    }

                    Rectangle {
                        visible: entry.modelData.separator !== true
                            && entry.modelData.header !== true
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 6
                        color: hover.containsMouse && entry.modelData.enabled !== false
                            ? Theme.fill
                            : "transparent"
                    }

                    Image {
                        visible: entry.modelData.separator !== true
                            && !!entry.modelData.icon
                            && status === Image.Ready
                        x: 12
                        width: 16
                        height: 16
                        anchors.verticalCenter: parent.verticalCenter
                        source: entry.modelData.icon || ""
                        fillMode: Image.PreserveAspectFit
                        asynchronous: true
                        cache: true
                    }

                    // A toggled row marks itself in the icon gutter, the way a
                    // native menu does. The label shifts right either way, so
                    // checked and unchecked rows keep one text edge.
                    Glyph {
                        visible: entry.modelData.separator !== true
                            && entry.modelData.header !== true
                            && entry.modelData.checked === true
                            && !entry.modelData.icon
                        x: 12
                        width: 14
                        height: 14
                        anchors.verticalCenter: parent.verticalCenter
                        kind: "check"
                        tint: Theme.interactive
                    }

                    Text {
                        visible: entry.modelData.separator !== true
                        anchors.left: parent.left
                        anchors.leftMargin: (entry.modelData.icon
                            || entry.modelData.checked === true) ? 34 : 12
                        anchors.right: parent.right
                        anchors.rightMargin: 30
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.label || ""
                        elide: Text.ElideRight
                        color: {
                            if (entry.modelData.header === true)
                                return Theme.inkFaint
                            if (entry.modelData.enabled === false)
                                return Theme.inkFaint
                            if (entry.modelData.danger === true)
                                return Theme.danger
                            return Theme.ink
                        }
                        font.pixelSize: entry.modelData.header === true
                            ? Theme.typeMicro : Theme.typeSmall
                        font.weight: entry.modelData.header === true
                            ? Font.Medium : Font.Normal
                    }

                    Text {
                        visible: entry.modelData.separator !== true
                            && entry.modelData.header !== true
                            && !entry.modelData.submenu
                            && !!entry.modelData.shortcut
                        anchors.right: parent.right
                        anchors.rightMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.shortcut || ""
                        color: Theme.inkFaint
                        font.pixelSize: 11
                    }

                    Glyph {
                        visible: entry.modelData.separator !== true
                            && entry.modelData.header !== true
                            && entry.modelData.submenu !== undefined
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        width: 12
                        height: 12
                        kind: "chevron"
                        tint: Theme.inkMuted
                    }

                    MouseArea {
                        id: hover
                        anchors.fill: parent
                        enabled: entry.modelData.separator !== true
                            && entry.modelData.header !== true
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
        radius: 12
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

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
                    height: submenuEntry.modelData.separator === true
                        ? 9 : (submenuEntry.modelData.header === true ? 20 : 27)

                    Rectangle {
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 6
                        color: submenuHover.containsMouse
                            ? Theme.fill
                            : "transparent"
                    }

                    Image {
                        visible: submenuEntry.modelData.separator !== true
                            && !!submenuEntry.modelData.icon
                            && status === Image.Ready
                        x: 12
                        width: 16
                        height: 16
                        anchors.verticalCenter: parent.verticalCenter
                        source: submenuEntry.modelData.icon || ""
                        fillMode: Image.PreserveAspectFit
                        asynchronous: true
                        cache: true
                    }

                    Glyph {
                        visible: submenuEntry.modelData.separator !== true
                            && submenuEntry.modelData.header !== true
                            && submenuEntry.modelData.checked === true
                            && !submenuEntry.modelData.icon
                        x: 12
                        width: 14
                        height: 14
                        anchors.verticalCenter: parent.verticalCenter
                        kind: "check"
                        tint: Theme.interactive
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: (submenuEntry.modelData.icon
                            || submenuEntry.modelData.checked === true) ? 34 : 12
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: submenuEntry.modelData.label || ""
                        elide: Text.ElideRight
                        color: submenuEntry.modelData.header === true
                            ? Theme.inkFaint
                            : (submenuEntry.modelData.enabled === false
                                ? Theme.inkFaint
                                : Theme.ink)
                        font.pixelSize: submenuEntry.modelData.header === true
                            ? Theme.typeMicro : Theme.typeSmall
                        font.weight: submenuEntry.modelData.header === true
                            ? Font.Medium : Font.Normal
                    }

                    MouseArea {
                        id: submenuHover
                        anchors.fill: parent
                        enabled: submenuEntry.modelData.separator !== true
                            && submenuEntry.modelData.header !== true
                            && submenuEntry.modelData.enabled !== false
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
