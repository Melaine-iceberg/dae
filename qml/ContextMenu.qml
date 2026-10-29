pragma ComponentBehavior: Bound
import QtQuick

// A small custom context menu.
//
// Qt Quick Controls would bring a whole style with it for one popup, and the
// menu has to sit inside the frameless window's own coordinate space anyway.
// The overlay covers the window while open, so clicking anywhere else closes
// it — the same behaviour as a native popup.
//
// The rows come from `MenuList`, which every other menu in the shell uses too.
// What this file owns is the two-panel arrangement: the actions, and at most one
// submenu hanging off a row.
Item {
    id: root

    property var actions: []
    property real openX: 0
    property real openY: 0
    property bool dark: true

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
        root.submenuActions = []
        list.currentIndex = -1
        list.resetScroll()
        root.visible = true
        // Deferred: the click that opened the menu is still being delivered to
        // the view underneath, and that view asks for focus on its way out.
        // Taking it now would be undone a moment later, leaving a menu that
        // answers the pointer but not the keyboard.
        Qt.callLater(list.takeFocus)
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
        if (root.visible)
            list.takeFocus()
    }

    /// Normalizes a caller's entries — `{ id, label, icon, checked, danger,
    /// enabled, submenu, separator, header, shortcut }` — into the shape
    /// `MenuList` paints. The index is preserved, so a triggered row maps back
    /// to the caller's array without a second lookup table.
    function normalized(source) {
        const entries = source || []
        const mapped = []
        for (let index = 0; index < entries.length; index += 1) {
            const entry = entries[index]
            mapped.push({
                kind: entry.separator === true ? "separator"
                    : (entry.header === true ? "header" : "item"),
                label: entry.label || "",
                image: entry.icon || "",
                checked: entry.checked === true,
                // A caller that never says `checked` is offering an action, not
                // a switch; carrying the distinction lets the row be announced
                // as checkable only when it really is.
                checkable: entry.checked !== undefined,
                danger: entry.danger === true,
                disabled: entry.enabled === false,
                shortcut: entry.shortcut || "",
                submenu: entry.submenu
            })
        }
        return mapped
    }

    /// Anchors the submenu to a row's right edge, flipping to the left when the
    /// window edge would clip it.
    function submenuFor(index) {
        const entry = root.actions[index]
        if (entry === undefined || entry.submenu === undefined || entry.submenu === null) {
            root.closeSubmenu()
            return
        }
        const row = list.rowItemAt(index)
        if (row === null)
            return
        const point = row.mapToItem(root, 0, 0)
        const right = point.x + row.width
        // Measured before the submenu exists, so the flip decision does not
        // depend on a panel that has not been laid out yet.
        const submenuWidth = list.widthFor(root.normalized(entry.submenu))
        const flip = right + submenuWidth > root.width - 8
        root.submenuX = flip ? point.x - submenuWidth + 4 : right - 4
        root.submenuY = point.y - 5
        root.submenuActions = entry.submenu
    }

    function triggerIndex(index) {
        const entry = root.actions[index]
        if (entry === undefined)
            return
        if (entry.submenu !== undefined && entry.submenu !== null) {
            root.submenuFor(index)
            return
        }
        const id = entry.id
        root.close()
        root.actionTriggered(id)
    }

    function triggerSubmenuIndex(index) {
        const entry = root.submenuActions[index]
        if (entry === undefined)
            return
        const id = entry.id
        root.close()
        root.actionTriggered(id)
    }

    /// → hands the keyboard to an open submenu, ← (or Escape there) hands it
    /// back. Without this, the sort submenu would be reachable by pointer only.
    Keys.onRightPressed: {
        if (root.submenuActions.length > 0)
            submenuList.takeFocus()
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel
        width: list.naturalWidth
        // Never taller than the window: the rows scroll inside whatever the
        // window can give them.
        height: Math.min(list.contentHeight + 8, root.height - 16)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, Math.min(root.openY, root.height - height - 8))
        radius: Theme.radiusMd
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MenuList {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 4
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            maxHeight: height
            items: root.normalized(root.actions)

            onTriggered: function (index) { root.triggerIndex(index) }
            onMoved: function (index) { root.submenuFor(index) }
            onEscapeRequested: {
                if (root.submenuActions.length > 0)
                    root.closeSubmenu()
                else
                    root.close()
            }
        }
    }

    Rectangle {
        id: submenuPanel
        visible: root.submenuActions.length > 0
        width: submenuList.naturalWidth
        height: Math.min(submenuList.contentHeight + 8, root.height - 16)
        x: Math.max(8, Math.min(root.submenuX, root.width - width - 8))
        y: Math.max(8, Math.min(root.submenuY, root.height - height - 8))
        radius: Theme.radiusMd
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MenuList {
            id: submenuList
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 4
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            maxHeight: height
            items: root.normalized(root.submenuActions)

            onTriggered: function (index) { root.triggerSubmenuIndex(index) }
            onEscapeRequested: root.closeSubmenu()
        }
    }
}
