pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import "Localize.js" as Localize

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
    /// The shell's shared key handler. Navigation keys stay with the column
    /// (they mean drill and pop here); commands use the same bindings as the
    /// list and grid instead of a second, smaller shortcut set.
    property var keyHandler: null

    /// Directories opened by each column, left to right: `chain[0]` is the
    /// folder the first column drilled into. The number of columns is
    /// `chain.length + 1`.
    property var chain: []

    /// The column with the keyboard and the current command target. Miller
    /// columns keep selection in every column; this index says which one the
    /// footer and the keyboard mean when a path could be ambiguous.
    property int activePaneIndex: 0

    signal activateRequested(string path, bool isDir)
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal dragOutRequested(var paths, string mode)

    readonly property string density: ExplorerPrefs.density
    readonly property int rowHeight: density === "compact" ? Theme.rowCompact
        : density === "spacious" ? Theme.rowSpacious : Theme.rowDefault
    readonly property int headerHeight: Theme.rowCompact
    readonly property int columnWidth: 260

    Connections {
        target: root.tab !== null ? root.tab.listing : null
        function onPathChanged() {
            root.chain = []
            root.activePaneIndex = 0
        }
    }

    onChainChanged: Qt.callLater(function () {
        root.focusActivePane()
    })

    function paneAt(index) {
        if (index < 0 || index >= panes.count)
            return null
        return panes.itemAt(index)
    }

    function clampedActivePane() {
        return Math.max(0, Math.min(panes.count - 1, root.activePaneIndex))
    }

    function activePane() {
        return root.paneAt(root.clampedActivePane())
    }

    function scrollActiveIntoView() {
        const index = root.clampedActivePane()
        const left = index * root.columnWidth
        if (left < flick.contentX)
            flick.contentX = left
        else if (left + root.columnWidth > flick.contentX + flick.width)
            flick.contentX = left + root.columnWidth - flick.width
    }

    function focusActivePane() {
        const pane = root.activePane()
        if (pane !== null)
            pane.takeFocus()
        root.scrollActiveIntoView()
    }

    function currentModel() {
        const pane = root.activePane()
        return pane !== null ? pane.listing : null
    }

    function currentIndex() {
        const pane = root.activePane()
        return pane !== null ? pane.currentIndex() : -1
    }

    function currentPath() {
        const pane = root.activePane()
        return pane !== null ? pane.currentPath() : ""
    }

    function pathAtIndex(index) {
        const pane = root.activePane()
        return pane !== null ? pane.listing.pathAt(index) : ""
    }

    function activateIndex(index) {
        const pane = root.activePane()
        if (pane === null)
            return
        const path = pane.listing.pathAt(index)
        if (path !== "")
            root.activateRequested(path, pane.listing.isDirectoryAt(index))
    }

    function activatePath(path) {
        const pane = root.paneForPath(path)
        if (pane === null) {
            Workspace.openPath(path)
            return
        }
        const index = pane.listing.indexOfPath(path)
        root.activateRequested(path, pane.listing.isDirectoryAt(index))
    }

    /// The pane that owns `path`, searched from the deepest column outward.
    /// A selected path is unique to one directory listing, so the first hit is
    /// also its parent model.
    function paneForPath(path) {
        if (path === undefined || path === null || String(path) === "")
            return null
        for (let index = panes.count - 1; index >= 0; index -= 1) {
            const pane = panes.itemAt(index)
            if (pane !== null && pane.listing.indexOfPath(String(path)) >= 0)
                return pane
        }
        return null
    }

    function modelForPath(path) {
        const pane = root.paneForPath(path)
        if (pane !== null)
            return pane.listing
        const active = root.activePane()
        return active !== null ? active.listing : null
    }

    function drill(path, depth) {
        if (!path)
            return
        const next = root.chain.slice(0, depth)
        next.push(path)
        root.chain = next
        root.activePaneIndex = next.length
    }

    function pop(depth) {
        if (depth <= 0)
            return
        root.chain = root.chain.slice(0, depth - 1)
        root.activePaneIndex = Math.max(0, depth - 1)
    }

    function takeFocus() {
        const pane = root.paneAt(root.clampedActivePane())
        if (pane)
            pane.takeFocus()
        root.scrollActiveIntoView()
    }

    /// Every column owns its own model, so the selection spans several of
    /// them. The shell asks this view once and gets the union, the same way it
    /// asked the tab's map before.
    function selectedPathList() {
        const paths = []
        for (let index = 0; index < panes.count; index += 1) {
            const pane = panes.itemAt(index)
            if (pane !== null)
                paths.push.apply(paths, pane.listing.selectedPaths())
        }
        return paths
    }

    /// Bumped whenever any column's selection moves, so the sum below is
    /// re-read even where a column had no delegate yet when it was first
    /// evaluated.
    property int selectionRevision: 0

    readonly property int selectionCount: {
        selectionRevision
        let total = 0
        for (let index = 0; index < panes.count; index += 1) {
            const pane = panes.itemAt(index)
            if (pane !== null)
                total += pane.listing.selectedCount
        }
        return total
    }

    /// The union selection's file total. Miller columns select at most one row
    /// per column, so asking the workspace to sum the selected paths is
    /// cheaper than teaching every pane model to publish a second total.
    readonly property string selectionSizeText: {
        selectionRevision
        return root.selectionCount > 0
            ? Workspace.selectionSizeText(root.selectedPathList()) : ""
    }

    function clearSelection() {
        for (let index = 0; index < panes.count; index += 1) {
            const pane = panes.itemAt(index)
            if (pane !== null) {
                // qmllint disable missing-property
                pane.listing.clearSelection()
            }
        }
    }

    function nameOf(path) {
        const text = String(path).replace(/[\\/]+$/, "")
        const parts = text.split(/[\\/]/)
        return parts.length > 0 ? parts[parts.length - 1] : text
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
                    /// §11.3: every column is 260 dp, including the last one;
                    /// the trail scrolls horizontally rather than stretching a
                    /// half-filled pane into one wide column.
                    width: root.columnWidth
                    depth: index
                    path: index === 0 ? "" : root.chain[index - 1]
                    activeChildPath: index < root.chain.length ? root.chain[index] : ""
                    dimmed: index < panes.count - 1
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
                    onSelectionChanged: {
                        root.selectionRevision += 1
                        root.selectionChanged()
                    }
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
        property bool dimmed: false
        property var tab: null
        property int rowHeight: Theme.controlLg
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
                             ExplorerPrefs.folderOrder)
            listing.setGroup(ExplorerPrefs.groupKey, ExplorerPrefs.groupDate)
            listing.setFilters(ExplorerPrefs.showHiddenFiles, ExplorerPrefs.kindFilter,
                               ExplorerPrefs.modifiedFilter, ExplorerPrefs.sizeFilter,
                               ExplorerPrefs.nameFilter)
        }

        function takeFocus() {
            root.activePaneIndex = pane.depth
            list.forceActiveFocus()
        }

        function currentIndex() {
            return list.currentIndex
        }

        function currentPath() {
            return list.currentIndex >= 0 ? pane.listing.pathAt(list.currentIndex) : ""
        }

        /// Arrow keys follow the same rule as the detail list: move the cursor,
        /// select with it, Shift extends from the click anchor and Ctrl moves
        /// without changing the selection.
        function moveCursor(event) {
            if (pane.listing.loaded <= 0)
                return false
            let step = 0
            switch (event.key) {
            case Qt.Key_Up: step = -1; break
            case Qt.Key_Down: step = 1; break
            case Qt.Key_PageUp:
                step = -Math.max(1, Math.floor(list.height / pane.rowHeight) - 1)
                break
            case Qt.Key_PageDown:
                step = Math.max(1, Math.floor(list.height / pane.rowHeight) - 1)
                break
            case Qt.Key_Home:
                return pane.selectCursor(0, event.modifiers)
            case Qt.Key_End:
                return pane.selectCursor(pane.listing.loaded - 1, event.modifiers)
            default:
                return false
            }
            const from = list.currentIndex
            const target = from < 0
                ? (step > 0 ? 0 : -1)
                : Math.max(0, Math.min(pane.listing.loaded - 1, from + step))
            return target >= 0 && pane.selectCursor(target, event.modifiers)
        }

        function selectCursor(target, modifiers) {
            const path = pane.listing.pathAt(target)
            if (path === "")
                return false
            list.currentIndex = target
            list.positionViewAtIndex(target, ListView.Contain)
            const ctrl = (modifiers & Qt.ControlModifier) !== 0
            const shift = (modifiers & Qt.ShiftModifier) !== 0
            if (shift && pane.tab !== null && pane.tab.selectionAnchor !== "") {
                const anchor = pane.listing.indexOfPath(pane.tab.selectionAnchor)
                if (anchor >= 0) {
                    pane.listing.selectRange(Math.min(anchor, target),
                                             Math.max(anchor, target))
                    pane.selectionChanged()
                    return true
                }
            }
            if (!ctrl) {
                pane.listing.selectOnly(target)
                if (pane.tab !== null)
                    pane.tab.selectionAnchor = path
                pane.selectionChanged()
            }
            return true
        }

        onPathChanged: {
            if (depth > 0 && ready && path !== "")
                ownModel.load(path)
        }

        /// A selection can also change from the keyboard or the shell, which
        /// goes straight to the model; the count the trail shows has to follow
        /// that as well as the clicks this pane handles itself.
        Connections {
            target: pane.listing
            function onSelectedCountChanged() {
                root.selectionRevision += 1
            }
        }

        Connections {
            target: ExplorerPrefs
            function onSortKeyChanged() { pane.applyPreferences() }
            function onSortOrderChanged() { pane.applyPreferences() }
            function onFolderOrderChanged() { pane.applyPreferences() }
            function onShowHiddenFilesChanged() { pane.applyPreferences() }
            function onKindFilterChanged() { pane.applyPreferences() }
            function onModifiedFilterChanged() { pane.applyPreferences() }
            function onSizeFilterChanged() { pane.applyPreferences() }
            function onNameFilterChanged() { pane.applyPreferences() }
        }

        /// The drag takes the whole trail's selection, not just this column's:
        /// a Ctrl-click in another column is part of the same gesture.
        function selectedPathList() {
            return root.selectedPathList()
        }

        function handleClick(index, ctrl, shift) {
            if (tab === null)
                return
            root.activePaneIndex = pane.depth
            list.forceActiveFocus()
            list.currentIndex = index
            const value = listing.pathAt(index)
            if (value === "")
                return

            if (shift && tab.selectionAnchor !== "") {
                const anchor = listing.indexOfPath(tab.selectionAnchor)
                if (anchor >= 0) {
                    // One call: the model sets the flags for the range and
                    // repaints only the rows that moved.
                    listing.selectRange(Math.min(anchor, index), Math.max(anchor, index))
                    pane.selectionChanged()
                    return
                }
            }

            if (ctrl)
                listing.toggleAt(index)
            else
                listing.selectOnly(index)
            tab.selectionAnchor = value
            pane.selectionChanged()
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
            // The scan and the folding run in Rust, so a miss costs one call
            // instead of one per row.
            const index = listing.indexOfNamePrefix(needle, start)
            if (index < 0)
                return
            pane.handleClick(index, false, false)
            list.positionViewAtIndex(index, ListView.Contain)
        }

        Rectangle {
            anchors.fill: parent
            color: Theme.content
        }

        Rectangle {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: Theme.ruleWidth(Screen.devicePixelRatio)
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
                    tint: Theme.inkSecondary
                }

                Text {
                    width: Math.max(0, parent.width - 21)
                    text: pane.title
                    color: Theme.inkSecondary
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Theme.ruleWidth(Screen.devicePixelRatio)
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

            onActiveFocusChanged: {
                if (activeFocus)
                    root.activePaneIndex = pane.depth
            }

            Keys.onPressed: function (event) {
                pane.keyboardCursor = true
                if ((event.modifiers & (Qt.AltModifier | Qt.MetaModifier)) !== 0)
                    return
                switch (event.key) {
                case Qt.Key_Up:
                case Qt.Key_Down:
                case Qt.Key_PageUp:
                case Qt.Key_PageDown:
                case Qt.Key_Home:
                case Qt.Key_End:
                    event.accepted = pane.moveCursor(event)
                    return
                case Qt.Key_Right: {
                    const path = list.currentIndex >= 0
                        ? pane.listing.pathAt(list.currentIndex) : ""
                    if (path !== "" && pane.listing.isDirectoryAt(list.currentIndex)) {
                        pane.drillRequested(path, pane.depth)
                        event.accepted = true
                    }
                    return
                }
                case Qt.Key_Left:
                    if (pane.depth > 0) {
                        pane.popRequested(pane.depth)
                        event.accepted = true
                    }
                    return
                case Qt.Key_Return:
                case Qt.Key_Enter:
                    if (list.currentIndex >= 0) {
                        const path = pane.listing.pathAt(list.currentIndex)
                        if (path !== "") {
                            pane.activateRequested(path,
                                pane.listing.isDirectoryAt(list.currentIndex))
                            event.accepted = true
                        }
                    }
                    return
                case Qt.Key_Backspace:
                    if (pane.depth > 0) {
                        pane.popRequested(pane.depth)
                        event.accepted = true
                        return
                    }
                    break
                }

                // Printable keys are type-ahead within this column; every
                // other command uses the shared catalogue, so Delete, cut,
                // copy, paste, preview, refresh and type-ahead all mean the
                // same thing here as in the detail list.
                const typing = event.text.length === 1 && event.text !== " "
                    && event.key < Qt.Key_Delete
                    && (event.modifiers & (Qt.ControlModifier | Qt.AltModifier
                                           | Qt.MetaModifier)) === 0
                if (typing) {
                    pane.typeAhead(event.text)
                    event.accepted = true
                } else if (root.keyHandler !== null
                           && root.keyHandler(event)) {
                    event.accepted = true
                }
            }

            delegate: ListRow {
                id: row

                required property int index
                required property string fileName
                required property string filePath
                required property bool isDir
                required property bool hidden
                required property string category
                required property string extension
                required property string thumbnailSource

                width: list.width
                rowHeight: pane.rowHeight

                required property bool selected
                readonly property bool expanded: pane.activeChildPath !== ""
                    && pane.activeChildPath === row.filePath
                readonly property bool current: list.currentIndex === row.index
                readonly property bool focusedCursor: list.activeFocus
                    && pane.keyboardCursor && row.current
                readonly property bool showThumbnail: ExplorerPrefs.rowThumbnails
                    && row.thumbnailSource !== ""

                title: row.fileName
                titleColor: row.hidden ? Theme.inkTertiary : Theme.ink
                isSelected: row.selected
                highlighted: row.expanded
                keyboardCursor: row.focusedCursor
                isCurrent: row.current
                showFocusRail: false
                iconCategory: row.isDir ? "folder" : row.category
                iconEdge: 16
                iconOpacity: pane.dimmed ? 0.7 : 1
                showLeading: row.showThumbnail
                accessibleName: row.fileName
                accessibleDescription: row.isDir ? qsTr("文件夹") : qsTr("文件")
                Accessible.expandable: row.isDir
                Accessible.expanded: row.expanded

                leading: Image {
                    width: 20
                    height: Theme.controlXs
                    visible: row.showThumbnail
                    source: row.showThumbnail ? row.thumbnailSource : ""
                    sourceSize.width: Math.round(40 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(40 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    smooth: true
                    opacity: pane.dimmed ? 0.7 : 1
                    Accessible.ignored: true
                }

                trailing: Glyph {
                    width: 12
                    height: 12
                    visible: row.isDir
                    kind: "chevron"
                    tint: Theme.inkTertiary
                    opacity: pane.dimmed ? 0.7 : 1
                }

                property bool dragArmed: false
                property bool dragStarted: false
                property real dragStartX: 0
                property real dragStartY: 0
                property var dragPaths: []

                function prepareDrag(mouse) {
                    clickSuppressed = false
                    dragArmed = false
                    dragStarted = false
                    dragPaths = []
                    if (pane.tab === null || pane.listing.remote)
                        return
                    const wasSelected = row.selected
                    if (!wasSelected && (mouse.modifiers
                                         & (Qt.ControlModifier | Qt.ShiftModifier)) === 0)
                        pane.handleClick(row.index, false, false)
                    dragPaths = wasSelected ? pane.selectedPathList() : [row.filePath]
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
                    clickSuppressed = true
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
                onMoved: function (mouse) {
                    continueDrag(mouse)
                }
                onReleased: dragArmed = false
                onCanceled: dragArmed = false
                onClicked: function (mouse) {
                    if (mouse.button === Qt.RightButton) {
                        if (!row.selected)
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
                onActivated: pane.activateRequested(row.filePath, row.isDir)
            }
        }

        /// Every column scrolls on its own, so every column carries its own
        /// bar; the last column's is the one a reader usually reaches.
        ScrollBar {
            target: list
            anchors.right: parent.right
            anchors.rightMargin: 2
            anchors.top: headerBar.bottom
            anchors.bottom: parent.bottom
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
                height: Theme.controlMd
                kind: pane.emptyError ? "file" : "folder"
                tint: Theme.inkSecondary
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
                text: pane.emptyError ? Localize.message(pane.listing.status) : ""
                color: Theme.inkTertiary
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
