pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0

// Card grid over the same listing model the detail list uses.
//
// Geometry follows the density preference: a compact cell is 120x126, a
// comfortable one 148x142, a spacious one 172x150, and the number of columns
// is whatever fits the pane. The cell is a plate rather than a bare label so a
// folder of forty files reads as a field of objects (the same grammar the
// Tauri shell's `file-grid-view.tsx` uses).
Item {
    id: root

    property var tab: null
    property var model: null
    property bool dark: true
    property color accent: Theme.interactive
    property color contentColor: Theme.canvas
    property color cardColor: Theme.canvasAlt
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline
    property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b, dark ? 0.30 : 0.18)
    property string dropTargetPath: ""
    property var keyHandler: null

    signal activateRequested(string path, bool isDir)
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal dragOutRequested(var paths, string mode)

    readonly property string density: ExplorerPrefs.density
    readonly property int gap: 12
    readonly property int padding: 12
    readonly property int minCellWidth: density === "compact" ? 120
        : density === "spacious" ? 172 : 148
    readonly property int cellHeight: density === "compact" ? 126
        : density === "spacious" ? 150 : 142
    readonly property int wellHeight: density === "compact" ? 40
        : density === "spacious" ? 64 : 56
    readonly property int glyphSize: density === "compact" ? 28
        : density === "spacious" ? 34 : 30
    readonly property int columns: {
        const available = Math.max(0, width - padding * 2)
        return Math.max(1, Math.floor((available + gap) / (minCellWidth + gap)))
    }
    readonly property real cardWidth: {
        const available = Math.max(0, width - padding * 2 - gap * (columns - 1))
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

    /// The folder under a drop point: a directory cell is the target,
    /// anything else means the directory on screen.
    function dropDirectoryAt(x, y) {
        if (model === null || model.path === "")
            return ""
        const index = grid.indexAt(x, grid.contentY + y)
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
        const firstRow = Math.max(0, Math.floor(
            (grid.contentY + rect.top - padding) / (cellHeight + gap)))
        const lastRow = Math.max(0, Math.ceil(
            (grid.contentY + rect.bottom - padding) / (cellHeight + gap)) - 1)
        const firstColumn = Math.max(0, Math.floor(
            (grid.contentX + rect.left - padding) / (cardWidth + gap)))
        const lastColumn = Math.min(columns - 1, Math.max(0, Math.ceil(
            (grid.contentX + rect.right - padding) / (cardWidth + gap)) - 1))

        const paths = []
        for (let row = firstRow; row <= lastRow; row += 1) {
            for (let column = firstColumn; column <= lastColumn; column += 1) {
                const index = row * columns + column
                const path = model.pathAt(index)
                if (path === "")
                    continue
                const cellLeft = padding + column * (cardWidth + gap)
                const cellTop = row * (cellHeight + gap)
                const cellRight = cellLeft + cardWidth
                const cellBottom = cellTop + cellHeight
                if (cellRight >= grid.contentX + rect.left
                        && cellLeft <= grid.contentX + rect.right
                        && cellBottom >= grid.contentY + rect.top
                        && cellTop <= grid.contentY + rect.bottom)
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
            root.applyMarquee()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
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
        cellWidth: root.cardWidth + root.gap
        cellHeight: root.cellHeight + root.gap
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
            if (root.keyHandler !== null && root.keyHandler(event))
                event.accepted = true
        }

        delegate: Item {
            id: cell

            required property int index
            required property string fileName
            required property string filePath
            required property string sizeText
            required property string modifiedText
            required property bool isDir
            required property bool hidden
            required property bool readOnly
            required property string iconSource
            required property string category
            required property string extension
            required property string thumbnailSource

            width: grid.cellWidth - root.gap
            height: grid.cellHeight - root.gap

            readonly property bool selected: root.tab !== null && root.tab.isSelected(cell.filePath)
            readonly property bool dropTarget: root.dropTargetPath !== "" && cell.isDir
                && cell.filePath === root.dropTargetPath
            readonly property bool showThumbnail: ExplorerPrefs.rowThumbnails
                && cell.thumbnailSource !== ""

            Rectangle {
                anchors.fill: parent
                radius: 12
                color: cell.selected ? root.selectionColor
                    : cell.dropTarget ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b,
                                                root.dark ? 0.24 : 0.14)
                    : hover.containsMouse ? Theme.canvasAlt
                    : root.cardColor
                border.width: cell.selected || cell.dropTarget ? 1 : 0
                border.color: cell.dropTarget ? root.accent
                    : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.55)
            }

            Rectangle {
                id: well
                anchors.top: parent.top
                anchors.topMargin: 8
                anchors.horizontalCenter: parent.horizontalCenter
                width: Math.min(parent.width - 20, root.wellHeight + 34)
                height: root.wellHeight
                radius: 6
                color: Theme.hover
                clip: true

                Image {
                    anchors.fill: parent
                    anchors.margins: 4
                    visible: cell.showThumbnail
                    source: cell.showThumbnail ? cell.thumbnailSource : ""
                    sourceSize.width: Math.round(128 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(128 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    smooth: true
                    cache: true
                }

                Image {
                    anchors.centerIn: parent
                    visible: !cell.showThumbnail && ExplorerPrefs.iconStyle === "system"
                        && status === Image.Ready
                    source: cell.showThumbnail || ExplorerPrefs.iconStyle !== "system"
                        ? "" : cell.iconSource
                    width: root.glyphSize
                    height: root.glyphSize
                    sourceSize.width: Math.round(root.glyphSize * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(root.glyphSize * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    asynchronous: true
                }

                ThemedIcon {
                    anchors.centerIn: parent
                    visible: !cell.showThumbnail && ExplorerPrefs.iconStyle === "themed"
                    category: cell.category
                    extension: cell.extension
                    isDir: cell.isDir
                    size: root.glyphSize
                    dark: root.dark
                    accent: root.accent
                }
            }

            Text {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                anchors.top: well.bottom
                anchors.topMargin: 5
                text: cell.fileName
                color: cell.hidden ? Theme.inkFaint : root.textColor
                font.pixelSize: 12
                font.weight: Font.Medium
                textFormat: Text.PlainText
                elide: Text.ElideRight
                horizontalAlignment: Text.AlignLeft
            }

            Text {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                anchors.top: parent.top
                anchors.topMargin: root.cellHeight - 24
                text: cell.isDir ? qsTr("文件夹") : cell.sizeText
                color: root.mutedColor
                font.pixelSize: 11
                font.family: Theme.monoFamily
                textFormat: Text.PlainText
                elide: Text.ElideRight
                horizontalAlignment: Text.AlignLeft
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

    /// Rubber-band selection. The overlay sits above the rows so a drag that
    /// began on empty space keeps receiving move events over them; a press
    /// that began on a cell is handed down instead.
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
            const index = grid.indexAt(mouse.x, grid.contentY + mouse.y)
            if (index >= 0) {
                mouse.accepted = false
                return
            }
            grid.forceActiveFocus()
            root.marqueeBase = (mouse.modifiers & Qt.ControlModifier) !== 0
                ? root.selectedPathList() : []
            root.marqueeStartX = mouse.x + grid.contentX
            root.marqueeStartY = mouse.y + grid.contentY
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
            root.marqueeX = mouse.x + grid.contentX
            root.marqueeY = mouse.y + grid.contentY
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
            x: Math.min(root.marqueeStartX, root.marqueeX) - grid.contentX
            y: Math.min(root.marqueeStartY, root.marqueeY) - grid.contentY
            width: Math.abs(root.marqueeX - root.marqueeStartX)
            height: Math.abs(root.marqueeY - root.marqueeStartY)
            color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.14)
            border.width: 1
            border.color: root.accent
            radius: 4
        }
    }
}
