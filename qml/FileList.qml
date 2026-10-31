pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import "Shortcut.js" as ShortcutJS

// The explorer listing. Three presentations over one model: the detail list,
// the card grid and Miller columns. Only the list has a header; the other two
// describe the same rows with their own geometry. Selection lives on the tab
// object so it survives a tab switch, a view change and a torn-off tab.
Item {
    id: root

    property var tab: null
    /// Action id -> canonical binding, supplied by the shell from
    /// `settings.toml`. The keys below consult it instead of hardcoding
    /// modifiers; an empty value means the action is deliberately unbound.
    property var shortcutMap: ({})
    /// Set while a filename search is showing. The same delegate and selection
    /// code then work for search rows, which expose the listing's roles plus
    /// `relativePath`. Search always uses the list: sorting and drilling are
    /// listing operations, not result operations.
    property var searchModel: null

    readonly property var model: searchModel !== null ? searchModel
        : (tab !== null ? tab.listing : null)
    /// The tab is browsing a network backend, so local-only actions (OS open,
    /// recycle bin, thumbnail extraction) are routed through the window.
    readonly property bool remote: tab !== null
        && (tab.listing.remote
            || /^(smb|sftp|ftp|webdav|webdavs|cloud):\/\//i.test(tab.listing.path))
    readonly property string viewMode: searchModel !== null ? "list" : ExplorerPrefs.viewMode
    readonly property string density: ExplorerPrefs.density
    /// Three steps on the 2 px grid. The header takes its paired value below
    /// so a row and the header above it cannot disagree by a half pixel while
    /// scrolling.
    readonly property int rowHeight: density === "compact" ? Theme.rowCompact
        : density === "spacious" ? Theme.rowSpacious : Theme.rowDefault
    /// The header is the row's paired step, the way the prototype pairs them:
    /// 28 / 32 / 36. A header that kept its own height would drift a pixel off
    /// the rows every time the density changed.
    readonly property int headerHeight: density === "compact" ? 28
        : density === "spacious" ? 36 : 32
    readonly property bool listHeaderVisible: viewMode === "list" && searchModel === null
    /// The detail columns stop at a readable measure instead of stretching
    /// with the window, the way Explorer's detail view does: past this point
    /// the row is background, and that strip still takes clicks, marquees and
    /// drops. 44 rem leaves the name column ~262 dp after the three fixed
    /// columns, which is the width the reference's table occupies.
    readonly property int contentMaxWidth: 704
    readonly property int contentWidth: Math.min(width, contentMaxWidth)
    /// Detail columns are dropped, not squeezed, when the pane is narrow: a
    /// split pane at half of a 1180px window has room for the name and the
    /// timestamp, not for four columns of ellipses.
    readonly property bool showModified: width >= 360
    readonly property bool showSize: width >= 470
    readonly property bool showType: width >= 550
    readonly property int listingTop: listHeaderVisible ? headerHeight : 0
    readonly property color contentColor: Theme.canvas
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline

    /// What the centred empty state says. A listing status of "N items" is
    /// the model's success message, anything else that arrives with zero rows
    /// is an error worth showing instead of pretending the folder is empty.
    readonly property bool emptyVisible: model !== null && model.finished
        && model.loaded === 0
    readonly property bool emptyIsSearch: searchModel !== null
    readonly property bool emptyIsError: {
        if (!emptyVisible || model === null || model.status === "")
            return false
        if (emptyIsSearch && model.status === "No matches")
            return false
        return !/^\d+ items?$/.test(model.status)
    }
    readonly property string emptyTitle: emptyIsSearch
        ? (emptyIsError ? qsTr("无法完成搜索") : qsTr("没有匹配项"))
        : (emptyIsError ? qsTr("无法打开此文件夹") : qsTr("这个文件夹是空的"))
    readonly property string emptyHint: emptyIsError ? model.status
        : emptyIsSearch ? qsTr("试试其他关键词，或检查大小写与类型筛选。")
        : qsTr("拖入文件，或在这里新建文件夹。")
    readonly property string emptyPrimaryLabel: emptyIsError ? ""
        : emptyIsSearch ? qsTr("清除搜索")
        : (remote ? "" : qsTr("新建文件夹"))
    readonly property string emptySecondaryLabel: !emptyIsSearch && !emptyIsError
        && !remote ? qsTr("粘贴") : ""

    /// Set while a key press owns the cursor, so the focus ring never paints
    /// over a row the user just clicked. The prototype's `focuskb` class is
    /// the same distinction: keyboard movement shows the 1 px signal ring,
    /// the pointer does not.
    property bool keyboardCursor: false
    property string typeAheadBuffer: ""

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
    /// The empty search state's "clear search" action. The query and the field
    /// belong to the shell, so the view only asks.
    signal clearSearchRequested()
    signal contextRequested(string path, bool isDir, real x, real y)
    signal emptyContextRequested(real x, real y)
    signal selectionChanged()
    signal remoteDeleteRequested(var paths)
    /// The Delete key moved rows to the recycle bin. The shell answers with the
    /// undo toast, because the undo stack is the window's, not the view's.
    signal trashed(int count)
    signal remoteOpenRequested(string path)
    /// Open the OS terminal at the current directory; the shell owns the
    /// command and the per-location gate.
    signal openTerminalRequested(string path)
    /// Toggle the preview panel for `path` (Space). The panel is the window's,
    /// but the key belongs to the focused list.
    signal previewRequested(string path)

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

    function binding(id) {
        return root.shortcutMap && root.shortcutMap[id] ? root.shortcutMap[id] : ""
    }

    function matches(id, event) {
        return ShortcutJS.matches(event, root.binding(id))
    }

    /// Pushes the display preferences into the tab's model. Ordering is
    /// applied in place; a filter change re-reads the directory, because the
    /// rows that survive the predicate are a different listing.
    function applyViewPreferences() {
        if (root.tab === null)
            return
        root.tab.listing.setOrder(ExplorerPrefs.sortKey, ExplorerPrefs.sortOrder,
                                  ExplorerPrefs.foldersFirst)
        root.tab.listing.setFilters(ExplorerPrefs.showHiddenFiles, ExplorerPrefs.kindFilter,
                                    ExplorerPrefs.modifiedFilter, ExplorerPrefs.sizeFilter)
    }

    onTabChanged: applyViewPreferences()
    Component.onCompleted: applyViewPreferences()

    Connections {
        target: ExplorerPrefs
        function onSortKeyChanged() { root.applyViewPreferences() }
        function onSortOrderChanged() { root.applyViewPreferences() }
        function onFoldersFirstChanged() { root.applyViewPreferences() }
        function onShowHiddenFilesChanged() { root.applyViewPreferences() }
        function onKindFilterChanged() { root.applyViewPreferences() }
        function onModifiedFilterChanged() { root.applyViewPreferences() }
        function onSizeFilterChanged() { root.applyViewPreferences() }
    }

    /// Keyboard focus lives on the view that is showing, not on this container.
    function takeFocus() {
        if (viewMode === "grid")
            gridView.takeFocus()
        else if (viewMode === "column")
            columnView.takeFocus()
        else
            list.forceActiveFocus()
    }

    function cursorIndex() {
        if (viewMode === "grid")
            return gridView.cursorIndex()
        return list.currentIndex
    }

    function currentPath() {
        return root.pathAtIndex(root.cursorIndex())
    }

    function pathAtIndex(index) {
        if (root.model === null || index < 0)
            return ""
        return root.model.pathAt(index)
    }

    function nameAt(index) {
        return root.model !== null && index >= 0 ? root.model.nameAt(index) : ""
    }

    Timer {
        id: typeAheadTimer
        interval: 800
        repeat: false
        onTriggered: root.typeAheadBuffer = ""
    }

    /// Windows-style type-ahead: a prefix match over the visible names,
    /// starting one row past the cursor for a single repeated letter so that
    /// "f", "f", "f" cycles, and on the cursor for a longer buffer so typing
    /// refines the current match. Matching wraps and ignores case.
    function typeAhead(text) {
        if (text.length !== 1 || root.model === null || root.tab === null)
            return
        root.typeAheadBuffer += text
        typeAheadTimer.restart()

        const count = root.model.loaded
        if (count <= 0)
            return
        const needle = root.typeAheadBuffer.toLowerCase()
        const current = root.cursorIndex()
        const start = current < 0 ? 0 : needle.length === 1 ? current + 1 : current
        for (let offset = 0; offset < count; offset += 1) {
            const index = (start + offset) % count
            if (!root.nameAt(index).toLowerCase().startsWith(needle))
                continue
            const path = root.pathAtIndex(index)
            if (path === "")
                continue
            if (root.viewMode === "grid") {
                gridView.setCursor(index)
                gridView.positionCursor(index)
            } else {
                list.currentIndex = index
                list.positionViewAtIndex(index, ListView.Contain)
            }
            root.tab.selectOnly(path)
            root.selectionChanged()
            return
        }
    }

    function pasteIntoCurrent() {
        if (root.model !== null && root.model.path !== "")
            Workspace.pasteInto(root.model.path)
    }

    /// Clicking the active column toggles direction; a new column starts at
    /// its conventional direction, matching the detail view in the old shell.
    function applySort(key) {
        if (ExplorerPrefs.sortKey === key) {
            ExplorerPrefs.setString("sortOrder",
                                    ExplorerPrefs.sortOrder === "asc" ? "desc" : "asc")
            return
        }
        ExplorerPrefs.setString("sortKey", key)
        ExplorerPrefs.setString("sortOrder",
                                (key === "modified" || key === "size") ? "desc" : "asc")
    }

    function toggleHiddenFiles() {
        ExplorerPrefs.setBool("showHiddenFiles", !ExplorerPrefs.showHiddenFiles)
    }

    function handleClick(index, ctrl, shift) {
        if (root.tab === null || root.model === null)
            return
        root.takeFocus()
        if (viewMode === "grid")
            gridView.setCursor(index)
        else
            list.currentIndex = index
        const path = root.pathAtIndex(index)
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
        const path = root.pathAtIndex(index)
        if (path === "")
            return
        root.activatePath(path, root.model.isDirectoryAt(index))
    }

    function activatePath(path, isDir) {
        if (isDir)
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
        if (viewMode === "grid")
            gridView.setCursor(index)
        else
            list.currentIndex = index
        root.tab.selectOnly(path)
        root.selectionChanged()
    }

    /// Reads the viewport position before the listing clears its rows. The
    /// reset that follows drops both the rows and the position, so this is the
    /// last moment it exists. Stored on the tab, keyed by the directory.
    /// Returns whether there was a position to remember.
    function saveScroll() {
        if (root.tab === null || root.searchModel !== null || viewMode !== "list")
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
                || root.pendingFocus !== "" || viewMode !== "list")
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

        const path = root.pathAtIndex(index)
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

    /// The folder under `x`,`y`: a directory row is the target, anything else
    /// (a file row, the empty area) means the directory on screen.
    function dropTargetAt(x, y) {
        if (root.model === null || root.model.path === "")
            return ""
        if (root.viewMode === "grid")
            return gridView.dropDirectoryAt(x, y)
        if (root.viewMode === "column")
            return root.model.path
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
        else {
            Workspace.trashEntries(paths)
            root.trashed(paths.length)
        }
    }

    function handleKey(event) {
        const index = root.cursorIndex()

        // The file operations below are the actions the settings panel edits.
        // Matching parses the stored binding, so a user can move Delete to
        // another key, or unbind it, without another hardcoded branch.
        if (root.matches("explorer.deletePermanent", event)) {
            root.deleteSelection(true)
            return true
        }
        if (root.matches("explorer.trash", event)) {
            root.deleteSelection(false)
            return true
        }
        if (root.matches("explorer.rename", event)) {
            const path = root.currentPath()
            if (path !== "")
                root.renameRequested(path, path.split(/[\\/]/).pop())
            return true
        }
        if (root.matches("explorer.preview", event)) {
            const path = root.currentPath()
            if (path !== "")
                root.previewRequested(path)
            return true
        }
        if (root.matches("explorer.clearSelection", event)) {
            if (root.tab !== null) {
                const hadSelection = root.selectionCount > 0
                root.tab.clearSelection()
                if (hadSelection)
                    root.selectionChanged()
            }
            return true
        }
        if (root.matches("explorer.selectAll", event)) {
            root.selectAll()
            return true
        }
        if (root.matches("explorer.copy", event)) {
            root.copySelection(false)
            return true
        }
        if (root.matches("explorer.cut", event)) {
            root.copySelection(true)
            return true
        }
        if (root.matches("explorer.paste", event)) {
            root.pasteIntoCurrent()
            return true
        }
        if (root.matches("explorer.toggleHidden", event)) {
            root.toggleHiddenFiles()
            return true
        }
        if (root.matches("explorer.redo", event) || root.matches("explorer.redoAlt", event)) {
            Workspace.redo()
            return true
        }
        if (root.matches("explorer.undo", event)) {
            Workspace.undo()
            return true
        }
        if (root.matches("explorer.openSystemTerminal", event)) {
            root.openTerminalRequested(root.model !== null ? root.model.path : "")
            return true
        }

        // Navigation is intrinsic to the focused view, not part of the shared
        // action catalogue, so it keeps its own keys.
        if (event.key === Qt.Key_F5) {
            if (root.model !== null)
                root.model.reload()
            return true
        }
        if (event.key === Qt.Key_Backspace) {
            root.upRequested()
            return true
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            if (index >= 0)
                root.handleActivate(index)
            return true
        }
        if ((event.modifiers & Qt.ControlModifier) !== 0
                && (event.modifiers & Qt.ShiftModifier) !== 0
                && event.key === Qt.Key_N) {
            root.createFolderRequested()
            return true
        }
        // Type-ahead comes last so every rebindable action and the navigation
        // keys get first refusal. Space is excluded: it is preview, and a
        // name that starts with a space is not worth stealing it for. Anything
        // below a space is excluded too: Tab arrives with a one-character text
        // of "\t", and swallowing it here would trap focus in the listing.
        if (event.text.length === 1 && event.text !== " "
                && event.text.charCodeAt(0) >= 0x20
                && (event.modifiers & (Qt.ControlModifier | Qt.AltModifier
                                       | Qt.MetaModifier)) === 0) {
            root.typeAhead(event.text)
            return true
        }
        return false
    }

    Connections {
        target: ExplorerPrefs
        function onViewModeChanged() {
            Qt.callLater(root.takeFocus)
        }
    }

    component SortCaret: Canvas {
        id: caret

        property bool ascending: true
        property color tint: Theme.ink

        width: 7
        height: 4

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            ctx.fillStyle = caret.tint
            ctx.beginPath()
            if (caret.ascending) {
                ctx.moveTo(0, 4)
                ctx.lineTo(7, 4)
                ctx.lineTo(3.5, 0)
            } else {
                ctx.moveTo(0, 0)
                ctx.lineTo(7, 0)
                ctx.lineTo(3.5, 4)
            }
            ctx.closePath()
            ctx.fill()
        }

        onAscendingChanged: requestPaint()
        onTintChanged: requestPaint()
    }

    component SortHeader: Item {
        id: cell

        property string label: ""
        property string sortKey: ""
        property bool alignRight: false
        readonly property bool active: ExplorerPrefs.sortKey === cell.sortKey

        signal clicked()

        Row {
            anchors.left: cell.alignRight ? undefined : parent.left
            anchors.right: cell.alignRight ? parent.right : undefined
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 5

            Text {
                text: cell.label
                color: cell.active ? Theme.ink
                    : (headerMouse.containsMouse ? Theme.inkMuted : Theme.inkFaint)
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.55
                elide: Text.ElideRight
            }

            SortCaret {
                anchors.verticalCenter: parent.verticalCenter
                visible: cell.active
                ascending: ExplorerPrefs.sortOrder === "asc"
                tint: Theme.ink
            }
        }

        MouseArea {
            id: headerMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: cell.clicked()
        }
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
        height: root.headerHeight
        visible: root.listHeaderVisible
        color: root.contentColor

        /// The column track shares the row cap, so a heading never floats
        /// over blank background to the right of the rows it labels.
        Item {
            id: headerTrack
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: root.contentWidth

            SortHeader {
                id: nameHeader
                anchors.left: parent.left
                anchors.leftMargin: 36
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: Math.max(0, (root.showType ? typeHeader.x
                    : root.showSize ? sizeHeader.x
                    : root.showModified ? modifiedHeader.x : parent.width) - 8 - x)
                label: qsTr("名称")
                sortKey: "name"
                onClicked: root.applySort("name")
            }

            SortHeader {
                id: typeHeader
                anchors.right: root.showSize ? sizeHeader.left
                    : (root.showModified ? modifiedHeader.left : parent.right)
                anchors.rightMargin: 16
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 108
                visible: root.showType
                label: qsTr("类型")
                sortKey: "type"
                onClicked: root.applySort("type")
            }

            SortHeader {
                id: sizeHeader
                anchors.right: root.showModified ? modifiedHeader.left : parent.right
                anchors.rightMargin: root.showModified ? 24 : 16
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 92
                visible: root.showSize
                label: qsTr("大小")
                sortKey: "size"
                alignRight: true
                onClicked: root.applySort("size")
            }

            SortHeader {
                id: modifiedHeader
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 142
                visible: root.showModified
                label: qsTr("修改时间")
                sortKey: "modified"
                alignRight: true
                onClicked: root.applySort("modified")
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: root.hairline
        }
    }

    Item {
        id: viewport
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: root.listingTop
        anchors.bottom: parent.bottom

        /// The space under the last row is part of the listing to a user, so
        /// a right-click there has to open the empty-area menu. It sits
        /// behind the views: the delegates' own right-clicks are above it.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.RightButton
            onPressed: function (mouse) {
                const point = mapToItem(null, mouse.x, mouse.y)
                root.emptyContextRequested(point.x, point.y)
            }
        }

        ListView {
            id: list
            anchors.fill: parent
            visible: root.viewMode === "list"
            model: root.model
            reuseItems: true
            cacheBuffer: 2600
            clip: true
            focus: true
            /// Tab reaches the listing from the sidebar and leaves it for the
            /// next region. Qt walks tab stops in scene order, so the three
            /// panes come out in the order the window is laid out.
            activeFocusOnTab: true
            boundsBehavior: Flickable.StopAtBounds
            spacing: 0
            currentIndex: -1

            /// Startup split, same contract as the overview: the model marks
            /// its publish point in Rust, this is the layout half, and the
            /// Rust side keeps only the first report of each stage.
            onContentHeightChanged: if (root.visible && count > 0 && contentHeight > 0)
                Startup.mark("qml-first-list")

            /// The listing announces itself as a list of rows; the empty space
            /// under the last row is part of it, which is why the tap handler
            /// below belongs to this element and not to a rectangle.
            Accessible.role: Accessible.List
            Accessible.name: qsTr("文件列表")

            /// A tap anywhere in the viewport — including the empty space
            /// below the last row — moves keyboard focus here.
            TapHandler {
                acceptedButtons: Qt.LeftButton
                onTapped: {
                    root.keyboardCursor = false
                    root.takeFocus()
                }
            }

            Keys.onPressed: function (event) {
                root.keyboardCursor = true
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
                required property string iconSource
                required property string category
                required property string extension
                required property string thumbnailSource
                /// `added`, `modified`, `untracked`, or empty when the file is
                /// clean. The search model answers empty for every row.
                required property string gitStatus

                /// The row stops at the column track's cap; ListView leaves the
                /// rest of the viewport as background, exactly like the
                /// Tauri shell's Windows-style detail view.
                width: Math.min(ListView.view.width, root.contentMaxWidth)
                height: root.rowHeight

                readonly property bool selected: root.tab !== null
                    && root.tab.isSelected(row.filePath)
                readonly property bool dropTarget: root.dropTargetPath !== ""
                    && row.isDir && row.filePath === root.dropTargetPath
                readonly property bool current: list.currentIndex === row.index
                readonly property bool focusedCursor: list.activeFocus
                    && root.keyboardCursor && row.current
                readonly property bool showThumbnail: ExplorerPrefs.rowThumbnails
                    && !row.isDir && row.thumbnailSource !== ""
                readonly property bool hasGitStatus: row.gitStatus !== ""
                readonly property string gitLetter: row.gitStatus === "added" ? "A"
                    : (row.gitStatus === "modified" ? "M"
                        : (row.gitStatus === "untracked" ? "U" : ""))
                readonly property color gitBadgeColor: row.gitStatus === "modified"
                    ? Theme.warn
                    : (row.gitStatus === "added"
                        ? Theme.success
                        : Theme.link)

                /// The row, not the four cells inside it: a listing read cell
                /// by cell is unlistenable, and the selection state lives on
                /// the row. Press is the double click, so a reader has the same
                /// one gesture a pointer does.
                Accessible.role: Accessible.ListItem
                Accessible.name: row.fileName
                Accessible.description: row.isDir ? qsTr("文件夹") : qsTr("文件")
                Accessible.selectable: true
                Accessible.selected: row.selected
                Accessible.onPressAction: root.handleActivate(row.index)

                Rectangle {
                    anchors.fill: parent
                    color: row.selected
                        ? (rowHover.containsMouse ? Theme.signalSoft2 : Theme.signalSoft)
                        : row.dropTarget ? Theme.signalSoft
                        : (rowHover.containsMouse ? Theme.hover : "transparent")
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
                    visible: row.dropTarget
                    color: "transparent"
                    border.width: 1
                    border.color: Theme.signal
                }

                Rectangle {
                    anchors.fill: parent
                    visible: row.focusedCursor
                    color: "transparent"
                    border.width: 1
                    border.color: Theme.signal
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.rowRule
                }

                Image {
                    id: rowIcon
                    x: 12
                    width: 16
                    height: 16
                    anchors.verticalCenter: parent.verticalCenter
                    source: row.showThumbnail || ExplorerPrefs.iconStyle !== "system"
                        ? "" : row.iconSource
                    sourceSize.width: Math.round(16 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(16 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    visible: !row.showThumbnail && ExplorerPrefs.iconStyle === "system"
                        && status === Image.Ready
                    Accessible.ignored: true
                }

                Image {
                    x: 10
                    width: 20
                    height: 20
                    anchors.verticalCenter: parent.verticalCenter
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
                    x: 12
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !row.showThumbnail && (ExplorerPrefs.iconStyle !== "system"
                        || rowIcon.status !== Image.Ready)
                    category: row.category
                    extension: row.extension
                    isDir: row.isDir
                    size: 16
                    tint: row.selected || rowHover.containsMouse
                        ? Theme.ink : Theme.inkMuted
                }

                Row {
                    id: nameCell
                    x: 36
                    width: Math.max(0, (root.showType ? typeTextItem.x
                        : root.showSize ? sizeTextItem.x
                        : root.showModified ? modifiedTextItem.x : row.width) - 8 - x)
                    height: parent.height
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Text {
                        id: fileNameText
                        width: Math.min(implicitWidth,
                            Math.max(0, parent.width * 0.62 - (row.hasGitStatus ? 22 : 0)))
                        height: parent.height
                        verticalAlignment: Text.AlignVCenter
                        text: row.fileName
                        color: row.hidden ? Theme.inkFaint : root.textColor
                        font.pixelSize: Theme.typeName
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                    }

                    Rectangle {
                        width: 14
                        height: 14
                        radius: Theme.radiusXs
                        anchors.verticalCenter: parent.verticalCenter
                        visible: row.hasGitStatus
                        color: "transparent"
                        border.width: 1
                        border.color: Theme.hairlineStrong

                        Text {
                            anchors.centerIn: parent
                            text: row.gitLetter
                            color: row.gitBadgeColor
                            font.pixelSize: Theme.typeMicro
                            font.family: Theme.monoFamily
                            font.weight: Font.Medium
                            textFormat: Text.PlainText
                        }
                    }

                    Text {
                        width: Math.max(0, parent.width - fileNameText.width - parent.spacing
                            - (row.hasGitStatus ? 22 : 0))
                        height: parent.height
                        verticalAlignment: Text.AlignVCenter
                        visible: row.relativePath !== row.fileName && row.relativePath !== ""
                        text: row.relativePath !== row.fileName ? row.relativePath : ""
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                    }
                }

                Text {
                    id: typeTextItem
                    anchors.right: root.showSize ? sizeTextItem.left
                        : (root.showModified ? modifiedTextItem.left : parent.right)
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 108
                    visible: root.showType
                    text: row.isDir ? qsTr("文件夹")
                        : (row.extension !== "" ? row.extension.toUpperCase() : qsTr("文件"))
                    color: root.mutedColor
                    font.pixelSize: Theme.typeSmall
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Text {
                    id: sizeTextItem
                    anchors.right: root.showModified ? modifiedTextItem.left : parent.right
                    anchors.rightMargin: root.showModified ? 24 : 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 92
                    visible: root.showSize
                    horizontalAlignment: Text.AlignRight
                    text: row.sizeText
                    color: root.mutedColor
                    font.pixelSize: Theme.typeSmall
                    font.features: { "tnum": 1 }
                    textFormat: Text.PlainText
                }

                Text {
                    id: modifiedTextItem
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 142
                    visible: root.showModified
                    horizontalAlignment: Text.AlignRight
                    text: row.modifiedText
                    color: root.mutedColor
                    font.pixelSize: Theme.typeSmall
                    font.features: { "tnum": 1 }
                    textFormat: Text.PlainText
                }

                MouseArea {
                    id: rowHover
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onPressed: function (mouse) {
                        root.keyboardCursor = false
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

        FileGrid {
            id: gridView
            anchors.fill: parent
            visible: root.viewMode === "grid"
            tab: root.tab
            model: root.model
            dropTargetPath: root.dropTargetPath
            keyHandler: root.handleKey

            onActivateRequested: function (path, isDir) {
                root.activatePath(path, isDir)
            }
            onContextRequested: function (path, isDir, x, y) {
                root.contextRequested(path, isDir, x, y)
            }
            onEmptyContextRequested: function (x, y) {
                root.emptyContextRequested(x, y)
            }
            onSelectionChanged: root.selectionChanged()
            onDragOutRequested: function (paths, mode) {
                Workspace.startDragOut(paths, mode)
            }
        }

        ColumnView {
            id: columnView
            anchors.fill: parent
            visible: root.viewMode === "column"
            tab: root.tab

            onActivateRequested: function (path, isDir) {
                root.activatePath(path, isDir)
            }
            onContextRequested: function (path, isDir, x, y) {
                root.contextRequested(path, isDir, x, y)
            }
            onEmptyContextRequested: function (x, y) {
                root.emptyContextRequested(x, y)
            }
            onSelectionChanged: root.selectionChanged()
            onDragOutRequested: function (paths, mode) {
                Workspace.startDragOut(paths, mode)
            }
        }

        /// Rubber-band selection over the detail list. The overlay sits above
        /// the rows so a drag that began on empty space keeps receiving moves
        /// over them; a press that began on a row is handed down instead.
        MouseArea {
            id: listMarquee
            anchors.fill: parent
            z: 20
            acceptedButtons: Qt.LeftButton
            visible: root.viewMode === "list"
            preventStealing: true
            // The band can be dragged past the pane's edge; the listing's own
            // clip is the boundary it should stop at.
            clip: true

            property bool active: false
            property real startX: 0
            property real startY: 0
            property real startContentY: 0
            property real pointerX: 0
            property real pointerY: 0
            property var baseSelection: []

            Timer {
                interval: 16
                repeat: true
                running: listMarquee.active
                onTriggered: {
                    const margin = 28
                    if (listMarquee.pointerY < margin)
                        list.contentY = Math.max(0, list.contentY - 12)
                    else if (listMarquee.pointerY > list.height - margin)
                        list.contentY = Math.max(0, Math.min(
                            list.contentHeight - list.height, list.contentY + 12))
                    else
                        return
                    listMarquee.apply()
                }
            }

            function apply() {
                if (!active || root.tab === null || root.model === null)
                    return
                // Rows stop at the column track's edge, so the band has to
                // cross that box horizontally as well: dragging over the blank
                // area right of the columns selects nothing.
                const left = Math.min(startX, pointerX)
                const right = Math.max(startX, pointerX)
                const paths = []
                if (right > 0 && left < root.contentWidth) {
                    const currentContent = list.contentY + pointerY
                    const first = Math.floor(Math.min(startContentY, currentContent)
                                              / root.rowHeight)
                    const last = Math.floor(Math.max(startContentY, currentContent)
                                            / root.rowHeight)
                    for (let row = Math.max(0, first);
                         row <= Math.min(root.model.loaded - 1, last); row += 1) {
                        const path = root.model.pathAt(row)
                        if (path !== "")
                            paths.push(path)
                    }
                }
                const next = baseSelection.length > 0
                    ? Array.from(new Set(baseSelection.concat(paths))) : paths
                root.tab.selectMany(next)
                root.selectionChanged()
            }

            onPressed: function (mouse) {
                if (root.model === null || root.searchModel !== null) {
                    mouse.accepted = false
                    return
                }
                const index = list.indexAt(mouse.x, list.contentY + mouse.y)
                if (index >= 0) {
                    mouse.accepted = false
                    return
                }
                root.keyboardCursor = false
                list.forceActiveFocus()
                baseSelection = (mouse.modifiers & Qt.ControlModifier) !== 0
                    ? root.selectedPathList() : []
                startX = mouse.x
                startY = mouse.y
                startContentY = list.contentY + mouse.y
                pointerX = mouse.x
                pointerY = mouse.y
                active = true
                if (baseSelection.length === 0 && root.tab !== null) {
                    root.tab.clearSelection()
                    root.selectionChanged()
                }
                mouse.accepted = true
            }

            onPositionChanged: function (mouse) {
                if (!active)
                    return
                pointerX = mouse.x
                pointerY = mouse.y
                apply()
            }

            onReleased: {
                if (!active)
                    return
                const width = Math.abs(pointerX - startX)
                const height = Math.abs(list.contentY + pointerY - startContentY)
                active = false
                if (width < 4 && height < 4 && baseSelection.length === 0 && root.tab !== null) {
                    root.tab.clearSelection()
                    root.selectionChanged()
                }
            }

            onCanceled: active = false

            Rectangle {
                visible: listMarquee.active
                // The band follows the pointer in both axes. A fixed
                // full-width plate jumped to the columns' edge on the first
                // pixel of the drag, which read as the bar animating on its
                // own rather than as the rectangle the user was drawing.
                x: Math.min(listMarquee.startX, listMarquee.pointerX)
                y: Math.max(0, Math.min(listMarquee.startContentY,
                                         list.contentY + listMarquee.pointerY) - list.contentY)
                width: Math.abs(listMarquee.pointerX - listMarquee.startX)
                height: Math.max(0, Math.abs(list.contentY + listMarquee.pointerY
                                             - listMarquee.startContentY))
                color: Theme.signalSoft
                border.width: 1
                border.color: Theme.signal
                radius: Theme.radiusXs
            }
        }

        EmptyState {
            anchors.fill: parent
            z: 30
            visible: root.emptyVisible
            glyph: root.emptyIsSearch ? "search" : "folder"
            title: root.emptyTitle
            hint: root.emptyHint
            primaryLabel: root.emptyPrimaryLabel
            secondaryLabel: root.emptySecondaryLabel
            onPrimaryClicked: {
                if (root.emptyIsSearch)
                    root.clearSearchRequested()
                else
                    root.createFolderRequested()
            }
            onSecondaryClicked: root.pasteIntoCurrent()
        }
    }

    /// Files dragged in from the OS: dropping on a folder row targets that
    /// folder, anything else targets the directory on screen.
    DropArea {
        id: dropArea
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: root.listingTop
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
            color: Theme.signalSoft
            border.color: Theme.signal
            border.width: 1
            radius: Theme.radiusSm

            Text {
                anchors.centerIn: parent
                readonly property string target:
                    (root.model !== null && root.dropTargetPath === root.model.path)
                        ? qsTr("此文件夹")
                        : String(root.dropTargetPath).split(/[\\/]/).pop()
                text: root.dropAction === "move"
                    ? qsTr("移动到 %1").arg(target)
                    : root.dropAction === "link"
                        ? qsTr("创建快捷方式到 %1").arg(target)
                        : qsTr("复制到 %1").arg(target)
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
