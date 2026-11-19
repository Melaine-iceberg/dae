pragma ComponentBehavior: Bound
import QtQuick

// One menu surface for the whole shell.
//
// The context menu, the view menu and the filter menu used to be three copies
// of the same overlay — three panel radii, three row heights and two border
// colours. §6.5 and §7.5 fix what a menu is: a `raised` plate, a 1 px
// `overlayLine` edge, `md` corners, 28 dp rows, one shadow, one level of
// submenu. This file owns that arrangement; `MenuList` still paints the rows.
//
// A caller fills it one of two ways:
//
//   `actions` — raw entries with ids and submenus. The panel normalizes them
//               and re-emits `actionTriggered(id)`.
//   `rows`    — rows a caller has already normalized against its own
//               vocabulary (the view and filter menus). The panel re-emits
//               `rowTriggered(index)` so the caller can keep that vocabulary.
//
// What every opening also needs, and this file answers:
//
//   - the panel stays inside the window on both axes;
//   - one `footer` band can be pinned under the rows (the icon-size slider,
//     the clear-filter row) without scrolling with them;
//   - Escape closes the submenu first and the menu second;
//   - the panel arrives with opacity plus a 2 dp lift and leaves instantly.
Item {
    id: root

    property var actions: []
    /// Already-normalized rows. Empty means "derive from `actions`".
    property var rows: []
    property real openX: 0
    property real openY: 0
    /// Anchored above the point rather than below it (toolbar menus).
    property bool above: false
    property bool dark: true
    /// A band pinned to the bottom of the panel. The item supplies its own
    /// `implicitHeight`; the panel reads it.
    property Component footer: null

    /// A band pinned to the top of the panel. The filter menu's name box
    /// lives there: typing is not a row that can be chosen, and the rows
    /// scroll under it the way they do over the footer.
    property Component header: null

    property var submenuActions: []
    /// The row whose submenu is open, so `replaceActions` can refresh it.
    property int submenuIndex: -1
    property real submenuX: 0
    property real submenuY: 0

    signal actionTriggered(string id)
    signal rowTriggered(int index)

    visible: false
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        OpacityAnimator { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1000

    /// §7.5: menu rows are 28 dp; the panel's own 4 dp top and bottom padding
    /// sit outside that. Both bands add their own height, which the Loader
    /// mirrors from the item it built.
    readonly property int footerHeight: footerLoader.implicitHeight
    readonly property int headerHeight: headerLoader.implicitHeight

    readonly property var entries: root.rows.length > 0
        ? root.rows : root.normalized(root.actions)

    function show() {
        root.closeSubmenu()
        list.currentIndex = -1
        list.resetScroll()
        root.visible = true
        // Deferred: the click that opened the menu is still being delivered to
        // the view underneath, and that view asks for focus on its way out.
        // Taking it now would be undone a moment later, leaving a menu that
        // answers the pointer but not the keyboard.
        Qt.callLater(list.takeFocus)
    }

    function openAt(x, y, items) {
        root.actions = items || []
        root.openX = x
        root.openY = y
        root.above = false
        root.show()
    }

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.above = true
        root.show()
    }

    function close() {
        root.visible = false
        root.actions = []
        root.closeSubmenu()
    }

    /// Hands the keyboard to the rows, the way `show()` does. The header band
    /// calls it to give the arrow keys back to the menu.
    function focusRows() {
        list.takeFocus()
    }

    /// Swaps the rows without moving the panel: the installed apps' commands
    /// arrive asynchronously, and the menu they belong to is already open.
    /// An open submenu is a snapshot of its parent row, so it is re-read from
    /// the new rows too — the answer may be what it was waiting for.
    function replaceActions(items) {
        root.actions = items
        // Re-anchored, not just refilled: the fresh rows may be wider, and the
        // flip decision was made for the old ones.
        if (root.submenuIndex >= 0)
            root.submenuFor(root.submenuIndex)
    }

    function closeSubmenu() {
        root.submenuIndex = -1
        root.submenuActions = []
        if (root.visible)
            list.takeFocus()
    }

    /// Normalizes a caller's entries — `{ id, label, glyph, icon, checked,
    /// danger, enabled, submenu, separator, header, shortcut }` — into the
    /// shape `MenuList` paints. The index is preserved, so a triggered row maps
    /// back to the caller's array without a second lookup table.
    function normalized(source) {
        const entries = source || []
        const mapped = []
        for (let index = 0; index < entries.length; index += 1) {
            const entry = entries[index]
            mapped.push({
                kind: entry.separator === true ? "separator"
                    : (entry.header === true ? "header" : "item"),
                label: entry.label || "",
                glyph: entry.glyph || "",
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
        root.submenuIndex = index
        root.submenuActions = entry.submenu
    }

    function triggerIndex(index) {
        // `entries` is the painted shape and deliberately drops `id`; the
        // raw source is what knows the verb. A row-based caller has no ids at
        // all, which is how it keeps its own vocabulary.
        const source = root.rows.length > 0 ? root.rows[index] : root.actions[index]
        if (source === undefined || source === null)
            return
        if (source.submenu !== undefined && source.submenu !== null) {
            root.submenuFor(index)
            return
        }
        if (source.id !== undefined && source.id !== null) {
            const id = source.id
            root.close()
            root.actionTriggered(id)
            return
        }
        root.rowTriggered(index)
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
    /// back. Without this, a submenu would be reachable by pointer only.
    Keys.onRightPressed: {
        if (root.submenuActions.length > 0)
            submenuList.takeFocus()
    }

    MouseArea {
        Accessible.ignored: true
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Shadow {
        target: panel
        // The shadow reads `target.y`, so the plate's own arrival lift below
        // moves both together without a second animation.
    }

    Rectangle {
        id: panel

        width: list.naturalWidth
        // Never taller than the window: the rows scroll inside whatever the
        // window can give them.
        height: Math.min(list.contentHeight + 8 + root.footerHeight + root.headerHeight,
                         root.height - 16)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        /// §6.6: the panel lifts the last 2 dp as it arrives, and hiding is
        /// instant, so the offset is only visible on the way in. It rides the
        /// panel's own `y` instead of a `Translate`: the shadow follows
        /// `target.y` for free, and a `YAnimator` can run it on the render
        /// thread. A `Translate` has no animator type.
        y: (root.above
            ? Math.max(8, Math.min(root.openY - height - 6,
                                   root.height - height - 8))
            : Math.max(8, Math.min(root.openY, root.height - height - 8)))
            + (root.visible ? 0 : 2)
        Behavior on y {
            /// The anchor position is settled while the menu is still hidden,
            /// so only the 2 dp arrival lift animates; a reopen elsewhere jumps
            /// rather than flying in from the last spot.
            enabled: Theme.motion && root.visible
            YAnimator { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
        }
        radius: Theme.radiusMd
        color: Theme.raised
        border.width: 1
        border.color: Theme.overlayLine

        // The panel itself is not an input item, so a press on its padding or
        // on a non-selectable row (a separator, a group label) would otherwise
        // reach the window-wide scrim behind it and read as a click outside:
        // the menu closed on release, and a drag that started there closed it
        // too. Swallow presses and wheel here; the rows above still get theirs.
        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            onWheel: {}
        }

        Loader {
            id: headerLoader
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            sourceComponent: root.header
        }

        MenuList {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 4 + root.headerHeight
            // The footer band keeps its place at the bottom; the rows scroll
            // above it rather than sliding under it.
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4 + root.footerHeight
            maxHeight: height
            items: root.entries

            onTriggered: function (index) { root.triggerIndex(index) }
            onMoved: function (index) { root.submenuFor(index) }
            onEscapeRequested: {
                if (root.submenuActions.length > 0)
                    root.closeSubmenu()
                else
                    root.close()
            }
        }

        Loader {
            id: footerLoader
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            sourceComponent: root.footer
        }
    }

    Shadow {
        target: submenuPanel
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
        border.color: Theme.overlayLine

        // The panel itself is not an input item, so a press on its padding or
        // on a non-selectable row (a separator, a group label) would otherwise
        // reach the window-wide scrim behind it and read as a click outside:
        // the menu closed on release, and a drag that started there closed it
        // too. Swallow presses and wheel here; the rows above still get theirs.
        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            onWheel: {}
        }

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
