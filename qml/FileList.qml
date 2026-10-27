pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0

// The explorer listing. Delegates stay at 26 dp, recycle, and read nothing but
// finished strings from the model. Selection lives on the tab object so it
// survives a tab switch and travels with a torn-off tab.
Item {
    id: root

    property var tab: null
    /// Set while a filename search is showing. The same delegate and selection
    /// code then work for search rows, which expose the listing's roles plus
    /// `relativePath`.
    property var searchModel: null
    property bool dark: true
    property color headerColor: dark ? "#14161a" : "#f4f6f8"
    property color accent: "#2f6fdf"

    readonly property var model: searchModel !== null ? searchModel
        : (tab !== null ? tab.listing : null)
    /// The tab is browsing a network backend, so local-only actions (OS open,
    /// recycle bin) are routed through the window instead of failing.
    readonly property bool remote: tab !== null
        && (tab.listing.remote
            || /^(smb|sftp|ftp|webdav|webdavs|cloud):\/\//i.test(tab.listing.path))
    readonly property int rowHeight: 26
    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color alternateColor: dark ? "#15181e" : "#f7f8fa"
    readonly property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8f98aa" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"
    readonly property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b,
                                                   dark ? 0.30 : 0.18)

    readonly property int selectionCount: {
        const version = tab !== null ? tab.selectionVersion : 0
        if (version < 0 || tab === null)
            return 0
        let count = 0
        for (const key in tab.selected) {
            if (tab.selected[key])
                count += 1
        }
        return count
    }

    property string pendingFocus: ""
    /// A reload is in flight and the viewport still has to be returned to the
    /// position `saveScroll` recorded. Cleared once the rows under it exist.
    property bool scrollPending: false

    /// A pressed row that could become a drag. The gesture becomes a native
    /// drag once it travels far enough; until then it is still a click.
    property bool dragArmed: false
    property bool dragStarted: false
    property real dragStartX: 0
    property real dragStartY: 0
    property var dragPaths: []
    /// A native drag ran, so the release it ended with must not select a row.
    property bool suppressClick: false

    /// A file drag is hovering the listing: where it would land, and which
    /// action the source proposes. Empty when the drop is not allowed.
    property var dropSourcePaths: []
    property string dropTargetPath: ""
    property string dropAction: ""

    signal navigateRequested(string path)
    signal upRequested()
    signal renameRequested(string path, string name)
    signal createFolderRequested()
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal remoteDeleteRequested(var paths)
    signal remoteOpenRequested(string path)

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

    /// Keyboard focus lives on the inner ListView, not on this container.
    function takeFocus() {
        list.forceActiveFocus()
    }

    function currentPath() {
        if (root.model === null || list.currentIndex < 0)
            return ""
        return root.model.pathAt(list.currentIndex)
    }

    function handleClick(index, ctrl, shift) {
        if (root.tab === null || root.model === null)
            return
        list.forceActiveFocus()
        list.currentIndex = index
        const path = root.model.pathAt(index)
        if (path === "")
            return

        if (shift && root.tab.selectionAnchor !== "") {
            const anchor = root.model.indexOfPath(root.tab.selectionAnchor)
            if (anchor >= 0) {
                const first = Math.min(anchor, index)
                const last = Math.max(anchor, index)
                const paths = []
                for (let row = first; row <= last; row += 1) {
                    const value = root.model.pathAt(row)
                    if (value !== "")
                        paths.push(value)
                }
                root.tab.selectMany(paths)
                root.selectionChanged()
                return
            }
        }

        if (ctrl)
            root.tab.toggle(path)
        else
            root.tab.selectOnly(path)
        root.selectionChanged()
    }

    function handleActivate(index) {
        if (root.model === null)
            return
        const path = root.model.pathAt(index)
        if (path === "")
            return
        if (root.model.isDirectoryAt(index))
            root.navigateRequested(path)
        else if (root.remote)
            root.remoteOpenRequested(path)
        else
            Workspace.openPath(path)
    }

    function selectAll() {
        if (root.model === null || root.tab === null)
            return
        const paths = []
        for (let row = 0; row < root.model.loaded; row += 1) {
            const path = root.model.pathAt(row)
            if (path !== "")
                paths.push(path)
        }
        root.tab.selectMany(paths)
        root.selectionChanged()
    }

    /// Reloads and, once the listing finishes, selects `path` and scrolls to
    /// it. Used after create/rename so the new row is visible and selected.
    function focusPath(path) {
        root.pendingFocus = path
        if (root.model !== null && root.model.finished)
            root.applyFocus()
    }

    function applyFocus() {
        const path = root.pendingFocus
        if (path === "" || root.model === null)
            return
        const index = root.model.indexOfPath(path)
        root.pendingFocus = ""
        if (index < 0)
            return
        list.currentIndex = index
        list.positionViewAtIndex(index, ListView.Contain)
        root.tab.selectOnly(path)
        root.selectionChanged()
    }

    /// Reads the viewport position before the listing clears its rows. The
    /// reset that follows drops both the rows and the position, so this is the
    /// last moment it exists. Stored on the tab, keyed by the directory.
    /// Returns whether there was a position to remember.
    function saveScroll() {
        if (root.tab === null || root.searchModel !== null)
            return false
        const path = root.tab.listing.path
        if (path === "")
            return false
        const state = Object.assign({}, root.tab.scrollState)
        state[path] = {
            y: list.contentY,
            current: root.tab.listing.pathAt(list.currentIndex)
        }
        root.tab.scrollState = state
        return true
    }

    /// Returns the viewport to the remembered position as soon as the rows
    /// under it have streamed in — not only when the whole listing finished,
    /// which on a large directory would leave the list at the top for the
    /// length of the read. Called again on every batch until it succeeds.
    function restoreScroll() {
        if (!root.scrollPending || root.tab === null || root.searchModel !== null
                || root.pendingFocus !== "")
            return
        const state = root.tab.scrollState[root.tab.listing.path]
        if (!state) {
            root.scrollPending = false
            return
        }
        const row = Math.floor(state.y / root.rowHeight)
        if (row >= root.tab.listing.loaded) {
            // The rows above the viewport are still arriving; the position
            // does not exist yet.
            if (!root.tab.listing.finished)
                return
            root.scrollPending = false
            if (state.current !== "") {
                const index = root.tab.listing.indexOfPath(state.current)
                if (index >= 0)
                    list.currentIndex = index
            }
            list.contentY = state.y
            return
        }

        root.scrollPending = false
        if (state.current !== "") {
            const index = root.tab.listing.indexOfPath(state.current)
            if (index >= 0)
                list.currentIndex = index
        }
        // `Beginning` places the row; the pixel remainder keeps the listing
        // exactly where it was inside that row.
        list.positionViewAtIndex(row, ListView.Beginning)
        list.contentY += state.y - row * root.rowHeight
    }

    /// Arms a press as a possible drag. The row being dragged is the whole
    /// selection when it is part of it, otherwise the pressed row alone —
    /// selecting it first unless a modifier says the gesture is extending the
    /// selection instead.
    function prepareDrag(index, mouse) {
        root.suppressClick = false
        root.dragArmed = false
        root.dragStarted = false
        root.dragPaths = []

        if (root.tab === null || root.model === null || root.searchModel !== null
                || root.remote || index < 0)
            return

        const path = root.model.pathAt(index)
        if (path === "")
            return

        const selected = root.tab.isSelected(path)
        if (!selected && (mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)) === 0)
            root.handleClick(index, false, false)

        root.dragPaths = selected ? root.selectedPathList() : [path]
        root.dragStartX = mouse.x
        root.dragStartY = mouse.y
        root.dragArmed = true
    }

    /// Turns the armed press into a native drag once the pointer has moved
    /// far enough. `"any"` offers copy, move and link and lets the target
    /// choose; a held modifier pins the effect, like Explorer.
    function continueDrag(mouse) {
        if (!root.dragArmed || root.dragStarted)
            return
        if ((mouse.buttons & Qt.LeftButton) === 0) {
            root.dragArmed = false
            return
        }
        const dx = mouse.x - root.dragStartX
        const dy = mouse.y - root.dragStartY
        if (Math.sqrt(dx * dx + dy * dy) < 6)
            return

        root.dragArmed = false
        root.dragStarted = true
        root.suppressClick = true
        const paths = root.dragPaths
        root.dragPaths = []
        if (paths.length > 0)
            Workspace.startDragOut(paths, root.dragOutMode(mouse.modifiers))
    }

    function dragOutMode(modifiers) {
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

    function dropUrls(drag) {
        const urls = []
        for (let index = 0; index < drag.urls.length; index += 1)
            urls.push(String(drag.urls[index]))
        return urls
    }

    function dropActionName(action) {
        if (action & Qt.LinkAction)
            return "link"
        if (action & Qt.MoveAction)
            return "move"
        if (action & Qt.CopyAction)
            return "copy"
        return "copy"
    }

    function qtAction(name) {
        return name === "move" ? Qt.MoveAction
            : name === "link" ? Qt.LinkAction : Qt.CopyAction
    }

    function parentPath(path) {
        const text = String(path).replace(/[\\/]+$/, "")
        const index = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"))
        if (index < 0)
            return ""
        let parent = text.substring(0, index)
        if (/^[a-zA-Z]:$/.test(parent))
            parent += text.charAt(index)
        return parent
    }

    /// A copy or move into the folder the sources already live in would only
    /// duplicate the rows on screen; a mixed batch still has something to
    /// transfer. A link drop into the same folder is a normal operation —
    /// that is where shortcutting a file next to itself lands.
    function canDrop(paths, destination, action) {
        if (paths.length === 0 || destination === "")
            return false
        let allHere = true
        for (let index = 0; index < paths.length; index += 1) {
            if (paths[index] === destination)
                return false
            if (root.parentPath(paths[index]) !== destination)
                allHere = false
        }
        return action === "link" || !allHere
    }

    /// The folder under `y`: a directory row is the target, anything else
    /// (a file row, the empty area) means the directory on screen.
    function dropTargetAt(x, y) {
        if (root.model === null || root.model.path === "")
            return ""
        const index = list.indexAt(x, list.contentY + y)
        if (index >= 0 && root.model.isDirectoryAt(index))
            return root.model.pathAt(index)
        return root.model.path
    }

    function updateDropTarget(drag) {
        if (root.searchModel !== null) {
            root.dropTargetPath = ""
            root.dropAction = ""
            return false
        }
        const target = root.dropTargetAt(drag.x, drag.y)
        const action = root.dropActionName(drag.proposedAction)
        if (!root.canDrop(root.dropSourcePaths, target, action)) {
            root.dropTargetPath = ""
            root.dropAction = ""
            return false
        }
        root.dropTargetPath = target
        root.dropAction = action
        return true
    }

    function clearDrop() {
        root.dropSourcePaths = []
        root.dropTargetPath = ""
        root.dropAction = ""
    }

    function copySelection(cut) {
        let paths = root.selectedPathList()
        if (paths.length === 0) {
            const path = root.currentPath()
            if (path !== "")
                paths = [path]
        }
        if (paths.length > 0)
            Workspace.setClipboard(cut ? "cut" : "copy", paths)
    }

    function deleteSelection(permanent) {
        const paths = root.selectedPathList()
        if (paths.length === 0)
            return
        if (permanent)
            Workspace.deleteEntries(paths)
        else if (root.remote)
            root.remoteDeleteRequested(paths)
        else
            Workspace.trashEntries(paths)
    }

    function handleKey(event) {
        const ctrl = (event.modifiers & Qt.ControlModifier) !== 0
        const shift = (event.modifiers & Qt.ShiftModifier) !== 0

        if (event.key === Qt.Key_Delete) {
            root.deleteSelection(shift)
            return true
        }
        if (event.key === Qt.Key_F2) {
            const path = root.currentPath()
            if (path !== "")
                root.renameRequested(path, path.split(/[\\/]/).pop())
            return true
        }
        if (event.key === Qt.Key_F5) {
            if (root.model !== null)
                root.model.reload()
            return true
        }
        if (event.key === Qt.Key_Backspace) {
            root.upRequested()
            return true
        }
        if (event.key === Qt.Key_Escape) {
            if (root.tab !== null) {
                const hadSelection = root.selectionCount > 0
                root.tab.clearSelection()
                if (hadSelection)
                    root.selectionChanged()
            }
            return true
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            if (list.currentIndex >= 0)
                root.handleActivate(list.currentIndex)
            return true
        }
        if (ctrl && shift && event.key === Qt.Key_N) {
            root.createFolderRequested()
            return true
        }
        if (ctrl && event.key === Qt.Key_A) {
            root.selectAll()
            return true
        }
        if (ctrl && event.key === Qt.Key_C) {
            root.copySelection(false)
            return true
        }
        if (ctrl && event.key === Qt.Key_X) {
            root.copySelection(true)
            return true
        }
        if (ctrl && event.key === Qt.Key_V) {
            if (root.model !== null)
                Workspace.pasteInto(root.model.path)
            return true
        }
        if (ctrl && shift && event.key === Qt.Key_Z) {
            Workspace.redo()
            return true
        }
        if (ctrl && event.key === Qt.Key_Z) {
            Workspace.undo()
            return true
        }
        return false
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    Rectangle {
        id: columnHeader
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 34
        color: root.headerColor

        Text {
            x: 36
            anchors.verticalCenter: parent.verticalCenter
            text: "名称"
            color: root.mutedColor
            font.pixelSize: 12
        }

        Text {
            anchors.right: modifiedLeft.left
            anchors.rightMargin: 24
            anchors.verticalCenter: parent.verticalCenter
            width: 90
            horizontalAlignment: Text.AlignRight
            text: "大小"
            color: root.mutedColor
            font.pixelSize: 12
        }

        Item {
            id: modifiedLeft
            anchors.right: parent.right
            anchors.rightMargin: 156
            width: 1
            height: 1
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            width: 140
            horizontalAlignment: Text.AlignRight
            text: "修改时间"
            color: root.mutedColor
            font.pixelSize: 12
        }
    }

    Text {
        anchors.centerIn: parent
        visible: root.model !== null && root.model.finished && root.model.loaded === 0
        text: root.searchModel !== null
            ? (root.searchModel.status !== "" ? root.searchModel.status : "没有匹配项")
            : (root.model !== null && root.model.status !== "0 项"
                ? root.model.status : "空目录")
        color: root.mutedColor
        font.pixelSize: 13
    }

    /// The space under the last row is part of the listing to a user, so a
    /// right-click there has to open the empty-area menu. It sits behind the
    /// ListView: the delegates' own right-clicks are above it, and the
    /// Flickable only claims the left button.
    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        acceptedButtons: Qt.RightButton
        onPressed: function (mouse) {
            const point = mapToItem(null, mouse.x, mouse.y)
            root.emptyContextRequested(point.x, point.y)
        }
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        model: root.model
        reuseItems: true
        cacheBuffer: 2600
        clip: true
        focus: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0
        currentIndex: -1

        /// A tap anywhere in the viewport — including the empty space below
        /// the last row — moves keyboard focus here, so an input that lost
        /// its composition (the path bar) also loses focus.
        TapHandler {
            acceptedButtons: Qt.LeftButton
            onTapped: root.takeFocus()
        }

        Keys.onPressed: function (event) {
            if (root.handleKey(event))
                event.accepted = true
        }

        delegate: Item {
            id: row

            required property int index
            required property string fileName
            required property string filePath
            required property string relativePath
            required property string sizeText
            required property string modifiedText
            required property bool isDir
            required property bool hidden
            required property bool readOnly
            required property string iconSource

            width: ListView.view.width
            height: root.rowHeight

            Rectangle {
                anchors.fill: parent
                color: (root.tab !== null && root.tab.isSelected(row.filePath))
                    ? root.selectionColor
                    : (list.currentIndex === row.index
                        ? (root.dark ? "#1d2026" : "#eef1f5")
                        : (root.dropTargetPath !== "" && row.isDir
                            && row.filePath === root.dropTargetPath
                            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b,
                                      root.dark ? 0.24 : 0.14)
                            : (row.index % 2 === 0 ? root.contentColor : root.alternateColor)))
            }

            Rectangle {
                x: 12
                y: 6
                width: 14
                height: 14
                radius: 3
                color: row.isDir ? root.accent : (row.readOnly ? "#59606e" : "#3b4250")
                visible: rowIcon.status !== Image.Ready
            }

            Image {
                id: rowIcon
                x: 9
                y: 4
                width: 18
                height: 18
                source: row.iconSource
                sourceSize.width: Math.round(18 * Screen.devicePixelRatio)
                sourceSize.height: Math.round(18 * Screen.devicePixelRatio)
                fillMode: Image.PreserveAspectFit
                smooth: true
                visible: status === Image.Ready
                Accessible.ignored: true
            }

            Row {
                x: 36
                width: Math.max(0, row.width - 36 - 340)
                height: parent.height
                anchors.verticalCenter: parent.verticalCenter
                spacing: 10

                Text {
                    id: fileNameText
                    width: Math.min(implicitWidth, parent.width * 0.6)
                    height: parent.height
                    verticalAlignment: Text.AlignVCenter
                    text: row.fileName
                    color: row.hidden ? (root.dark ? "#6b7280" : "#9aa1ad") : root.textColor
                    font.pixelSize: 13
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Text {
                    width: Math.max(0, parent.width - fileNameText.width - parent.spacing)
                    height: parent.height
                    verticalAlignment: Text.AlignVCenter
                    visible: row.relativePath !== row.fileName && row.relativePath !== ""
                    text: row.relativePath !== row.fileName ? row.relativePath : ""
                    color: root.mutedColor
                    font.pixelSize: 11
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                }
            }

            Text {
                id: sizeTextItem
                anchors.right: modifiedTextItem.left
                anchors.rightMargin: 24
                anchors.verticalCenter: parent.verticalCenter
                width: 90
                horizontalAlignment: Text.AlignRight
                text: row.sizeText
                color: root.mutedColor
                font.pixelSize: 12
                textFormat: Text.PlainText
            }

            Text {
                id: modifiedTextItem
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 140
                horizontalAlignment: Text.AlignRight
                text: row.modifiedText
                color: root.mutedColor
                font.pixelSize: 12
                textFormat: Text.PlainText
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                onPressed: function (mouse) {
                    if (mouse.button === Qt.LeftButton)
                        root.prepareDrag(row.index, mouse)
                }
                onPositionChanged: function (mouse) {
                    root.continueDrag(mouse)
                }
                onReleased: root.dragArmed = false
                onCanceled: root.dragArmed = false
                onClicked: function (mouse) {
                    if (mouse.button === Qt.LeftButton && root.suppressClick) {
                        root.suppressClick = false
                        return
                    }
                    if (mouse.button === Qt.RightButton) {
                        if (root.tab === null || !root.tab.isSelected(row.filePath))
                            root.handleClick(row.index, false, false)
                        const point = row.mapToItem(null, mouse.x, mouse.y)
                        root.contextRequested(row.filePath, row.isDir, point.x, point.y)
                        return
                    }
                    root.handleClick(row.index,
                                     (mouse.modifiers & Qt.ControlModifier) !== 0,
                                     (mouse.modifiers & Qt.ShiftModifier) !== 0)
                }
                onDoubleClicked: function (mouse) {
                    if (mouse.button === Qt.LeftButton)
                        root.handleActivate(row.index)
                }
            }
        }
    }

    /// Files dragged in from the OS: dropping on a folder row targets that
    /// folder, anything else targets the directory on screen.
    DropArea {
        id: dropArea
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom

        onEntered: function (drag) {
            root.dropSourcePaths = Workspace.localDropPaths(root.dropUrls(drag))
            if (!root.updateDropTarget(drag)) {
                drag.accepted = false
                return
            }
            drag.accept(root.qtAction(root.dropAction))
        }

        onPositionChanged: function (drag) {
            if (!root.updateDropTarget(drag)) {
                drag.accepted = false
                return
            }
            drag.accept(root.qtAction(root.dropAction))
        }

        onExited: root.clearDrop()

        onDropped: function (drop) {
            const paths = root.dropSourcePaths
            const target = root.dropTargetPath
            const action = root.dropAction
            root.clearDrop()
            if (paths.length === 0 || target === "" || action === "") {
                drop.accepted = false
                return
            }

            drop.accept(root.qtAction(action))
            if (action === "link")
                Workspace.createShortcuts(paths, target)
            else
                Workspace.transferEntries(paths, target, action === "move")
            if (root.tab !== null) {
                root.tab.clearSelection()
                root.selectionChanged()
            }
        }

        Rectangle {
            anchors.fill: parent
            visible: dropArea.containsDrag && root.dropTargetPath !== ""
            color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b,
                           root.dark ? 0.10 : 0.05)
            border.color: root.accent
            border.width: 2
            radius: 6

            Text {
                anchors.centerIn: parent
                text: (root.dropAction === "move" ? "移动到 "
                      : root.dropAction === "link" ? "创建快捷方式到 " : "复制到 ")
                      + (root.dropTargetPath === root.model.path
                         ? "此文件夹"
                         : String(root.dropTargetPath).split(/[\\/]/).pop())
                color: root.textColor
                font.pixelSize: 13
            }
        }
    }

    Connections {
        target: root.tab !== null ? root.tab.listing : null

        /// The listing is about to clear its rows; the scroll position has to
        /// be read before the reset discards it.
        function onAboutToReload() {
            root.scrollPending = root.saveScroll()
        }

        /// Restore as soon as the batch holding the old viewport arrives.
        function onLoadedChanged() {
            root.restoreScroll()
        }
    }

    Connections {
        target: root.model

        function onFinishedChanged() {
            if (root.model === null || !root.model.finished)
                return
            // An operation that asked to reveal a row owns the viewport; its
            // position is where the user's new file is, not the old scroll.
            if (root.pendingFocus !== "") {
                root.scrollPending = false
                root.applyFocus()
                return
            }
            root.restoreScroll()
        }
    }
}
