pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0

// The grid is the same listing with its rows laid out as tiles.
//
// It used to be a field of cards: a plate per file, a well for the glyph and a
// second line of metadata. The card carried no information the name did not,
// and forty of them read as forty buttons. A tile is now a glyph and a name on
// the canvas, the way an instrument's channel strip is a label rather than a
// box; selection supplies the only plate.
//
// QML has no CSS `auto-fill`, so the column count is computed from the pane
// width: `floor((width - 32 + 12) / (100 + 12))`, then the remainder is split
// evenly. A resize therefore changes the column count instead of stretching
// five fixed columns.
Item {
    id: root

    property var tab: null
    property var model: null
    property string dropTargetPath: ""
    property var keyHandler: null
    /// True while a key press owns the cursor. The focus ring is drawn only
    /// for keyboard movement, never for a click.
    property bool keyboardCursor: false

    signal activateRequested(string path, bool isDir)
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal dragOutRequested(var paths, string mode)

    readonly property int padding: 16
    readonly property int gapX: 12
    readonly property int gapY: 14
    /// One icon-size preference drives the tile, the thumbnail well and the
    /// minimum cell width, so the slider cannot make a 96 px glyph overflow a
    /// tile sized for 24.
    readonly property int glyphSize: ExplorerPrefs.iconSize
    readonly property int thumbHeight: Math.max(56, ExplorerPrefs.iconSize + 24)
    readonly property int tileHeight: thumbHeight + 56
    readonly property int minCellWidth: Math.max(96, ExplorerPrefs.iconSize + 40)

    readonly property int columns: {
        const available = Math.max(0, width - padding * 2)
        return Math.max(1, Math.floor((available + gapX) / (minCellWidth + gapX)))
    }
    readonly property real tileWidth: {
        const available = Math.max(0, width - padding * 2 - gapX * (columns - 1))
        return Math.max(72, available / Math.max(1, columns))
    }
    /// The tiles `GridView` actually fits per row. It flows on `cellWidth`,
    /// which is `tileWidth + gapX`, so the even split above leaves a remainder
    /// and fits one fewer than `columns` whenever that remainder is smaller
    /// than the gap. A marquee has to address rows by this number, not by
    /// `columns`, or a band selects the row below the one under the pointer.
    readonly property int flowColumns: Math.max(1, Math.floor(
        Math.max(0, width - padding * 2) / (tileWidth + gapX)))

    function selectedPathList() {
        return model !== null ? model.selectedPaths() : []
    }

    function handleClick(index, ctrl, shift) {
        if (model === null)
            return
        grid.forceActiveFocus()
        grid.currentIndex = index
        const path = model.pathAt(index)
        if (path === "")
            return

        if (shift && tab !== null && tab.selectionAnchor !== "") {
            const anchor = model.indexOfPath(tab.selectionAnchor)
            if (anchor >= 0) {
                // One call: the model sets the flags for the whole range.
                model.selectRange(Math.min(anchor, index), Math.max(anchor, index))
                root.selectionChanged()
                return
            }
        }

        if (ctrl)
            model.toggleAt(index)
        else
            model.selectOnly(index)
        if (tab !== null)
            tab.selectionAnchor = path
        root.selectionChanged()
    }

    function activate(index) {
        if (model === null)
            return
        const path = model.pathAt(index)
        if (path === "")
            return
        root.activateRequested(path, model.isDirectoryAt(index))
    }

    function currentPath() {
        if (model === null || grid.currentIndex < 0)
            return ""
        return model.pathAt(grid.currentIndex)
    }

    function cursorIndex() {
        return grid.currentIndex
    }

    function setCursor(index) {
        grid.currentIndex = index
    }

    function takeFocus() {
        grid.forceActiveFocus()
    }

    /// The folder under a drop point: a directory tile is the target,
    /// anything else means the directory on screen.
    function dropDirectoryAt(x, y) {
        if (model === null || model.path === "")
            return ""
        const index = grid.indexAt(x - padding, grid.contentY + y - padding)
        if (index >= 0 && model.isDirectoryAt(index))
            return model.pathAt(index)
        return model.path
    }

    function positionCursor(index) {
        if (index < 0)
            return
        grid.currentIndex = index
        grid.positionViewAtIndex(index, GridView.Contain)
    }

    // ---- marquee ---------------------------------------------------------
    // All coordinates below are content coordinates: the pointer position plus
    // the scroll offset, minus the inner padding. Keeping one coordinate space
    // is what makes the band agree with the tiles while the view auto-scrolls.
    property bool marqueeActive: false
    property real marqueeStartX: 0
    property real marqueeStartY: 0
    property real marqueeX: 0
    property real marqueeY: 0
    /// Ctrl was held when the drag began: the band extends the selection
    /// rather than replacing it. The model snapshots the base itself.
    property bool marqueeKept: false
    property real marqueePointerY: 0
    /// The cell range the last move applied, so a pointer that stays inside the
    /// same band does not rebuild the whole selection.
    property int appliedFirstIndex: -2
    property int appliedLastIndex: -2

    function marqueeRect() {
        return {
            left: Math.min(marqueeStartX, marqueeX),
            top: Math.min(marqueeStartY, marqueeY),
            right: Math.max(marqueeStartX, marqueeX),
            bottom: Math.max(marqueeStartY, marqueeY)
        }
    }

    /// The tiles the band can touch, clipped to the rows that exist, or `null`
    /// when it touches none. A column is in the band only when its tile
    /// reaches it: the band's edge can land in the gap between two tiles, and
    /// that gap belongs to neither.
    function marqueeRange() {
        if (model === null || model.loaded <= 0)
            return null
        const rect = marqueeRect()
        const stepX = tileWidth + gapX
        const stepY = tileHeight + gapY
        const rows = Math.ceil(model.loaded / flowColumns)
        const firstRow = Math.max(0, Math.ceil((rect.top - tileHeight) / stepY))
        const lastRow = Math.min(rows - 1, Math.floor(rect.bottom / stepY))
        if (lastRow < firstRow)
            return null
        const firstColumn = Math.max(0, Math.ceil((rect.left - tileWidth) / stepX))
        const lastColumn = Math.min(flowColumns - 1, Math.floor(rect.right / stepX))
        if (lastColumn < firstColumn)
            return null
        const first = firstRow * flowColumns + firstColumn
        const last = Math.min(model.loaded - 1, lastRow * flowColumns + lastColumn)
        if (last < first)
            return null
        return {
            firstRow: firstRow,
            lastRow: lastRow,
            firstColumn: firstColumn,
            lastColumn: lastColumn,
            first: first,
            last: last
        }
    }

    function applyMarquee() {
        if (!marqueeActive || model === null)
            return
        const range = marqueeRange()
        const first = range === null ? -1 : range.first
        const last = range === null ? -1 : range.last
        if (first === appliedFirstIndex && last === appliedLastIndex)
            return
        appliedFirstIndex = first
        appliedLastIndex = last
        // One call for the whole rectangle, in the model's own cell grid; the
        // base the drag started from lives there too.
        if (range === null)
            model.marqueeCells(-1, -1, -1, -1, flowColumns)
        else
            model.marqueeCells(range.firstRow, range.firstColumn,
                               range.lastRow, range.lastColumn, flowColumns)
        root.selectionChanged()
    }

    Timer {
        interval: 16
        repeat: true
        running: root.marqueeActive
        onTriggered: {
            const margin = 28
            if (root.marqueePointerY < margin)
                grid.contentY = Math.max(0, grid.contentY - 12)
            else if (root.marqueePointerY > grid.height - margin)
                grid.contentY = Math.max(0, Math.min(grid.contentHeight - grid.height,
                                                     grid.contentY + 12))
            else
                return
            root.marqueeY = root.marqueePointerY - root.padding + grid.contentY
            root.applyMarquee()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.content
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.RightButton
        onPressed: function (mouse) {
            const point = mapToItem(null, mouse.x, mouse.y)
            root.emptyContextRequested(point.x, point.y)
        }
    }

    GridView {
        id: grid

        anchors.fill: parent
        anchors.leftMargin: root.padding
        anchors.topMargin: root.padding
        anchors.rightMargin: root.padding
        anchors.bottomMargin: root.padding
        cellWidth: root.tileWidth + root.gapX
        cellHeight: root.tileHeight + root.gapY
        leftMargin: 0
        topMargin: 0
        model: root.model
        clip: true
        focus: true
        Keys.onPressed: function (event) {
            root.keyboardCursor = true
            if (root.keyHandler !== null && root.keyHandler(event))
                event.accepted = true
        }

        delegate: Item {
            id: cell

            required property int index
            required property string fileName
            required property string filePath
            required property bool isDir
            required property bool hidden
            required property string iconSource
            required property string category
            required property string extension
            required property string thumbnailSource
            required property string tagColors

            width: grid.cellWidth - root.gapX
            height: grid.cellHeight - root.gapY

            required property bool selected
            readonly property bool dropTarget: root.dropTargetPath !== "" && cell.isDir
                && cell.filePath === root.dropTargetPath
            readonly property bool current: grid.currentIndex === cell.index
            readonly property bool focusedCursor: grid.activeFocus
                && root.keyboardCursor && cell.current
            readonly property bool hasThumbnail: cell.thumbnailSource !== ""
            readonly property bool showImage: cell.hasThumbnail
                && thumbImage.status === Image.Ready

            /// A tile is one row of the grid to a reader, even though it has
            /// two lines of its own: the name is the name, and press opens the
            /// item the way a double click does.
            Accessible.role: Accessible.ListItem
            Accessible.name: cell.fileName
            Accessible.description: cell.isDir ? qsTr("文件夹") : qsTr("文件")
            Accessible.selectable: true
            Accessible.selected: cell.selected
            Accessible.onPressAction: root.activate(cell.index)

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusSm
                color: cell.selected
                    ? (hover.containsMouse ? Theme.signalSoft2 : Theme.signalSoft)
                    : cell.dropTarget ? Theme.signalSoft
                    : (hover.containsMouse ? Theme.hover : "transparent")
                border.width: cell.selected || cell.dropTarget || cell.focusedCursor ? 1 : 0
                border.color: Theme.signal
            }

            Item {
                id: thumb

                anchors.top: parent.top
                anchors.topMargin: 8
                anchors.horizontalCenter: parent.horizontalCenter
                width: Math.min(parent.width - 8, root.tileWidth - 8)
                height: root.thumbHeight

                Rectangle {
                    anchors.fill: parent
                    visible: cell.hasThumbnail
                    radius: Theme.radiusXs
                    color: Theme.contentAlt
                    border.width: 1
                    border.color: Theme.hairline
                }

                Image {
                    id: thumbImage
                    anchors.fill: parent
                    anchors.margins: 8
                    visible: cell.showImage
                    source: cell.hasThumbnail ? cell.thumbnailSource : ""
                    sourceSize.width: Math.round(128 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(128 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    smooth: true
                    cache: true
                    Accessible.ignored: true
                }

                ThemedIcon {
                    anchors.centerIn: parent
                    visible: !cell.showImage
                    category: cell.category
                    extension: cell.extension
                    isDir: cell.isDir
                    size: root.glyphSize
                    tint: cell.selected || hover.containsMouse
                        ? Theme.ink : Theme.inkMuted
                }
            }

            Text {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                anchors.top: thumb.bottom
                anchors.topMargin: 7
                text: {
                    if (ExplorerPrefs.showExtensions || cell.extension === "")
                        return cell.fileName
                    const suffix = "." + cell.extension
                    return cell.fileName.endsWith(suffix)
                        ? cell.fileName.slice(0, -suffix.length) : cell.fileName
                }
                color: cell.hidden ? Theme.inkFaint : Theme.ink
                font.pixelSize: Theme.typeBody
                lineHeight: Theme.leadingTight
                horizontalAlignment: Text.AlignHCenter
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: thumb.bottom
                anchors.topMargin: 44
                spacing: 3

                Repeater {
                    model: cell.tagColors.length > 0 ? cell.tagColors.split(",") : []

                    Rectangle {
                        required property string modelData
                        width: 6
                        height: 6
                        radius: 3
                        color: modelData
                    }
                }
            }

            MouseArea {
                id: hover
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                cursorShape: Qt.PointingHandCursor

                property bool dragArmed: false
                property bool dragStarted: false
                property real dragStartX: 0
                property real dragStartY: 0
                property var dragPaths: []
                property bool suppressClick: false

                function prepareDrag(mouse) {
                    suppressClick = false
                    dragArmed = false
                    dragStarted = false
                    dragPaths = []
                    if (root.model === null || root.model.remote)
                        return
                    const selected = cell.selected
                    if (!selected && (mouse.modifiers
                                      & (Qt.ControlModifier | Qt.ShiftModifier)) === 0)
                        root.handleClick(cell.index, false, false)
                    dragPaths = selected ? root.selectedPathList() : [cell.filePath]
                    dragStartX = mouse.x
                    dragStartY = mouse.y
                    dragArmed = true
                }

                function continueDrag(mouse) {
                    if (!dragArmed || dragStarted)
                        return
                    if ((mouse.buttons & Qt.LeftButton) === 0) {
                        dragArmed = false
                        return
                    }
                    const dx = mouse.x - dragStartX
                    const dy = mouse.y - dragStartY
                    if (Math.sqrt(dx * dx + dy * dy) < 6)
                        return
                    dragArmed = false
                    dragStarted = true
                    suppressClick = true
                    const paths = dragPaths
                    dragPaths = []
                    if (paths.length > 0)
                        root.dragOutRequested(paths, modeFor(mouse.modifiers))
                }

                function modeFor(modifiers) {
                    const ctrl = (modifiers & Qt.ControlModifier) !== 0
                    const shift = (modifiers & Qt.ShiftModifier) !== 0
                    if ((modifiers & Qt.AltModifier) !== 0 || (ctrl && shift))
                        return "link"
                    if (ctrl)
                        return "copy"
                    if (shift)
                        return "move"
                    return "any"
                }

                onPressed: function (mouse) {
                    root.keyboardCursor = false
                    if (mouse.button === Qt.LeftButton)
                        prepareDrag(mouse)
                }
                onPositionChanged: function (mouse) {
                    continueDrag(mouse)
                }
                onReleased: dragArmed = false
                onCanceled: dragArmed = false
                onClicked: function (mouse) {
                    if (mouse.button === Qt.LeftButton && suppressClick) {
                        suppressClick = false
                        return
                    }
                    if (mouse.button === Qt.RightButton) {
                        if (!cell.selected)
                            root.handleClick(cell.index, false, false)
                        const point = cell.mapToItem(null, mouse.x, mouse.y)
                        root.contextRequested(cell.filePath, cell.isDir, point.x, point.y)
                        return
                    }
                    root.handleClick(cell.index,
                                     (mouse.modifiers & Qt.ControlModifier) !== 0,
                                     (mouse.modifiers & Qt.ShiftModifier) !== 0)
                }
                onDoubleClicked: function (mouse) {
                    if (mouse.button === Qt.LeftButton)
                        root.activate(cell.index)
                }
            }
        }
    }

    /// Rubber-band selection. The overlay sits above the tiles so a drag that
    /// began on empty space keeps receiving moves over them; a press that
    /// began on a tile is handed down instead.
    MouseArea {
        id: marquee
        anchors.fill: parent
        z: 20
        acceptedButtons: Qt.LeftButton
        preventStealing: true

        onPressed: function (mouse) {
            if (root.model === null || root.model.remote) {
                mouse.accepted = false
                return
            }
            const index = grid.indexAt(mouse.x - root.padding,
                                       grid.contentY + mouse.y - root.padding)
            if (index >= 0) {
                mouse.accepted = false
                return
            }
            root.keyboardCursor = false
            grid.forceActiveFocus()
            root.marqueeKept = (mouse.modifiers & Qt.ControlModifier) !== 0
            root.model.beginMarquee(root.marqueeKept)
            root.marqueeStartX = mouse.x - root.padding + grid.contentX
            root.marqueeStartY = mouse.y - root.padding + grid.contentY
            root.marqueeX = root.marqueeStartX
            root.marqueeY = root.marqueeStartY
            root.marqueePointerY = mouse.y
            root.appliedFirstIndex = -2
            root.appliedLastIndex = -2
            root.marqueeActive = true
            // A plain drag starts from an empty selection, and the press is
            // what clears it — before the first move reaches the model.
            if (!root.marqueeKept)
                root.selectionChanged()
            mouse.accepted = true
        }

        onPositionChanged: function (mouse) {
            if (!root.marqueeActive)
                return
            root.marqueeX = mouse.x - root.padding + grid.contentX
            root.marqueeY = mouse.y - root.padding + grid.contentY
            root.marqueePointerY = mouse.y
            root.applyMarquee()
        }

        onReleased: {
            if (!root.marqueeActive)
                return
            const width = Math.abs(root.marqueeX - root.marqueeStartX)
            const height = Math.abs(root.marqueeY - root.marqueeStartY)
            root.marqueeActive = false
            root.model.endMarquee()
            // A drag that never travelled is a click in the empty space
            // between tiles, which clears the selection.
            if (width < 4 && height < 4 && !root.marqueeKept) {
                root.model.clearSelection()
                root.selectionChanged()
            }
        }

        onCanceled: {
            root.marqueeActive = false
            if (root.model !== null)
                root.model.endMarquee()
        }

        Rectangle {
            visible: root.marqueeActive
            x: Math.min(root.marqueeStartX, root.marqueeX) - grid.contentX + root.padding
            y: Math.min(root.marqueeStartY, root.marqueeY) - grid.contentY + root.padding
            width: Math.abs(root.marqueeX - root.marqueeStartX)
            height: Math.abs(root.marqueeY - root.marqueeStartY)
            color: Theme.signalSoft
            border.width: 1
            border.color: Theme.signal
            radius: Theme.radiusXs
        }
    }
}
