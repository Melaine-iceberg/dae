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
Item {
    id: root

    property var tab: null
    property bool dark: true
    property color accent: "#2f6fdf"
    property color contentColor: dark ? "#191b1f" : "#ffffff"
    property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    property color mutedColor: dark ? "#8f98aa" : "#657080"
    property color hairline: dark ? "#26ffffff" : "#16000000"
    property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b, dark ? 0.30 : 0.18)
    property var keyHandler: null

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
    readonly property int rowHeight: density === "compact" ? 28
        : density === "spacious" ? 44 : 34
    readonly property int columnWidth: 236

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
                    width: root.columnWidth
                    depth: index
                    path: index === 0 ? "" : root.chain[index - 1]
                    activeChildPath: index < root.chain.length ? root.chain[index] : ""
                    dark: root.dark
                    accent: root.accent
                    contentColor: root.contentColor
                    textColor: root.textColor
                    mutedColor: root.mutedColor
                    hairline: root.hairline
                    selectionColor: root.selectionColor
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
        property bool dark: true
        property color accent: "#2f6fdf"
        property color contentColor: dark ? "#191b1f" : "#ffffff"
        property color textColor: dark ? "#e6e9ef" : "#1c1f24"
        property color mutedColor: dark ? "#8f98aa" : "#657080"
        property color hairline: dark ? "#26ffffff" : "#16000000"
        property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b, dark ? 0.30 : 0.18)
        property int rowHeight: 34

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
                        const path = listing.pathAt(row)
                        if (path !== "")
                            paths.push(path)
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

        Rectangle {
            anchors.fill: parent
            color: pane.depth % 2 === 0
                ? pane.contentColor : (pane.dark ? "#171a1f" : "#fafbfc")
        }

        Rectangle {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 1
            color: pane.hairline
        }

        ListView {
            id: list
            anchors.fill: parent
            anchors.margins: 6
            model: pane.listing
            clip: true
            reuseItems: true
            cacheBuffer: 1200
            boundsBehavior: Flickable.StopAtBounds
            currentIndex: -1
            focus: true

            Keys.onPressed: function (event) {
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
                readonly property bool showThumbnail: ExplorerPrefs.rowThumbnails
                    && row.thumbnailSource !== ""

                Rectangle {
                    anchors.fill: parent
                    radius: 5
                    color: row.selected ? pane.selectionColor
                        : row.expanded ? Qt.rgba(pane.accent.r, pane.accent.g, pane.accent.b,
                                                 pane.dark ? 0.18 : 0.1)
                        : (hover.containsMouse ? (pane.dark ? "#1e2229" : "#eef1f5")
                                               : "transparent")
                    border.width: row.selected ? 1 : 0
                    border.color: Qt.rgba(pane.accent.r, pane.accent.g, pane.accent.b, 0.5)
                }

                Image {
                    id: systemIcon
                    x: 6
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18
                    height: 18
                    visible: !row.showThumbnail && ExplorerPrefs.iconStyle === "system"
                        && status === Image.Ready
                    source: row.showThumbnail || ExplorerPrefs.iconStyle !== "system"
                        ? "" : row.iconSource
                    sourceSize.width: Math.round(18 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(18 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    asynchronous: true
                }

                Image {
                    x: 5
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
                }

                ThemedIcon {
                    x: 6
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !row.showThumbnail && ExplorerPrefs.iconStyle === "themed"
                    category: row.category
                    extension: row.extension
                    isDir: row.isDir
                    size: 18
                    dark: pane.dark
                    accent: pane.accent
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 30
                    anchors.right: chevron.visible ? chevron.left : parent.right
                    anchors.rightMargin: chevron.visible ? 4 : 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.fileName
                    color: row.hidden ? (pane.dark ? "#6b7280" : "#9aa1ad") : pane.textColor
                    font.pixelSize: 12
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Text {
                    id: chevron
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    visible: row.isDir
                    text: "\u203a"
                    color: pane.mutedColor
                    font.pixelSize: 13
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

        Text {
            anchors.centerIn: parent
            visible: pane.listing.finished && pane.listing.loaded === 0
                && pane.listing.status !== ""
            text: pane.listing.status === "0 项" ? qsTr("空文件夹") : pane.listing.status
            color: pane.mutedColor
            font.pixelSize: 11
        }
    }
}
