import QtQuick
import QtQuick.Window
import dae 1.0
import dae.activation 1.0
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
    // Follows the OS preference live; the native watcher updates
    // `Workspace.systemDark`, which re-tints every window's backdrop below.
    property bool dark: Workspace.systemDark
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
    /// Set after the first window has built its starting tab. An activation
    /// that arrives before this stays in the native pending list.
    property bool activationReady: false
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
    property string contextTrashId: ""
    property string contextTrashParent: ""
    property string confirmMode: ""
    property var confirmTrashIds: []
    property string archiveMode: ""
    property string archiveTarget: ""
    property string archiveDestination: ""
    property var archivePaths: []
    /// True while the cloud dialog owns an in-flight browser round trip. If the
    /// user dismisses the dialog first, a late success only refreshes the
    /// sidebar instead of navigating away from what they are doing.
    property bool cloudPending: false

    readonly property var activeTab: (activeIndex >= 0 && activeIndex < tabs.length)
        ? tabs[activeIndex] : null
    readonly property string activePath: activeTab !== null ? activeTab.listing.path : ""
    readonly property bool trashActive: activeTab !== null && activeTab.trash
    /// True while the tab is browsing a network backend. Local-only actions
    /// (terminal, recycle bin, content search, OS open/reveal) are gated on
    /// this instead of failing after the click.
    readonly property bool remoteLocation: activeTab !== null && !trashActive
        && (activeTab.listing.remote || root.isRemotePath(activeTab.listing.path))
    readonly property bool searchActive: searchQuery.trim().length > 0
    readonly property bool contentSearchActive: searchActive && searchMode === "content"
    readonly property bool searchBusy: activeTab === null ? false
        : (searchMode === "content" ? activeTab.contentSearch.busy : activeTab.nameSearch.busy)
    readonly property string mod: Qt.platform.os === "osx" ? "Meta" : "Ctrl"
    readonly property color accent: Workspace.accent
    readonly property string sourceLabel: config === null ? ""
        : config.startupSource === "deepLink" ? "深链"
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
    readonly property bool modalOpen: menu.visible || namePrompt.visible
        || passwordDialog.visible || settingsPanel.visible || confirmDialog.visible
        || connectDialog.visible || cloudDialog.visible || openWithDialog.visible
        || updateDialog.visible

    Timer {
        id: searchTimer
        interval: root.searchMode === "content" ? 350 : 180
        repeat: false
        onTriggered: root.runSearch()
    }

    // One silent feed check per launch, after the window has settled. Deployed
    // builds only: `Updater.supported` is false when running from `target/` or
    // from any layout without the deployed Qt runtime, and this startup check
    // never puts a network error in front of the user.
    Timer {
        id: updateCheck
        interval: 4000
        repeat: false
        running: root.firstWindow && Updater.supported && !Updater.checked
        onTriggered: Updater.check(false)
    }

    onActiveIndexChanged: {
        root.resetSearchState()
        previewPanel.showPaths([])
        root.applyRemotePolicy()
        if (root.trashActive)
            trashModel.refresh()
    }

    onActiveChanged: {
        ShellState.noteActive(root.token, active)
        // Files may have been copied in Explorer while this window was in the
        // background; make the paste affordances follow the system clipboard.
        if (active)
            Workspace.refreshClipboard()
    }

    onDarkChanged: {
        if (chrome && chrome.attached)
            chrome.applyMaterial(root.material, root.dark)
    }

    WindowController {
        id: chrome
        token: root.objectName
    }

    /// One bin per window. The bin itself is process-wide, but a window that
    /// tears a tab off refreshes its own snapshot rather than sharing rows
    /// across windows.
    TrashModel {
        id: trashModel
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
        root.applyRemotePolicy()
        Qt.callLater(fileList.takeFocus)
    }

    /// Handles an external `dae://` open (or a second launch with paths).
    ///
    /// The window is brought back first, together with every tab it already
    /// had. Each target then either restores an existing tab (in any window)
    /// or becomes a new tab here; it never replaces what the user was viewing.
    function receiveActivation(paths) {
        if (!Shell.focusWindow(root.token)) {
            if (root.visibility === Window.Minimized)
                root.showNormal()
            else if (!root.visible)
                root.show()
            root.raise()
            root.requestActivate()
        }

        if (!paths)
            return
        for (let index = 0; index < paths.length; index += 1)
            root.openActivationPath(paths[index])
    }

    function openActivationPath(path) {
        if (!path)
            return

        const existing = ShellState.findOpenTab(path)
        if (existing !== null) {
            const existingWindow = ShellState.windowFor(existing.token)
            if (existingWindow !== null) {
                existingWindow.activeIndex = existing.index
                Shell.focusWindow(existingWindow.token)
                return
            }
        }

        root.addTab(path)
        Workspace.recordRecent(path, true, false)
    }

    /// Pulls what the native side buffered. `focusAlways` is for the signal:
    /// a second launch with no openable argument still raises the window.
    function consumePendingActivation(focusAlways) {
        if (ShellState.activationTarget() !== root)
            return
        const paths = Activation.takePending()
        if (paths.length > 0 || focusAlways)
            root.receiveActivation(paths)
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
            "material": root.material,
            "x": Math.round(root.x + 32),
            "y": Math.round(root.y + 32),
            "width": root.width,
            "height": root.height
        })
    }

    function isRemotePath(path) {
        return /^(smb|sftp|ftp|webdav|webdavs|cloud):\/\//i.test(path || "")
    }

    /// Local-only surfaces are switched off when the active tab moves to a
    /// network backend. An already running terminal is hidden, not killed:
    /// hiding was never closing, and it keeps its local working directory.
    function applyRemotePolicy() {
        if (!root.remoteLocation)
            return
        root.terminalActive = false
        if (root.searchMode === "content")
            root.searchMode = "name"
    }

    function navigate(path) {
        if (root.activeTab === null || !path || path.length === 0)
            return
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.navigate(path)
        root.applyRemotePolicy()
        Workspace.recordRecent(path, true, false)
        Qt.callLater(fileList.takeFocus)
    }

    function openTrash() {
        if (root.activeTab === null || root.modalOpen)
            return
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.openTrash()
        trashModel.refresh()
        Qt.callLater(trashView.takeFocus)
    }

    function trashTargetIds() {
        const ids = trashView.selectedIdList()
        if (ids.length > 0)
            return ids
        return root.contextTrashId !== "" ? [root.contextTrashId] : []
    }

    function restoreTrash(ids) {
        if (!ids || ids.length === 0 || Workspace.busy)
            return
        trashView.clearSelection()
        Workspace.restoreTrashEntries(ids)
    }

    function requestTrashPurge(ids) {
        if (!ids || ids.length === 0 || Workspace.busy)
            return
        root.confirmMode = "purge"
        root.confirmTrashIds = ids
        confirmDialog.open("永久删除这些项目？",
                           ids.length + " 个项目将被永久删除，无法恢复。",
                           "永久删除", true)
    }

    /// A network entry has no path through the local recycle bin, so Delete
    /// asks before permanently deleting it instead of calling the trash.
    function requestRemoteDelete(paths) {
        if (!paths || paths.length === 0 || Workspace.busy)
            return
        root.confirmMode = "remoteDelete"
        root.confirmTrashIds = paths
        confirmDialog.open("永久删除这些项目？",
                           paths.length + " 个远程项目无法移入回收站，将被永久删除。",
                           "永久删除", true)
    }

    function requestEmptyTrash() {
        if (Workspace.busy)
            return
        root.confirmMode = "empty"
        root.confirmTrashIds = []
        confirmDialog.open("清空回收站？",
                           "回收站中的 " + trashModel.count + " 个项目将被永久删除，无法恢复。",
                           "清空回收站", true)
    }

    function acceptConfirm() {
        const mode = root.confirmMode
        const ids = root.confirmTrashIds
        root.confirmMode = ""
        root.confirmTrashIds = []
        if (mode === "purge") {
            trashView.clearSelection()
            Workspace.deleteTrashEntries(ids)
        } else if (mode === "empty") {
            trashView.clearSelection()
            Workspace.emptyTrash()
        } else if (mode === "remoteDelete") {
            Workspace.deleteEntries(ids)
        }
        root.modalClosed()
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
        if (root.remoteLocation) {
            toast.show("内容搜索只支持本地目录", false)
            return
        }
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
        else if (root.isRemotePath(path))
            root.acceptRemotePreview(path)
        else
            Workspace.openPath(path)
    }

    /// Enter on a remote file cannot reach a local handler. Show the preview
    /// (the one operation that works over the VFS) and say what to do instead
    /// of letting the open call fail.
    function acceptRemotePreview(path) {
        root.previewVisible = true
        previewPanel.showPaths([path])
        toast.show("远程文件可预览；用本地应用打开前请先复制到本地", false)
    }

    function openConnectDialog() {
        if (root.modalOpen)
            return
        connectDialog.open()
    }

    function openCloudDialog() {
        if (root.modalOpen)
            return
        cloudDialog.open()
    }

    function startConnectionTest(protocol, host, port, username, password) {
        connectDialog.setTesting(true)
        Workspace.testConnection(protocol, host, port, username, password)
    }

    function startConnectionSave(protocol, host, port, username, password, remember) {
        connectDialog.setSaving(true)
        Workspace.saveConnection(protocol, host, port, username, password, remember)
    }

    function startCloudAuthorize(provider, clientId, clientSecret) {
        root.cloudPending = true
        cloudDialog.setAuthorizing(true)
        Workspace.authorizeCloudAccount(provider, clientId, clientSecret)
    }

    function parentOf(path) {
        if (!path)
            return ""
        let trimmed = path
        while (trimmed.length > 1
               && (trimmed.endsWith("/") || trimmed.endsWith("\\")))
            trimmed = trimmed.slice(0, -1)
        // `smb://host:445`, `sftp://host:22` and `cloud://provider:email` are
        // roots. Without this guard the next cut would produce `smb:/`.
        if (/^[a-z][a-z0-9+.-]*:\/\/[^/]+$/i.test(trimmed))
            return ""
        const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
        if (cut < 0)
            return ""
        let parent = trimmed.slice(0, cut)
        if (parent.length === 2 && parent.charAt(1) === ":")
            parent += "\\"
        return parent
    }

    function goUp() {
        if (root.trashActive)
            return
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

    function isArchivePath(path) {
        return /\.(zip|tar|tar\.gz|tgz|7z)$/i.test(path)
    }

    function displayNameOfPath(path) {
        const parts = path.split(/[\\/]/)
        return parts.length > 0 ? parts[parts.length - 1] : path
    }

    function startCompress(format, encrypted) {
        const paths = root.currentSelection()
        if (paths.length === 0 || root.activePath === "")
            return
        if (encrypted) {
            root.archiveMode = "compress"
            root.archivePaths = paths
            root.archiveTarget = ""
            root.archiveDestination = root.activePath
            passwordDialog.open("compress", "", "")
            return
        }
        Workspace.compressEntries(paths, root.activePath, format, "")
    }

    function startExtract(path) {
        if (!root.isArchivePath(path))
            return
        root.archiveMode = "extract"
        root.archiveTarget = path
        root.archiveDestination = ""
        Workspace.extractArchive(path, "", "")
    }

    function requestArchivePassword(target, message) {
        root.archiveMode = "extract"
        root.archiveTarget = target
        root.archiveDestination = ""
        if (passwordDialog.visible) {
            passwordDialog.showError(message)
            return
        }
        passwordDialog.open("extract", root.displayNameOfPath(target), "")
    }

    function acceptArchivePassword(password) {
        if (root.archiveMode === "compress")
            Workspace.compressEntries(root.archivePaths, root.archiveDestination, "7z", password)
        else if (root.archiveMode === "extract")
            Workspace.extractArchive(root.archiveTarget, root.archiveDestination, password)
    }

    function resetArchiveState() {
        root.archiveMode = ""
        root.archiveTarget = ""
        root.archiveDestination = ""
        root.archivePaths = []
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
        const remote = root.remoteLocation
        // Archiving streams remote sources into a local container, so the
        // sources may be remote but the destination (here) may not.
        const canExtract = hasPath && !isDir && !remote && root.isArchivePath(path)
        menu.openAt(x, y, [
            { id: "open", label: isDir ? "打开" : "用默认应用打开",
              enabled: hasPath && (!remote || isDir), shortcut: "Enter" },
            { id: "openTab", label: "在新标签页打开", enabled: hasPath && isDir },
            { id: "openWindow", label: "在新窗口打开", enabled: hasPath && isDir },
            { separator: true },
            { id: "reveal", label: "在资源管理器中显示", enabled: hasPath && !remote },
            { id: "openWith", label: "选择打开方式…", enabled: hasPath && !remote },
            { id: "terminal", label: "在终端中打开", enabled: hasPath && isDir && !remote },
            { separator: true },
            { id: "copy", label: "复制", shortcut: "Ctrl+C", enabled: hasPath },
            { id: "cut", label: "剪切", shortcut: "Ctrl+X", enabled: hasPath },
            { id: "paste", label: "粘贴到此处", shortcut: "Ctrl+V", enabled: hasClipboard && isDir },
            { id: "pasteShortcut", label: "粘贴快捷方式",
              enabled: hasClipboard && isDir && !remote },
            { id: "duplicate", label: "创建副本", enabled: hasPath },
            { id: "createShortcut", label: "创建快捷方式", enabled: hasPath && !remote },
            { id: "compress", label: "压缩为…", enabled: hasPath && !remote, submenu: [
                { id: "compressZip", label: "ZIP 压缩包" },
                { id: "compressTar", label: "TAR 归档" },
                { id: "compressTarGz", label: "TAR.GZ 压缩包" },
                { id: "compress7z", label: "7Z 压缩包" },
                { id: "compress7zEncrypted", label: "7Z 压缩包（加密）" }
            ] },
            { id: "extractHere", label: "解压到此处", enabled: canExtract },
            { id: "rename", label: "重命名", shortcut: "F2", enabled: hasPath && !many },
            { separator: true },
            { id: "favorite", label: "加入收藏", enabled: hasPath && isDir },
            { id: "trash", label: "移到回收站", shortcut: "Delete",
              enabled: hasPath && !remote, danger: true },
            { id: "delete", label: "永久删除", shortcut: "Shift+Delete", enabled: hasPath, danger: true }
        ])
    }

    function showTrashRowMenu(id, originalParent, x, y) {
        root.contextTrashId = id
        root.contextTrashParent = originalParent
        menu.openAt(x, y, [
            { id: "trashRestore", label: "恢复", shortcut: "Enter" },
            { id: "trashOpenOriginal", label: "打开原位置",
              enabled: originalParent !== "" },
            { separator: true },
            { id: "trashPurge", label: "永久删除", shortcut: "Delete", danger: true }
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
            { id: "pasteShortcut", label: "粘贴快捷方式",
              enabled: Workspace.clipboardMode !== "" && !root.remoteLocation },
            { id: "selectAll", label: "全选", shortcut: "Ctrl+A" },
            { id: "refresh", label: "刷新", shortcut: "F5" },
            { separator: true },
            { id: "terminal", label: "在终端中打开", enabled: !root.remoteLocation }
        ])
    }

    function showSidebarMenu(kind, path, x, y) {
        root.contextPath = path
        root.contextIsDir = true
        if (kind === "connection" || kind === "cloud") {
            menu.openAt(x, y, [
                { id: "open", label: "打开" },
                { id: "openTab", label: "在新标签页打开" },
                { id: "openWindow", label: "在新窗口打开" },
                { separator: true },
                { id: kind === "connection" ? "removeConnection" : "removeCloud",
                  label: kind === "connection" ? "移除连接" : "移除云盘账户",
                  danger: true }
            ])
            return
        }
        const items = []
        if (kind === "favorite")
            items.push({ id: "removeFavorite", label: "从收藏移除", danger: true })
        if (kind === "recent" && !root.isRemotePath(path))
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
            if (Workspace.openWithPickerSupported) {
                openWithDialog.open(path)
            } else {
                // Windows: the native dialog also owns the association UI.
                Workspace.openWith(path)
            }
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
        case "pasteShortcut":
            Workspace.pasteShortcutInto(isDir && path !== "" ? path : root.activePath)
            break
        case "createShortcut":
            Workspace.createShortcuts(root.currentSelection(), root.activePath)
            break
        case "duplicate":
            Workspace.duplicateEntries(root.currentSelection())
            break
        case "compressZip":
            root.startCompress("zip", false)
            break
        case "compressTar":
            root.startCompress("tar", false)
            break
        case "compressTarGz":
            root.startCompress("tar.gz", false)
            break
        case "compress7z":
            root.startCompress("7z", false)
            break
        case "compress7zEncrypted":
            root.startCompress("7z", true)
            break
        case "extractHere":
            root.startExtract(path)
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
        case "removeConnection":
            Workspace.deleteConnection(path)
            break
        case "removeCloud":
            Workspace.deleteCloudAccount(path)
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
        case "trashRestore":
            root.restoreTrash(root.trashTargetIds())
            break
        case "trashOpenOriginal":
            if (root.contextTrashParent !== "")
                root.navigate(root.contextTrashParent)
            break
        case "trashPurge":
            root.requestTrashPurge(root.trashTargetIds())
            break
        }
    }

    function reloadAfterOperation(kind, focus) {
        if (root.activeTab === null)
            return
        const trashKinds = ["trash", "restore_trash", "purge_trash", "empty_trash"]
        if (trashKinds.indexOf(kind) >= 0 && root.trashActive)
            trashModel.refresh()
        const reloadKinds = ["create", "rename", "trash", "delete", "copy", "move",
                             "duplicate", "shortcut", "compress", "extract", "undo", "redo"]
        if (reloadKinds.indexOf(kind) < 0)
            return
        root.activeTab.listing.reload()
        if (focus !== "")
            fileList.focusPath(focus)
    }

    function modalClosed() {
        Qt.callLater(function () {
            if (root.modalOpen)
                return
            if (root.trashActive)
                trashView.takeFocus()
            else
                fileList.takeFocus()
        })
    }

    function toggleTerminal() {
        if (root.remoteLocation) {
            toast.show("终端只能打开本地目录", false)
            return
        }
        root.terminalActive = !root.terminalActive
        if (root.terminalActive)
            Qt.callLater(terminalPanel.focusTerminal)
        else if (root.trashActive)
            trashView.takeFocus()
        else
            fileList.takeFocus()
    }

    Connections {
        target: Workspace

        function onOperationFinished(kind, focus) {
            if (kind === "openWith" && openWithDialog.visible) {
                openWithDialog.close()
                root.modalClosed()
            }
            if (kind === "compress" || kind === "extract") {
                root.resetArchiveState()
                if (passwordDialog.visible) {
                    passwordDialog.close()
                    root.modalClosed()
                }
            }
            root.reloadAfterOperation(kind, focus)
            if (Workspace.status !== "")
                toast.show(Workspace.status, false)
        }

        function onOperationFailed(kind, message) {
            if (kind === "openWith" && openWithDialog.visible) {
                openWithDialog.showError(message)
                return
            }
            if (passwordDialog.visible && kind === root.archiveMode) {
                passwordDialog.showError(message)
                return
            }
            if (kind === "compress" || kind === "extract")
                root.resetArchiveState()
            toast.show(message, true)
        }

        function onArchivePasswordRequired(target, message) {
            root.requestArchivePassword(target, message)
        }

        function onConnectionTestFinished(ok, message) {
            if (connectDialog.visible)
                connectDialog.applyTestResult(ok, message)
        }

        function onConnectionSaveFinished(ok, id, message) {
            if (!ok) {
                if (connectDialog.visible)
                    connectDialog.applySaveError(message)
                else
                    toast.show(message, true)
                return
            }
            if (connectDialog.visible) {
                connectDialog.close()
                root.modalClosed()
            }
            toast.show("连接已保存", false)
            if (id !== "")
                root.navigate(id)
        }

        function onCloudAuthorizeFinished(ok, message) {
            const pending = root.cloudPending
            root.cloudPending = false
            if (!ok) {
                if (cloudDialog.visible)
                    cloudDialog.applyError(message)
                else
                    toast.show(message, true)
                return
            }
            if (cloudDialog.visible) {
                cloudDialog.close()
                root.modalClosed()
            }
            toast.show("云盘账户已连接", false)
            if (pending)
                root.navigate(message)
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
                color: !root.trashActive && upMouse.containsMouse
                    ? (root.dark ? "#24ffffff" : "#12000000") : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: "\u2191"
                    color: root.trashActive ? root.hairline : root.mutedColor
                    font.pixelSize: 13
                }

                MouseArea {
                    id: upMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: !root.trashActive
                    onClicked: root.goUp()
                }
            }

            Rectangle {
                id: pathField
                anchors.left: upButton.right
                anchors.leftMargin: 6
                anchors.right: root.trashActive ? parent.right : searchBar.left
                anchors.rightMargin: root.trashActive ? 12 : 6
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
                    readOnly: root.trashActive

                    // TextInput assigns to `text` while the user edits, which
                    // permanently drops any binding on it. Keep the display in
                    // sync explicitly instead: reset on focus loss and
                    // whenever the tab navigates.
                    onActiveFocusChanged: {
                        text = root.trashActive ? "回收站" : root.activePath
                        if (activeFocus && !root.trashActive)
                            selectAll()
                    }

                    onAccepted: {
                        if (root.trashActive)
                            return
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
                visible: !root.trashActive
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 224
                mode: root.searchMode
                busy: root.searchBusy
                contentEnabled: !root.remoteLocation
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
                trashActive: root.trashActive

                onNavigateRequested: function (path) {
                    root.navigate(path)
                }
                onTrashRequested: root.openTrash()
                onAddRequested: function (kind) {
                    if (kind === "cloud")
                        root.openCloudDialog()
                    else
                        root.openConnectDialog()
                }
                onContextRequested: function (kind, path, x, y) {
                    root.showSidebarMenu(kind, path, x, y)
                }
                onRemoteOpenRequested: function (path) {
                    root.acceptRemotePreview(path)
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
                        visible: !root.trashActive && !root.contentSearchActive
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
                        onRemoteDeleteRequested: function (paths) {
                            root.requestRemoteDelete(paths)
                        }
                        onRemoteOpenRequested: function (path) {
                            root.acceptRemotePreview(path)
                        }
                    }

                    ContentResults {
                        anchors.fill: parent
                        visible: !root.trashActive && root.contentSearchActive
                        model: root.activeTab !== null ? root.activeTab.contentSearch : null
                        dark: root.dark
                        accent: root.accent

                        onPreviewRequested: function (path) {
                            previewPanel.showPaths([path])
                        }
                    }

                    TrashView {
                        id: trashView
                        anchors.fill: parent
                        visible: root.trashActive
                        model: trashModel
                        dark: root.dark
                        accent: root.accent
                        headerColor: root.chromeColor

                        onRestoreRequested: function (ids) {
                            root.restoreTrash(ids)
                        }
                        onPurgeRequested: function (ids) {
                            root.requestTrashPurge(ids)
                        }
                        onEmptyRequested: root.requestEmptyTrash()
                        onContextRequested: function (id, originalParent, x, y) {
                            root.showTrashRowMenu(id, originalParent, x, y)
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
                width: root.previewVisible && !root.trashActive ? 300 : 0
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
                    if (root.trashActive) {
                        if (Workspace.busy && Workspace.progressLabel !== "")
                            return Workspace.progressLabel
                        const bin = trashModel.loaded ? trashModel.status : "读取回收站…"
                        if (trashView.selectionCount > 0)
                            return trashView.selectionCount + " 项已选择 · " + bin
                        return bin
                    }
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
                opacity: root.trashActive ? 0.35 : 1

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
                    enabled: !root.trashActive
                    onClicked: {
                        root.previewVisible = !root.previewVisible
                        if (!root.previewVisible)
                            fileList.takeFocus()
                    }
                }
            }

            Item {
                id: updateButton
                anchors.right: settingsButton.left
                anchors.rightMargin: Updater.supported ? 4 : 0
                anchors.verticalCenter: parent.verticalCenter
                width: Updater.supported ? 64 : 0
                height: 18

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: updateMouse.containsMouse
                        ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                    Text {
                        anchors.centerIn: parent
                        text: Updater.restarting ? "重启中…"
                            : Updater.downloading ? "更新中…"
                            : Updater.available ? "更新可用"
                            : Updater.checking ? "检查中…" : "检查更新"
                        color: Updater.available ? root.accent
                            : Updater.checking || Updater.downloading ? root.textColor
                            : root.mutedColor
                        font.pixelSize: 10
                    }
                }

                MouseArea {
                    id: updateMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: Updater.supported
                    onClicked: updateDialog.open()
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
                opacity: root.remoteLocation ? 0.35 : 1
                color: !root.remoteLocation && terminalMouse.containsMouse
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
                    enabled: !root.remoteLocation
                    onClicked: root.toggleTerminal()
                }
            }

            Text {
                anchors.left: statusText.right
                anchors.leftMargin: 16
                anchors.right: updateButton.left
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

    // A tab dragged from any window. The platform's drag loop owns the pointer
    // and the preview; this area only records where the release landed, and the
    // source strip applies the reorder/merge after the loop returns. Recording
    // instead of moving keeps both tab lists untouched while the modal loop is
    // still running. The tab MIME type is the one `cpp/drag.cpp` advertises.
    DropArea {
        id: tabDropArea
        anchors.fill: parent
        z: 1000
        keys: ["application/x-dae-tab"]

        function draggedTab(drag) {
            const payload = drag.getDataAsString("application/x-dae-tab")
            const split = payload.indexOf("\t")
            if (split < 0)
                return null
            return {
                "token": payload.substring(0, split),
                "index": parseInt(payload.substring(split + 1), 10),
                "onStrip": drag.y >= tabStrip.y
                    && drag.y < tabStrip.y + tabStrip.height
            }
        }

        onEntered: function (drag) {
            const tab = draggedTab(drag)
            if (tab === null) {
                drag.accepted = false
                return
            }
            drag.accept(Qt.MoveAction)
            tabStrip.showDrop(tab.onStrip ? drag.x : -1)
        }
        onPositionChanged: function (drag) {
            const tab = draggedTab(drag)
            if (tab === null) {
                drag.accepted = false
                return
            }
            drag.accept(Qt.MoveAction)
            tabStrip.showDrop(tab.onStrip ? drag.x : -1)
        }
        onExited: tabStrip.clearDrop()
        onDropped: function (drop) {
            const tab = draggedTab(drop)
            tabStrip.clearDrop()
            if (tab === null) {
                drop.accepted = false
                return
            }
            drop.accept(Qt.MoveAction)
            ShellState.recordTabDrop(tab.token, tab.index, root.token,
                                     drop.x, drop.y, tab.onStrip)
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

    ConfirmDialog {
        id: confirmDialog
        anchors.fill: parent
        dark: root.dark

        onAccepted: root.acceptConfirm()
        onCanceled: {
            root.confirmMode = ""
            root.confirmTrashIds = []
            root.modalClosed()
        }
    }

    ArchivePasswordDialog {
        id: passwordDialog
        anchors.fill: parent
        dark: root.dark

        onAccepted: function (password) {
            root.acceptArchivePassword(password)
        }
        onCanceled: {
            root.resetArchiveState()
            root.modalClosed()
        }
    }

    SettingsPanel {
        id: settingsPanel
        anchors.fill: parent
        dark: root.dark

        onClosed: root.modalClosed()
    }

    ConnectDialog {
        id: connectDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onTestRequested: function (protocol, host, port, username, password) {
            root.startConnectionTest(protocol, host, port, username, password)
        }
        onSaveRequested: function (protocol, host, port, username, password, remember) {
            root.startConnectionSave(protocol, host, port, username, password, remember)
        }
        onCanceled: root.modalClosed()
    }

    CloudAccountDialog {
        id: cloudDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onAuthorizeRequested: function (provider, clientId, clientSecret) {
            root.startCloudAuthorize(provider, clientId, clientSecret)
        }
        onCanceled: {
            root.cloudPending = false
            root.modalClosed()
        }
    }

    OpenWithDialog {
        id: openWithDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onCanceled: root.modalClosed()
    }

    UpdateDialog {
        id: updateDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onCanceled: root.modalClosed()
    }

    // The detached installer is already waiting for this process to exit; the
    // quit is what lets it replace the mapped Qt libraries and relaunch.
    Connections {
        target: Updater
        enabled: root.firstWindow

        function onRestartRequested() {
            Qt.quit()
        }

        function onAvailableChanged() {
            if (Updater.available && !updateDialog.visible)
                toast.show("发现新版本 v" + Updater.latestVersion, false)
        }
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
        enabled: !root.modalOpen && !root.trashActive
        onActivated: {
            pathInput.forceActiveFocus()
            pathInput.selectAll()
        }
    }

    Shortcut {
        sequence: "Ctrl+J"
        enabled: !root.modalOpen && !root.remoteLocation
        onActivated: root.toggleTerminal()
    }

    Shortcut {
        sequence: "Ctrl+F"
        enabled: !root.modalOpen && !root.trashActive
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
            else if (updateDialog.visible)
                updateDialog.dismiss()
            else if (namePrompt.visible)
                namePrompt.close()
            else if (passwordDialog.visible) {
                passwordDialog.close()
                root.resetArchiveState()
            } else if (cloudDialog.visible)
                cloudDialog.dismiss()
            else if (connectDialog.visible)
                connectDialog.dismiss()
            else if (settingsPanel.visible)
                settingsPanel.close()
            else if (confirmDialog.visible) {
                confirmDialog.close()
                root.confirmMode = ""
                root.confirmTrashIds = []
            }
        }
    }

    Component.onCompleted: {
        ShellState.register(root.token, root)

        if (!chrome.attach())
            console.warn("dae: native window attach failed")

        if (root.firstWindow) {
            Workspace.startSystemWatch()
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

        root.applyRemotePolicy()
        pathInput.text = root.trashActive ? "回收站" : root.activePath
        root.visible = true
        root.activationReady = true
        ShellState.noteActive(root.token, root.active)
        Workspace.refreshClipboard()
        // A second launch that arrived while this window was being built has
        // been waiting in the native list; apply it now that the starting tab
        // exists, so the first target reuses that tab instead of duplicating it.
        root.consumePendingActivation(false)
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
        target: Activation

        function onActivated() {
            if (!root.activationReady || ShellState.activationTarget() !== root)
                return
            root.consumePendingActivation(true)
        }
    }

    Connections {
        target: root

        function onActivePathChanged() {
            if (!pathInput.activeFocus)
                pathInput.text = root.trashActive ? "回收站" : root.activePath
            root.applyRemotePolicy()
        }

        function onTrashActiveChanged() {
            if (!pathInput.activeFocus)
                pathInput.text = root.trashActive ? "回收站" : root.activePath
            if (root.trashActive) {
                root.terminalActive = false
                trashView.takeFocus()
            }
        }
    }
}
