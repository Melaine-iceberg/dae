pragma ComponentBehavior: Bound
import QtQuick

// The rows of a menu.
//
// The shell has four menus — the row/empty/sidebar context menu, the view
// menu, the filter menu and the nav bar's two overflow menus — and they had
// three separate copies of this list, drifted to 27 px rows with a 10 px group
// label, and not one of them could be driven from the keyboard: opening a menu
// and pressing ↓ did nothing.
//
// The list takes normalized entries so each caller keeps its own vocabulary:
//
//     { kind: "item" | "header" | "separator",
//       label, glyph, image, checked, danger, disabled, shortcut, submenu }
//
// `image` is a real file icon (an installed application's), `glyph` is one of
// `Glyph`'s kinds. A row shows one or the other in a 16 px gutter; a checked
// row shows the tick there instead, because a menu that stacks a tick and an
// icon in the same slot has to choose one.
FocusScope {
    id: root

    property var items: []
    property int currentIndex: -1

    signal triggered(int index)
    /// The highlighted row changed, by hover or by key. A host with submenus
    /// opens or closes them here, which is why arrowing down a list behaves
    /// like moving the pointer down it.
    signal moved(int index)
    /// Escape with no submenu open, or with one: the host knows which.
    signal escapeRequested()

    readonly property int rowHeight: 32

    /// The width these rows want: the widest one is gutter + label + chord +
    /// chevron, plus the panel's 4 px padding on both sides. Clamped to the
    /// 212–286 range the design uses — below it a two-word menu looks like a
    /// stamp, above it a long entry turns the menu into a banner and pushes
    /// the pointer away from the list it belongs to.
    function widthFor(entries) {
        const rows = entries || []
        let widest = 0
        for (let index = 0; index < rows.length; index += 1) {
            const entry = rows[index]
            if (entry === undefined || entry === null)
                continue
            if (entry.kind === "separator")
                continue
            labelMeasure.text = entry.label || ""
            if (entry.kind === "header") {
                widest = Math.max(widest,
                    (entry.image ? 28 : 8) + labelMeasure.implicitWidth + 8)
                continue
            }
            let need = entry.image || entry.glyph || entry.checked ? 34 : 12
            need += labelMeasure.implicitWidth + 12
            if (entry.shortcut) {
                chordMeasure.text = entry.shortcut
                need += chordMeasure.implicitWidth + 12
            }
            if (entry.submenu !== undefined && entry.submenu !== null)
                need += 18
            widest = Math.max(widest, need)
        }
        return Math.max(212, Math.min(286, widest + 8))
    }

    /// Two hidden `Text`s used only to measure: setting `.text` and reading
    /// `.implicitWidth` is synchronous, so this stays a pure question — no
    /// rows have to exist before a host can size the panel that will hold
    /// them.
    Text {
        id: labelMeasure
        visible: false
        font.pixelSize: Theme.typeBody
    }

    Text {
        id: chordMeasure
        visible: false
        font.family: Theme.monoFamily
        font.pixelSize: Theme.typeMicro
    }

    /// Measured on every change of `items` instead of in a binding: the
    /// measurement writes into two hidden `Text`s, and a binding that reads
    /// their implicit widths while writing their text is a loop — Qt reports
    /// it and then drops the binding, leaving the panel at a stale width.
    property real naturalWidth: 240
    onItemsChanged: root.recomputeWidth()
    Component.onCompleted: root.recomputeWidth()

    function recomputeWidth() {
        root.naturalWidth = root.widthFor(root.items)
    }

    /// A menu with twenty rows is taller than a window, and that is the normal
    /// case for the view menu rather than the exception. The panel asks for the
    /// height it can actually get, and the rows scroll inside it — clipped rows
    /// with no way to reach them would make the bottom half of every long menu
    /// unreachable from the keyboard and the pointer alike.
    property real maxHeight: Number.POSITIVE_INFINITY

    /// True for a moment after the list scrolls. Scrolling moves the rows under
    /// a stationary pointer, which fires the same position events real motion
    /// does; without this the highlight would jump back to whatever row slid
    /// under the cursor and the arrow keys would appear to do nothing.
    property bool scrollGuard: false

    /// The row the pointer is on, which is not always the highlighted row: the
    /// arrow keys can move the highlight away from a pointer that has stopped
    /// moving, and two filled rows would read as two selections.
    property int hoverIndex: -1

    /// What the rows want, before any clamp: the host sizes its panel from
    /// this, so the bindings never chase the panel's own height.
    readonly property real contentHeight: column.implicitHeight

    implicitHeight: Math.min(root.contentHeight, root.maxHeight)
    implicitWidth: root.naturalWidth

    function isSelectable(index) {
        const entry = root.items[index]
        if (entry === undefined || entry === null)
            return false
        if (entry.kind === "header" || entry.kind === "separator")
            return false
        return entry.disabled !== true && entry.enabled !== false
    }

    function firstSelectable() {
        for (let index = 0; index < root.items.length; index += 1) {
            if (root.isSelectable(index))
                return index
        }
        return -1
    }

    function moveCurrent(delta) {
        if (root.items.length === 0)
            return
        let index = root.currentIndex
        for (let step = 0; step < root.items.length; step += 1) {
            index = (index + delta + root.items.length) % root.items.length
            if (root.isSelectable(index)) {
                root.hoverIndex = -1
                root.currentIndex = index
                root.moved(index)
                root.revealCurrent()
                return
            }
        }
    }

    function activateCurrent() {
        if (root.isSelectable(root.currentIndex))
            root.triggered(root.currentIndex)
    }

    /// The delegate for a row, so a host can anchor a submenu to it. Rows are
    /// not a uniform height (a separator is 9, a group label 22), so the index
    /// cannot be multiplied by a constant.
    function rowItemAt(index) {
        return rowRepeater.itemAt(index)
    }

    /// Called by the host right after it becomes visible, or when the pointer
    /// leaves a submenu. Focus lands on the first row that can be run, not on
    /// the panel, so Enter works immediately; an already-valid highlight is
    /// kept, because hovering a row must not send the highlight back to the
    /// top of the list.
    function takeFocus() {
        root.forceActiveFocus()
        if (!root.isSelectable(root.currentIndex))
            root.currentIndex = root.firstSelectable()
        root.revealCurrent()
    }

    /// Back to the first row. The panels are reused between openings, so
    /// without this a menu opened near the window's bottom edge would start
    /// scrolled to wherever the last one left off.
    function resetScroll() {
        flick.contentY = 0
    }

    /// Scrolls the highlighted row into view. Only the keyboard needs this:
    /// a row reached by the pointer is already on screen. A zero height means
    /// the panel has not been laid out yet, and scrolling then would leave the
    /// list one row off before it ever appeared.
    function revealCurrent() {
        if (flick.height <= 0)
            return
        const row = rowRepeater.itemAt(root.currentIndex)
        if (row === null)
            return
        if (row.y < flick.contentY)
            flick.contentY = row.y
        else if (row.y + row.height > flick.contentY + flick.height)
            flick.contentY = row.y + row.height - flick.height
    }

    focus: true

    Keys.onUpPressed: function (event) {
        root.moveCurrent(-1)
        event.accepted = true
    }
    Keys.onDownPressed: function (event) {
        root.moveCurrent(1)
        event.accepted = true
    }
    Keys.onPressed: function (event) {
        if (event.key === Qt.Key_Home) {
            root.hoverIndex = -1
            root.currentIndex = root.firstSelectable()
            root.moved(root.currentIndex)
            root.revealCurrent()
            event.accepted = true
        } else if (event.key === Qt.Key_End) {
            root.hoverIndex = -1
            for (let index = root.items.length - 1; index >= 0; index -= 1) {
                if (root.isSelectable(index)) {
                    root.currentIndex = index
                    root.moved(index)
                    root.revealCurrent()
                    break
                }
            }
            event.accepted = true
        } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            root.activateCurrent()
            event.accepted = true
        } else if (event.key === Qt.Key_Escape) {
            root.escapeRequested()
            event.accepted = true
        }
    }

    Timer {
        id: scrollGuardTimer
        interval: 140
        onTriggered: root.scrollGuard = false
    }

    Flickable {
        id: flick

        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        interactive: root.contentHeight > root.height

        onContentYChanged: {
            root.scrollGuard = true
            root.hoverIndex = -1
            scrollGuardTimer.restart()
        }

        Column {
            id: column

            width: flick.width

            Repeater {
                id: rowRepeater

                model: root.items

                delegate: Item {
                    id: entry

                    required property int index
                    required property var modelData

                    readonly property bool separator: modelData.kind === "separator"
                    readonly property bool header: modelData.kind === "header"
                    readonly property bool selectable: root.isSelectable(entry.index)
                    readonly property bool current: root.currentIndex === entry.index
                    readonly property bool hasImage: !separator && !header
                        && !!modelData.image
                    readonly property bool hasGlyph: !separator && !header
                        && !!modelData.glyph && !modelData.checked && !hasImage
                    readonly property bool hasCheck: !separator && !header
                        && modelData.checked === true && !hasImage

                    width: column.width
                    height: separator ? 9 : (header ? 22 : root.rowHeight)

                    // The rule that splits groups: 1 px of hairline with 4 px of
                    // air on each side, inset from the panel's own padding.
                    Rectangle {
                        visible: entry.separator
                        anchors.verticalCenter: parent.verticalCenter
                        x: 8
                        width: Math.max(0, parent.width - 16)
                        height: 1
                        color: Theme.hairline
                    }

                    // A group label. The application groups in the shell-command
                    // submenu carry the app's own icon, so the slot is not always
                    // empty even at 11 px.
                    Item {
                        visible: entry.header
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4

                        Image {
                            visible: entry.header && !!entry.modelData.image
                            x: 8
                            width: 14
                            height: 14
                            anchors.verticalCenter: parent.verticalCenter
                            source: entry.modelData.image || ""
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                            cache: true
                        }

                        Text {
                            x: entry.modelData.image ? 28 : 8
                            width: parent.width - x - 8
                            anchors.verticalCenter: parent.verticalCenter
                            text: entry.modelData.label || ""
                            color: Theme.inkFaint
                            font.pixelSize: Theme.typeMicro
                            font.weight: Font.Medium
                            font.letterSpacing: 0.5
                            elide: Text.ElideRight
                        }
                    }

                    Rectangle {
                        visible: entry.selectable
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: Theme.radiusSm
                        color: entry.current || root.hoverIndex === entry.index
                            ? Theme.fill : "transparent"
                    }

                    Image {
                        visible: entry.hasImage
                        x: 12
                        width: 16
                        height: 16
                        anchors.verticalCenter: parent.verticalCenter
                        source: entry.hasImage ? (entry.modelData.image || "") : ""
                        fillMode: Image.PreserveAspectFit
                        asynchronous: true
                        cache: true
                    }

                    Glyph {
                        visible: entry.hasGlyph
                        x: 12
                        width: 14
                        height: 14
                        anchors.verticalCenter: parent.verticalCenter
                        kind: entry.modelData.glyph || ""
                        tint: entry.modelData.danger === true ? Theme.danger
                                                              : Theme.inkMuted
                    }

                    Glyph {
                        visible: entry.hasCheck
                        x: 12
                        width: 14
                        height: 14
                        anchors.verticalCenter: parent.verticalCenter
                        kind: "check"
                        tint: Theme.ink
                    }

                    Text {
                        visible: !entry.separator && !entry.header
                        anchors.left: parent.left
                        anchors.leftMargin: entry.hasImage || entry.hasGlyph || entry.hasCheck
                            ? 34 : 12
                        anchors.right: chord.visible ? chord.left : parent.right
                        anchors.rightMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.label || ""
                        color: {
                            if (entry.modelData.disabled === true
                                    || entry.modelData.enabled === false)
                                return Theme.inkFaint
                            if (entry.modelData.danger === true)
                                return Theme.danger
                            return Theme.ink
                        }
                        font.pixelSize: Theme.typeBody
                        elide: Text.ElideRight
                    }

                    Text {
                        id: chord

                        visible: !entry.separator && !entry.header
                            && !!entry.modelData.shortcut
                        anchors.right: submenuMark.visible ? submenuMark.left
                                                           : parent.right
                        anchors.rightMargin: submenuMark.visible ? 6 : 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.shortcut || ""
                        color: Theme.inkFaint
                        font.family: Theme.monoFamily
                        font.pixelSize: Theme.typeMicro
                    }

                    Glyph {
                        id: submenuMark

                        visible: !entry.separator && !entry.header
                            && entry.modelData.submenu !== undefined
                            && entry.modelData.submenu !== null
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
                        enabled: entry.selectable
                        hoverEnabled: true
                        cursorShape: entry.selectable ? Qt.PointingHandCursor
                                                      : Qt.ArrowCursor

                        // Motion, not entry: a row sliding under a
                        // stationary pointer — because the keyboard scrolled
                        // the list — must not steal the highlight back from
                        // the row the reader is steering with the arrow keys.
                        onPositionChanged: {
                            if (root.scrollGuard)
                                return
                            root.hoverIndex = entry.index
                            root.currentIndex = entry.index
                            root.moved(entry.index)
                        }

                        onClicked: root.triggered(entry.index)
                    }
                }
            }
        }
    }
}
