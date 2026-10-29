import QtQuick
import QtQuick.Window
import dae 1.0
import dae.activation 1.0
import "Shell.js" as ShellState
import "Shortcut.js" as Shortcut

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
    // Theme follows the appearance preference first and the OS only when the
    // preference asks for it. `dark` stays a binding so a change in the panel
    // repaints every window, not just the one that made the change.
    property bool dark: Appearance.theme === "dark"
        || (Appearance.theme === "system" && Workspace.systemDark)
    /// The material the native window actually drew, which may be `none` even
    /// when the user asked for a material this machine cannot composite.
    /// Children read this to choose solid or translucent chrome.
    readonly property string material: chrome.appliedMaterial
    /// Cmd on macOS, Ctrl elsewhere; also how a recorded binding is written.
    readonly property bool isMac: Qt.platform.os === "osx"
    /// Where the tab strip starts in the title bar: after the traffic lights on
    /// macOS, after the 17 dp mark elsewhere. The native caption band and the
    /// QML anchors read the same number so they cannot disagree.
    readonly property real tabLeftInset: root.isMac ? 78 : 44

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
    /// Whether the search field is out. It stays out while a query is live, so
    /// the results and the field that produced them never separate.
    property bool searchOpen: false
    property bool searchRegex: false
    property bool searchCase: false
    /// The content query's file filter, as the glob list the backend takes.
    property string searchFilter: ""
    /// Which category produced `searchFilter`. The bar paints the label; the
    /// shell keeps the id so a language change or a mode switch cannot lose
    /// the selection.
    property string searchType: "all"
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
            return qsTr("回收站")
        if (overviewActive)
            return qsTr("概览")
        if (spaceActive)
            return activeTab.spaceTitle.length > 0 ? activeTab.spaceTitle : qsTr("空间")
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
    /// What the filter row reports on the right. Name search counts rows;
    /// content search counts the files that hold a match, because one file can
    /// answer with a dozen lines and "12 项匹配" would then be a lie about the
    /// number of files the user is about to see.
    readonly property int searchMatchCount: {
        if (activeTab === null || !searchActive)
            return 0
        return searchMode === "content" ? activeTab.contentSearch.fileCount
                                        : activeTab.nameSearch.loaded
    }
    readonly property string mod: Qt.platform.os === "osx" ? "Meta" : "Ctrl"
    /// System accent follows the OS; the brand accent is the app's own blue
    /// pair, one shade per theme. `accent` is the only colour views consume,
    /// so switching the preference retints the whole shell.
    readonly property color accent: Appearance.accent === "brand"
        ? Theme.brand
        : Workspace.accent
    /// Whether the active tab has somewhere to go back or forward to. The
    /// stacks live on the tab, so a torn-off window keeps its own.
    /// One switch for every Behavior in the shell. Qt 6 has no system-wide
    /// reduced-motion query, so the preference lives in `explorer.ini`.
    readonly property bool reduceMotion: ExplorerPrefs.reduceMotion
    readonly property bool canGoBack: activeTab !== null && activeTab.canGoBack
    readonly property bool canGoForward: activeTab !== null && activeTab.canGoForward
    /// `settings.toml` arrives as JSON because QML has no typed map property.
    /// Empty values are deliberate unbinds and therefore stay empty.
    readonly property var shortcutMap: {
        try {
            return JSON.parse(Workspace.shortcutsJson)
        } catch (error) {
            return ({})
        }
    }
    function shortcutBinding(id) {
        return root.shortcutMap && root.shortcutMap[id] ? root.shortcutMap[id] : ""
    }
    function shortcutDisplay(id) {
        return Shortcut.display(root.shortcutBinding(id), root.isMac)
    }
    /// A chord the shell fixes itself, with no row in `settings.toml`. It
    /// prints through the same formatter, so `Mod+Shift+N` reads `⇧⌘N` on
    /// macOS instead of leaking the internal `Meta` spelling into a menu.
    function chord(binding) {
        return Shortcut.display(binding, root.isMac)
    }
    function shortcutSequence(id) {
        return Shortcut.qtSequence(root.shortcutBinding(id), root.isMac)
    }
    function shortcutMatches(event, id) {
        return Shortcut.matches(event, root.shortcutBinding(id))
    }

    /// A few status lines are formatted in Rust, where the shared model code
    /// also feeds the Tauri shell. Translate the fixed shapes here rather than
    /// making the backend pick a language; anything unrecognised is passed
    /// through untouched.
    function localizeBackend(text) {
        if (!text)
            return text
        let match = /^Items: (\d+) · (.+)$/.exec(text)
        if (match)
            return qsTr("%1 项 · %2").arg(match[1]).arg(match[2])
        match = /^(?:Items: )?(\d+)$/.exec(text)
        if (match)
            return qsTr("%1 项").arg(match[1])
        match = /^(\d+) items?$/.exec(text)
        if (match)
            return qsTr("%1 项").arg(match[1])
        match = /^Results: (\d+)( · Truncated)?$/.exec(text)
        if (match)
            return qsTr("%1 项结果").arg(match[1])
                + (match[2] ? " · " + qsTr("已截断") : "")
        match = /^(\d+) changes?$/.exec(text)
        if (match)
            return qsTr("%1 项变更").arg(match[1])
        match = /^(Copy|Move) (\d+) items? → (.+)$/.exec(text)
        if (match)
            return qsTr("%1 %2 项 → %3")
                .arg(match[1] === "Copy" ? qsTr("复制") : qsTr("移动"))
                .arg(match[2]).arg(match[3])
        match = /^Switched to branch (.+)$/.exec(text)
        if (match)
            return qsTr("已切换到分支 %1").arg(match[1])
        match = /^Created branch (.+)$/.exec(text)
        if (match)
            return qsTr("已创建分支 %1").arg(match[1])
        match = /^Could not create log directory: (.+)$/.exec(text)
        if (match)
            return qsTr("无法创建日志目录：%1").arg(match[1])
        // Workspace status and progress lines are built in shared Rust code.
        // Their fixed vocabulary is translated here; a current file name in a
        // progress label is user data and falls through unchanged.
        switch (text) {
        case "No matches": return qsTr("没有匹配项")
        case "No matching content": return qsTr("没有匹配内容")
        case "Invalid search query": return qsTr("搜索表达式无效")
        case "Loading…": return qsTr("读取中…")
        case "Loading Trash…": return qsTr("读取回收站…")
        case "Loading folder…": return qsTr("读取文件夹…")
        case "Searching…": return qsTr("搜索中…")
        case "Searching file contents…": return qsTr("搜索文件内容…")
        case "Trash is empty": return qsTr("回收站是空的")
        case "Creating…": return qsTr("新建中…")
        case "Renaming…": return qsTr("重命名中…")
        case "Moving to Trash…": return qsTr("移到回收站…")
        case "Deleting…": return qsTr("删除中…")
        case "Duplicating…": return qsTr("创建副本…")
        case "Creating shortcut…": return qsTr("创建快捷方式…")
        case "Copying…": return qsTr("复制中…")
        case "Moving…": return qsTr("移动中…")
        case "Compressing…": return qsTr("压缩中…")
        case "Extracting…": return qsTr("解压中…")
        case "Undoing…": return qsTr("撤销中…")
        case "Redoing…": return qsTr("重做中…")
        case "Restoring…": return qsTr("恢复中…")
        case "Deleting permanently…": return qsTr("永久删除中…")
        case "Emptying Trash…": return qsTr("清空回收站…")
        case "Opening…": return qsTr("正在打开…")
        case "Setting as default…": return qsTr("正在设为默认打开方式…")
        case "Calculating…": return qsTr("计算中…")
        case "Created": return qsTr("已创建")
        case "Renamed": return qsTr("已重命名")
        case "Moved to Trash": return qsTr("已移到回收站")
        case "Permanently deleted": return qsTr("已永久删除")
        case "Duplicated": return qsTr("已创建副本")
        case "Shortcut created": return qsTr("已创建快捷方式")
        case "Copied": return qsTr("已复制")
        case "Moved": return qsTr("已移动")
        case "Compressed": return qsTr("已压缩")
        case "Extracted": return qsTr("已解压")
        case "Undone": return qsTr("已撤销")
        case "Redone": return qsTr("已重做")
        case "Opened": return qsTr("已打开")
        case "Restored": return qsTr("已恢复")
        case "Trash emptied": return qsTr("已清空回收站")
        case "Done": return qsTr("完成")
        case "Opened in system terminal": return qsTr("已在系统终端中打开")
        case "Added to favorites": return qsTr("已加入收藏")
        case "Removed from favorites": return qsTr("已从收藏移除")
        case "Connection removed": return qsTr("已移除连接")
        case "Cloud account removed": return qsTr("已移除云盘账户")
        case "Connection test succeeded": return qsTr("连接测试成功")
        case "Synced with remote": return qsTr("已同步远程仓库")
        case "Pulled remote updates": return qsTr("已拉取远程更新")
        case "Pushed to remote": return qsTr("已推送到远程")
        case "Fetched remote updates": return qsTr("已获取远程更新")
        case "Closed": return qsTr("已关闭")
        case "Shortcuts saved": return qsTr("快捷键已保存")
        case "Terminal settings saved": return qsTr("终端设置已保存")
        case "Opened log directory": return qsTr("已打开日志目录")
        default: return text
        }
    }
    readonly property string sourceLabel: config === null ? ""
        : config.startupSource === "deepLink" ? qsTr("深链")
        : config.startupSource === "recent" ? qsTr("最近访问")
        : config.startupSource === "commandLine" ? qsTr("命令行") : qsTr("主目录")
    readonly property string profileText: config === null ? ""
        : qsTr("启动: ") + sourceLabel
          + " · " + (config.settingsFound ? "settings.toml" : qsTr("默认设置"))
          + " · " + (config.configDir !== "" ? config.configDir : qsTr("配置目录不可用"))

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: Theme.chrome
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline

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

    function applyBackdrop() {
        chrome.applyMaterial(Appearance.windowMaterial, Appearance.blurEnabled, root.dark)
    }

    onDarkChanged: {
        if (chrome && chrome.attached)
            root.applyBackdrop()
    }

    // A hidden window may not have its native view yet (and macOS needs the
    // window to exist before the effect view can be inserted), so apply once
    // more when the window first becomes visible.
    onVisibleChanged: {
        if (visible && chrome && chrome.attached)
            root.applyBackdrop()
    }

    Connections {
        target: Appearance
        function onBlurEnabledChanged() {
            if (chrome && chrome.attached)
                root.applyBackdrop()
        }
        function onWindowMaterialChanged() {
            if (chrome && chrome.attached)
                root.applyBackdrop()
        }
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
        confirmDialog.open(qsTr("永久删除这些项目？"),
                           ids.length + qsTr(" 个项目将被永久删除，无法恢复。"),
                           qsTr("永久删除"), true)
    }

    /// A network entry has no path through the local recycle bin, so Delete
    /// asks before permanently deleting it instead of calling the trash.
    function requestRemoteDelete(paths) {
        if (!paths || paths.length === 0 || Workspace.busy)
            return
        root.confirmMode = "remoteDelete"
        root.confirmTrashIds = paths
        confirmDialog.open(qsTr("永久删除这些项目？"),
                           paths.length + qsTr(" 个远程项目无法移入回收站，将被永久删除。"),
                           qsTr("永久删除"), true)
    }

    function requestEmptyTrash() {
        if (Workspace.busy)
            return
        root.confirmMode = "empty"
        root.confirmTrashIds = []
        confirmDialog.open(qsTr("清空回收站？"),
                           qsTr("回收站中的 ") + trashModel.count + qsTr(" 个项目将被永久删除，无法恢复。"),
                           qsTr("清空回收站"), true)
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
                toast.show(Spaces.error !== "" ? Spaces.error : qsTr("无法删除空间"), true)
            else
                toast.show(qsTr("空间已删除"), false)
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
        root.searchType = "all"
        if (searchBar)
            searchBar.setText("")
    }

    function openSearch() {
        if (!root.folderActive)
            return
        root.searchOpen = true
        // The field is only focusable once the expanded state has been laid
        // out; focusing a hidden item silently does nothing.
        Qt.callLater(searchBar.focusInput)
    }

    /// Escape, or the icon again: drop the query *and* the field, because
    /// leaving a live query with no field would strand the results on screen.
    function closeSearch() {
        if (root.searchQuery !== "")
            root.acceptSearchText("")
        root.searchOpen = false
        root.focusActiveSurface()
    }

    /// Says what an operation did and, when it went on the undo stack, offers
    /// to take it back. The verb is offered for anything the stack recorded;
    /// the alternative — trusting `Workspace.canUndo` at this instant — reads
    /// the flag before the backend's own event has landed and would show the
    /// button only after a visible pop.
    function announceUndoable(message) {
        toast.show(message, false, true)
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

    /// The filter row's mode chips are the one entry point now: the field
    /// itself only types, so a mode cannot be switched by two different
    /// controls that disagree about which one is lit.
    function setSearchMode(mode) {
        if (mode === "content" && root.remoteLocation) {
            toast.show(qsTr("内容搜索只支持本地目录"), false)
            return
        }
        if (root.searchMode === mode)
            return
        root.searchMode = mode
        if (root.searchActive)
            searchTimer.restart()
    }

    function setSearchType(id) {
        if (root.searchType === id && root.searchFilter === searchFilterBar.typeGlob(id))
            return
        root.searchType = id
        root.searchFilter = searchFilterBar.typeGlob(id)
        if (root.searchActive)
            searchTimer.restart()
    }

    function searchTypeMenuItems() {
        const options = searchFilterBar.typeOptions
        const items = []
        for (let index = 0; index < options.length; index += 1)
            items.push({ id: options[index].id,
                         label: options[index].label,
                         checked: root.searchType === options[index].id })
        return items
    }

    /// The palette's "search inside files" fallback: the same field and the
    /// same bar, only the mode and the query arrive already filled in, so the
    /// user lands where the query is running instead of in a second search UI.
    function startContentSearch(value) {
        if (!root.folderActive || root.remoteLocation || value.length === 0)
            return
        root.openSearch()
        root.searchMode = "content"
        if (searchBar)
            searchBar.setText(value)
        root.acceptSearchText(value)
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
        toast.show(qsTr("远程文件可预览；用本地应用打开前请先复制到本地"), false)
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

    /// A history move is not a new place, so it does not feed the recents list
    /// the way `navigate` does; everything else around it is the same teardown.
    function goBack() {
        if (root.activeTab === null || !root.activeTab.canGoBack)
            return
        root.pathEditing = false
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.goBack()
        root.applyRemotePolicy()
        Qt.callLater(fileList.takeFocus)
    }

    function goForward() {
        if (root.activeTab === null || !root.activeTab.canGoForward)
            return
        root.pathEditing = false
        root.resetSearchState()
        previewPanel.showPaths([])
        root.activeTab.goForward()
        root.applyRemotePolicy()
        Qt.callLater(fileList.takeFocus)
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
        namePrompt.open(folder ? qsTr("新建文件夹") : qsTr("新建文件"),
                        folder ? qsTr("新建文件夹") : qsTr("新建文件.txt"), "")
    }

    function promptCreateIn(path, folder) {
        if (root.modalOpen || path === "")
            return
        root.promptMode = folder ? "folder" : "file"
        root.promptPath = path
        namePrompt.open(folder ? qsTr("新建文件夹") : qsTr("新建文件"),
                        folder ? qsTr("新建文件夹") : qsTr("新建文件.txt"), "")
    }

    function promptRename(path, name) {
        if (root.modalOpen)
            return
        root.promptMode = "rename"
        root.promptPath = path
        namePrompt.open(qsTr("重命名"), name, "")
    }

    function promptSpaceCreate(addPath) {
        if (root.modalOpen)
            return
        root.promptMode = "spaceCreate"
        root.promptPath = addPath !== undefined ? addPath : ""
        namePrompt.open(qsTr("新建空间"), qsTr("新空间"), "")
    }

    function promptSpaceRename(id, name) {
        if (root.modalOpen || !id)
            return
        root.promptMode = "spaceRename"
        root.promptPath = id
        namePrompt.open(qsTr("重命名空间"), name, "")
    }

    /// One entry point for the palette. It closes first, then the surface
    /// switch runs against a clean modal state (the palette itself is part of
    /// `modalOpen`).
    function handlePaletteActivation(kind, value) {
        if (kind === "command") {
            root.runPaletteCommand(value)
            return
        }
        if (kind === "content-search") {
            root.startContentSearch(value)
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
        else if (id === "new-window")
            root.newWindowAt(root.activePath)
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
            if (!root.remoteLocation) {
                const selection = root.currentSelection()
                Workspace.trashEntries(selection)
                root.announceUndoable(selection.length > 1
                    ? qsTr("已将 %1 项移到回收站").arg(selection.length)
                    : qsTr("已移到回收站"))
            }
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


    /// The nav bar's overflow menu: the actions that used to sit as text in the
    /// status strip, plus the visibility toggles the hamburger used to own and
    /// the sort keys that kept the view menu too tall.
    function moreMenuItems() {
        const items = [
            { id: "refresh", label: qsTr("刷新"), shortcut: "F5" },
            { id: "toggleHidden",
              label: qsTr("显示隐藏文件"),
              checked: ExplorerPrefs.showHiddenFiles,
              shortcut: root.shortcutDisplay("explorer.toggleHidden") },
            { id: "toggleSidebar",
              label: qsTr("侧栏"),
              checked: root.sidebarVisible },
            { id: "toggleSplit",
              label: qsTr("分屏"),
              checked: ExplorerPrefs.splitEnabled },
            { id: "sort", label: qsTr("排序"), submenu: root.sortMenuItems() },
            { separator: true },
            { id: "filters", label: qsTr("筛选…") },
            { separator: true },
            { id: "settings", label: qsTr("设置"),
              shortcut: root.shortcutDisplay("app.openSettings") }
        ]
        if (Updater.supported)
            items.push({ id: "checkUpdates",
                         label: Updater.available ? qsTr("更新可用") : qsTr("检查更新") })
        return items
    }

    function sortMenuItems() {
        function key(id, label) {
            return { id: "sort:" + id, label: label,
                     checked: ExplorerPrefs.sortKey === id }
        }
        function order(id, label) {
            return { id: "order:" + id, label: label,
                     checked: ExplorerPrefs.sortOrder === id }
        }
        return [
            key("name", qsTr("名称")),
            key("size", qsTr("大小")),
            key("modified", qsTr("修改时间")),
            key("type", qsTr("类型")),
            { separator: true },
            order("asc", qsTr("升序")),
            order("desc", qsTr("降序")),
            { id: "foldersFirst", label: qsTr("文件夹优先"),
              checked: ExplorerPrefs.foldersFirst }
        ]
    }

    function runMoreAction(id) {
        if (id === "refresh") {
            if (root.activeTab !== null)
                root.activeTab.listing.reload()
        } else if (id === "toggleHidden") {
            ExplorerPrefs.setBool("showHiddenFiles", !ExplorerPrefs.showHiddenFiles)
        } else if (id === "toggleSidebar") {
            root.sidebarVisible = !root.sidebarVisible
        } else if (id === "toggleSplit") {
            ExplorerPrefs.setBool("splitEnabled", !ExplorerPrefs.splitEnabled)
        } else if (id === "foldersFirst") {
            ExplorerPrefs.setBool("foldersFirst", !ExplorerPrefs.foldersFirst)
        } else if (id.indexOf("sort:") === 0) {
            const value = id.substring(5)
            ExplorerPrefs.setString("sortKey", value)
            ExplorerPrefs.setString("sortOrder",
                                    value === "modified" || value === "size" ? "desc" : "asc")
        } else if (id.indexOf("order:") === 0) {
            ExplorerPrefs.setString("sortOrder", id.substring(6))
        } else if (id === "filters") {
            const point = moreSlot.mapToItem(null, 0, 0)
            filterMenu.openAbove(point.x + moreSlot.width - 252, point.y)
        } else if (id === "settings") {
            settingsPanel.open()
        } else if (id === "checkUpdates") {
            updateDialog.open()
        }
    }

    function requestSpaceDelete(id, name) {
        if (root.modalOpen || !id)
            return
        if (Spaces.isPresetOfId(id)) {
            toast.show(qsTr("预设空间不能删除"), true)
            return
        }
        root.confirmMode = "deleteSpace"
        root.confirmTrashIds = [id]
        confirmDialog.open(qsTr("删除空间？"),
                           qsTr("空间 “") + name + qsTr("” 将被删除，其中的项目不会从磁盘移除。"),
                           qsTr("删除"), true)
    }

    function addPathToSpace(spaceId, path) {
        if (!spaceId || !path)
            return
        if (!Spaces.addItem(spaceId, path))
            toast.show(Spaces.error !== "" ? Spaces.error : qsTr("无法添加到空间"), true)
        else
            toast.show(qsTr("已添加到“") + Spaces.nameOfId(spaceId) + "”", false)
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
                toast.show(Spaces.error !== "" ? Spaces.error : qsTr("无法创建空间"), true)
            else {
                if (path !== "")
                    Spaces.addItem(id, path)
                root.openSpace(id)
                toast.show(qsTr("已创建空间"), false)
            }
        } else if (mode === "spaceRename") {
            if (!Spaces.renameSpace(path, text))
                toast.show(Spaces.error !== "" ? Spaces.error : qsTr("无法重命名空间"), true)
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
        entries.push({ id: "newSpaceAdd", label: qsTr("新建空间并添加…") })
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
            { id: "open", label: isDir ? qsTr("打开") : qsTr("用默认应用打开"),
              enabled: hasPath && (!remote || isDir), shortcut: "Enter" },
            { id: "openTab", label: qsTr("在新标签页打开"), enabled: hasPath && isDir },
            { id: "openWindow", label: qsTr("在新窗口打开"), enabled: hasPath && isDir },
            { separator: true },
            { id: "reveal", label: qsTr("在资源管理器中显示"), enabled: hasPath && !remote },
            { id: "openWith", label: qsTr("选择打开方式…"), enabled: hasPath && !remote },
            { id: "terminal", label: qsTr("在终端中打开"), enabled: hasPath && isDir && !remote },
            { separator: true },
            { id: "preview", label: root.previewShows(path) ? qsTr("隐藏预览") : qsTr("预览"),
              enabled: hasPath && !many, shortcut: root.shortcutDisplay("explorer.preview") },
            { id: "copyPath", label: qsTr("复制路径"), enabled: hasPath }
        ]

        const extensions = root.shellCommandSubmenu()
        if (extensions.length > 0) {
            items.push({ separator: true })
            items.push({ id: "shellCommands", label: qsTr("应用扩展"), submenu: extensions })
        }

        items.push({ separator: true })
        items.push({ id: "copy", label: qsTr("复制"),
                     shortcut: root.shortcutDisplay("explorer.copy"), enabled: hasPath })
        items.push({ id: "cut", label: qsTr("剪切"),
                     shortcut: root.shortcutDisplay("explorer.cut"), enabled: hasPath })
        items.push({ id: "paste", label: qsTr("粘贴到此处"),
                     shortcut: root.shortcutDisplay("explorer.paste"),
                     enabled: hasClipboard && isDir })
        items.push({ id: "pasteShortcut", label: qsTr("粘贴快捷方式"),
                     enabled: hasClipboard && isDir && !remote })
        items.push({ id: "duplicate", label: qsTr("创建副本"), enabled: hasPath })
        items.push({ id: "createShortcut", label: qsTr("创建快捷方式"), enabled: hasPath && !remote })
        items.push({ id: "compress", label: qsTr("压缩为…"), enabled: hasPath && !remote, submenu: [
            { id: "compressZip", label: qsTr("ZIP 压缩包") },
            { id: "compressTar", label: qsTr("TAR 归档") },
            { id: "compressTarGz", label: qsTr("TAR.GZ 压缩包") },
            { id: "compress7z", label: qsTr("7Z 压缩包") },
            { id: "compress7zEncrypted", label: qsTr("7Z 压缩包（加密）") }
        ] })
        items.push({ id: "extractHere", label: qsTr("解压到此处"), enabled: canExtract })
        items.push({ id: "moveTo", label: qsTr("移动到…"), enabled: hasPath })
        items.push({ id: many ? "bulkRename" : "rename",
                     label: many ? qsTr("批量重命名…") : qsTr("重命名"),
                     enabled: hasPath,
                     shortcut: many ? "" : root.shortcutDisplay("explorer.rename") })
        items.push({ separator: true })
        items.push({ id: "favorite", label: qsTr("加入收藏"), enabled: hasPath && isDir })
        items.push({ id: "addToSpace", label: qsTr("添加到空间"), enabled: hasPath,
                     submenu: root.spaceSubmenu() })
        items.push({ id: "trash", label: qsTr("移到回收站"),
                     shortcut: root.shortcutDisplay("explorer.trash"),
                     enabled: hasPath && !remote, danger: true })
        items.push({ id: "delete", label: qsTr("永久删除"),
                     shortcut: root.shortcutDisplay("explorer.deletePermanent"),
                     enabled: hasPath, danger: true })
        items.push({ separator: true })
        items.push({ id: "properties", label: qsTr("属性"), shortcut: "Alt+Enter",
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
            { id: "trashRestore", label: qsTr("恢复"), shortcut: "Enter" },
            { id: "trashOpenOriginal", label: qsTr("打开原位置"),
              enabled: originalParent !== "" },
            { separator: true },
            { id: "trashPurge", label: qsTr("永久删除"), shortcut: "Delete", danger: true }
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
            { id: "newFolder", label: qsTr("新建文件夹"), shortcut: root.chord("Mod+Shift+N") },
            { id: "newFile", label: qsTr("新建文件") },
            { separator: true },
            { id: "paste", label: qsTr("粘贴"),
              shortcut: root.shortcutDisplay("explorer.paste"),
              enabled: Workspace.clipboardMode !== "" },
            { id: "pasteShortcut", label: qsTr("粘贴快捷方式"),
              enabled: Workspace.clipboardMode !== "" && !remote },
            { id: "selectAll", label: qsTr("全选"),
              shortcut: root.shortcutDisplay("explorer.selectAll") },
            { id: "refresh", label: qsTr("刷新"), shortcut: "F5" },
            { separator: true },
            { id: "terminal", label: qsTr("在终端中打开"), enabled: !remote }
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
                { id: "open", label: qsTr("打开") },
                { id: "openTab", label: qsTr("在新标签页打开") },
                { id: "openWindow", label: qsTr("在新窗口打开") },
                { separator: true },
                { id: kind === "connection" ? "removeConnection" : "removeCloud",
                  label: kind === "connection" ? qsTr("移除连接") : qsTr("移除云盘账户"),
                  danger: true }
            ])
            return
        }
        if (kind === "space") {
            const preset = Spaces.isPresetOfId(path)
            menu.openAt(x, y, [
                { id: "spaceOpen", label: qsTr("打开空间") },
                { id: "spaceTab", label: qsTr("在新标签页打开") },
                { separator: true },
                { id: "spaceRename", label: qsTr("重命名空间") },
                { id: "spaceDelete", label: qsTr("删除空间"), enabled: !preset, danger: true }
            ])
            return
        }
        const items = []
        if (kind === "favorite")
            items.push({ id: "removeFavorite", label: qsTr("从收藏移除"), danger: true })
        if (kind === "recent" && !root.isRemotePath(path))
            items.push({ id: "reveal", label: qsTr("在资源管理器中显示") })
        items.push({ id: "openTab", label: qsTr("在新标签页打开") })
        items.push({ id: "openWindow", label: qsTr("在新窗口打开") })
        if (kind !== "favorite")
            items.push({ id: "favorite", label: qsTr("加入收藏") })
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
                toast.show(qsTr("已复制路径"), false)
            else
                toast.show(qsTr("无法写入剪贴板"), true)
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
            {
                const selection = root.currentSelection()
                Workspace.trashEntries(selection)
                root.announceUndoable(selection.length > 1
                    ? qsTr("已将 %1 项移到回收站").arg(selection.length)
                    : qsTr("已移到回收站"))
            }
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
        // The sidebar shows the bin's item count too, so the model has to be
        // re-read after a trash operation even while another folder is on
        // screen.
        if (trashKinds.indexOf(kind) >= 0)
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

    function openSystemTerminal(path) {
        if (root.remoteLocation) {
            toast.show(qsTr("终端只能打开本地目录"), false)
            return
        }
        const target = path !== "" ? path : root.activePath
        if (target !== "")
            Workspace.openTerminal(target)
    }

    function toggleTerminal() {
        if (!root.folderActive)
            return
        if (root.remoteLocation) {
            toast.show(qsTr("终端只能打开本地目录"), false)
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
            // The completion message replaces whatever the action announced
            // optimistically, so it has to carry the undo offer too —
            // otherwise the verb appears and then vanishes half a second
            // later, which reads as "too late".
            // A reversed step does not offer to reverse itself again, even
            // when the stack still holds older steps: "Undone — Undo" reads as
            // a question the shell has already answered.
            const reversible = Workspace.canUndo && kind !== "undo" && kind !== "redo"
            if (Workspace.status !== "")
                toast.show(root.localizeBackend(Workspace.status), false, reversible)
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
            toast.show(qsTr("连接已保存"), false)
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
            toast.show(qsTr("云盘账户已连接"), false)
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

    // The frameless window keeps an alpha buffer for the native frame. When a
    // material is active, the canvas must stay transparent or it would paint
    // over the backdrop; every pane and chrome band paints its own colour, so
    // only the seams and chrome tints let the backdrop through. With no
    // material the canvas is the solid fill the regular shell expects.
    Rectangle {
        anchors.fill: parent
        color: root.material === "none" ? Theme.canvas : "transparent"
    }

    Column {
        anchors.fill: parent

        // The chrome bar is one row now: mark, tabs, window buttons. 44 dp is the
        // number `kTitleBarDp` in crates/dae-qt/cpp/window.h hard-codes for the
        // native hit test, so the two move together.
        Item {
            id: chromeBar
            width: parent.width
            height: Theme.shellTitle

            TitleBar {
                id: titleBar
                anchors.fill: parent
                window: root
                dark: root.dark
                material: root.material
                title: root.title
            }

            TabStrip {
                id: tabStrip
                anchors.left: parent.left
                anchors.leftMargin: root.tabLeftInset
                anchors.right: parent.right
                // Three 46 dp buttons; `buttonsWidth` keeps that number in one
                // place rather than repeating it in two files. macOS has no
                // buttons on this side and lets the strip run to the edge.
                anchors.rightMargin: titleBar.buttonsWidth + 8
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                tabs: root.tabs
                activeIndex: root.activeIndex
                hostWindow: root
                accent: root.accent

                // Native hit test input. Without this the chips sit inside the
                // caption and a click on a tab starts a window drag instead of
                // activating it. `leftMargin` is the band's origin in window
                // coordinates; the strip's own numbers are strip-local. On
                // macOS the band starts after the traffic lights, so it must
                // use the same inset as the anchors above.
                function pushCaptionBand() {
                    if (chrome.attached)
                        chrome.setCaptionClient(root.tabLeftInset + clientLeft,
                                                root.tabLeftInset + clientRight)
                }
                onClientRightChanged: pushCaptionBand()

                onTabActivated: function (index) {
                    root.activeIndex = index
                }
                onTabClosed: function (index) {
                    root.closeTab(index)
                }
                onNewTabRequested: root.addTabHere()
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
        }

        Rectangle {
            id: pathBar
            width: parent.width
            height: Theme.shellNav
            color: root.chromeColor

            // Back and forward keep their place when the stack is empty: a
            // button that disappears moves every control to its right, and the
            // one after it is the address bar, which is the wrong thing to
            // move under the pointer.
            Rectangle {
                id: backButton
                anchors.left: parent.left
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28
                radius: Theme.radiusSm
                color: root.canGoBack && backMouse.containsMouse
                    ? Theme.fill : "transparent"
                opacity: root.canGoBack ? 1 : 0.32

                readonly property bool canGo: root.canGoBack

                Text {
                    anchors.centerIn: parent
                    text: "←"
                    color: root.mutedColor
                    font.pixelSize: Theme.typeName
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("后退")

                MouseArea {
                    id: backMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: backButton.canGo
                    onClicked: root.goBack()
                }
            }

            Rectangle {
                id: forwardButton
                anchors.left: backButton.right
                anchors.leftMargin: 2
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28
                radius: Theme.radiusSm
                color: root.canGoForward && forwardMouse.containsMouse
                    ? Theme.fill : "transparent"
                opacity: root.canGoForward ? 1 : 0.32

                readonly property bool canGo: root.canGoForward

                Text {
                    anchors.centerIn: parent
                    text: "→"
                    color: root.mutedColor
                    font.pixelSize: Theme.typeName
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("前进")

                MouseArea {
                    id: forwardMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: forwardButton.canGo
                    onClicked: root.goForward()
                }
            }

            Rectangle {
                id: upButton
                anchors.left: forwardButton.right
                anchors.leftMargin: 2
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28
                radius: Theme.radiusSm
                color: root.folderActive && upMouse.containsMouse
                    ? Theme.fill : "transparent"
                opacity: root.folderActive ? 1 : 0.32

                Text {
                    anchors.centerIn: parent
                    text: "\u2191"
                    color: root.mutedColor
                    font.pixelSize: Theme.typeName
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("上一级")

                MouseArea {
                    id: upMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: root.folderActive
                    onClicked: root.goUp()
                }
            }

            // Separates the fixed navigation from the path itself, the way the
            // instrument's control cluster is set apart from its readout.
            Rectangle {
                id: navSeparator
                anchors.left: upButton.right
                anchors.leftMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                width: 1
                height: 16
                color: Theme.hairline
            }

            Item {
                id: pathField
                anchors.left: navSeparator.right
                anchors.leftMargin: 6
                anchors.right: searchSlot.left
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                height: 28

                // The trail is a readout, not a field: it only looks like one
                // while it is being typed into. The plate is the editing state.
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: root.pathEditing ? Theme.field : "transparent"
                    border.width: root.pathEditing ? 1 : 0
                    border.color: Theme.signal
                }

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
                    anchors.leftMargin: 2
                    anchors.rightMargin: 2
                    fontSize: Theme.typeBody
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
                    font.pixelSize: Theme.typeName
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
                    selectedTextColor: Theme.onInteractive
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeSmall
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

            // ── Search ────────────────────────────────────────────────────
            // A 224 px field parked here for the whole session takes a quarter
            // of the bar from the breadcrumb, to serve an action used a few
            // times an hour. It is an icon until asked for, and it stays out
            // while a query is live so the field and its results never part.
            Item {
                id: searchSlot

                readonly property bool expanded: root.searchOpen
                    || root.searchQuery.length > 0

                anchors.right: viewSlot.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                height: 28
                width: expanded ? 210 : 28
                visible: root.folderActive

                Behavior on width {
                    enabled: Theme.motion
                    NumberAnimation {
                        duration: Theme.motionPanel
                        easing.type: Easing.OutCubic
                    }
                }

                Rectangle {
                    anchors.fill: parent
                    visible: !searchSlot.expanded
                    radius: Theme.radiusSm
                    color: searchIconMouse.containsMouse ? Theme.fill : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        kind: "search"
                        tint: searchIconMouse.containsMouse
                            ? root.textColor : root.mutedColor
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("搜索")

                    MouseArea {
                        id: searchIconMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.openSearch()
                    }
                }

                SearchBar {
                    id: searchBar
                    anchors.fill: parent
                    visible: searchSlot.expanded
                    mode: root.searchMode
                    busy: root.searchBusy
                    dark: root.dark
                    accent: root.accent
                    textColor: root.textColor
                    mutedColor: root.mutedColor
                    hairline: root.hairline

                    // The field owns the focus as soon as it exists. Asking for
                    // it from `openSearch` lands in the same frame the field is
                    // being laid out, and the listing can still take it back.
                    onVisibleChanged: {
                        if (visible && root.searchOpen)
                            focusInput()
                    }
                    onQueryEdited: function (value) {
                        root.acceptSearchText(value)
                    }
                    onClearRequested: root.acceptSearchText("")
                    onDismissRequested: root.closeSearch()
                }
            }

            // ── Trash ─────────────────────────────────────────────────────
            // Emptying the bin is a text action that only exists here, and only
            // while there is something to empty. The hover is the only place it
            // turns dangerous.
            Rectangle {
                id: trashClear
                anchors.right: searchSlot.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                width: visible ? 44 : 0
                height: 24
                radius: Theme.radiusXs
                visible: root.trashActive && trashModel.count > 0
                color: trashClearMouse.containsMouse ? Theme.hover : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: qsTr("清空")
                    color: trashClearMouse.containsMouse ? Theme.danger : root.mutedColor
                    font.pixelSize: Theme.typeCaption
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("清空回收站")

                MouseArea {
                    id: trashClearMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.requestEmptyTrash()
                }
            }

            // ── View ──────────────────────────────────────────────────────
            // One button showing the view that is on, opening the one display
            // menu.
            Item {
                id: viewSlot
                anchors.right: moreSlot.left
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28
                visible: root.folderActive

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: viewMouse.containsMouse || displayMenu.visible
                        ? Theme.fill : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        kind: ExplorerPrefs.viewMode === "grid" ? "grid"
                            : ExplorerPrefs.viewMode === "column" ? "column" : "list"
                        tint: viewMouse.containsMouse || displayMenu.visible
                            ? root.textColor : root.mutedColor
                    }
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("切换视图")

                MouseArea {
                    id: viewMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        const point = viewSlot.mapToItem(null, 0, 0)
                        displayMenu.openAbove(point.x + viewSlot.width - 252, point.y)
                    }
                }
            }

            // ── More ──────────────────────────────────────────────────────
            // Refresh, the visibility toggles, filters and settings live here
            // now that the sidebar gives up its hamburger and the status bar
            // gives up its settings text.
            Item {
                id: moreSlot
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: moreMouse.containsMouse || moreMenu.visible
                        ? Theme.fill : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        kind: "more"
                        tint: moreMouse.containsMouse || moreMenu.visible
                            ? root.textColor : root.mutedColor
                    }
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("更多")

                MouseArea {
                    id: moreMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        const point = moreSlot.mapToItem(null, 0, 0)
                        moreMenu.openAt(point.x + moreSlot.width - 226, point.y,
                                        root.moreMenuItems())
                    }
                }
            }
        }

        // ── Search filters ────────────────────────────────────────────
        // One bar for both modes, and only while the query is live. It sits
        // on the canvas rather than the chrome because it belongs to the
        // field that opened it, not to the window's frame.
        SearchFilterBar {
            id: searchFilterBar

            width: parent.width
            height: visible ? 32 : 0
            visible: root.folderActive
                && (root.searchOpen || root.searchQuery.length > 0)

            mode: root.searchMode
            contentEnabled: !root.remoteLocation
            caseSensitive: root.searchCase
            regex: root.searchRegex
            typeId: root.searchType
            busy: root.searchBusy
            matchCount: root.searchMatchCount
            query: root.searchQuery.trim()

            onModeRequested: function (mode) { root.setSearchMode(mode) }
            onCaseToggled: {
                root.searchCase = !root.searchCase
                root.scheduleSearch()
            }
            onRegexToggled: {
                root.searchRegex = !root.searchRegex
                root.scheduleSearch()
            }
            onTypeMenuRequested: function (x, y) {
                searchTypeMenu.openAt(x, y, root.searchTypeMenuItems())
            }
        }

        Row {
            id: body
            width: parent.width
            height: parent.height - Theme.shellTitle - Theme.shellNav
                - Theme.shellStatus - searchFilterBar.height

            Item {
                id: sidebarHandle

                readonly property bool live: root.sidebarVisible

                width: live ? 6 : 0
                height: parent.height

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 1
                    height: parent.height
                    color: sidebarHandleMouse.containsMouse ? root.accent : Theme.hairline
                }

                MouseArea {
                    id: sidebarHandleMouse
                    anchors.fill: parent
                    enabled: sidebarHandle.live
                    hoverEnabled: true
                    cursorShape: Qt.SizeHorCursor

                    property real startX: 0
                    property int startWidth: 0

                    onPressed: function (mouse) {
                        startX = mapToItem(null, mouse.x, 0).x
                        startWidth = ExplorerPrefs.sidebarWidth
                    }
                    onPositionChanged: function (mouse) {
                        if (!pressed)
                            return
                        const dx = mapToItem(null, mouse.x, 0).x - startX
                        ExplorerPrefs.setNumber("sidebarWidth", Math.round(startWidth + dx))
                    }
                }
            }

            Sidebar {
                id: sidebar
                width: root.sidebarVisible ? ExplorerPrefs.sidebarWidth : 0
                height: parent.height
                visible: width > 0
                dark: root.dark
                material: root.material
                activePath: root.activePath
                trashActive: root.trashActive
                trashCount: trashModel.count
                spaceActive: root.spaceActive
                activeSpaceId: root.activeSpaceId
                onNavigateRequested: function (path) {
                    root.navigate(path)
                }
                onTrashRequested: root.openTrash()
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
                width: parent.width - sidebar.width - sidebarHandle.width
                    - previewPanel.width - previewHandle.width
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
                        shortcutMap: root.shortcutMap

                        onNavigateRequested: function (path) {
                            root.navigate(path)
                        }
                        onUpRequested: root.goUp()
                        onRenameRequested: function (path, name) {
                            root.promptRename(path, name)
                        }
                        onCreateFolderRequested: root.promptCreate(true)
                        onClearSearchRequested: root.closeSearch()
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
                        onTrashed: function (count) {
                            root.announceUndoable(count > 1
                                ? qsTr("已将 %1 项移到回收站").arg(count)
                                : qsTr("已移到回收站"))
                        }
                        onRemoteOpenRequested: function (path) {
                            root.acceptRemotePreview(path)
                        }
                        onOpenTerminalRequested: function (path) {
                            root.openSystemTerminal(path)
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
                            shortcutMap: root.shortcutMap

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
                            onOpenTerminalRequested: function (path) {
                                root.openSystemTerminal(path)
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
                        onPreviewRequested: function (item) {
                            previewPanel.showTrash(item)
                        }
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
                        commandShortcut: root.shortcutDisplay("app.commandBar")

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
                    tab: root.activeTab
                    dark: root.dark
                    material: root.material
                    onExited: {
                        fileList.takeFocus()
                    }
                }
            }

            // A one-pixel seam that is six pixels wide to the pointer. The
            // preview is the one pane whose useful width depends on what is in
            // it — a code file wants more, a thumbnail less — so the number is
            // the user's, remembered in explorer.ini. 260–420 is the range the
            // preference clamps to; a double click restores the default.
            Item {
                id: previewHandle

                readonly property bool live: root.previewVisible
                    && (root.folderActive || root.trashActive)

                width: live ? 6 : 0
                height: parent.height

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 1
                    height: parent.height
                    color: handleMouse.containsMouse ? Theme.signal : Theme.hairline
                }

                MouseArea {
                    id: handleMouse
                    anchors.fill: parent
                    enabled: previewHandle.live
                    hoverEnabled: true
                    cursorShape: Qt.SizeHorCursor

                    property real startX: 0
                    property int startWidth: 0

                    onPressed: function (mouse) {
                        startX = mapToItem(null, mouse.x, 0).x
                        startWidth = ExplorerPrefs.previewWidth
                    }
                    onPositionChanged: function (mouse) {
                        if (!pressed)
                            return
                        const dx = mapToItem(null, mouse.x, 0).x - startX
                        ExplorerPrefs.setNumber("previewWidth", Math.round(startWidth - dx))
                    }
                    onDoubleClicked: ExplorerPrefs.setNumber("previewWidth", 300)
                }
            }

            PreviewPanel {
                id: previewPanel
                width: root.previewVisible && (root.folderActive || root.trashActive)
                    ? Math.min(ExplorerPrefs.previewWidth, Math.max(260, parent.width * 0.5))
                    : 0
                height: parent.height
                visible: width > 0
                dark: root.dark
                accent: root.accent
                inTrash: root.trashActive

                onCloseRequested: {
                    root.previewVisible = false
                    if (root.trashActive)
                        trashView.takeFocus()
                    else
                        fileList.takeFocus()
                }
                onOpenRequested: function (path, isDir) {
                    root.acceptPreview(path, isDir)
                }
                onPropertiesRequested: function (path) {
                    propertiesDialog.open(path)
                }
            }
        }

        Rectangle {
            id: statusBar
            width: parent.width
            height: Theme.shellStatus
            color: root.chromeColor

            // A hairline instead of a colour step: the strip is the same surface
            // as the path bar above, and two shades of chrome would read as a
            // third region rather than as the bottom of the window.
            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: Theme.hairline
            }

            // ── Left: what the listing holds ──────────────────────────────
            Text {
                id: statusText
                x: 10
                width: Math.min(implicitWidth, parent.width * 0.42)
                anchors.verticalCenter: parent.verticalCenter
                text: {
                    if (root.activeTab === null)
                        return ""
                    if (root.overviewActive)
                        return overviewModel.projectBusy
                            ? qsTr("查找项目…")
                            : (overviewModel.loaded
                                ? overviewModel.count + qsTr(" 个入口")
                                : qsTr("读取概览…"))
                    if (root.spaceActive)
                        return qsTr("空间 · ") + spaceView.itemCount + qsTr(" 项")
                    if (root.trashActive) {
                        const bin = trashModel.loaded
                            ? root.localizeBackend(trashModel.status)
                            : qsTr("读取回收站…")
                        if (trashView.selectionCount > 0)
                            return bin + " · " + qsTr("已选 %1 项").arg(trashView.selectionCount)
                        return bin
                    }
                    if (root.searchActive) {
                        const search = root.searchMode === "content"
                            ? root.activeTab.contentSearch : root.activeTab.nameSearch
                        if (search.busy)
                            return root.searchMode === "content" ? qsTr("搜索文件内容…") : qsTr("搜索中…")
                        return search.status !== ""
                            ? root.localizeBackend(search.status) : qsTr("没有匹配项")
                    }
                    const listing = root.activeTab.listing
                    if (Workspace.busy && Workspace.progressLabel !== "")
                        return root.localizeBackend(Workspace.progressLabel)
                    const base = listing.finished
                        ? root.localizeBackend(listing.status)
                        : (listing.loaded + qsTr(" 项 · 读取中…"))
                    if (fileList.selectionCount > 0)
                        return base + " · " + qsTr("已选 %1 项").arg(fileList.selectionCount)
                    return base
                }
                color: Theme.inkMuted
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideRight
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }

            // ── Centre: the running transfer ──────────────────────────────
            // It used to be a "queue" chip bolted after the status text. The
            // centre is where the eye looks for progress, and the chip is the
            // queue's only door: without it a second paste looks like it did
            // nothing until the first copy finished.
            Rectangle {
                id: transferChip
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                width: transferRow.implicitWidth + 14
                height: 22
                radius: Theme.radiusXs
                visible: transferChip.count > 0
                color: transferMouse.containsMouse ? Theme.raised : Theme.canvas
                border.width: 1
                border.color: Theme.hairline

                readonly property int count: Workspace.queuedTransfers.length
                    + (Workspace.busy ? 1 : 0)
                readonly property real percent: Workspace.progressTotal > 0
                    ? Math.round(Workspace.progressDone * 100 / Workspace.progressTotal)
                    : (Workspace.busy ? 0 : 100)

                function openPanel() {
                    const point = transferChip.mapToItem(null, 0, 0)
                    transferQueue.openAbove(point.x, point.y)
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("%1 项传输，%2%").arg(count).arg(percent)
                    + qsTr("，查看传输队列")
                Accessible.onPressAction: transferChip.openPanel()

                Row {
                    id: transferRow
                    anchors.centerIn: parent
                    spacing: 6

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("%1 项传输").arg(transferChip.count)
                        color: Theme.inkMuted
                        font.pixelSize: Theme.typeMicro
                    }

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 32
                        height: 2
                        color: Theme.hairlineStrong

                        Rectangle {
                            width: parent.width * transferChip.percent / 100
                            height: parent.height
                            color: Theme.signal
                        }
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: transferChip.percent + "%"
                        color: Theme.inkMuted
                        font.pixelSize: Theme.typeMicro
                        font.family: Theme.monoFamily
                    }
                }

                MouseArea {
                    id: transferMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: transferChip.openPanel()
                }
            }

            // ── Right: repository and pane switches ───────────────────────
            Row {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                // The branch is a name, not a pill: the colour in this bar
                // belongs to the signal and to nothing else.
                Item {
                    id: gitStatus
                    width: gitStatusRow.implicitWidth + 8
                    height: 22
                    visible: root.gitVisible

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("分支 %1，打开分支面板")
                        .arg(root.gitListing !== null ? root.gitListing.gitBranch : "")
                    Accessible.onPressAction: gitStatus.openPanel()

                    function openPanel() {
                        const point = gitStatus.mapToItem(null, 0, 0)
                        gitPanel.openAbove(point.x, point.y)
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: gitStatusMouse.containsMouse || gitPanel.visible
                            ? Theme.hover : "transparent"
                    }

                    Row {
                        id: gitStatusRow
                        anchors.centerIn: parent
                        spacing: 4

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            kind: "branch"
                            implicitWidth: 12
                            implicitHeight: 12
                            tint: Theme.inkFaint
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: Math.min(implicitWidth, 120)
                            text: root.gitListing !== null ? root.gitListing.gitBranch : ""
                            color: root.gitListing !== null && root.gitListing.gitDetached
                                ? Theme.inkFaint : Theme.inkMuted
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            textFormat: Text.PlainText
                            elide: Text.ElideMiddle
                        }
                    }

                    MouseArea {
                        id: gitStatusMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: gitStatus.openPanel()
                    }
                }

                Rectangle {
                    width: 1
                    height: 12
                    anchors.verticalCenter: parent.verticalCenter
                    color: Theme.hairline
                    visible: gitStatus.visible
                }

                Item {
                    id: previewToggle
                    width: previewToggleRow.implicitWidth + 12
                    height: 22
                    visible: root.folderActive || root.trashActive

                    Accessible.role: Accessible.Button
                    Accessible.name: root.previewVisible ? qsTr("隐藏预览") : qsTr("显示预览")

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: previewToggleMouse.containsMouse
                            ? Theme.hover : "transparent"
                    }

                    Row {
                        id: previewToggleRow
                        anchors.centerIn: parent
                        spacing: 4

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            kind: "panel"
                            implicitWidth: 14
                            implicitHeight: 14
                            tint: root.previewVisible ? Theme.inkMuted : Theme.inkFaint
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("预览")
                            color: root.previewVisible ? Theme.ink : Theme.inkFaint
                            font.pixelSize: Theme.typeMicro
                        }
                    }

                    MouseArea {
                        id: previewToggleMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            root.previewVisible = !root.previewVisible
                            if (root.previewVisible)
                                return
                            if (root.trashActive)
                                trashView.takeFocus()
                            else
                                fileList.takeFocus()
                        }
                    }
                }

                Item {
                    id: terminalToggle
                    width: terminalToggleRow.implicitWidth + 12
                    height: 22
                    visible: root.folderActive
                    opacity: root.remoteLocation ? 0.35 : 1

                    Accessible.role: Accessible.Button
                    Accessible.name: root.terminalActive ? qsTr("隐藏终端") : qsTr("显示终端")

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: terminalToggleMouse.containsMouse
                            ? Theme.hover : "transparent"
                    }

                    Row {
                        id: terminalToggleRow
                        anchors.centerIn: parent
                        spacing: 4

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            kind: "terminal"
                            implicitWidth: 14
                            implicitHeight: 14
                            tint: root.terminalActive ? Theme.inkMuted : Theme.inkFaint
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("终端")
                            color: root.terminalActive ? Theme.ink : Theme.inkFaint
                            font.pixelSize: Theme.typeMicro
                        }
                    }

                    MouseArea {
                        id: terminalToggleMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !root.remoteLocation
                        onClicked: root.toggleTerminal()
                    }
                }
            }
        }
    }

    // The native resize border Windows gets from its hit test. Invisible there,
    // and only the six-pixel frame is interactive elsewhere.
    ResizeHandles {
        anchors.fill: parent
        window: root
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
        sidebarVisible: root.sidebarVisible
        previewVisible: root.previewVisible
        terminalVisible: root.terminalActive

        onToggleRequested: function (property) {
            if (property === "sidebarVisible") {
                root.sidebarVisible = !root.sidebarVisible
            } else if (property === "previewVisible") {
                root.previewVisible = !root.previewVisible
                if (!root.previewVisible)
                    fileList.takeFocus()
            } else if (property === "terminalVisible") {
                root.toggleTerminal()
            }
        }

        onActionRequested: function (id) {
            if (id === "overview")
                root.openOverview()
        }
    }

    FilterMenu {
        id: filterMenu
        anchors.fill: parent
        dark: root.dark
        accent: root.accent
        hiddenShortcut: root.shortcutDisplay("explorer.toggleHidden")
    }

    ContextMenu {
        id: moreMenu
        anchors.fill: parent
        dark: root.dark

        onActionTriggered: function (id) {
            root.runMoreAction(id)
        }
    }

    /// The filter row's type categories. It is its own menu rather than a row
    /// of chips because the catalogue is six entries long and the bar has to
    /// stay one row; the entries are the categories the backend already
    /// filters by, not a second taxonomy.
    ContextMenu {
        id: searchTypeMenu
        anchors.fill: parent
        dark: root.dark

        onActionTriggered: function (id) {
            root.setSearchType(id)
        }
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

    // History comes before the pane switch: it is the gesture a file manager
    // is used most, and it has to work from the listing, the split pane and the
    // path bar alike, so it is a window-level Shortcut rather than a Key
    // handler inside one of them.
    Shortcut {
        sequence: root.shortcutSequence("explorer.goBack")
        enabled: sequence !== "" && root.canGoBack
        onActivated: root.goBack()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.goForward")
        enabled: sequence !== "" && root.canGoForward
        onActivated: root.goForward()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.switchPane")
        enabled: root.splitActive && sequence !== ""
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
            const point = viewSlot.mapToItem(null, 0, 0)
            displayMenu.openAbove(point.x + viewSlot.width - 252, point.y)
        }
    }

    Shortcut {
        sequence: "Ctrl+Alt+F"
        onActivated: {
            // The filter menu lost its status-bar button; it now opens under
            // the nav bar's overflow button, where the rest of the listing
            // switches live.
            const point = moreSlot.mapToItem(null, 0, 0)
            filterMenu.openAbove(point.x + moreSlot.width - 252, point.y)
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
        accent: root.accent
        isMac: root.isMac
        shortcutMap: root.shortcutMap
        blurSupported: chrome.blurSupported

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
        overviewModel: overviewModel
        folderActive: root.folderActive
        scopePath: root.folderActive ? root.activePath : Workspace.homeDirectory
        shortcutMap: root.shortcutMap
        isMac: root.isMac

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
                toast.show(qsTr("发现新版本 v") + Updater.latestVersion, false)
        }
    }

    Toast {
        id: toast
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 38
        dark: root.dark

        onUndoRequested: Workspace.undo()
    }

    Shortcut {
        sequence: root.mod + "+T"
        enabled: !root.modalOpen
        onActivated: root.addTabHere()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.commandBar")
        enabled: !root.modalOpen && sequence !== ""
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
        sequence: root.shortcutSequence("app.pathJump")
        enabled: !root.modalOpen && root.folderActive && sequence !== ""
        onActivated: root.beginPathEditing()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.toggleTerminal")
        enabled: !root.modalOpen && root.folderActive && !root.remoteLocation
            && sequence !== ""
        onActivated: root.toggleTerminal()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.focusSearch")
        enabled: !root.modalOpen && root.folderActive && sequence !== ""
        onActivated: root.openSearch()
    }

    Shortcut {
        sequence: "Ctrl+B"
        enabled: !root.modalOpen
        onActivated: root.sidebarVisible = !root.sidebarVisible
    }

    Shortcut {
        sequence: root.shortcutSequence("app.openSettings")
        enabled: !root.modalOpen && sequence !== ""
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
        enabled: root.modalOpen && !settingsPanel.capturing
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
        // The tab band is a client area of the caption. It can only be declared
        // once the window exists natively, so this runs after `attach` rather
        // than from the strip's own completion.
        tabStrip.pushCaptionBand()

        if (root.firstWindow) {
            Workspace.startSystemWatch()
            // One process-wide warm-up, so the first right-click already has
            // the installed apps' verbs instead of waiting on a COM surrogate.
            Workspace.warmShellCommands()
            if (root.config !== null)
                root.startupPath = root.config.startupPath
        }
        root.applyBackdrop()

        // The sidebar shows the bin's count, so the scan has to run before the
        // trash surface is ever opened.
        trashModel.refresh()

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
