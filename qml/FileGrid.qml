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
    /// The sheet width: twice the glyph preference, which puts drift's 76 dp
    /// card on the 38 dp default and lets the icon slider resize the drawing
    /// rather than swapping in a different one.
    readonly property int sheetWidth: Math.max(44, Math.min(96, Math.round(glyphSize * 2)))
    /// The sheet's box follows its own proportions (4:3 for a folder, 3:4 for
    /// a card), so the column rhythm is the same at every icon size.
    readonly property int thumbHeight: Math.round(sheetWidth * 4 / 3)
    /// 10 dp of air above the sheet, the sheet, 6 dp, then the name plate
    /// (two name lines plus the size line) and the tag row. The cell has to
    /// cover all of it: a plate that overflows its cell paints the next row's
    /// size line above that row's sheet.
    readonly property int tileHeight: thumbHeight + 90
    readonly property int minCellWidth: Math.max(100, ExplorerPrefs.iconSize + 40)

    readonly property int columns: {
        const available = Math.max(0, width - padding * 2)
        return Math.max(1, Math.floor((available + gapX) / (minCellWidth + gapX)))
    }
    readonly property real tileWidth: {
        const available = Math.max(0, width - padding * 2 - gapX * columns)
        return Math.max(72, available / Math.max(1, columns))
    }
    /// The tiles `GridView` actually fits per row. Every cell carries its own
    /// trailing gap, so the width reserved above has to account for `columns`
    /// gaps, not `columns - 1`; otherwise the row loses a whole column to the
    /// leftover. A marquee addresses rows by this number, not by `columns`,
    /// or a band selects the row below the one under the pointer.
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

    /// The listing asks the visible surface for its scroll offset and the
    /// index under a point when it fetches tags. Without these two the grid
    /// threw a TypeError the first time a tagged directory was shown. The
    /// caller's point is inset from the pane's left edge, so add the padding
    /// back before hitting the grid's own content coordinates.
    readonly property real contentY: grid.contentY
    function indexAt(x, y) {
        return grid.indexAt(x + root.padding, y)
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
        Accessible.ignored: true
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
            required property string sizeText

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

            /// A drop target is the only thing that gets a frame; selection is
            /// carried by the ink plate under the name, not by tinting the
            /// cell. The frame is dashed so it cannot be mistaken for the
            /// solid ring the keyboard cursor draws.
            Rectangle {
                anchors.fill: parent
                visible: cell.dropTarget
                color: Theme.signalSoft
            }

            DashFrame {
                anchors.fill: parent
                visible: cell.dropTarget
            }

            Rectangle {
                anchors.fill: parent
                visible: cell.focusedCursor && !cell.dropTarget
                color: "transparent"
                border.width: Theme.edgeWidth
                border.color: Theme.signal
            }

            Item {
                id: thumb

                /// The sheet rises a little under the pointer, the way a card
                /// does when it is picked up. Selection leaves it down, because
                /// the ink plate below already says which tile is chosen.
                readonly property bool lifted: hover.containsMouse && !cell.selected

                anchors.top: parent.top
                anchors.topMargin: 10 - (lifted ? 3 : 0)
                anchors.horizontalCenter: parent.horizontalCenter
                width: root.sheetWidth
                height: Math.round(width * (cell.isDir ? 96 / 120 : 120 / 90))

                Behavior on anchors.topMargin {
                    enabled: Theme.motion
                    NumberAnimation { duration: Theme.motionFast; easing.type: Easing.OutCubic }
                }

                Rectangle {
                    anchors.fill: parent
                    visible: cell.showImage
                    color: Theme.contentAlt
                    border.width: Theme.edgeWidth
                    border.color: Theme.hairlineStrong
                }

                Image {
                    id: thumbImage
                    anchors.fill: parent
                    visible: cell.showImage
                    source: cell.hasThumbnail ? cell.thumbnailSource : ""
                    sourceSize.width: Math.round(128 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(128 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    smooth: true
                    cache: true
                    Accessible.ignored: true
                }

                DriftSheet {
                    anchors.fill: parent
                    visible: !cell.showImage
                    category: cell.category
                    extension: cell.extension
                    isDir: cell.isDir
                    name: cell.fileName
                }
            }

            /// The name plate. Selection is a solid ink block behind the label
            /// with the paper showing through the type — drift's one way of
            /// marking a chosen row, and the reason the tile needs no tinted
            /// card behind the sheet.
            Rectangle {
                id: namePlate
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 2
                anchors.rightMargin: 2
                /// Anchored to the cell, not to the sheet above it: a folder
                /// sheet is 4:3 and a card is 3:4, so anchoring to the sheet
                /// would put the two names on different lines inside one row.
                anchors.top: parent.top
                anchors.topMargin: 10 + root.thumbHeight + 6
                /// The plate covers the name only — the two lines it may
                /// wrap to — and the size line below it stays on the paper.
                /// A plate swallowed the size line it would read as a black
                /// bar with a file name somewhere inside it.
                height: Math.ceil(Theme.typeBody * Theme.leadingTight * 2) + 8
                color: cell.selected ? Theme.ink : "transparent"
            }

            /// The label flows: name, then size directly under it. The plate
            /// around it has a fixed height, so a one-line name and a two-line
            /// name still leave every tile's sheet on the same row — the
            /// difference lands in the empty space below the size line rather
            /// than in the next row's position.
            Column {
                id: label
                anchors.top: namePlate.top
                anchors.topMargin: 4
                anchors.left: namePlate.left
                anchors.right: namePlate.right
                spacing: 1

                Text {
                    id: nameText
                    width: parent.width
                    text: {
                        if (ExplorerPrefs.showExtensions || cell.extension === "")
                            return cell.fileName
                        const suffix = "." + cell.extension
                        return cell.fileName.endsWith(suffix)
                            ? cell.fileName.slice(0, -suffix.length) : cell.fileName
                    }
                    color: cell.selected ? Theme.canvas
                        : cell.hidden ? Theme.inkFaint : Theme.ink
                    font.pixelSize: Theme.typeBody
                    lineHeight: Theme.leadingTight
                    horizontalAlignment: Text.AlignHCenter
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }

                Text {
                    visible: text !== ""
                    width: parent.width
                    text: cell.isDir ? "" : cell.sizeText
                    color: Theme.inkMuted
                    opacity: 0.8
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    horizontalAlignment: Text.AlignHCenter
                }
            }

            Row {
                id: tagRow
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: namePlate.bottom
                anchors.topMargin: 3
                spacing: 3

                Repeater {
                    model: cell.tagColors.length > 0 ? cell.tagColors.split(",") : []

                    Rectangle {
                        required property string modelData
                        width: 6
                        height: 6
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
        /// A rubber band, not a control: it draws the marquee for a drag that
        /// began on empty space and has no keyboard equivalent of its own.
        Accessible.ignored: true
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
