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
    /// The second pane's tab. Created the first time split view is enabled and
    /// disposed when it is turned off, so an unused pane never holds a watcher.
    property var splitTab: null
    property real splitRatio: 0.5
    property bool splitFocused: false
    /// True while the path bar shows its editor instead of the breadcrumb
    /// trail. Only the editor has a path to type into; the trail is the bar's
    /// normal shape.
    property bool pathEditing: false
    property string searchQuery: ""
    property string searchMode: "name"
    property bool searchRegex: false
    property bool searchCase: false
    property string searchFilter: ""
    property string promptMode: ""
    property string promptPath: ""
    property string contextPath: ""
    property bool contextIsDir: false
    /// Which menu is open: `row`, `empty`, `trash` or `sidebar`. The shell
    /// commands answer asynchronously, and only the row menu is rebuilt when
    /// they arrive.
    property string contextKind: ""
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
    /// The primary pane's Git view. The status bar and branch panel bind to the
    /// listing model because that model owns the repository the pane shows.
    readonly property var gitListing: activeTab !== null ? activeTab.listing : null
    readonly property bool trashActive: activeTab !== null && activeTab.trash
    readonly property bool overviewActive: activeTab !== null && activeTab.overview
    readonly property bool spaceActive: activeTab !== null && activeTab.space
    readonly property bool folderActive: activeTab !== null && activeTab.folder
    readonly property string activeSpaceId: spaceActive ? activeTab.spaceId : ""
    /// The bar's label for the three non-folder surfaces. A folder tab keeps
    /// using the breadcrumb trail or the path editor.
    readonly property string surfaceTitle: {
        if (trashActive)
            return "回收站"
        if (overviewActive)
            return "概览"
        if (spaceActive)
            return activeTab.spaceTitle.length > 0 ? activeTab.spaceTitle : "空间"
        return ""
    }
    readonly property bool gitVisible: folderActive && gitListing !== null
        && gitListing.gitRoot !== "" && gitListing.gitBranch !== ""
    /// True while the tab is browsing a network backend. Local-only actions
    /// (terminal, recycle bin, content search, OS open/reveal) are gated on
    /// this instead of failing after the click.
    readonly property bool remoteLocation: folderActive
        && (activeTab.listing.remote || root.isRemotePath(activeTab.listing.path))
    readonly property bool searchActive: folderActive && searchQuery.trim().length > 0
    readonly property bool contentSearchActive: searchActive && searchMode === "content"
    readonly property bool splitActive: ExplorerPrefs.splitEnabled
        && folderActive && !searchActive
    /// The tab and list a context menu was opened from. `null` means the
    /// primary pane, which is also the only pane when split view is off.
    property var contextTab: null
    property var contextList: null
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
        || commandPalette.visible
        || passwordDialog.visible || settingsPanel.visible || confirmDialog.visible
        || connectDialog.visible || cloudDialog.visible || openWithDialog.visible
        || updateDialog.visible || transferConflict.visible || transferQueue.visible
        || propertiesDialog.visible || bulkRenameDialog.visible || moveToDialog.visible

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
        else if (root.overviewActive)
            // The landing page's recents change as folders are visited; an
            // already-open overview tab must re-read them when it comes back.
            overviewModel.refresh()
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
    /// The landing page is per-window so its git scan and refresh lifetime
    /// follow the window; the command palette reads the same instance.
    OverviewModel {
        id: overviewModel
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

    function insertTab(tab) {
        const next = root.tabs.slice()
        const at = Math.max(0, Math.min(root.activeIndex + 1, next.length))
        next.splice(at, 0, tab)
        root.tabs = next
        root.activeIndex = at
        root.applyRemotePolicy()
        root.focusActiveSurface()
    }

    function focusActiveSurface() {
        if (root.folderActive)
            Qt.callLater(fileList.takeFocus)
        else if (root.overviewActive)
            Qt.callLater(overviewView.takeFocus)
        else if (root.spaceActive)
            Qt.callLater(spaceView.takeFocus)
    }

    function addTab(path) {
        const tab = createTab(path !== undefined ? path : root.activePath)
        if (tab !== null)
            root.insertTab(tab)
    }

    /// A new tab inherits the current surface rather than always landing on a
    /// folder. From Overview, a space or the recycle bin, the new tab shows
    /// the same surface again.
    function addTabHere() {
        if (root.overviewActive) {
            root.addSurfaceTab("overview", "")
        } else if (root.spaceActive) {
            root.addSurfaceTab("space", root.activeSpaceId)
        } else if (root.trashActive) {
            root.addSurfaceTab("trash", "")
        } else {
            root.addTab(root.activePath)
        }
    }

    function addSurfaceTab(kind, id) {
        const tab = createTab("")
        if (tab === null)
            return
        if (kind === "overview")
            tab.openOverview()
        else if (kind === "space")
            tab.openSpace(id)
        else if (kind === "trash")
            tab.openTrash()
        root.insertTab(tab)
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

    /// Opens the path editor seeded with the current path and selected, so
    /// typing overtypes it and Enter replaces the whole thing. Shared by the
    /// bar's own click target and the path-jump shortcut.
    function beginPathEditing() {
        if (!root.folderActive || root.activeTab === null)
            return
        root.pathEditing = true
        pathInput.text = root.activePath
        // The field is only visible once `pathEditing` is on, and a hidden item
        // cannot take focus: wait for the state to settle before focusing.
        Qt.callLater(function () {
            pathInput.forceActiveFocus()
            pathInput.selectAll()
        })
    }

    /// The split pane keeps its own trail: navigating it never moves the
    /// primary tab, and vice versa.
    function navigateSplit(path) {
        if (root.splitTab === null || !path || path.length === 0)
            return
        root.splitTab.navigate(path)
        Workspace.recordRecent(path, true, false)
        Qt.callLater(splitList.takeFocus)
    }

    function goUpSplit() {
        if (root.splitTab === null)
            return
        const parent = root.parentOf(root.splitTab.listing.path)
        if (parent !== "")
            root.navigateSplit(parent)
    }

    function ensureSplitTab() {
        if (root.splitTab === null) {
            const tab = root.createTab(root.activePath)
            if (tab === null)
                return
            root.splitTab = tab
        } else if (root.splitTab.listing.path === "" && root.activePath !== "") {
            root.splitTab.navigate(root.activePath)
        }
    }

    function applySplitPreference() {
        if (ExplorerPrefs.splitEnabled) {
            root.ensureSplitTab()
            return
        }
        if (root.splitTab !== null) {
            root.splitTab.dispose()
            root.splitTab = null
        }
        root.splitFocused = false
    }

    function navigatePane(tab, path) {
        if (tab === null)
            root.navigate(path)
        else
            root.navigateSplit(path)
    }

    Connections {
        target: ExplorerPrefs
        function onSplitEnabledChanged() {
            root.applySplitPreference()
        }
    }

    function openTrash() {
        if (root.activeTab === null || root.modalOpen)
            return
        root.pathEditing = false
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.openTrash()
        trashModel.refresh()
        Qt.callLater(trashView.takeFocus)
    }

    /// Opens the landing page. The model's cheap sections rebuild synchronously
    /// so the page is usable in the same event; the project scan fills in later.
    function openOverview() {
        if (root.activeTab === null || root.modalOpen)
            return
        root.pathEditing = false
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.openOverview()
        overviewModel.refresh()
        Qt.callLater(overviewView.takeFocus)
    }

    /// Opens a space surface. `SpaceView` binds to the active tab's space id and
    /// reloads its items when the shared `Spaces` revision changes.
    function openSpace(id) {
        if (root.activeTab === null || root.modalOpen || !id || id.length === 0)
            return
        root.pathEditing = false
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.openSpace(id)
        Qt.callLater(spaceView.takeFocus)
    }

    /// A space deleted in another window must not leave a tab pointing at a
    /// missing id. Any such tab falls back to the overview, the closest safe
    /// surface; the active one is re-focused by `openOverview` only if the user
    /// was already on another tab.
    function reconcileSpaces() {
        for (let index = 0; index < root.tabs.length; index += 1) {
            const tab = root.tabs[index]
            if (tab.space && Spaces.nameOfId(tab.spaceId) === "")
                tab.openOverview()
        }
        if (root.overviewActive)
            overviewModel.refresh()
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
        } else if (mode === "deleteSpace") {
            const spaceId = ids.length > 0 ? ids[0] : ""
            if (!Spaces.deleteSpace(spaceId))
                toast.show(Spaces.error !== "" ? Spaces.error : "无法删除空间", true)
            else
                toast.show("空间已删除", false)
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
        if (root.splitTab !== null) {
            root.splitTab.dispose()
            root.splitTab = null
        }
        root.tabs = []
    }

    function currentSelection() {
        const list = root.contextList !== null ? root.contextList : fileList
        const paths = list.selectedPathList()
        if (paths.length > 0)
            return paths
        const path = root.contextPath !== "" ? root.contextPath : list.currentPath()
        return path !== "" ? [path] : []
    }

    function contextDirectory() {
        return root.contextTab !== null ? root.contextTab.listing.path : root.activePath
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
        const destination = root.contextDirectory()
        if (paths.length === 0 || destination === "")
            return
        if (encrypted) {
            root.archiveMode = "compress"
            root.archivePaths = paths
            root.archiveTarget = ""
            root.archiveDestination = destination
            passwordDialog.open("compress", "", "")
            return
        }
        Workspace.compressEntries(paths, destination, format, "")
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
        root.promptPath = root.contextTab !== null
            ? root.contextTab.listing.path : root.activePath
        namePrompt.open(folder ? "新建文件夹" : "新建文件",
                        folder ? "新建文件夹" : "新建文件.txt", "")
    }

    function promptCreateIn(path, folder) {
        if (root.modalOpen || path === "")
            return
        root.promptMode = folder ? "folder" : "file"
        root.promptPath = path
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

    function promptSpaceCreate(addPath) {
        if (root.modalOpen)
            return
        root.promptMode = "spaceCreate"
        root.promptPath = addPath !== undefined ? addPath : ""
        namePrompt.open("新建空间", "新空间", "")
    }

    function promptSpaceRename(id, name) {
        if (root.modalOpen || !id)
            return
        root.promptMode = "spaceRename"
        root.promptPath = id
        namePrompt.open("重命名空间", name, "")
    }

    /// One entry point for the palette. It closes first, then the surface
    /// switch runs against a clean modal state (the palette itself is part of
    /// `modalOpen`).
    function handlePaletteActivation(kind, value) {
        if (kind === "command") {
            root.runPaletteCommand(value)
            return
        }
        if (kind === "overview") {
            root.openOverview()
            return
        }
        if (kind === "trash") {
            root.openTrash()
            return
        }
        if (kind === "space") {
            root.openSpace(value)
            return
        }
        if (kind === "navigate") {
            root.navigate(value)
            return
        }
        if (kind === "file" || kind === "recent-file") {
            Workspace.openPath(value)
            return
        }
        if (kind === "path") {
            root.navigate(value)
        }
    }

    /// The palette's command ids are deliberately action-shaped, not menu ids:
    /// they describe what the user asked for, so the same id keeps working if
    /// the menu grows another entry or moves a shortcut.
    function runPaletteCommand(id) {
        if (id === "overview")
            root.openOverview()
        else if (id === "trash")
            root.openTrash()
        else if (id === "new-tab")
            root.addTabHere()
        else if (id === "close-tab")
            root.closeTab(root.activeIndex)
        else if (id === "settings")
            settingsPanel.open()
        else if (id === "new-space")
            root.promptSpaceCreate()
        else if (id === "path-jump") {
            if (root.folderActive)
                root.beginPathEditing()
        } else if (!root.folderActive) {
            return
        } else if (id === "new-folder") {
            root.promptCreateIn(root.activePath, true)
        } else if (id === "new-file") {
            root.promptCreateIn(root.activePath, false)
        } else if (id === "rename") {
            const path = fileList.currentPath()
            if (path !== "")
                root.promptRename(path, root.activeTab.listing.nameOf(path))
        } else if (id === "trash-selection") {
            if (!root.remoteLocation)
                Workspace.trashEntries(root.currentSelection())
        } else if (id === "copy") {
            Workspace.setClipboard("copy", root.currentSelection())
        } else if (id === "cut") {
            Workspace.setClipboard("cut", root.currentSelection())
        } else if (id === "paste") {
            Workspace.pasteInto(root.activePath)
        } else if (id === "copy-path") {
            Workspace.copyText(root.currentSelection().join("\n"))
        } else if (id === "select-all") {
            fileList.selectAll()
        } else if (id === "refresh") {
            root.activeTab.listing.reload()
        } else if (id === "go-up") {
            root.goUp()
        } else if (id === "open-terminal") {
            root.toggleTerminal()
        } else if (id === "toggle-favorite") {
            if (root.activePath !== "")
                Workspace.addFavorite(root.activePath)
        } else if (id === "toggle-split") {
            ExplorerPrefs.setBool("splitEnabled", !ExplorerPrefs.splitEnabled)
        } else if (id === "toggle-preview") {
            root.togglePreview()
        } else if (id === "toggle-sidebar") {
            root.sidebarVisible = !root.sidebarVisible
        } else if (id === "view-list") {
            ExplorerPrefs.setString("viewMode", "list")
        } else if (id === "view-grid") {
            ExplorerPrefs.setString("viewMode", "grid")
        } else if (id === "view-column") {
            ExplorerPrefs.setString("viewMode", "column")
        } else if (id === "sort-name") {
            ExplorerPrefs.setString("sortKey", "name")
            ExplorerPrefs.setString("sortOrder", "asc")
        } else if (id === "sort-modified") {
            ExplorerPrefs.setString("sortKey", "modified")
            ExplorerPrefs.setString("sortOrder", "desc")
        } else if (id === "sort-type") {
            ExplorerPrefs.setString("sortKey", "type")
            ExplorerPrefs.setString("sortOrder", "asc")
        } else if (id === "sort-size") {
            ExplorerPrefs.setString("sortKey", "size")
            ExplorerPrefs.setString("sortOrder", "desc")
        } else if (id === "sort-order") {
            ExplorerPrefs.setString("sortOrder",
                                    ExplorerPrefs.sortOrder === "asc" ? "desc" : "asc")
        } else if (id === "folders-first") {
            ExplorerPrefs.setBool("foldersFirst", !ExplorerPrefs.foldersFirst)
        } else if (id === "hidden-files") {
            ExplorerPrefs.setBool("showHiddenFiles", !ExplorerPrefs.showHiddenFiles)
        } else if (id === "filters-reset") {
            ExplorerPrefs.resetFilters()
        }
    }


    function requestSpaceDelete(id, name) {
        if (root.modalOpen || !id)
            return
        if (Spaces.isPresetOfId(id)) {
            toast.show("预设空间不能删除", true)
            return
        }
        root.confirmMode = "deleteSpace"
        root.confirmTrashIds = [id]
        confirmDialog.open("删除空间？",
                           "空间 “" + name + "” 将被删除，其中的项目不会从磁盘移除。",
                           "删除", true)
    }

    function addPathToSpace(spaceId, path) {
        if (!spaceId || !path)
            return
        if (!Spaces.addItem(spaceId, path))
            toast.show(Spaces.error !== "" ? Spaces.error : "无法添加到空间", true)
        else
            toast.show("已添加到“" + Spaces.nameOfId(spaceId) + "”", false)
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
        else if (mode === "spaceCreate") {
            const id = Spaces.createSpace(text)
            if (id === "")
                toast.show(Spaces.error !== "" ? Spaces.error : "无法创建空间", true)
            else {
                if (path !== "")
                    Spaces.addItem(id, path)
                root.openSpace(id)
                toast.show("已创建空间", false)
            }
        } else if (mode === "spaceRename") {
            if (!Spaces.renameSpace(path, text))
                toast.show(Spaces.error !== "" ? Spaces.error : "无法重命名空间", true)
        }
    }

    /// The row menu's space submenu is built from the shared singleton, so it
    /// never lists a stale rename. The final entry creates a space and pins the
    /// row in one prompt.
    function spaceSubmenu() {
        const entries = []
        for (let index = 0; index < Spaces.count; index += 1)
            entries.push({ id: "addSpace:" + Spaces.idAt(index),
                           label: Spaces.nameAt(index) })
        entries.push({ separator: true })
        entries.push({ id: "newSpaceAdd", label: "新建空间并添加…" })
        return entries
    }

    function showRowMenu(path, isDir, x, y) {
        root.openRowMenu(null, null, path, isDir, x, y)
    }

    function showSplitRowMenu(path, isDir, x, y) {
        root.openRowMenu(root.splitTab, splitList, path, isDir, x, y)
    }

    function openRowMenu(paneTab, paneList, path, isDir, x, y) {
        root.contextTab = paneTab
        root.contextList = paneList
        root.contextPath = path
        root.contextIsDir = isDir
        root.contextKind = "row"
        // Discovery runs off the Qt thread. The rows already in the model are
        // drawn at once, and `onShellCommandRevisionChanged` swaps in the fresh
        // answer without moving the panel.
        shellCommands.load(root.currentSelection(), path)
        menu.openAt(x, y, root.rowMenuItems())
    }

    /// The row menu's rows. Split out from `openRowMenu` because the installed
    /// apps' commands arrive after the menu is already on screen and the same
    /// list has to be rebuilt in place.
    function rowMenuItems() {
        const path = root.contextPath
        const isDir = root.contextIsDir
        const many = (root.contextList !== null
            ? root.contextList.selectionCount : fileList.selectionCount) > 1
        const hasPath = path !== ""
        const hasClipboard = Workspace.clipboardMode !== ""
        const remote = root.contextTab !== null
            ? (root.contextTab.listing.remote || root.isRemotePath(root.contextTab.listing.path))
            : root.remoteLocation
        // Archiving streams remote sources into a local container, so the
        // sources may be remote but the destination (here) may not.
        const canExtract = hasPath && !isDir && !remote && root.isArchivePath(path)
        const items = [
            { id: "open", label: isDir ? "打开" : "用默认应用打开",
              enabled: hasPath && (!remote || isDir), shortcut: "Enter" },
            { id: "openTab", label: "在新标签页打开", enabled: hasPath && isDir },
            { id: "openWindow", label: "在新窗口打开", enabled: hasPath && isDir },
            { separator: true },
            { id: "reveal", label: "在资源管理器中显示", enabled: hasPath && !remote },
            { id: "openWith", label: "选择打开方式…", enabled: hasPath && !remote },
            { id: "terminal", label: "在终端中打开", enabled: hasPath && isDir && !remote },
            { separator: true },
            { id: "preview", label: root.previewShows(path) ? "隐藏预览" : "预览",
              enabled: hasPath && !many, shortcut: "Space" },
            { id: "copyPath", label: "复制路径", enabled: hasPath }
        ]

        const extensions = root.shellCommandSubmenu()
        if (extensions.length > 0) {
            items.push({ separator: true })
            items.push({ id: "shellCommands", label: "应用扩展", submenu: extensions })
        }

        items.push({ separator: true })
        items.push({ id: "copy", label: "复制", shortcut: "Ctrl+C", enabled: hasPath })
        items.push({ id: "cut", label: "剪切", shortcut: "Ctrl+X", enabled: hasPath })
        items.push({ id: "paste", label: "粘贴到此处", shortcut: "Ctrl+V",
                     enabled: hasClipboard && isDir })
        items.push({ id: "pasteShortcut", label: "粘贴快捷方式",
                     enabled: hasClipboard && isDir && !remote })
        items.push({ id: "duplicate", label: "创建副本", enabled: hasPath })
        items.push({ id: "createShortcut", label: "创建快捷方式", enabled: hasPath && !remote })
        items.push({ id: "compress", label: "压缩为…", enabled: hasPath && !remote, submenu: [
            { id: "compressZip", label: "ZIP 压缩包" },
            { id: "compressTar", label: "TAR 归档" },
            { id: "compressTarGz", label: "TAR.GZ 压缩包" },
            { id: "compress7z", label: "7Z 压缩包" },
            { id: "compress7zEncrypted", label: "7Z 压缩包（加密）" }
        ] })
        items.push({ id: "extractHere", label: "解压到此处", enabled: canExtract })
        items.push({ id: "moveTo", label: "移动到…", enabled: hasPath })
        items.push({ id: many ? "bulkRename" : "rename",
                     label: many ? "批量重命名…" : "重命名",
                     enabled: hasPath,
                     shortcut: many ? "" : "F2" })
        items.push({ separator: true })
        items.push({ id: "favorite", label: "加入收藏", enabled: hasPath && isDir })
        items.push({ id: "addToSpace", label: "添加到空间", enabled: hasPath,
                     submenu: root.spaceSubmenu() })
        items.push({ id: "trash", label: "移到回收站", shortcut: "Delete",
                     enabled: hasPath && !remote, danger: true })
        items.push({ id: "delete", label: "永久删除", shortcut: "Shift+Delete",
                     enabled: hasPath, danger: true })
        items.push({ separator: true })
        items.push({ id: "properties", label: "属性", shortcut: "Alt+Enter",
                     enabled: hasPath && !many })
        return items
    }

    /// The installed apps' commands as a submenu: inline rows first, then one
    /// header per app that contributed more than one command.
    ///
    /// A header carries the app's own icon — the first command in the group
    /// that has one, which is always after the backend's app-logo fallback.
    function shellCommandSubmenu() {
        const items = []
        const groupIcons = ({})
        for (let index = 0; index < shellCommands.count; index += 1) {
            const name = shellCommands.groupAt(index)
            if (name === "" || groupIcons[name] !== undefined)
                continue
            groupIcons[name] = shellCommands.iconAt(index)
        }
        let group = ""
        for (let index = 0; index < shellCommands.count; index += 1) {
            const next = shellCommands.groupAt(index)
            if (next !== group) {
                group = next
                if (group !== "")
                    items.push({ header: true, label: group, icon: groupIcons[group] || "" })
            }
            items.push({
                id: "shell:" + shellCommands.idAt(index),
                label: shellCommands.labelAt(index),
                icon: shellCommands.iconAt(index),
                enabled: !shellCommands.disabledAt(index)
            })
        }
        return items
    }

    /// True when the preview panel is already showing exactly this entry.
    function previewShows(path) {
        return root.previewVisible && previewPanel.paths.length === 1
            && previewPanel.paths[0] === path
    }

    function togglePreview(path) {
        if (root.previewShows(path)) {
            root.previewVisible = false
            previewPanel.showPaths([])
            return
        }
        root.previewVisible = true
        previewPanel.showPaths([path])
    }

    /// The ancestors the breadcrumb bar folded into its `…`. The bar draws the
    /// button; the menu is the window's, the way every other menu here is.
    /// `split` says which pane the chosen ancestor belongs to, because the two
    /// panes keep separate trails.
    function openBreadcrumbMenu(x, y, names, paths, split) {
        root.contextKind = "breadcrumb"
        const items = []
        for (let index = 0; index < paths.length; index += 1)
            items.push({ id: "crumb:" + paths[index], label: names[index] })
        if (items.length === 0)
            return
        root.contextTab = split ? root.splitTab : null
        menu.openAt(x, y, items)
    }

    function showTrashRowMenu(id, originalParent, x, y) {
        root.contextKind = "trash"
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
        root.openEmptyMenu(null, null, x, y)
    }

    function showSplitEmptyMenu(x, y) {
        root.openEmptyMenu(root.splitTab, splitList, x, y)
    }

    function openEmptyMenu(paneTab, paneList, x, y) {
        root.contextKind = "empty"
        root.contextTab = paneTab
        root.contextList = paneList
        root.contextPath = ""
        root.contextIsDir = true
        const remote = paneTab !== null
            ? (paneTab.listing.remote || root.isRemotePath(paneTab.listing.path))
            : root.remoteLocation
        menu.openAt(x, y, [
            { id: "newFolder", label: "新建文件夹", shortcut: root.mod + "+Shift+N" },
            { id: "newFile", label: "新建文件" },
            { separator: true },
            { id: "paste", label: "粘贴", shortcut: "Ctrl+V", enabled: Workspace.clipboardMode !== "" },
            { id: "pasteShortcut", label: "粘贴快捷方式",
              enabled: Workspace.clipboardMode !== "" && !remote },
            { id: "selectAll", label: "全选", shortcut: "Ctrl+A" },
            { id: "refresh", label: "刷新", shortcut: "F5" },
            { separator: true },
            { id: "terminal", label: "在终端中打开", enabled: !remote }
        ])
    }

    function showSidebarMenu(kind, path, x, y) {
        root.contextKind = "sidebar"
        root.contextTab = null
        root.contextList = null
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
        if (kind === "space") {
            const preset = Spaces.isPresetOfId(path)
            menu.openAt(x, y, [
                { id: "spaceOpen", label: "打开空间" },
                { id: "spaceTab", label: "在新标签页打开" },
                { separator: true },
                { id: "spaceRename", label: "重命名空间" },
                { id: "spaceDelete", label: "删除空间", enabled: !preset, danger: true }
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

    /// Forgets which entry the menu was opened on. Every action that reaches
    /// the window through the menu ends here, including the ones that only
    /// needed the path for a moment.
    function clearContext() {
        root.contextTab = null
        root.contextList = null
        root.contextKind = ""
    }

    function runMenuAction(id) {
        const path = root.contextPath
        const isDir = root.contextIsDir
        // A folded ancestor carries its destination in the id: the menu is
        // rebuilt from whatever the bar hid, so there is no state to look up.
        if (id.indexOf("crumb:") === 0) {
            root.navigatePane(root.contextTab, id.substring("crumb:".length))
            root.clearContext()
            return
        }
        // The installed apps' commands carry their own id after the prefix;
        // the selection captured by the menu is what they run on.
        if (id.indexOf("shell:") === 0) {
            Workspace.invokeShellCommand(id.substring("shell:".length),
                                         root.currentSelection())
            root.clearContext()
            return
        }
        // The row menu's space submenu also encodes its destination, because
        // the menu itself has no per-item state once it opens.
        if (id.indexOf("addSpace:") === 0) {
            root.addPathToSpace(id.substring("addSpace:".length), path)
            root.clearContext()
            return
        }
        if (id === "newSpaceAdd") {
            root.promptSpaceCreate(path)
            root.clearContext()
            return
        }
        switch (id) {
        case "spaceOpen":
            root.openSpace(path)
            break
        case "spaceTab":
            root.addSurfaceTab("space", path)
            break
        case "spaceRename":
            root.promptSpaceRename(path, Spaces.nameOfId(path))
            break
        case "spaceDelete":
            root.requestSpaceDelete(path, Spaces.nameOfId(path))
            break
        case "open":
            if (isDir)
                root.navigatePane(root.contextTab, path)
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
            Workspace.openTerminal(path !== "" ? path : root.contextDirectory())
            break
        case "preview":
            root.togglePreview(path)
            break
        case "copyPath":
            if (Workspace.copyText(path))
                toast.show("已复制路径", false)
            else
                toast.show("无法写入剪贴板", true)
            break
        case "moveTo":
            moveToDialog.open(root.contextDirectory(), root.currentSelection())
            break
        case "bulkRename":
            bulkRenameDialog.open(root.currentSelection())
            break
        case "properties":
            // Windows has the real sheet; elsewhere the in-app dialog is the
            // only one there is.
            if (!Workspace.showNativeProperties(path))
                propertiesDialog.open(path)
            break
        case "copy":
            Workspace.setClipboard("copy", root.currentSelection())
            break
        case "cut":
            Workspace.setClipboard("cut", root.currentSelection())
            break
        case "paste":
            Workspace.pasteInto(isDir && path !== "" ? path : root.contextDirectory())
            break
        case "pasteShortcut":
            Workspace.pasteShortcutInto(isDir && path !== "" ? path : root.contextDirectory())
            break
        case "createShortcut":
            Workspace.createShortcuts(root.currentSelection(), root.contextDirectory())
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
            if (root.contextList !== null)
                root.contextList.selectAll()
            else
                fileList.selectAll()
            break
        case "refresh":
            if (root.contextTab !== null)
                root.contextTab.listing.reload()
            else if (root.activeTab !== null)
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
        root.clearContext()
    }

    function reloadAfterOperation(kind, focus) {
        if (root.activeTab === null)
            return
        const trashKinds = ["trash", "restore_trash", "purge_trash", "empty_trash"]
        if (trashKinds.indexOf(kind) >= 0 && root.trashActive)
            trashModel.refresh()
        const reloadKinds = ["create", "rename", "trash", "delete", "copy", "move",
                             "duplicate", "shortcut", "compress", "extract", "undo", "redo",
                             "properties"]
        if (reloadKinds.indexOf(kind) < 0)
            return
        root.activeTab.listing.reload()
        if (root.splitTab !== null)
            root.splitTab.listing.reload()
        if (focus !== "")
            fileList.focusPath(focus)
    }

    function modalClosed() {
        Qt.callLater(function () {
            if (root.modalOpen)
                return
            if (root.overviewActive)
                overviewView.takeFocus()
            else if (root.spaceActive)
                spaceView.takeFocus()
            else if (root.trashActive)
                trashView.takeFocus()
            else if (root.splitActive && root.splitFocused)
                splitList.takeFocus()
            else
                fileList.takeFocus()
        })
    }

    function toggleTerminal() {
        if (!root.folderActive)
            return
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
        target: Spaces

        function onChanged() {
            root.reconcileSpaces()
            // The landing page lists spaces too; refresh it in place so a
            // rename or a new space is not a stale row.
            overviewModel.refresh()
            if (commandPalette.visible)
                commandPalette.rebuild()
        }
    }

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            overviewModel.refresh()
        }

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

    // Branch commands belong to the active pane's repository. The create form
    // shows its own inline error, so it keeps that failure instead of also
    // raising a toast behind the panel.
    Connections {
        target: root.gitListing

        function onGitOperationFailed(operation, message) {
            if (operation === "create" && gitPanel.visible && gitPanel.creating)
                return
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
            onNewTabRequested: root.addTabHere()
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
                color: root.folderActive && upMouse.containsMouse
                    ? (root.dark ? "#24ffffff" : "#12000000") : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: "\u2191"
                    color: root.folderActive ? root.mutedColor : root.hairline
                    font.pixelSize: 13
                }

                MouseArea {
                    id: upMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: root.folderActive
                    onClicked: root.goUp()
                }
            }

            Rectangle {
                id: pathField
                anchors.left: upButton.right
                anchors.leftMargin: 6
                anchors.right: root.folderActive ? searchBar.left : parent.right
                anchors.rightMargin: root.folderActive ? 6 : 12
                anchors.verticalCenter: parent.verticalCenter
                height: 26
                radius: 7
                color: root.dark ? "#1affffff" : "#0d000000"
                border.width: 1
                border.color: root.pathEditing
                    ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf") : root.hairline

                // The bar's normal shape is the trail: every ancestor is a
                // destination, and the current folder is the one name it must
                // keep. Typing a path is the exception, so the editor is a
                // state of this bar rather than the bar itself.
                MouseArea {
                    anchors.fill: parent
                    enabled: root.folderActive && !root.pathEditing
                    cursorShape: Qt.IBeamCursor
                    onClicked: root.beginPathEditing()
                }

                PathBreadcrumbs {
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 4
                    visible: root.folderActive && !root.pathEditing
                    names: root.activeTab !== null ? root.activeTab.listing.breadcrumbNames : []
                    paths: root.activeTab !== null ? root.activeTab.listing.breadcrumbPaths : []
                    dark: root.dark
                    textColor: root.textColor
                    mutedColor: root.mutedColor
                    accent: root.accent

                    onCrumbActivated: function (path) {
                        root.navigate(path)
                    }
                    onAncestorsRequested: function (x, y, names, paths) {
                        root.openBreadcrumbMenu(x, y, names, paths, false)
                    }
                }

                // Overview, spaces and the recycle bin are surfaces, not paths:
                // there is nothing to walk back through, and nothing to type.
                // One label serves all three, so the bar does not grow a branch
                // per surface.
                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !root.folderActive
                    text: root.surfaceTitle
                    color: root.textColor
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }

                TextInput {
                    id: pathInput

                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    visible: root.pathEditing && root.folderActive
                    verticalAlignment: TextInput.AlignVCenter
                    text: ""
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: "#ffffff"
                    font.pixelSize: 13
                    selectByMouse: true
                    clip: true

                    // TextInput assigns to `text` while the user edits, which
                    // permanently drops any binding on it. `beginPathEditing`
                    // seeds it instead, and the field goes back to naming the
                    // trail once it loses focus — every way out of the editor
                    // (Enter, Escape, a click elsewhere) ends here.
                    onActiveFocusChanged: {
                        if (!activeFocus)
                            root.pathEditing = false
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
                visible: root.folderActive
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
                overviewActive: root.overviewActive
                spaceActive: root.spaceActive
                activeSpaceId: root.activeSpaceId
                onNavigateRequested: function (path) {
                    root.navigate(path)
                }
                onTrashRequested: root.openTrash()
                onOverviewRequested: root.openOverview()
                onSpaceRequested: function (id) {
                    root.openSpace(id)
                }
                onCreateSpaceRequested: root.promptSpaceCreate()
                onAddRequested: function (kind) {
                    if (kind === "cloud")
                        root.openCloudDialog()
                    else if (kind === "space")
                        root.promptSpaceCreate()
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
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        anchors.right: root.splitActive ? splitDivider.left : parent.right
                        visible: root.folderActive && !root.contentSearchActive
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
                        onPreviewRequested: function (path) {
                            root.togglePreview(path)
                        }
                    }

                    /// The second pane. Its tab is independent down to the
                    /// listing and the selection; the only shared state is the
                    /// display preferences, which a file manager should not
                    /// ask twice.
                    Item {
                        id: splitPane
                        anchors.left: splitDivider.right
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        visible: root.splitActive

                        Rectangle {
                            id: splitPathBar
                            width: parent.width
                            height: 32
                            color: root.chromeColor

                            Text {
                                id: splitClose
                                anchors.right: parent.right
                                anchors.rightMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                text: "\u00D7"
                                color: splitCloseMouse.containsMouse
                                    ? root.textColor : root.mutedColor
                                font.pixelSize: 13
                            }

                            // The second pane names its own directory the same
                            // way the first one does; a bare path string here
                            // would be the only place in the window where the
                            // trail is missing.
                            //
                            // Declared after the bar's own click area: later
                            // siblings paint on top, and crumbs the click
                            // target covered would never receive their clicks.
                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: {
                                    if (splitCloseMouse.containsMouse)
                                        ExplorerPrefs.setBool("splitEnabled", false)
                                    else
                                        splitList.takeFocus()
                                }
                            }

                            PathBreadcrumbs {
                                anchors.left: parent.left
                                anchors.leftMargin: 8
                                anchors.right: splitClose.left
                                anchors.rightMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                height: 22
                                fontSize: 12
                                names: root.splitTab !== null
                                    ? root.splitTab.listing.breadcrumbNames : []
                                paths: root.splitTab !== null
                                    ? root.splitTab.listing.breadcrumbPaths : []
                                dark: root.dark
                                textColor: root.textColor
                                mutedColor: root.mutedColor
                                accent: root.accent

                                onCrumbActivated: function (path) {
                                    root.navigateSplit(path)
                                }
                                onAncestorsRequested: function (x, y, names, paths) {
                                    root.openBreadcrumbMenu(x, y, names, paths, true)
                                }
                            }

                            MouseArea {
                                id: splitCloseMouse
                                x: splitClose.x - 6
                                y: splitClose.y - 6
                                width: splitClose.width + 12
                                height: splitClose.height + 12
                                hoverEnabled: true
                                acceptedButtons: Qt.LeftButton
                                onClicked: ExplorerPrefs.setBool("splitEnabled", false)
                            }

                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                height: 1
                                color: root.hairline
                            }
                        }

                        FileList {
                            id: splitList
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: splitPathBar.bottom
                            anchors.bottom: parent.bottom
                            visible: root.splitTab !== null
                            tab: root.splitTab
                            dark: root.dark
                            headerColor: root.chromeColor
                            accent: root.accent

                            onNavigateRequested: function (path) {
                                root.navigateSplit(path)
                            }
                            onUpRequested: root.goUpSplit()
                            onRenameRequested: function (path, name) {
                                root.promptRename(path, name)
                            }
                            onCreateFolderRequested: root.promptCreateIn(
                                root.splitTab !== null ? root.splitTab.listing.path : "", true)
                            onContextRequested: function (path, isDir, x, y) {
                                root.showSplitRowMenu(path, isDir, x, y)
                            }
                            onEmptyContextRequested: function (x, y) {
                                root.showSplitEmptyMenu(x, y)
                            }
                            onSelectionChanged: {
                                previewPanel.showPaths(splitList.selectedPathList())
                            }
                            onRemoteDeleteRequested: function (paths) {
                                root.requestRemoteDelete(paths)
                            }
                            onRemoteOpenRequested: function (path) {
                                root.acceptRemotePreview(path)
                            }
                        }
                    }

                    Item {
                        id: splitDivider
                        visible: root.splitActive
                        width: 6
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        x: Math.round((parent.width - width) * root.splitRatio)

                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: 1
                            height: parent.height
                            color: root.hairline
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.SplitHCursor
                            property real startPointerX: 0
                            property real startRatio: 0

                            onPressed: function (mouse) {
                                startPointerX = mapToItem(null, mouse.x, 0).x
                                startRatio = root.splitRatio
                            }
                            onPositionChanged: function (mouse) {
                                if (!pressed)
                                    return
                                const now = mapToItem(null, mouse.x, 0).x
                                const space = splitDivider.parent.width
                                if (space <= 0)
                                    return
                                root.splitRatio = Math.max(0.25, Math.min(0.75,
                                    startRatio + (now - startPointerX) / space))
                            }
                        }
                    }

                    ContentResults {
                        anchors.fill: parent
                        visible: root.folderActive && root.contentSearchActive
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

                    OverviewView {
                        id: overviewView
                        anchors.fill: parent
                        visible: root.overviewActive
                        model: overviewModel
                        dark: root.dark
                        accent: root.accent
                        textColor: root.textColor
                        mutedColor: root.mutedColor
                        hairline: root.hairline
                        headerColor: root.chromeColor
                        commandShortcut: root.config !== null && root.config.commandBar !== ""
                            ? root.config.commandBar : "Ctrl+K"

                        onOpenRequested: function (kind, path) {
                            if (kind === "space")
                                root.openSpace(path)
                            else if (kind === "file")
                                Workspace.openPath(path)
                            else
                                root.navigate(path)
                        }
                        onCommandPaletteRequested: commandPalette.openPalette()
                    }

                    SpaceView {
                        id: spaceView
                        anchors.fill: parent
                        visible: root.spaceActive
                        spaceId: root.activeSpaceId
                        dark: root.dark
                        accent: root.accent
                        textColor: root.textColor
                        mutedColor: root.mutedColor
                        hairline: root.hairline
                        headerColor: root.chromeColor

                        onOpenRequested: function (path) {
                            root.navigate(path)
                        }
                        onRenameRequested: function (id, name) {
                            root.promptSpaceRename(id, name)
                        }
                        onDeleteRequested: function (id, name) {
                            root.requestSpaceDelete(id, name)
                        }
                        onErrorReported: function (message) {
                            toast.show(message, true)
                        }
                    }
                }

                TerminalPanel {
                    id: terminalPanel
                    width: parent.width
                    active: root.folderActive && root.terminalActive
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
                width: root.previewVisible && root.folderActive ? 300 : 0
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
                width: parent.width * (gitChip.visible ? 0.32 : 0.46)
                anchors.verticalCenter: parent.verticalCenter
                text: {
                    if (root.activeTab === null)
                        return ""
                    if (root.overviewActive)
                        return overviewModel.projectBusy
                            ? "查找项目…"
                            : (overviewModel.loaded
                                ? overviewModel.count + " 个入口"
                                : "读取概览…")
                    if (root.spaceActive)
                        return "空间 · " + spaceView.itemCount + " 项"
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
                visible: root.folderActive
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
                id: filterButton
                anchors.right: updateButton.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                width: 46
                height: 18
                visible: root.folderActive

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: filterMouse.containsMouse || filterMenu.visible
                        ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                    Text {
                        anchors.centerIn: parent
                        text: "筛选"
                        color: filterMenu.active ? root.textColor : root.mutedColor
                        font.pixelSize: 10
                    }

                    Rectangle {
                        visible: filterMenu.active
                        anchors.right: parent.right
                        anchors.rightMargin: 6
                        anchors.top: parent.top
                        anchors.topMargin: 3
                        width: 5
                        height: 5
                        radius: 2.5
                        color: root.accent
                    }
                }

                MouseArea {
                    id: filterMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        const point = filterButton.mapToItem(null, 0, 0)
                        filterMenu.openAbove(point.x + filterButton.width - 252, point.y)
                    }
                }
            }

            Item {
                id: viewButton
                anchors.right: filterButton.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                width: 46
                height: 18
                visible: root.folderActive

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: viewMouse.containsMouse || displayMenu.visible
                        ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                    Text {
                        anchors.centerIn: parent
                        text: "视图"
                        color: root.mutedColor
                        font.pixelSize: 10
                    }
                }

                MouseArea {
                    id: viewMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        const point = viewButton.mapToItem(null, 0, 0)
                        displayMenu.openAbove(point.x + viewButton.width - 252, point.y)
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
                visible: root.folderActive
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
                anchors.left: queueChip.visible ? queueChip.right
                    : (gitChip.visible ? gitChip.right : statusText.right)
                anchors.leftMargin: queueChip.visible || gitChip.visible ? 12 : 16
                anchors.right: viewButton.left
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

            // The repository the pane is inside. Branch, divergence and a
            // one-click sync live here; the whole chip opens the branch panel.
            Rectangle {
                id: gitChip
                anchors.left: statusText.right
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: gitChipContent.implicitWidth + 14
                    + (gitSyncButton.visible ? gitSyncButton.width : 0)
                height: 16
                radius: 8
                visible: root.gitVisible
                color: gitChipMouse.containsMouse || gitPanel.visible
                    ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.24)
                    : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.12)
                border.width: 1
                border.color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.42)

                Accessible.role: Accessible.Button
                Accessible.name: "Git 分支 "
                    + (root.gitListing !== null ? root.gitListing.gitBranch : "")
                    + "，打开分支面板"
                Accessible.onPressAction: gitChip.openPanel()

                function branchName() {
                    return root.gitListing !== null ? root.gitListing.gitBranch : ""
                }

                function ahead() {
                    return root.gitListing !== null ? root.gitListing.gitAhead : 0
                }

                function behind() {
                    return root.gitListing !== null ? root.gitListing.gitBehind : 0
                }

                function openPanel() {
                    const point = gitChip.mapToItem(null, 0, 0)
                    gitPanel.openAbove(point.x, point.y)
                }

                Row {
                    id: gitChipContent
                    anchors.left: parent.left
                    anchors.leftMargin: 7
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Git"
                        color: root.accent
                        font.pixelSize: 9
                        font.weight: Font.DemiBold
                        textFormat: Text.PlainText
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.min(implicitWidth, 112)
                        text: gitChip.branchName()
                        color: root.gitListing !== null && root.gitListing.gitDetached
                            ? root.mutedColor : root.textColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: gitChip.ahead() > 0
                        text: "↑" + gitChip.ahead()
                        color: root.dark ? "#7fc98a" : "#1a7f37"
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: gitChip.behind() > 0
                        text: "↓" + gitChip.behind()
                        color: root.dark ? "#6cb6ff" : "#0969da"
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }
                }

                Item {
                    id: gitSyncButton
                    anchors.right: parent.right
                    anchors.rightMargin: 2
                    anchors.verticalCenter: parent.verticalCenter
                    width: visible ? 16 : 0
                    height: 16
                    visible: root.gitListing !== null
                        && root.gitListing.gitRemotes.length > 0

                    Text {
                        id: gitSyncGlyph
                        anchors.centerIn: parent
                        text: "↻"
                        color: root.gitListing !== null && root.gitListing.gitBusy
                            ? root.accent : root.mutedColor
                        font.pixelSize: 11
                        textFormat: Text.PlainText
                        transformOrigin: Item.Center

                        RotationAnimator on rotation {
                            from: 0
                            to: 360
                            duration: 900
                            loops: Animation.Infinite
                            running: root.gitListing !== null && root.gitListing.gitBusy
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: root.gitListing !== null && !root.gitListing.gitBusy
                        onClicked: {
                            if (root.gitListing !== null)
                                root.gitListing.gitSync()
                        }
                    }
                }

                MouseArea {
                    id: gitChipMouse
                    anchors.fill: parent
                    anchors.rightMargin: gitSyncButton.width
                    hoverEnabled: true
                    onClicked: gitChip.openPanel()
                }
            }

            // Transfers waiting behind the one on the progress row. The chip is
            // the queue's only door: without it a second paste would look like
            // it did nothing until the first copy finished.
            Rectangle {
                id: queueChip
                anchors.left: gitChip.visible ? gitChip.right : statusText.right
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: queueChipLabel.implicitWidth + 14
                height: 16
                radius: 8
                visible: Workspace.queuedTransfers.length > 0
                color: queueChipMouse.containsMouse
                    ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.24)
                    : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.14)
                border.width: 1
                border.color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.45)

                Accessible.role: Accessible.Button
                Accessible.name: queueChipLabel.text + "，查看传输队列"
                Accessible.onPressAction: queueChip.openPanel()

                Text {
                    id: queueChipLabel
                    anchors.centerIn: parent
                    text: "队列 " + Workspace.queuedTransfers.length
                    color: root.textColor
                    font.pixelSize: 10
                }

                function openPanel() {
                    const point = queueChip.mapToItem(null, 0, 0)
                    transferQueue.openAbove(point.x, point.y)
                }

                MouseArea {
                    id: queueChipMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: queueChip.openPanel()
                }
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

    DisplayMenu {
        id: displayMenu
        anchors.fill: parent
        dark: root.dark
        accent: root.accent
    }

    FilterMenu {
        id: filterMenu
        anchors.fill: parent
        dark: root.dark
        accent: root.accent
    }

    Shortcut {
        sequence: "Ctrl+1"
        onActivated: ExplorerPrefs.setString("viewMode", "list")
    }

    Shortcut {
        sequence: "Ctrl+2"
        onActivated: ExplorerPrefs.setString("viewMode", "grid")
    }

    Shortcut {
        sequence: "Ctrl+3"
        onActivated: ExplorerPrefs.setString("viewMode", "column")
    }

    Shortcut {
        sequence: "F6"
        enabled: root.splitActive
        onActivated: {
            root.splitFocused = !root.splitFocused
            if (root.splitFocused)
                splitList.takeFocus()
            else
                fileList.takeFocus()
        }
    }

    Shortcut {
        sequence: "Ctrl+Alt+V"
        onActivated: {
            const point = viewButton.mapToItem(null, 0, 0)
            displayMenu.openAbove(point.x + viewButton.width - 252, point.y)
        }
    }

    Shortcut {
        sequence: "Ctrl+Alt+F"
        onActivated: {
            const point = filterButton.mapToItem(null, 0, 0)
            filterMenu.openAbove(point.x + filterButton.width - 252, point.y)
        }
    }

    Shortcut {
        sequence: "Ctrl+Alt+S"
        onActivated: ExplorerPrefs.setBool("splitEnabled", !ExplorerPrefs.splitEnabled)
    }

    Shortcut {
        sequence: "Alt+Return"
        enabled: !root.modalOpen && !root.pathEditing && !root.trashActive
        onActivated: {
            const list = root.splitActive && root.splitFocused ? splitList : fileList
            const path = list.currentPath()
            if (path !== "" && !Workspace.showNativeProperties(path))
                propertiesDialog.open(path)
        }
    }

    /// Bumped by every shell-command answer. The row menu is rebuilt in place
    /// when it lands, so the applications' rows appear under the pointer
    /// instead of on the next right-click.
    readonly property int shellCommandRevision: shellCommands.revision

    onShellCommandRevisionChanged: {
        if (menu.visible && root.contextKind === "row" && root.contextPath !== "")
            menu.replaceActions(root.rowMenuItems())
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

    /// The installed apps' right-click commands. One model per window: the
    /// rows follow this window's selection, and a second window has its own.
    ShellCommandModel {
        id: shellCommands
    }

    PropertiesDialog {
        id: propertiesDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onChanged: root.reloadAfterOperation("properties", "")
        onClosed: root.modalClosed()
    }

    BulkRenameDialog {
        id: bulkRenameDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onClosed: root.modalClosed()
    }

    MoveToDialog {
        id: moveToDialog
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onClosed: root.modalClosed()
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

    CommandPalette {
        id: commandPalette
        anchors.fill: parent
        z: 1200
        dark: root.dark
        accent: root.accent
        textColor: root.textColor
        mutedColor: root.mutedColor
        hairline: root.hairline
        headerColor: root.chromeColor
        overviewModel: overviewModel
        folderActive: root.folderActive
        scopePath: root.folderActive ? root.activePath : Workspace.homeDirectory
        commandShortcut: root.config !== null && root.config.commandBar !== ""
            ? root.config.commandBar : "Ctrl+K"
        pathShortcut: root.config !== null && root.config.pathJump !== ""
            ? root.config.pathJump : "Ctrl+P"

        onActivated: function (kind, value) {
            commandPalette.closePalette()
            root.handlePaletteActivation(kind, value)
        }
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

    // Both are bound to `Workspace` rather than opened by a function: the
    // pending batch and the queue live in the command surface, so a dialog that
    // also kept its own open flag could disagree with them.
    TransferConflictDialog {
        id: transferConflict
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onModalClosed: root.modalClosed()
    }

    TransferQueue {
        id: transferQueue
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onVisibleChanged: {
            if (!visible)
                root.modalClosed()
        }
    }

    GitPanel {
        id: gitPanel
        anchors.fill: parent
        repo: root.gitListing
        dark: root.dark
        accent: root.accent

        onVisibleChanged: {
            if (!visible)
                root.modalClosed()
        }
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
        onActivated: root.addTabHere()
    }

    Shortcut {
        sequence: root.config !== null && root.config.commandBar !== ""
            ? root.config.commandBar : "Ctrl+K"
        enabled: !root.modalOpen
        onActivated: commandPalette.openPalette()
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
        enabled: !root.modalOpen && root.folderActive
        onActivated: root.beginPathEditing()
    }

    Shortcut {
        sequence: "Ctrl+J"
        enabled: !root.modalOpen && root.folderActive && !root.remoteLocation
        onActivated: root.toggleTerminal()
    }

    Shortcut {
        sequence: "Ctrl+F"
        enabled: !root.modalOpen && root.folderActive
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
        enabled: !root.modalOpen && root.folderActive
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
            else if (commandPalette.visible)
                commandPalette.closePalette()
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
            // One process-wide warm-up, so the first right-click already has
            // the installed apps' verbs instead of waiting on a COM surrogate.
            Workspace.warmShellCommands()
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
        root.applySplitPreference()
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
            // A path that changed under the editor (a tab switch, a navigate
            // from anywhere else) ends the edit: the text in it belongs to the
            // directory the user was on, not to the one now shown.
            root.pathEditing = false
            root.applyRemotePolicy()
        }

        function onTrashActiveChanged() {
            root.pathEditing = false
            if (root.trashActive) {
                root.terminalActive = false
                trashView.takeFocus()
            }
        }
    }
}
