pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0

// Miller columns: the tab's directory on the left, the selected folder's
// children one column to the right, and so on.
//
// The first column is the tab's own listing, so switching between list and
// column mode keeps the same rows, selection and stream. Every deeper column
// owns a `DirectoryListModel`; drilling replaces the chain from the clicked
// depth, which is what makes clicking an earlier column truncate the trail.
//
// The row grammar is the detail list's, narrowed: 32 px rows on the canvas,
// the same signalSoft track for selection, a `fill` plate for the folder whose
// children are open, and the chevron the prototype draws for drillable rows.
Item {
    id: root

    property var tab: null

    /// Directories opened by each column, left to right: `chain[0]` is the
    /// folder the first column drilled into. The number of columns is
    /// `chain.length + 1`.
    property var chain: []

    signal activateRequested(string path, bool isDir)
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal dragOutRequested(var paths, string mode)

    readonly property string density: ExplorerPrefs.density
    readonly property int rowHeight: density === "compact" ? Theme.rowCompact
        : density === "spacious" ? Theme.rowSpacious : Theme.rowDefault
    readonly property int headerHeight: 32
    readonly property int columnWidth: 248

    Connections {
        target: root.tab !== null ? root.tab.listing : null
        function onPathChanged() {
            root.chain = []
        }
    }

    onChainChanged: Qt.callLater(function () {
        flick.contentX = Math.max(0, flick.contentWidth - flick.width)
    })

    function drill(path, depth) {
        if (!path)
            return
        const next = root.chain.slice(0, depth)
        next.push(path)
        root.chain = next
    }

    function pop(depth) {
        if (depth <= 0)
            return
        root.chain = root.chain.slice(0, depth - 1)
    }

    function takeFocus() {
        const pane = panes.itemAt(Math.max(0, root.chain.length))
        if (pane)
            pane.takeFocus()
    }

    function nameOf(path) {
        const text = String(path).replace(/[\\/]+$/, "")
        const parts = text.split(/[\\/]/)
        return parts.length > 0 ? parts[parts.length - 1] : text
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

    Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: columns.width
        contentHeight: height
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Row {
            id: columns
            height: flick.height

            Repeater {
                id: panes
                model: root.chain.length + 1

                delegate: ColumnPane {
                    required property int index

                    height: columns.height
                    /// The last column takes the space left over, so a single
                    /// column does not leave a bare half-pane; earlier columns
                    /// keep the 248 px step and scroll when the trail outgrows
                    /// the pane.
                    width: index === panes.count - 1
                        ? Math.max(root.columnWidth, flick.width - root.columnWidth * index)
                        : root.columnWidth
                    depth: index
                    path: index === 0 ? "" : root.chain[index - 1]
                    activeChildPath: index < root.chain.length ? root.chain[index] : ""
                    rowHeight: root.rowHeight
                    tab: root.tab

                    onDrillRequested: function (path, depth) {
                        root.drill(path, depth)
                    }
                    onPopRequested: function (depth) {
                        root.pop(depth)
                    }
                    onActivateRequested: function (path, isDir) {
                        root.activateRequested(path, isDir)
                    }
                    onContextRequested: function (path, isDir, x, y) {
                        root.contextRequested(path, isDir, x, y)
                    }
                    onSelectionChanged: root.selectionChanged()
                    onDragOutRequested: function (paths, mode) {
                        root.dragOutRequested(paths, mode)
                    }
                }
            }
        }
    }

    component ColumnPane: Item {
        id: pane

        required property int depth
        required property string path
        required property string activeChildPath
        property var tab: null
        property int rowHeight: 32
        property bool keyboardCursor: false
        property string typeAheadBuffer: ""

        signal drillRequested(string path, int depth)
        signal popRequested(int depth)
        signal activateRequested(string path, bool isDir)
        signal contextRequested(string path, bool isDir, real x, real y)
        signal selectionChanged()
        signal dragOutRequested(var paths, string mode)

        property bool ready: false

        /// Deeper columns own a model; the first column shows the tab's own
        /// listing so the two views stay in sync.
        DirectoryListModel {
            id: ownModel
            Component.onCompleted: {
                pane.ready = true
                if (pane.depth > 0) {
                    pane.applyPreferences()
                    ownModel.load(pane.path)
                }
            }
            Component.onDestruction: {
                if (pane.depth > 0)
                    ownModel.dispose()
            }
        }

        readonly property var listing: pane.depth === 0 && pane.tab !== null
            ? pane.tab.listing : ownModel

        readonly property string title: root.nameOf(pane.depth === 0 && pane.tab !== null
            ? pane.tab.listing.path : pane.path)
        readonly property bool empty: pane.listing.finished && pane.listing.loaded === 0
        readonly property bool emptyError: pane.empty && pane.listing.status !== ""
            && !/^\d+ items?$/.test(pane.listing.status)

        function applyPreferences() {
            if (pane.depth === 0)
                return
            listing.setOrder(ExplorerPrefs.sortKey, ExplorerPrefs.sortOrder,
                             ExplorerPrefs.foldersFirst)
            listing.setFilters(ExplorerPrefs.showHiddenFiles, ExplorerPrefs.kindFilter,
                               ExplorerPrefs.modifiedFilter, ExplorerPrefs.sizeFilter)
        }

        function takeFocus() {
            list.forceActiveFocus()
        }

        onPathChanged: {
            if (depth > 0 && ready && path !== "")
                ownModel.load(path)
        }

        Connections {
            target: ExplorerPrefs
            function onSortKeyChanged() { pane.applyPreferences() }
            function onSortOrderChanged() { pane.applyPreferences() }
            function onFoldersFirstChanged() { pane.applyPreferences() }
            function onShowHiddenFilesChanged() { pane.applyPreferences() }
            function onKindFilterChanged() { pane.applyPreferences() }
            function onModifiedFilterChanged() { pane.applyPreferences() }
            function onSizeFilterChanged() { pane.applyPreferences() }
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
            if (tab === null)
                return
            list.forceActiveFocus()
            list.currentIndex = index
            const value = listing.pathAt(index)
            if (value === "")
                return

            if (shift && tab.selectionAnchor !== "") {
                const anchor = listing.indexOfPath(tab.selectionAnchor)
                if (anchor >= 0) {
                    const first = Math.min(anchor, index)
                    const last = Math.max(anchor, index)
                    const paths = []
                    for (let row = first; row <= last; row += 1) {
                        const value = listing.pathAt(row)
                        if (value !== "")
                            paths.push(value)
                    }
                    tab.selectMany(paths)
                    pane.selectionChanged()
                    return
                }
            }

            if (ctrl)
                tab.toggle(value)
            else
                tab.selectOnly(value)
            pane.selectionChanged()
        }

        function nameAt(index) {
            return listing.nameAt(index)
        }

        Timer {
            id: typeAheadTimer
            interval: 800
            repeat: false
            onTriggered: pane.typeAheadBuffer = ""
        }

        /// The same prefix match the detail list uses: one letter cycles from
        /// the row after the cursor, a longer buffer refines from the cursor.
        function typeAhead(text) {
            if (text.length !== 1)
                return
            pane.typeAheadBuffer += text
            typeAheadTimer.restart()

            const count = listing.loaded
            if (count <= 0)
                return
            const needle = pane.typeAheadBuffer.toLowerCase()
            const current = list.currentIndex
            const start = current < 0 ? 0 : needle.length === 1 ? current + 1 : current
            for (let offset = 0; offset < count; offset += 1) {
                const index = (start + offset) % count
                if (!pane.nameAt(index).toLowerCase().startsWith(needle))
                    continue
                pane.handleClick(index, false, false)
                list.positionViewAtIndex(index, ListView.Contain)
                return
            }
        }

        Rectangle {
            anchors.fill: parent
            color: Theme.canvas
        }

        Rectangle {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 1
            color: Theme.hairline
        }

        Item {
            id: headerBar

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: root.headerHeight

            Row {
                anchors.left: parent.left
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 7

                Glyph {
                    width: 14
                    height: 14
                    kind: "folder"
                    tint: Theme.inkMuted
                }

                Text {
                    width: Math.max(0, parent.width - 21)
                    text: pane.title
                    color: Theme.inkMuted
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Theme.hairline
            }
        }

        ListView {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: headerBar.bottom
            anchors.bottom: parent.bottom
            model: pane.listing
            clip: true
            reuseItems: true
            cacheBuffer: 1200
            boundsBehavior: Flickable.StopAtBounds
            currentIndex: -1
            focus: true
            activeFocusOnTab: true

            Keys.onPressed: function (event) {
                pane.keyboardCursor = true
                if (event.key === Qt.Key_Right) {
                    const path = list.currentIndex >= 0
                        ? pane.listing.pathAt(list.currentIndex) : ""
                    if (path !== "" && pane.listing.isDirectoryAt(list.currentIndex)) {
                        pane.drillRequested(path, pane.depth)
                        event.accepted = true
                    }
                } else if (event.key === Qt.Key_Left && pane.depth > 0) {
                    // Step back out to the column that opened this one.
                    pane.popRequested(pane.depth)
                    event.accepted = true
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (list.currentIndex >= 0) {
                        const path = pane.listing.pathAt(list.currentIndex)
                        if (path !== "") {
                            pane.activateRequested(path,
                                pane.listing.isDirectoryAt(list.currentIndex))
                            event.accepted = true
                        }
                    }
                } else if (event.text.length === 1 && event.text !== " "
                           && event.text.charCodeAt(0) >= 0x20
                           && (event.modifiers & (Qt.ControlModifier | Qt.AltModifier
                                                  | Qt.MetaModifier)) === 0) {
                    pane.typeAhead(event.text)
                    event.accepted = true
                }
            }

            delegate: Item {
                id: row

                required property int index
                required property string fileName
                required property string filePath
                required property bool isDir
                required property bool hidden
                required property string iconSource
                required property string category
                required property string extension
                required property string thumbnailSource

                width: list.width
                height: pane.rowHeight

                readonly property bool selected: pane.tab !== null
                    && pane.tab.isSelected(row.filePath)
                readonly property bool expanded: pane.activeChildPath !== ""
                    && pane.activeChildPath === row.filePath
                readonly property bool current: list.currentIndex === row.index
                readonly property bool focusedCursor: list.activeFocus
                    && pane.keyboardCursor && row.current
                readonly property bool showThumbnail: ExplorerPrefs.rowThumbnails
                    && row.thumbnailSource !== ""

                /// One row per folder entry. `expanded` is the column view's
                /// "this folder is open in the next column", which is the same
                /// idea a tree states, so it is announced the same way.
                Accessible.role: Accessible.ListItem
                Accessible.name: row.fileName
                Accessible.description: row.isDir ? qsTr("文件夹") : qsTr("文件")
                Accessible.selectable: true
                Accessible.selected: row.selected
                Accessible.expandable: row.isDir
                Accessible.expanded: row.expanded
                Accessible.onPressAction: pane.activateRequested(row.filePath, row.isDir)

                Rectangle {
                    anchors.fill: parent
                    color: row.selected
                        ? (hover.containsMouse ? Theme.signalSoft2 : Theme.signalSoft)
                        : row.expanded ? Theme.fill
                        : (hover.containsMouse ? Theme.hover : "transparent")
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 2
                    color: Theme.signal
                    visible: row.selected
                }

                Rectangle {
                    anchors.fill: parent
                    visible: row.focusedCursor
                    color: "transparent"
                    border.width: 1
                    border.color: Theme.signal
                }

                Image {
                    id: systemIcon
                    x: 10
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16
                    height: 16
                    visible: !row.showThumbnail && ExplorerPrefs.iconStyle === "system"
                        && status === Image.Ready
                    source: row.showThumbnail || ExplorerPrefs.iconStyle !== "system"
                        ? "" : row.iconSource
                    sourceSize.width: Math.round(16 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(16 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    asynchronous: true
                    Accessible.ignored: true
                }

                Image {
                    x: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: 20
                    height: 20
                    visible: row.showThumbnail
                    source: row.showThumbnail ? row.thumbnailSource : ""
                    sourceSize.width: Math.round(40 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(40 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    smooth: true
                    Accessible.ignored: true
                }

                ThemedIcon {
                    x: 10
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !row.showThumbnail && (ExplorerPrefs.iconStyle !== "system"
                        || systemIcon.status !== Image.Ready)
                    category: row.category
                    extension: row.extension
                    isDir: row.isDir
                    size: 16
                    tint: row.selected || row.expanded || hover.containsMouse
                        ? Theme.ink : Theme.inkMuted
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 34
                    anchors.right: chevron.visible ? chevron.left : parent.right
                    anchors.rightMargin: chevron.visible ? 4 : 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.fileName
                    color: row.hidden ? Theme.inkFaint : Theme.ink
                    font.pixelSize: Theme.typeBody
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Glyph {
                    id: chevron
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: 12
                    height: 12
                    visible: row.isDir
                    kind: "chevron"
                    tint: Theme.inkFaint
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
                        if (pane.tab === null || pane.listing.remote)
                            return
                        const selected = pane.tab.isSelected(row.filePath)
                        if (!selected && (mouse.modifiers
                                          & (Qt.ControlModifier | Qt.ShiftModifier)) === 0)
                            pane.handleClick(row.index, false, false)
                        dragPaths = selected ? pane.selectedPathList() : [row.filePath]
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
                            pane.dragOutRequested(paths, modeFor(mouse.modifiers))
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
                        pane.keyboardCursor = false
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
                            if (pane.tab === null || !pane.tab.isSelected(row.filePath))
                                pane.handleClick(row.index, false, false)
                            const point = row.mapToItem(null, mouse.x, mouse.y)
                            pane.contextRequested(row.filePath, row.isDir, point.x, point.y)
                            return
                        }
                        pane.handleClick(row.index,
                                         (mouse.modifiers & Qt.ControlModifier) !== 0,
                                         (mouse.modifiers & Qt.ShiftModifier) !== 0)
                        if (row.isDir && (mouse.modifiers
                                          & (Qt.ControlModifier | Qt.ShiftModifier)) === 0)
                            pane.drillRequested(row.filePath, pane.depth)
                    }
                    onDoubleClicked: function (mouse) {
                        if (mouse.button === Qt.LeftButton)
                            pane.activateRequested(row.filePath, row.isDir)
                    }
                }
            }
        }

        /// Only deeper columns show this: the first column's empty state is
        /// the shell's full-size one with its new-folder and paste actions.
        Column {
            anchors.centerIn: parent
            anchors.verticalCenterOffset: root.headerHeight / 2
            width: Math.min(parent.width - 32, 200)
            spacing: 6
            visible: pane.depth > 0 && pane.empty

            Glyph {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 28
                height: 28
                kind: pane.emptyError ? "file" : "folder"
                tint: Theme.inkMuted
            }

            Text {
                width: parent.width
                text: pane.emptyError ? qsTr("无法打开此文件夹") : qsTr("这个文件夹是空的")
                color: Theme.ink
                font.pixelSize: Theme.typeBody
                horizontalAlignment: Text.AlignHCenter
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: pane.emptyError
                text: pane.emptyError ? pane.listing.status : ""
                color: Theme.inkFaint
                font.pixelSize: Theme.typeCaption
                horizontalAlignment: Text.AlignHCenter
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                maximumLineCount: 3
                elide: Text.ElideRight
            }
        }
    }
}
