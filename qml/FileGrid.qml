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
    readonly property int minCellWidth: 100
    readonly property int tileHeight: 132
    readonly property int thumbHeight: 76
    readonly property int glyphSize: 38

    readonly property int columns: {
        const available = Math.max(0, width - padding * 2)
        return Math.max(1, Math.floor((available + gapX) / (minCellWidth + gapX)))
    }
    readonly property real tileWidth: {
        const available = Math.max(0, width - padding * 2 - gapX * (columns - 1))
        return Math.max(72, available / Math.max(1, columns))
    }

    function selectedPathList() {
        const paths = []
        if (tab === null)
            return paths
        for (const key in tab.selected) {
            if (tab.selected[key])
                paths.push(key)
        }
        return paths
    }

    function handleClick(index, ctrl, shift) {
        if (tab === null || model === null)
            return
        grid.forceActiveFocus()
        grid.currentIndex = index
        const path = model.pathAt(index)
        if (path === "")
            return

        if (shift && tab.selectionAnchor !== "") {
            const anchor = model.indexOfPath(tab.selectionAnchor)
            if (anchor >= 0) {
                const first = Math.min(anchor, index)
                const last = Math.max(anchor, index)
                const paths = []
                for (let row = first; row <= last; row += 1) {
                    const value = model.pathAt(row)
                    if (value !== "")
                        paths.push(value)
                }
                tab.selectMany(paths)
                root.selectionChanged()
                return
            }
        }

        if (ctrl)
            tab.toggle(path)
        else
            tab.selectOnly(path)
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
    property var marqueeBase: []
    property real marqueePointerY: 0

    function marqueeRect() {
        return {
            left: Math.min(marqueeStartX, marqueeX),
            top: Math.min(marqueeStartY, marqueeY),
            right: Math.max(marqueeStartX, marqueeX),
            bottom: Math.max(marqueeStartY, marqueeY)
        }
    }

    function marqueePaths() {
        if (model === null)
            return []
        const rect = marqueeRect()
        const stepX = tileWidth + gapX
        const stepY = tileHeight + gapY
        const firstRow = Math.max(0, Math.floor(rect.top / stepY))
        const lastRow = Math.max(0, Math.floor(rect.bottom / stepY))
        const firstColumn = Math.max(0, Math.floor(rect.left / stepX))
        const lastColumn = Math.min(columns - 1, Math.max(0, Math.floor(rect.right / stepX)))

        const paths = []
        for (let row = firstRow; row <= lastRow; row += 1) {
            for (let column = firstColumn; column <= lastColumn; column += 1) {
                const index = row * columns + column
                const path = model.pathAt(index)
                if (path === "")
                    continue
                const cellLeft = column * stepX
                const cellTop = row * stepY
                const cellRight = cellLeft + tileWidth
                const cellBottom = cellTop + tileHeight
                if (cellRight >= rect.left && cellLeft <= rect.right
                        && cellBottom >= rect.top && cellTop <= rect.bottom)
                    paths.push(path)
            }
        }
        return paths
    }

    function applyMarquee() {
        if (!marqueeActive || tab === null)
            return
        const matched = marqueePaths()
        const paths = marqueeBase.length > 0
            ? Array.from(new Set(marqueeBase.concat(matched))) : matched
        tab.selectMany(paths)
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
        color: Theme.canvas
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
        reuseItems: true
        cacheBuffer: 1800
        boundsBehavior: Flickable.StopAtBounds
        currentIndex: -1

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

            width: grid.cellWidth - root.gapX
            height: grid.cellHeight - root.gapY

            readonly property bool selected: root.tab !== null && root.tab.isSelected(cell.filePath)
            readonly property bool dropTarget: root.dropTargetPath !== "" && cell.isDir
                && cell.filePath === root.dropTargetPath
            readonly property bool current: grid.currentIndex === cell.index
            readonly property bool focusedCursor: grid.activeFocus
                && root.keyboardCursor && cell.current
            readonly property bool hasThumbnail: cell.thumbnailSource !== ""
            readonly property bool showImage: cell.hasThumbnail
                && thumbImage.status === Image.Ready

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
                    color: Theme.canvasAlt
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
                        ? Theme.inkMuted : Theme.inkFaint
                }
            }

            Text {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                anchors.top: thumb.bottom
                anchors.topMargin: 7
                text: cell.fileName
                color: cell.hidden ? Theme.inkFaint : Theme.ink
                font.pixelSize: Theme.typeBody
                lineHeight: 1.25
                horizontalAlignment: Text.AlignHCenter
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
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
                    if (root.tab === null || root.model === null || root.model.remote)
                        return
                    const selected = root.tab.isSelected(cell.filePath)
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
                        if (root.tab === null || !root.tab.isSelected(cell.filePath))
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
            root.marqueeBase = (mouse.modifiers & Qt.ControlModifier) !== 0
                ? root.selectedPathList() : []
            root.marqueeStartX = mouse.x - root.padding + grid.contentX
            root.marqueeStartY = mouse.y - root.padding + grid.contentY
            root.marqueeX = root.marqueeStartX
            root.marqueeY = root.marqueeStartY
            root.marqueePointerY = mouse.y
            root.marqueeActive = true
            if (root.marqueeBase.length === 0 && root.tab !== null) {
                root.tab.clearSelection()
                root.selectionChanged()
            }
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
            if (width < 4 && height < 4
                    && root.marqueeBase.length === 0 && root.tab !== null) {
                root.tab.clearSelection()
                root.selectionChanged()
            }
        }

        onCanceled: root.marqueeActive = false

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
