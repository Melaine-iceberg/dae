import QtQuick
import QtQuick.Window
import dae 1.0
import "Shell.js" as ShellState

// One top-level window and its tabs.
//
// Gate 4 measured the second window on the same native path as the first:
// `WindowController.attach()` writes the frame, backdrop and hit test for
// whatever HWND this window got. Tear-off is therefore "create another
// ShellWindow, hand it the tab object"; there is no pool, and the tab's model
// (and its rows) moves without being copied.
//
// This file also owns every command that needs a dialog or a menu: the
// listing itself only selects rows and emits intent, so a second view of the
// same model (a future details pane) would not have to duplicate the policy.
Window {
    id: root

    property string token: "dae-main"
    property bool firstWindow: false
    property string startupPath: ""
    property var initialTabs: []
    property int initialActive: 0
    property bool dark: true
    property string material: "none"

    objectName: token
    width: 1180
    height: 780
    visible: false
    title: root.activeTab !== null ? root.activeTab.title : "dae"
    color: "transparent"
    flags: Qt.Window | Qt.FramelessWindowHint
        | Qt.WindowMinimizeButtonHint | Qt.WindowMaximizeButtonHint
        | Qt.WindowCloseButtonHint

    property var tabs: []
    property int activeIndex: 0
    property bool closingDisposed: false
    property bool terminalActive: false
    property bool sidebarVisible: true
    property bool previewVisible: true
    property string searchQuery: ""
    property string searchMode: "name"
    property bool searchRegex: false
    property bool searchCase: false
    property string searchFilter: ""
    property string promptMode: ""
    property string promptPath: ""
    property string contextPath: ""
    property bool contextIsDir: false

    readonly property var activeTab: (activeIndex >= 0 && activeIndex < tabs.length)
        ? tabs[activeIndex] : null
    readonly property string activePath: activeTab !== null ? activeTab.listing.path : ""
    readonly property bool searchActive: searchQuery.trim().length > 0
    readonly property bool contentSearchActive: searchActive && searchMode === "content"
    readonly property bool searchBusy: activeTab === null ? false
        : (searchMode === "content" ? activeTab.contentSearch.busy : activeTab.nameSearch.busy)
    readonly property string mod: Qt.platform.os === "osx" ? "Meta" : "Ctrl"
    readonly property color accent: Workspace.accent
    readonly property string sourceLabel: config === null ? ""
        : config.startupSource === "recent" ? "最近访问"
        : config.startupSource === "commandLine" ? "命令行" : "主目录"
    readonly property string profileText: config === null ? ""
        : "启动: " + sourceLabel
          + " · " + (config.settingsFound ? "settings.toml" : "默认设置")
          + " · " + (config.configDir !== "" ? config.configDir : "配置目录不可用")

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"

    Loader {
        id: configLoader
        active: root.firstWindow
        sourceComponent: Component {
            AppConfig {}
        }
    }

    readonly property var config: configLoader.item
    readonly property bool modalOpen: menu.visible || namePrompt.visible || settingsPanel.visible

    Timer {
        id: searchTimer
        interval: root.searchMode === "content" ? 350 : 180
        repeat: false
        onTriggered: root.runSearch()
    }

    onActiveIndexChanged: {
        root.resetSearchState()
        previewPanel.showPaths([])
    }

    WindowController {
        id: chrome
        token: root.objectName
    }

    Component {
        id: tabComponent
        Tab {}
    }

    // Loaded by URL rather than declared as a `TearOffWindow {}` component:
    // that type wraps this file, and a static reference both ways is a cycle
    // the QML engine refuses at load time. PreferSynchronous keeps the
    // `createObject` below returning a live window, which is what makes the
    // "close the source only after the new window exists" ordering work.
    function createWindow(properties) {
        const component = Qt.createComponent("qrc:/qt/qml/dae/qml/TearOffWindow.qml",
                                             Component.PreferSynchronous)
        if (component === null || component.status !== Component.Ready) {
            console.warn("dae: could not create a tab window: "
                         + (component ? component.errorString() : "no component"))
            return null
        }
        return component.createObject(null, properties)
    }

    function createTab(path) {
        const tab = tabComponent.createObject(null)
        if (tab === null)
            return null
        if (path && path.length > 0)
            tab.navigate(path)
        return tab
    }

    function addTab(path) {
        const tab = createTab(path !== undefined ? path : root.activePath)
        if (tab === null)
            return
        const next = root.tabs.slice()
        const at = Math.max(0, Math.min(root.activeIndex + 1, next.length))
        next.splice(at, 0, tab)
        root.tabs = next
        root.activeIndex = at
        Qt.callLater(fileList.takeFocus)
    }

    function closeTab(index) {
        if (index < 0 || index >= root.tabs.length)
            return
        const closing = root.tabs[index]
        const next = root.tabs.slice()
        next.splice(index, 1)
        if (next.length === 0) {
            if (closing)
                closing.dispose()
            root.tabs = next
            root.close()
            return
        }
        root.tabs = next
        if (root.activeIndex > index)
            root.activeIndex -= 1
        else if (root.activeIndex === index)
            root.activeIndex = Math.min(index, next.length - 1)
        if (closing)
            closing.dispose()
    }

    function moveTab(from, to) {
        if (from === to || from < 0 || from >= root.tabs.length || to < 0 || to >= root.tabs.length)
            return
        const next = root.tabs.slice()
        const tab = next.splice(from, 1)[0]
        next.splice(to, 0, tab)
        root.tabs = next
        if (root.activeIndex === from)
            root.activeIndex = to
        else if (from < root.activeIndex && to >= root.activeIndex)
            root.activeIndex -= 1
        else if (from > root.activeIndex && to <= root.activeIndex)
            root.activeIndex += 1
    }

    function cycleTab(delta) {
        if (root.tabs.length < 2)
            return
        root.activeIndex = (root.activeIndex + delta + root.tabs.length) % root.tabs.length
    }

    /// Receives a tab from another window's drag. `localX` is in this window's
    /// logical coordinates so the tab lands at the drop position.
    function adoptTab(tab, localX) {
        if (!tab)
            return
        const next = root.tabs.slice()
        let at = tabStrip.indexAt(localX)
        if (at < 0 || at > next.length)
            at = next.length
        next.splice(at, 0, tab)
        root.tabs = next
        root.activeIndex = at
        root.requestActivate()
        Shell.focusWindow(root.token)
    }

    /// Creates the gate-4 window and moves this tab into it. The source window
    /// closes only after the new one exists, so quitting on the last window
    /// closed cannot race the tear-off.
    function detachTab(index, globalX, globalY) {
        if (index < 0 || index >= root.tabs.length)
            return
        const tab = root.tabs[index]
        const next = root.tabs.slice()
        next.splice(index, 1)
        const sourceEmpty = next.length === 0
        root.tabs = next
        if (!sourceEmpty)
            root.activeIndex = Math.min(index, next.length - 1)

        const window = root.createWindow({
            "token": Shell.newToken(),
            "firstWindow": false,
            "initialTabs": [tab],
            "initialActive": 0,
            "dark": root.dark,
            "material": root.material,
            "x": Math.round(globalX - 90),
            "y": Math.round(globalY - 18),
            "width": root.width,
            "height": root.height
        })
        if (window === null) {
            const restored = root.tabs.slice()
            restored.splice(index, 0, tab)
            root.tabs = restored
            root.activeIndex = index
            return
        }
        if (sourceEmpty)
            root.close()
    }

    function dropTabOnWindow(index, token, targetLocalX) {
        if (index < 0 || index >= root.tabs.length)
            return
        const target = ShellState.windowFor(token)
        if (target === null) {
            root.detachTab(index, Shell.cursorLogicalX(), Shell.cursorLogicalY())
            return
        }
        const tab = root.tabs[index]
        const next = root.tabs.slice()
        next.splice(index, 1)
        const sourceEmpty = next.length === 0
        root.tabs = next
        if (!sourceEmpty)
            root.activeIndex = Math.min(index, next.length - 1)
        target.adoptTab(tab, targetLocalX)
        if (sourceEmpty)
            root.close()
    }

    function newWindowAt(path) {
        return root.createWindow({
            "token": Shell.newToken(),
            "firstWindow": false,
            "startupPath": path || root.activePath,
            "dark": root.dark,
            "material": root.material,
            "x": Math.round(root.x + 32),
            "y": Math.round(root.y + 32),
            "width": root.width,
            "height": root.height
        })
    }

    function navigate(path) {
        if (root.activeTab === null || !path || path.length === 0)
            return
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.navigate(path)
        Workspace.recordRecent(path, true, false)
        Qt.callLater(fileList.takeFocus)
    }

    function resetSearchState() {
        searchTimer.stop()
        for (let index = 0; index < root.tabs.length; index += 1) {
            if (root.tabs[index])
                root.tabs[index].cancelSearch()
        }
        root.searchQuery = ""
        root.searchRegex = false
        root.searchCase = false
        root.searchFilter = ""
        if (searchBar)
            searchBar.setText("")
    }

    function acceptSearchText(value) {
        root.searchQuery = value
        root.scheduleSearch()
    }

    function scheduleSearch() {
        if (root.searchActive)
            searchTimer.restart()
        else
            root.runSearch()
    }

    function toggleSearchMode() {
        root.searchMode = root.searchMode === "name" ? "content" : "name"
        if (root.searchActive)
            searchTimer.restart()
    }

    function runSearch() {
        const tab = root.activeTab
        if (tab === null)
            return
        const query = root.searchQuery.trim()
        tab.nameSearch.cancel()
        tab.contentSearch.cancel()
        if (query === "")
            return
        tab.clearSelection()
        previewPanel.showPaths([])
        if (root.searchMode === "content")
            tab.contentSearch.search(tab.listing.path, query, root.searchRegex,
                                    root.searchCase, root.searchFilter)
        else
            tab.nameSearch.search(tab.listing.path, query)
    }

    function acceptPreview(path, isDir) {
        if (isDir)
            root.navigate(path)
        else
            Workspace.openPath(path)
    }

    function parentOf(path) {
        if (!path)
            return ""
        let trimmed = path
        while (trimmed.length > 1
               && (trimmed.endsWith("/") || trimmed.endsWith("\\")))
            trimmed = trimmed.slice(0, -1)
        const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
        if (cut < 0)
            return ""
        let parent = trimmed.slice(0, cut)
        if (parent.length === 2 && parent.charAt(1) === ":")
            parent += "\\"
        return parent
    }

    function goUp() {
        const parent = root.parentOf(root.activePath)
        if (parent !== "")
            root.navigate(parent)
    }

    function disposeTabs() {
        if (root.closingDisposed)
            return
        root.closingDisposed = true
        for (let index = 0; index < root.tabs.length; index += 1) {
            if (root.tabs[index])
                root.tabs[index].dispose()
        }
        root.tabs = []
    }

    function currentSelection() {
        const paths = fileList.selectedPathList()
        if (paths.length > 0)
            return paths
        const path = root.contextPath !== "" ? root.contextPath : fileList.currentPath()
        return path !== "" ? [path] : []
    }

    function promptCreate(folder) {
        if (root.modalOpen)
            return
        root.promptMode = folder ? "folder" : "file"
        root.promptPath = root.activePath
        namePrompt.open(folder ? "新建文件夹" : "新建文件",
                        folder ? "新建文件夹" : "新建文件.txt", "")
    }

    function promptRename(path, name) {
        if (root.modalOpen)
            return
        root.promptMode = "rename"
        root.promptPath = path
        namePrompt.open("重命名", name, "")
    }

    function acceptName(text) {
        const mode = root.promptMode
        const path = root.promptPath
        root.promptMode = ""
        if (mode === "folder")
            Workspace.createFolder(path, text)
        else if (mode === "file")
            Workspace.createFile(path, text)
        else if (mode === "rename")
            Workspace.renameEntry(path, text)
    }

    function showRowMenu(path, isDir, x, y) {
        root.contextPath = path
        root.contextIsDir = isDir
        const many = fileList.selectionCount > 1
        const hasPath = path !== ""
        const hasClipboard = Workspace.clipboardMode !== ""
        menu.openAt(x, y, [
            { id: "open", label: isDir ? "打开" : "用默认应用打开", enabled: hasPath, shortcut: "Enter" },
            { id: "openTab", label: "在新标签页打开", enabled: hasPath && isDir },
            { id: "openWindow", label: "在新窗口打开", enabled: hasPath && isDir },
            { separator: true },
            { id: "reveal", label: "在资源管理器中显示", enabled: hasPath },
            { id: "openWith", label: "选择打开方式…", enabled: hasPath },
            { id: "terminal", label: "在终端中打开", enabled: hasPath && isDir },
            { separator: true },
            { id: "copy", label: "复制", shortcut: "Ctrl+C", enabled: hasPath },
            { id: "cut", label: "剪切", shortcut: "Ctrl+X", enabled: hasPath },
            { id: "paste", label: "粘贴到此处", shortcut: "Ctrl+V", enabled: hasClipboard && isDir },
            { id: "duplicate", label: "创建副本", enabled: hasPath },
            { id: "rename", label: "重命名", shortcut: "F2", enabled: hasPath && !many },
            { separator: true },
            { id: "favorite", label: "加入收藏", enabled: hasPath && isDir },
            { id: "trash", label: "移到回收站", shortcut: "Delete", enabled: hasPath, danger: true },
            { id: "delete", label: "永久删除", shortcut: "Shift+Delete", enabled: hasPath, danger: true }
        ])
    }

    function showEmptyMenu(x, y) {
        root.contextPath = ""
        root.contextIsDir = true
        menu.openAt(x, y, [
            { id: "newFolder", label: "新建文件夹", shortcut: root.mod + "+Shift+N" },
            { id: "newFile", label: "新建文件" },
            { separator: true },
            { id: "paste", label: "粘贴", shortcut: "Ctrl+V", enabled: Workspace.clipboardMode !== "" },
            { id: "selectAll", label: "全选", shortcut: "Ctrl+A" },
            { id: "refresh", label: "刷新", shortcut: "F5" },
            { separator: true },
            { id: "terminal", label: "在终端中打开" }
        ])
    }

    function showSidebarMenu(kind, path, x, y) {
        root.contextPath = path
        root.contextIsDir = true
        const items = []
        if (kind === "favorite")
            items.push({ id: "removeFavorite", label: "从收藏移除", danger: true })
        if (kind === "recent")
            items.push({ id: "reveal", label: "在资源管理器中显示" })
        items.push({ id: "openTab", label: "在新标签页打开" })
        items.push({ id: "openWindow", label: "在新窗口打开" })
        if (kind !== "favorite")
            items.push({ id: "favorite", label: "加入收藏" })
        menu.openAt(x, y, items)
    }

    function runMenuAction(id) {
        const path = root.contextPath
        const isDir = root.contextIsDir
        switch (id) {
        case "open":
            if (isDir)
                root.navigate(path)
            else
                Workspace.openPath(path)
            break
        case "openTab":
            root.addTab(path)
            break
        case "openWindow":
            root.newWindowAt(path)
            break
        case "reveal":
            Workspace.reveal(path)
            break
        case "openWith":
            Workspace.openWith(path)
            break
        case "terminal":
            Workspace.openTerminal(path !== "" ? path : root.activePath)
            break
        case "copy":
            Workspace.setClipboard("copy", root.currentSelection())
            break
        case "cut":
            Workspace.setClipboard("cut", root.currentSelection())
            break
        case "paste":
            Workspace.pasteInto(isDir && path !== "" ? path : root.activePath)
            break
        case "duplicate":
            Workspace.duplicateEntries(root.currentSelection())
            break
        case "rename":
            root.promptRename(path, path.split(/[\\/]/).pop())
            break
        case "favorite":
            Workspace.addFavorite(path)
            break
        case "removeFavorite":
            Workspace.removeFavorite(path)
            break
        case "trash":
            Workspace.trashEntries(root.currentSelection())
            break
        case "delete":
            Workspace.deleteEntries(root.currentSelection())
            break
        case "newFolder":
            root.promptCreate(true)
            break
        case "newFile":
            root.promptCreate(false)
            break
        case "selectAll":
            fileList.selectAll()
            break
        case "refresh":
            if (root.activeTab !== null)
                root.activeTab.listing.reload()
            break
        }
    }

    function reloadAfterOperation(kind, focus) {
        if (root.activeTab === null)
            return
        const reloadKinds = ["create", "rename", "trash", "delete", "copy", "move",
                             "duplicate", "undo", "redo"]
        if (reloadKinds.indexOf(kind) < 0)
            return
        root.activeTab.listing.reload()
        if (focus !== "")
            fileList.focusPath(focus)
    }

    function modalClosed() {
        Qt.callLater(function () {
            if (!root.modalOpen)
                fileList.takeFocus()
        })
    }

    function toggleTerminal() {
        root.terminalActive = !root.terminalActive
        if (root.terminalActive)
            Qt.callLater(terminalPanel.focusTerminal)
        else
            fileList.takeFocus()
    }

    Connections {
        target: Workspace

        function onOperationFinished(kind, focus) {
            root.reloadAfterOperation(kind, focus)
            if (Workspace.status !== "")
                toast.show(Workspace.status, false)
        }

        function onOperationFailed(kind, message) {
            toast.show(message, true)
        }
    }

    Column {
        anchors.fill: parent

        TitleBar {
            width: parent.width
            window: root
            dark: root.dark
            material: root.material
            title: root.title
        }

        TabStrip {
            id: tabStrip
            width: parent.width
            tabs: root.tabs
            activeIndex: root.activeIndex
            hostWindow: root
            dark: root.dark
            material: root.material

            onTabActivated: function (index) {
                root.activeIndex = index
            }
            onTabClosed: function (index) {
                root.closeTab(index)
            }
            onNewTabRequested: root.addTab(root.activePath)
            onNewWindowRequested: root.newWindowAt(root.activePath)
            onTabMoved: function (from, to) {
                root.moveTab(from, to)
            }
            onTabDroppedOnWindow: function (index, token, localX) {
                root.dropTabOnWindow(index, token, localX)
            }
            onTabTornOff: function (index, globalX, globalY) {
                root.detachTab(index, globalX, globalY)
            }
        }

        Rectangle {
            id: pathBar
            width: parent.width
            height: 40
            color: root.chromeColor

            Rectangle {
                id: sidebarButton
                x: 8
                anchors.verticalCenter: parent.verticalCenter
                width: 24
                height: 24
                radius: 5
                color: sidebarMouse.containsMouse
                    ? (root.dark ? "#24ffffff" : "#12000000") : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: "\u2630"
                    color: root.sidebarVisible ? root.textColor : root.mutedColor
                    font.pixelSize: 12
                }

                MouseArea {
                    id: sidebarMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.sidebarVisible = !root.sidebarVisible
                }
            }

            Rectangle {
                id: upButton
                anchors.left: sidebarButton.right
                anchors.leftMargin: 2
                anchors.verticalCenter: parent.verticalCenter
                width: 24
                height: 24
                radius: 5
                color: upMouse.containsMouse
                    ? (root.dark ? "#24ffffff" : "#12000000") : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: "\u2191"
                    color: root.mutedColor
                    font.pixelSize: 13
                }

                MouseArea {
                    id: upMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.goUp()
                }
            }

            Rectangle {
                id: pathField
                anchors.left: upButton.right
                anchors.leftMargin: 6
                anchors.right: searchBar.left
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                height: 26
                radius: 7
                color: root.dark ? "#1affffff" : "#0d000000"
                border.width: 1
                border.color: pathInput.activeFocus
                    ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf") : root.hairline

                TextInput {
                    id: pathInput

                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    text: ""
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: "#ffffff"
                    font.pixelSize: 13
                    selectByMouse: true
                    clip: true

                    // TextInput assigns to `text` while the user edits, which
                    // permanently drops any binding on it. Keep the display in
                    // sync explicitly instead: reset on focus loss and
                    // whenever the tab navigates.
                    onActiveFocusChanged: {
                        text = root.activePath
                        if (activeFocus)
                            selectAll()
                    }

                    onAccepted: {
                        let target = text.trim()
                        if (target.length >= 2
                                && target.charAt(0) === '"'
                                && target.charAt(target.length - 1) === '"')
                            target = target.substring(1, target.length - 1)
                        focus = false
                        if (target.length > 0)
                            root.navigate(target)
                    }

                    Keys.onEscapePressed: focus = false
                }
            }

            SearchBar {
                id: searchBar
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 224
                mode: root.searchMode
                busy: root.searchBusy
                dark: root.dark
                accent: root.accent
                textColor: root.textColor
                mutedColor: root.mutedColor
                hairline: root.hairline

                onQueryEdited: function (value) {
                    root.acceptSearchText(value)
                }
                onModeToggled: root.toggleSearchMode()
                onClearRequested: root.acceptSearchText("")
            }
        }

        Rectangle {
            id: searchOptions
            width: parent.width
            height: visible ? 30 : 0
            visible: root.contentSearchActive
            color: root.chromeColor
            clip: true

            Row {
                x: 12
                height: parent.height
                spacing: 8

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28
                    height: 20
                    radius: 4
                    color: root.searchRegex ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.24)
                                            : "transparent"
                    border.width: 1
                    border.color: root.searchRegex ? root.accent : root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: ".*"
                        color: root.searchRegex ? root.textColor : root.mutedColor
                        font.pixelSize: 11
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.searchRegex = !root.searchRegex
                            root.scheduleSearch()
                        }
                    }
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28
                    height: 20
                    radius: 4
                    color: root.searchCase ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.24)
                                           : "transparent"
                    border.width: 1
                    border.color: root.searchCase ? root.accent : root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: "Aa"
                        color: root.searchCase ? root.textColor : root.mutedColor
                        font.pixelSize: 11
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.searchCase = !root.searchCase
                            root.scheduleSearch()
                        }
                    }
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 170
                    height: 22
                    radius: 5
                    color: root.dark ? "#1affffff" : "#0d000000"
                    border.width: 1
                    border.color: filterInput.activeFocus ? root.accent : root.hairline

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 7
                        anchors.verticalCenter: parent.verticalCenter
                        visible: filterInput.text.length === 0
                        text: "类型过滤，如 *.rs"
                        color: root.mutedColor
                        font.pixelSize: 11
                    }

                    TextInput {
                        id: filterInput
                        anchors.fill: parent
                        anchors.leftMargin: 7
                        anchors.rightMargin: 7
                        verticalAlignment: TextInput.AlignVCenter
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: "#ffffff"
                        font.pixelSize: 11
                        clip: true

                        onTextEdited: {
                            root.searchFilter = text
                            root.scheduleSearch()
                        }
                    }
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.searchBusy ? "搜索中…"
                        : (root.activeTab !== null && root.activeTab.contentSearch.status !== ""
                            ? root.activeTab.contentSearch.status : "遵循 .gitignore")
                    color: root.mutedColor
                    font.pixelSize: 11
                }
            }
        }

        Row {
            id: body
            width: parent.width
            height: parent.height - 40 - 34 - 40 - 22 - searchOptions.height

            Sidebar {
                id: sidebar
                width: root.sidebarVisible ? 198 : 0
                height: parent.height
                visible: width > 0
                dark: root.dark
                material: root.material
                activePath: root.activePath
                accent: root.accent
                showHeader: true

                onNavigateRequested: function (path) {
                    root.navigate(path)
                }
                onContextRequested: function (kind, path, x, y) {
                    root.showSidebarMenu(kind, path, x, y)
                }
            }

            Column {
                width: parent.width - sidebar.width - previewPanel.width
                height: parent.height

                Item {
                    width: parent.width
                    height: parent.height - terminalPanel.height

                    FileList {
                        id: fileList
                        anchors.fill: parent
                        visible: !root.contentSearchActive
                        tab: root.activeTab
                        searchModel: root.searchActive && root.activeTab !== null
                                     && root.searchMode === "name"
                                     ? root.activeTab.nameSearch : null
                        dark: root.dark
                        headerColor: root.chromeColor
                        accent: root.accent

                        onNavigateRequested: function (path) {
                            root.navigate(path)
                        }
                        onUpRequested: root.goUp()
                        onRenameRequested: function (path, name) {
                            root.promptRename(path, name)
                        }
                        onCreateFolderRequested: root.promptCreate(true)
                        onContextRequested: function (path, isDir, x, y) {
                            root.showRowMenu(path, isDir, x, y)
                        }
                        onEmptyContextRequested: function (x, y) {
                            root.showEmptyMenu(x, y)
                        }
                        onSelectionChanged: {
                            previewPanel.showPaths(fileList.selectedPathList())
                        }
                    }

                    ContentResults {
                        anchors.fill: parent
                        visible: root.contentSearchActive
                        model: root.activeTab !== null ? root.activeTab.contentSearch : null
                        dark: root.dark
                        accent: root.accent

                        onPreviewRequested: function (path) {
                            previewPanel.showPaths([path])
                        }
                    }
                }

                TerminalPanel {
                    id: terminalPanel
                    width: parent.width
                    active: root.terminalActive
                    cwd: root.activePath
                    dark: root.dark
                    material: root.material
                    onExited: {
                        fileList.takeFocus()
                    }
                }
            }

            PreviewPanel {
                id: previewPanel
                width: root.previewVisible ? 300 : 0
                height: parent.height
                visible: width > 0
                dark: root.dark
                accent: root.accent

                onCloseRequested: {
                    root.previewVisible = false
                    fileList.takeFocus()
                }
                onOpenRequested: function (path, isDir) {
                    root.acceptPreview(path, isDir)
                }
            }
        }

        Rectangle {
            width: parent.width
            height: 22
            color: root.chromeColor

            Text {
                id: statusText
                x: 12
                width: parent.width * 0.46
                anchors.verticalCenter: parent.verticalCenter
                text: {
                    if (root.activeTab === null)
                        return ""
                    if (root.searchActive) {
                        const search = root.searchMode === "content"
                            ? root.activeTab.contentSearch : root.activeTab.nameSearch
                        if (search.busy)
                            return root.searchMode === "content" ? "搜索文件内容…" : "搜索中…"
                        return search.status !== "" ? search.status : "没有匹配项"
                    }
                    const listing = root.activeTab.listing
                    const base = listing.finished ? listing.status
                        : (listing.loaded + " 项 · 读取中…")
                    if (fileList.selectionCount > 0)
                        return fileList.selectionCount + " 项已选择 · " + base
                    if (Workspace.busy && Workspace.progressLabel !== "")
                        return Workspace.progressLabel
                    return base
                }
                color: root.mutedColor
                font.family: "Cascadia Mono"
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            Item {
                id: previewButton
                anchors.right: terminalButton.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                width: 58
                height: 18

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: previewMouse.containsMouse
                        ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                    Text {
                        anchors.centerIn: parent
                        text: root.previewVisible ? "隐藏预览" : "预览"
                        color: root.previewVisible ? root.textColor : root.mutedColor
                        font.pixelSize: 10
                    }
                }

                MouseArea {
                    id: previewMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        root.previewVisible = !root.previewVisible
                        if (!root.previewVisible)
                            fileList.takeFocus()
                    }
                }
            }

            Item {
                id: settingsButton
                anchors.right: previewButton.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                width: 52
                height: 18

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: settingsMouse.containsMouse
                        ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                    Text {
                        anchors.centerIn: parent
                        text: "设置"
                        color: root.mutedColor
                        font.pixelSize: 10
                    }
                }

                MouseArea {
                    id: settingsMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: settingsPanel.open()
                }
            }

            Rectangle {
                id: terminalButton
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 62
                height: 18
                radius: 4
                color: terminalMouse.containsMouse
                    ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: root.terminalActive ? "隐藏终端" : "终端"
                    color: root.terminalActive ? root.textColor : root.mutedColor
                    font.pixelSize: 10
                }

                MouseArea {
                    id: terminalMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.toggleTerminal()
                }
            }

            Text {
                anchors.left: statusText.right
                anchors.leftMargin: 16
                anchors.right: settingsButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                horizontalAlignment: Text.AlignRight
                text: root.firstWindow ? root.profileText
                    : (root.tabs.length + " 个标签 · " + root.token)
                color: root.mutedColor
                font.family: "Cascadia Mono"
                font.pixelSize: 11
                elide: Text.ElideMiddle
            }
        }
    }

    ContextMenu {
        id: menu
        anchors.fill: parent
        dark: root.dark

        onActionTriggered: function (id) {
            root.runMenuAction(id)
            root.modalClosed()
        }
        onVisibleChanged: {
            if (!menu.visible)
                root.modalClosed()
        }
    }

    NamePrompt {
        id: namePrompt
        anchors.fill: parent
        dark: root.dark

        onAccepted: function (text) {
            root.acceptName(text)
            root.modalClosed()
        }
        onCanceled: root.modalClosed()
    }

    SettingsPanel {
        id: settingsPanel
        anchors.fill: parent
        dark: root.dark

        onClosed: root.modalClosed()
    }

    Toast {
        id: toast
        width: 360
        height: 40
        x: (root.width - width) / 2
        y: root.height - 96
        dark: root.dark
    }

    Shortcut {
        sequence: root.mod + "+T"
        enabled: !root.modalOpen
        onActivated: root.addTab(root.activePath)
    }

    Shortcut {
        sequence: root.mod + "+W"
        enabled: !root.modalOpen
        onActivated: root.closeTab(root.activeIndex)
    }

    Shortcut {
        sequence: "Ctrl+Tab"
        enabled: !root.modalOpen
        onActivated: root.cycleTab(1)
    }

    Shortcut {
        sequence: "Ctrl+Shift+Tab"
        enabled: !root.modalOpen
        onActivated: root.cycleTab(-1)
    }

    Shortcut {
        sequence: root.config !== null && root.config.pathJump !== ""
            ? root.config.pathJump : "Ctrl+P"
        enabled: !root.modalOpen
        onActivated: {
            pathInput.forceActiveFocus()
            pathInput.selectAll()
        }
    }

    Shortcut {
        sequence: "Ctrl+J"
        enabled: !root.modalOpen
        onActivated: root.toggleTerminal()
    }

    Shortcut {
        sequence: "Ctrl+F"
        enabled: !root.modalOpen
        onActivated: searchBar.focusInput()
    }

    Shortcut {
        sequence: "Ctrl+B"
        enabled: !root.modalOpen
        onActivated: root.sidebarVisible = !root.sidebarVisible
    }

    Shortcut {
        sequence: "Ctrl+,"
        enabled: !root.modalOpen
        onActivated: settingsPanel.open()
    }

    Shortcut {
        sequence: root.mod + "+Shift+N"
        enabled: !root.modalOpen
        onActivated: root.promptCreate(true)
    }

    /// One Escape handler for whichever overlay is on top. Per-overlay
    /// shortcuts would race each other when two are visible.
    Shortcut {
        sequence: "Escape"
        enabled: root.modalOpen
        onActivated: {
            if (menu.visible)
                menu.close()
            else if (namePrompt.visible)
                namePrompt.close()
            else if (settingsPanel.visible)
                settingsPanel.close()
        }
    }

    Component.onCompleted: {
        ShellState.register(root.token, root)

        if (!chrome.attach())
            console.warn("dae: native window attach failed")

        if (root.firstWindow) {
            if (root.config !== null)
                root.dark = root.config.systemDark
            root.material = chrome.startupMaterial()
            if (root.config !== null)
                root.startupPath = root.config.startupPath
        }
        chrome.applyMaterial(root.material, root.dark)

        if (root.initialTabs && root.initialTabs.length > 0) {
            root.tabs = root.initialTabs
            root.activeIndex = Math.max(0, Math.min(root.initialActive, root.tabs.length - 1))
        } else {
            const tab = root.createTab(root.startupPath)
            if (tab !== null) {
                root.tabs = [tab]
                root.activeIndex = 0
            }
        }

        if (root.activePath !== "")
            Workspace.recordRecent(root.activePath, true, false)

        pathInput.text = root.activePath
        root.visible = true
    }

    Component.onDestruction: {
        ShellState.unregister(root.token)
        root.disposeTabs()
    }

    onClosing: {
        root.disposeTabs()
        Qt.callLater(root.destroy)
    }

    Connections {
        target: root

        function onActivePathChanged() {
            if (!pathInput.activeFocus)
                pathInput.text = root.activePath
        }
    }
}
