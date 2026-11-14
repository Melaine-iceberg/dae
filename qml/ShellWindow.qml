pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import dae.activation 1.0
import "Localize.js" as Localize
import "Shell.js" as ShellState
import "Shortcut.js" as ShortcutJS

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

    /// Right-to-left locales mirror the whole shell: Qt resolves the direction
    /// from the translator, and children inherit because QML does not mirror by
    /// default. Glyphs that encode a direction (chevrons, the back arrow) stay
    /// as drawn; their semantics follow the mirrored anchors.
    // qmllint cannot see `layoutDirection` on its `Qt.application` stub; it is
    // a real QGuiApplication property.
    // qmllint disable missing-property
    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    // qmllint enable missing-property
    LayoutMirroring.childrenInherit: true

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
    /// Tabs closed in this window, oldest first. Reopening pops the newest;
    /// the cap keeps a session's undo from growing without bound.
    property var closedTabs: []
    readonly property int closedTabLimit: 16
    /// Toolbar action ids in display order. The defaults match the components
    /// declared in the path bar; `syncToolbarOrder` reconciles a stored value.
    property var toolbarLeadingItems: ["back", "forward", "up"]
    property var toolbarTrailingItems: ["search", "trash", "view", "more"]
    /// The tab the open tab menu belongs to.
    property int tabMenuIndex: -1
    property int activeIndex: 0
    property bool closingDisposed: false
    /// Set after the first window has built its starting tab. An activation
    /// that arrives before this stays in the native pending list.
    property bool activationReady: false
    property bool terminalActive: false
    /// The terminal panel is built the first time the terminal is opened;
    /// nothing in the window needs it before that. This is not a startup win
    /// by itself (docs/qml-aot.md §14): the one-time font work its first
    /// `TerminalItem` used to pay simply moved to the first other text item.
    property bool terminalLoaded: false
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
    /// Where a name search looks: `current`, `favorites`, `recent` or
    /// `volumes`. A tag click switches this to `volumes` for its one query.
    property string searchScope: "current"
    /// Name search reuses the listing's buckets: one filter vocabulary, one
    /// menu, one place to explain what "images" means.
    readonly property string searchKind: ExplorerPrefs.kindFilter
    readonly property string searchModified: ExplorerPrefs.modifiedFilter
    readonly property string searchSize: ExplorerPrefs.sizeFilter
    readonly property var searchHistoryEntries: {
        try {
            return JSON.parse(ExplorerPrefs.searchHistory)
        } catch (error) {
            return []
        }
    }
    /// Named searches, as the preference singleton stores them. The sidebar
    /// draws the rows; this is what a click resolves an id against.
    readonly property var savedSearches: {
        try {
            const entries = JSON.parse(ExplorerPrefs.savedSearches)
            return Array.isArray(entries) ? entries : []
        } catch (error) {
            return []
        }
    }
    property string promptMode: ""
    property string promptPath: ""
    property string contextPath: ""
    /// The current folder's display name, captured with the bar menu so
    /// "copy name" copies what the crumb showed, not a re-derived basename.
    property string contextName: ""
    property bool contextIsDir: false
    /// Which menu is open: `row`, `empty`, `trash`, `sidebar`, `breadcrumb`
    /// (the folded ancestors) or `breadcrumbBar` (a right-click on the trail).
    /// The shell commands answer asynchronously, and only the row menu is
    /// rebuilt when they arrive.
    property string contextKind: ""
    /// The row menu's second page is open (see `rowMoreMenuItems`). The page is
    /// part of the menu rather than a flyout, so the installed apps' commands
    /// and the archive/space submenus keep working one level deep.
    property bool rowMenuMore: false
    property string contextTrashId: ""
    property string contextTrashParent: ""
    property string confirmMode: ""
    property var confirmTrashIds: []
    property string archiveMode: ""
    property string archiveTarget: ""
    property string archiveDestination: ""
    property string archiveFormat: ""
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
    /// (terminal, recycle bin, name/content search, OS open/reveal) are gated
    /// on this instead of failing after the click.
    readonly property bool remoteLocation: folderActive
        && (activeTab.listing.remote || root.isRemotePath(activeTab.listing.path))
    readonly property bool searchActive: folderActive && searchQuery.trim().length > 0
    readonly property bool contentSearchActive: searchActive && searchMode === "content"
    readonly property bool splitActive: ExplorerPrefs.splitEnabled
        && folderActive && !searchActive
    /// The listing the footer's action bar acts on: the focused pane when
    /// split view is on, the primary one otherwise — the same rule a
    /// window-level shortcut follows.
    readonly property var actionList: (splitActive && splitFocused) ? splitList : fileList
    readonly property int actionCount: actionList !== null ? actionList.selectionCount : 0
    /// A selection turns the 28 dp status strip into the 36 dp action bar in
    /// place (proposal §7.3.3). The eight extra dp come out of the listing
    /// above, not out of a floating layer, so the footer never covers a row.
    readonly property bool actionActive: folderActive && actionCount > 0
    readonly property int footerHeight: actionActive ? Theme.actionBar : Theme.shellStatus
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
    /// System accent follows the OS; the brand accent is the app's own signal
    /// orange, one shade per theme. `accent` is the only colour views consume,
    /// so switching the preference retints the whole shell.
    readonly property color accent: Appearance.accent === "brand"
        ? Theme.accent
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
        return ShortcutJS.display(root.shortcutBinding(id), root.isMac)
    }
    /// A chord the shell fixes itself, with no row in `settings.toml`. It
    /// prints through the same formatter, so `Mod+Shift+N` reads `⇧⌘N` on
    /// macOS instead of leaking the internal `Meta` spelling into a menu.
    function chord(binding) {
        return ShortcutJS.display(binding, root.isMac)
    }
    function shortcutSequence(id) {
        return ShortcutJS.qtSequence(root.shortcutBinding(id), root.isMac)
    }
    function shortcutMatches(event, id) {
        return ShortcutJS.matches(event, root.shortcutBinding(id))
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
        case "Copying as administrator…": return qsTr("以管理员身份复制中…")
        case "Moving as administrator…": return qsTr("以管理员身份移动中…")
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
        case "Transfer cancelled": return qsTr("传输已取消")
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
        default: return Localize.message(text)
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

    readonly property color chromeTint: Theme.backdropTint(material, dark, false)
    readonly property color solidChrome: Theme.chrome
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color hairline: Theme.hairline

    Loader {
        id: configLoader
        active: root.firstWindow
        sourceComponent: Component {
            AppConfig {}
        }
    }

    readonly property var config: configLoader.item
    readonly property bool modalOpen: menu.visible || namePrompt.shown
        || commandPalette.shown
        || passwordDialog.shown || settingsPanel.shown || confirmDialog.shown
        || connectDialog.shown || cloudDialog.shown || openWithDialog.shown
        || updateDialog.shown || transferConflict.visible || transferQueue.visible
        || statusCenter.visible || quickPreview.open
        || propertiesDialog.shown || bulkRenameDialog.shown || moveToDialog.shown

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
            overviewModelInstance.refresh()
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
        id: overviewModelInstance
    }

    /// The landing page reads the hidden-section list; the model drops names
    /// it does not know, so a stale preference cannot blank the page.
    Connections {
        target: ExplorerPrefs

        function onHiddenHomeSectionsChanged() {
            overviewModelInstance.setHiddenSections(ExplorerPrefs.hiddenHomeSections)
        }
        function onToolbarLeadingChanged() {
            root.syncToolbarOrder()
        Qt.callLater(function () {
            const first = leadingToolbarRepeater.itemAt(0)
            console.log("dae-after", leadingToolbar.width, trailingToolbar.width,
                        first ? first.item !== null : false,
                        first ? first.width : -1)
        })
        }
        function onToolbarTrailingChanged() {
            root.syncToolbarOrder()
        }
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

    /// The part of a tab a session record carries. Selections are not stored:
    /// a path in a history entry may not exist any more, and a restored
    /// selection would point at rows the new listing does not contain.
    function tabState(tab) {
        if (tab === null)
            return null
        const back = []
        for (let index = 0; index < tab.backStack.length; index += 1)
            back.push(tab.backStack[index].path)
        const forward = []
        for (let index = 0; index < tab.forwardStack.length; index += 1)
            forward.push(tab.forwardStack[index].path)
        return {
            "surface": tab.surface,
            "spaceId": tab.spaceId,
            "path": tab.listing.path,
            "pinned": tab.pinned,
            "back": back,
            "forward": forward
        }
    }

    function historyEntryFor(path) {
        return { "path": path, "selected": [], "anchor": "" }
    }

    function restoreTabState(tab, state) {
        if (tab === null || !state)
            return
        tab.pinned = state.pinned === true
        if (state.surface === "trash") {
            tab.openTrash()
        } else if (state.surface === "overview") {
            tab.openOverview()
        } else if (state.surface === "space" && state.spaceId) {
            tab.openSpace(state.spaceId)
        } else if (state.path) {
            tab.loadFolder(state.path, false)
        }
        const back = []
        const storedBack = state.back ? state.back : []
        for (let index = 0; index < storedBack.length; index += 1)
            back.push(root.historyEntryFor(storedBack[index]))
        tab.backStack = back
        const forward = []
        const storedForward = state.forward ? state.forward : []
        for (let index = 0; index < storedForward.length; index += 1)
            forward.push(root.historyEntryFor(storedForward[index]))
        tab.forwardStack = forward
    }

    function rememberClosed(tab) {
        if (tab === null)
            return
        const entry = root.tabState(tab)
        if (entry === null)
            return
        const next = root.closedTabs.slice()
        next.push(entry)
        while (next.length > root.closedTabLimit)
            next.shift()
        root.closedTabs = next
    }

    function reopenClosedTab() {
        if (root.closedTabs.length === 0)
            return
        const entry = root.closedTabs[root.closedTabs.length - 1]
        root.closedTabs = root.closedTabs.slice(0, root.closedTabs.length - 1)
        const tab = root.createTab("")
        if (tab === null)
            return
        root.restoreTabState(tab, entry)
        root.insertTab(tab)
    }

    function duplicateTab(index) {
        if (index < 0 || index >= root.tabs.length)
            return
        const tab = root.createTab("")
        if (tab === null)
            return
        root.restoreTabState(tab, root.tabState(root.tabs[index]))
        const next = root.tabs.slice()
        next.splice(index + 1, 0, tab)
        root.tabs = next
        root.activeIndex = index + 1
    }

    function toggleTabPin(index) {
        if (index < 0 || index >= root.tabs.length)
            return
        root.tabs[index].pinned = !root.tabs[index].pinned
    }

    /// Closes every tab except `index` and the pinned ones.
    function closeOtherTabs(index) {
        if (index < 0 || index >= root.tabs.length)
            return
        const target = root.tabs[index]
        const kept = []
        for (let cursor = 0; cursor < root.tabs.length; cursor += 1) {
            const tab = root.tabs[cursor]
            if (cursor === index || tab.pinned) {
                kept.push(tab)
                continue
            }
            root.rememberClosed(tab)
            tab.dispose()
        }
        root.tabs = kept
        root.activeIndex = Math.max(0, kept.indexOf(target))
    }

    /// `direction` is -1 for the tabs to the left of `index`, +1 for the right.
    function closeTabsBeside(index, direction) {
        if (index < 0 || index >= root.tabs.length)
            return
        const target = root.tabs[index]
        const kept = []
        for (let cursor = 0; cursor < root.tabs.length; cursor += 1) {
            const tab = root.tabs[cursor]
            const beyond = direction < 0 ? cursor < index : cursor > index
            if (!beyond || tab.pinned) {
                kept.push(tab)
                continue
            }
            root.rememberClosed(tab)
            tab.dispose()
        }
        root.tabs = kept
        root.activeIndex = Math.max(0, kept.indexOf(target))
    }

    function showTabMenu(index, x, y) {
        if (index < 0 || index >= root.tabs.length)
            return
        root.tabMenuIndex = index
        const tab = root.tabs[index]
        const point = topBar.mapToItem(null, x, y)
        menu.openAt(point.x, point.y, [
            { id: "tabDuplicate", glyph: "copy", label: qsTr("复制标签页") },
            { id: "tabPin", glyph: "star",
              label: tab.pinned ? qsTr("取消固定") : qsTr("固定标签页") },
            { separator: true },
            { id: "tabClose", glyph: "close", label: qsTr("关闭标签页") },
            { id: "tabCloseOthers", glyph: "close", label: qsTr("关闭其他标签页"),
              enabled: root.tabs.length > 1 },
            { id: "tabCloseLeft", glyph: "chevronLeft", label: qsTr("关闭左侧标签页"),
              enabled: index > 0 },
            { id: "tabCloseRight", glyph: "chevronRight", label: qsTr("关闭右侧标签页"),
              enabled: index < root.tabs.length - 1 },
            { separator: true },
            { id: "tabReopen", glyph: "undo", label: qsTr("重新打开关闭的标签页"),
              shortcut: root.chord("Mod+Shift+T"),
              enabled: root.closedTabs.length > 0 }
        ])
    }

    /// The window's session record, written when it closes.
    function sessionRecord() {
        const tabs = []
        for (let index = 0; index < root.tabs.length; index += 1)
            tabs.push(root.tabState(root.tabs[index]))
        return {
            "token": root.token,
            "x": root.x,
            "y": root.y,
            "width": root.width,
            "height": root.height,
            "activeIndex": root.activeIndex,
            "splitEnabled": ExplorerPrefs.splitEnabled,
            "tabs": tabs
        }
    }

    /// Rebuilds the previous run's windows. Runs in the first window only, and
    /// consumes the document so a second window cannot restore it again.
    function restoreSessionWindows() {
        if (!ExplorerPrefs.restoreSession)
            return false
        const raw = Session.take()
        if (!raw || raw === "")
            return false
        let document = null
        try {
            document = JSON.parse(raw)
        } catch (error) {
            return false
        }
        const records = document && document.windows ? document.windows : []
        if (records.length === 0)
            return false

        let restoredMain = false
        for (let index = 0; index < records.length; index += 1) {
            const record = records[index]
            const tabs = []
            for (let cursor = 0; cursor < record.tabs.length; cursor += 1) {
                const tab = root.createTab("")
                if (tab === null)
                    continue
                root.restoreTabState(tab, record.tabs[cursor])
                tabs.push(tab)
            }
            if (tabs.length === 0)
                continue

            const active = Math.max(0, Math.min(record.activeIndex, tabs.length - 1))
            if (!restoredMain) {
                // The record's token belonged to the previous run; this
                // window (or the one `createWindow` mints below) saves under a
                // fresh one, so the old record would linger as a duplicate.
                if (record.token && record.token !== root.token)
                    Session.removeWindow(record.token)
                root.tabs = tabs
                root.activeIndex = active
                ExplorerPrefs.setBool("splitEnabled", record.splitEnabled === true)
                if (record.width >= 480) {
                    root.width = record.width
                    root.height = record.height
                    // Wayland may ignore the position; the size still applies.
                    root.x = record.x
                    root.y = record.y
                }
                restoredMain = true
                continue
            }

            if (record.token)
                Session.removeWindow(record.token)
            const sibling = root.createWindow({
                "token": Shell.newToken(),
                "firstWindow": false,
                "initialTabs": tabs,
                "initialActive": active,
                "material": root.material,
                "x": record.x,
                "y": record.y,
                "width": record.width,
                "height": record.height
            })
            if (sibling === null) {
                for (let cursor = 0; cursor < tabs.length; cursor += 1)
                    tabs[cursor].dispose()
            }
        }

        if (restoredMain) {
            if (Session.crashed)
                Qt.callLater(function () {
                    toast.show(qsTr("上次会话未正常退出，已恢复窗口与标签页"), false)
                })
        }
        return restoredMain
    }

    function closeTab(index) {
        if (index < 0 || index >= root.tabs.length)
            return
        const closing = root.tabs[index]
        root.rememberClosed(closing)
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
        let at = topBar.indexAt(localX)
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
        root.searchOpen = false
        root.resetSearchState()
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
        // Search and editing are states of the same frame (§7.2), so opening
        // the editor ends a live query instead of stacking two modes.
        if (root.searchOpen) {
            root.searchOpen = false
            if (root.searchQuery !== "")
                root.acceptSearchText("")
        }
        root.pathEditing = true
        // The field is only visible once `pathEditing` is on, and a hidden item
        // cannot take focus: wait for the state to settle before focusing.
        Qt.callLater(locator.focusAddress)
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
        overviewModelInstance.refresh()
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
            overviewModelInstance.refresh()
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
                toast.show(Spaces.error !== ""
                    ? root.localizeBackend(Spaces.error) : qsTr("无法删除空间"), true)
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
        root.setSearchText("")
    }

    function openSearch() {
        if (!root.folderActive || root.remoteLocation)
            return
        // One frame, one mode: entering search takes the locator out of the
        // path editor.
        root.pathEditing = false
        root.searchOpen = true
        // The field is only focusable once the mode has been laid out;
        // focusing a hidden item silently does nothing.
        Qt.callLater(root.focusSearchField)
    }

    /// Escape, or the icon again: drop the query *and* the field, because
    /// leaving a live query with no field would strand the results on screen.
    function closeSearch() {
        if (root.searchQuery !== "")
            root.acceptSearchText("")
        root.searchOpen = false
        root.focusActiveSurface()
    }

    // ── Toolbar model ─────────────────────────────────────────────────────
    // The bar is data, not a fixed row: `toolbarLeadingItems` and
    // `toolbarTrailingItems` name the components the path bar instantiates.
    // A stored order is free text (settings.toml is hand-editable), so it is
    // reconciled against the ids this build knows before use: unknown ids are
    // dropped and missing ones appended, which is also how a new action
    // appears for someone with an old preference file.

    /// Reconciles a stored comma list against `defaults`.
    function toolbarIds(value, defaults) {
        const result = []
        const parts = String(value || "").split(",")
        for (let index = 0; index < parts.length; index += 1) {
            const id = parts[index].trim()
            if (id !== "" && defaults.indexOf(id) >= 0 && result.indexOf(id) < 0)
                result.push(id)
        }
        for (let index = 0; index < defaults.length; index += 1) {
            if (result.indexOf(defaults[index]) < 0)
                result.push(defaults[index])
        }
        return result
    }

    function syncToolbarOrder() {
        const leading = root.toolbarIds(ExplorerPrefs.toolbarLeading,
                                        ["back", "forward", "up"])
        const trailing = root.toolbarIds(ExplorerPrefs.toolbarTrailing,
                                         ["search", "trash", "view", "more"])
        if (leading.join(",") !== root.toolbarLeadingItems.join(","))
            root.toolbarLeadingItems = leading
        if (trailing.join(",") !== root.toolbarTrailingItems.join(","))
            root.toolbarTrailingItems = trailing
    }

    /// The loaded delegate for `id`, or null when the action is not on the
    /// bar. Menus opened from a shortcut anchor to this instead of an id that
    /// only exists while the component is instantiated.
    function toolbarItem(id) {
        const groups = [
            { model: root.toolbarLeadingItems, repeater: leadingToolbarRepeater },
            { model: root.toolbarTrailingItems, repeater: trailingToolbarRepeater }
        ]
        for (let group = 0; group < groups.length; group += 1) {
            const index = groups[group].model.indexOf(id)
            if (index >= 0) {
                const item = groups[group].repeater.itemAt(index)
                if (item)
                    return item
            }
        }
        return null
    }

    /// Moves one action inside its group and persists the new order. `to` is
    /// the insertion point in the list as it looked before the move, so a
    /// rightward move needs the index shifted back after the removal.
    function moveToolbarItem(group, from, to) {
        const current = (group === "leading"
                         ? root.toolbarLeadingItems
                         : root.toolbarTrailingItems).slice()
        if (from < 0 || from >= current.length)
            return
        const target = Math.max(0, Math.min(to, current.length))
        if (target === from || target === from + 1)
            return
        const moved = current.splice(from, 1)[0]
        current.splice(target > from ? target - 1 : target, 0, moved)
        if (group === "leading") {
            root.toolbarLeadingItems = current
            ExplorerPrefs.setString("toolbarLeading", current.join(","))
        } else {
            root.toolbarTrailingItems = current
            ExplorerPrefs.setString("toolbarTrailing", current.join(","))
        }
    }

    /// The index a drag that ends at `sceneX` should insert into: one past
    /// every item whose centre is left of the pointer.
    function toolbarDropIndex(group, sceneX) {
        const repeater = group === "leading" ? leadingToolbarRepeater : trailingToolbarRepeater
        let index = 0
        for (let item = 0; item < repeater.count; item += 1) {
            const delegate = repeater.itemAt(item)
            if (delegate && sceneX > delegate.mapToItem(null, delegate.width / 2, 0).x)
                index = item + 1
        }
        return index
    }

    function finishToolbarDrag(group, from, sceneX) {
        root.moveToolbarItem(group, from, root.toolbarDropIndex(group, sceneX))
    }

    // The three menus the toolbar owns. They anchor to the live delegate so a
    // reordered or missing action cannot make a shortcut point at nothing.
    function openDisplayMenu() {
        const anchor = root.toolbarItem("view")
        if (anchor === null)
            return
        const point = anchor.mapToItem(null, 0, 0)
        displayMenu.openAbove(point.x + anchor.width - 252, point.y)
    }

    function openFiltersMenu() {
        const anchor = root.toolbarItem("more")
        if (anchor === null)
            return
        const point = anchor.mapToItem(null, 0, 0)
        filterMenu.openAbove(point.x + anchor.width - 252, point.y)
    }

    function openMoreMenu() {
        const anchor = root.toolbarItem("more")
        if (anchor === null)
            return
        const point = anchor.mapToItem(null, 0, 0)
        moreMenu.openAt(point.x + anchor.width - 226, point.y, root.moreMenuItems())
    }

    /// The search field lives in the locator now, so the shell reaches it
    /// directly instead of through the toolbar model (proposal §7.2).
    function setSearchText(value) {
        locator.setSearchText(value)
    }

    function focusSearchField() {
        locator.focusSearch()
    }

    /// `Image.source` wants a URL, and the stored preference is a native path
    /// from a file dialog. Windows drive paths need the third slash; a POSIX
    /// path already supplies it.
    function backgroundImageSource(path) {
        if (path === "")
            return ""
        const normalized = String(path).replace(/\\/g, "/")
        if (normalized.startsWith("file:"))
            return normalized
        if (/^[A-Za-z]:\//.test(normalized))
            return "file:///" + normalized
        if (normalized.startsWith("//"))
            return "file:" + normalized
        return "file://" + (normalized.startsWith("/") ? "" : "/") + normalized
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
                         glyph: options[index].glyph,
                         label: options[index].label,
                         checked: root.searchType === options[index].id })
        return items
    }

    function searchHistoryItems() {
        const entries = root.searchHistoryEntries
        const items = []
        for (let index = 0; index < entries.length; index += 1)
            items.push({ id: "history:" + index, label: entries[index], glyph: "search" })
        if (items.length === 0)
            items.push({ id: "history:none", label: qsTr("尚无搜索历史"), enabled: false })
        items.push({ separator: true })
        items.push({ id: "history:clear", label: qsTr("清除历史"), danger: true })
        return items
    }

    function runHistoryEntry(id) {
        const value = id.substring("history:".length)
        if (value === "clear") {
            ExplorerPrefs.clearSearchHistory()
            return
        }
        if (value === "none")
            return
        const index = parseInt(value, 10)
        const entries = root.searchHistoryEntries
        if (isNaN(index) || index < 0 || index >= entries.length)
            return
        root.openSearch()
        root.searchMode = "name"
        root.setSearchText(entries[index])
        root.acceptSearchText(entries[index])
    }

    /// A tag is a fact about the disk, not about the open folder, so its
    /// search runs over every local volume with the tag filter already typed.
    function searchTag(name) {
        if (root.remoteLocation)
            return
        if (!root.folderActive) {
            // The overview and a space have no listing to search in; open the
            // folder the tab came from first, then run the tag search there.
            root.navigate(root.activePath)
        }
        const query = "tag:" + name
        root.openSearch()
        root.searchMode = "name"
        root.searchScope = "volumes"
        root.setSearchText(query)
        root.acceptSearchText(query)
    }

    /// The filter row's "保存" chip. The name comes from the shell's one-line
    /// prompt; the query and its settings are read from the live bar, so what
    /// is stored is what produced the results on screen.
    function requestSaveSearch() {
        const query = root.searchQuery.trim()
        if (query.length === 0 || root.modalOpen)
            return
        root.promptMode = "saveSearch"
        const suggestion = query.length > 40 ? query.slice(0, 40) : query
        namePrompt.open(qsTr("保存搜索"), suggestion, qsTr("名称"))
    }

    /// Re-runs a stored search. The mode, scope, type and the listing's own
    /// kind/date/size buckets are all part of the query, so every one of them
    /// is restored before the text is accepted: the results must not come from
    /// a half-restored bar.
    function applySavedSearch(id) {
        if (root.modalOpen)
            return
        if (root.remoteLocation) {
            toast.show(qsTr("保存的搜索只支持本地目录"), false)
            return
        }
        let entry = null
        for (let index = 0; index < root.savedSearches.length; index += 1) {
            if (root.savedSearches[index].id === id) {
                entry = root.savedSearches[index]
                break
            }
        }
        if (entry === null || !entry.query)
            return
        if (!root.folderActive)
            root.navigate(root.activePath)
        root.openSearch()
        root.searchMode = entry.mode === "content" ? "content" : "name"
        root.searchScope = entry.scope ? entry.scope : "current"
        root.searchType = entry.typeId ? entry.typeId : "all"
        root.searchFilter = entry.filter ? entry.filter : ""
        root.searchRegex = entry.regex === true
        root.searchCase = entry.caseSensitive === true
        ExplorerPrefs.setString("kindFilter", entry.kind ? entry.kind : "all")
        ExplorerPrefs.setString("modifiedFilter", entry.modified ? entry.modified : "any")
        ExplorerPrefs.setString("sizeFilter", entry.size ? entry.size : "any")
        root.setSearchText(entry.query)
        root.acceptSearchText(entry.query)
    }

    /// Tells the preferences which directory's layout is on screen. Writing
    /// the property is what makes a per-directory layout restore (and what a
    /// change in that directory is saved against); with layouts synchronised
    /// it is a no-op.
    /// Pushes a tag write to every listing showing one of the paths, and to
    /// the split pane's model when it is a different one.
    /// The palette toggles a column by id; the preference owns the list.
    function toggleColumn(id) {
        const configured = (ExplorerPrefs.columns || "").split(",").filter(function (value) {
            return value.length > 0
        })
        const index = configured.indexOf(id)
        if (index >= 0)
            configured.splice(index, 1)
        else
            configured.push(id)
        ExplorerPrefs.setString("columns", configured.join(","))
    }

    /// A filter changed while a name search is live: the buckets are part of
    /// the query, so the results have to follow without retyping.
    Connections {
        target: ExplorerPrefs
        function onKindFilterChanged() {
            if (root.searchActive && root.searchMode === "name")
                searchTimer.restart()
        }
        function onModifiedFilterChanged() {
            if (root.searchActive && root.searchMode === "name")
                searchTimer.restart()
        }
        function onSizeFilterChanged() {
            if (root.searchActive && root.searchMode === "name")
                searchTimer.restart()
        }
    }

    function syncTagsToLists(paths) {
        if (root.activeTab !== null)
            root.activeTab.listing.refreshTags(paths)
        if (root.splitActive && root.splitTab !== null)
            root.splitTab.listing.refreshTags(paths)
    }

    function syncTagDefinitions() {
        if (root.activeTab !== null)
            root.activeTab.listing.refreshTagDefinitions()
        if (root.splitActive && root.splitTab !== null)
            root.splitTab.listing.refreshTagDefinitions()
    }

    function syncLayoutForActiveTab() {
        const tab = root.activeTab
        if (tab === null || !tab.folder || tab.listing.path === "")
            return
        ExplorerPrefs.setString("currentPath", tab.listing.path)
    }

    /// First visit to a directory with no stored layout: let the contents pick
    /// one. The mode is a suggestion the user can override with one menu
    /// click, which is why the rule stays this simple.
    function applyAdaptiveForActiveTab() {
        if (!ExplorerPrefs.adaptiveLayout || ExplorerPrefs.syncLayout)
            return
        const tab = root.activeTab
        if (tab === null || !tab.folder || tab.listing.remote || !tab.listing.finished)
            return
        const path = tab.listing.path
        if (path === "")
            return
        const depth = path.split(/[\\/]/).filter(function (part) {
            return part.length > 0
        }).length
        ExplorerPrefs.applyAdaptiveView(path, tab.listing.images, tab.listing.files, depth)
    }

    Connections {
        target: root.activeTab
        enabled: root.activeTab !== null
        function onListingChanged() {
            root.syncLayoutForActiveTab()
        }
    }

    Connections {
        target: root.activeTab !== null ? root.activeTab.listing : null
        enabled: root.activeTab !== null
        function onPathChanged() {
            root.syncLayoutForActiveTab()
        }
        function onFinishedChanged() {
            root.applyAdaptiveForActiveTab()
        }
    }

    /// The palette's "search inside files" fallback: the same field and the
    /// same bar, only the mode and the query arrive already filled in, so the
    /// user lands where the query is running instead of in a second search UI.
    function startContentSearch(value) {
        if (!root.folderActive || root.remoteLocation || value.length === 0)
            return
        root.openSearch()
        root.searchMode = "content"
        root.setSearchText(value)
        root.acceptSearchText(value)
    }

    function runSearch() {
        const tab = root.activeTab
        if (tab === null)
            return
        const query = root.searchQuery.trim()
        tab.nameSearch.cancel()
        tab.contentSearch.cancel()
        // Both modes walk the local disk; a network backend only answers the
        // directory listing it already has.
        if (query === "" || root.remoteLocation)
            return
        tab.listing.clearSelection()
        previewPanel.showPaths([])
        if (root.searchMode === "content") {
            tab.contentSearch.search(tab.listing.path, query, root.searchRegex,
                                    root.searchCase, root.searchFilter)
        } else {
            tab.nameSearch.search(tab.listing.path, query, root.searchScope,
                                  root.searchKind, root.searchModified,
                                  root.searchSize)
            ExplorerPrefs.rememberSearch(query)
        }
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


    /// The selection a window-level shortcut acts on. Deliberately not
    /// `currentSelection()`: that one may still point at the row whose context
    /// menu was open, and a shortcut must never act on a stale menu target.
    function shortcutSelection() {
        const list = root.splitActive && root.splitFocused ? splitList : fileList
        return list.selectedPathList()
    }

    function shortcutPath() {
        const paths = root.shortcutSelection()
        return paths.length > 0 ? paths[0] : root.activePath
    }

    function shortcutDirectory() {
        if (root.activeTab !== null && root.activeTab.listing !== null)
            return root.activeTab.listing.path
        return root.activePath
    }

    /// List → grid → columns, the same order the view menu shows them in.
    function cycleViewMode() {
        const order = ["list", "grid", "column"]
        const next = (order.indexOf(ExplorerPrefs.viewMode) + 1) % order.length
        ExplorerPrefs.setString("viewMode", order[next])
    }

    function toggleSizeUnits() {
        ExplorerPrefs.setString("sizeUnits",
                                ExplorerPrefs.sizeUnits === "binary" ? "decimal" : "binary")
    }

    function contextDirectory() {
        return root.contextTab !== null ? root.contextTab.listing.path : root.activePath
    }

    /// Hands the current selection to the platform's share sheet. The window
    /// handle comes from the native controller because QML's `Window` has no
    /// `winId`; a failure is reported instead of silently doing nothing.
    function shareSelection() {
        const paths = root.currentSelection()
        if (paths.length === 0)
            return
        if (!Workspace.sharePaths(paths, chrome.nativeHandle()))
            toast.show(qsTr("无法打开系统共享面板"), true)
    }

    /// `rar`/`iso` are readable only when an external tool is installed; the
    /// list comes from the shell so the row disappears rather than failing.
    readonly property var externalArchiveFormats: {
        let formats = []
        try {
            formats = JSON.parse(Workspace.externalArchiveFormats)
        } catch (error) {
            formats = []
        }
        return formats
    }

    function isArchivePath(path) {
        const builtin = /\.(zip|tar|tar\.gz|tgz|7z)$/i.test(path)
        if (builtin)
            return true
        for (let index = 0; index < root.externalArchiveFormats.length; index += 1) {
            const format = root.externalArchiveFormats[index]
            if (new RegExp("\\." + format + "$", "i").test(path))
                return true
        }
        return false
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
            root.archiveFormat = format
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

    /// "Extract to…": the same folder picker the move dialog uses, with the
    /// confirm wired to `extractArchive` instead of the transfer queue.
    function startExtractTo(path) {
        if (!root.isArchivePath(path))
            return
        root.archiveMode = "extract"
        root.archiveTarget = path
        root.archiveDestination = ""
        moveToDialog.openExtract(root.contextDirectory(), path)
    }

    function requestArchivePassword(target, message) {
        root.archiveMode = "extract"
        root.archiveTarget = target
        root.archiveDestination = ""
        if (passwordDialog.shown) {
            passwordDialog.showError(message)
            return
        }
        passwordDialog.open("extract", root.displayNameOfPath(target), "")
    }

    function acceptArchivePassword(password) {
        if (root.archiveMode === "compress")
            Workspace.compressEntries(root.archivePaths, root.archiveDestination,
                                      root.archiveFormat, password)
        else if (root.archiveMode === "extract")
            Workspace.extractArchive(root.archiveTarget, root.archiveDestination, password)
    }

    function resetArchiveState() {
        root.archiveMode = ""
        root.archiveTarget = ""
        root.archiveDestination = ""
        root.archiveFormat = ""
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
        } else if (id === "sort-created") {
            ExplorerPrefs.setString("sortKey", "created")
            ExplorerPrefs.setString("sortOrder", "desc")
        } else if (id === "sort-path") {
            ExplorerPrefs.setString("sortKey", "path")
            ExplorerPrefs.setString("sortOrder", "asc")
        } else if (id === "sort-tag") {
            ExplorerPrefs.setString("sortKey", "tag")
            ExplorerPrefs.setString("sortOrder", "asc")
        } else if (id.indexOf("group-") === 0) {
            const key = id.substring(6)
            if (key === "date-day") {
                ExplorerPrefs.setString("groupDate", "day")
            } else if (key === "off") {
                ExplorerPrefs.setString("groupKey", "")
            } else {
                ExplorerPrefs.setString("groupKey", key)
            }
        } else if (id.indexOf("columns-") === 0) {
            root.toggleColumn(id.substring(8))
        } else if (id === "extensions-toggle") {
            ExplorerPrefs.setBool("showExtensions", !ExplorerPrefs.showExtensions)
        } else if (id === "clear-tags") {
            TagStore.clearTags(root.currentSelection())
        } else if (id === "sort-order") {
            ExplorerPrefs.setString("sortOrder",
                                    ExplorerPrefs.sortOrder === "asc" ? "desc" : "asc")
        } else if (id === "folders-first") {
            ExplorerPrefs.setString("folderOrder",
                                    ExplorerPrefs.folderOrder === "foldersFirst"
                                        ? "mixed" : "foldersFirst")
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
            { id: "refresh", glyph: "refresh", label: qsTr("刷新"),
              shortcut: root.shortcutDisplay("explorer.refresh") },
            { id: "toggleHidden",
              glyph: "eye",
              label: qsTr("显示隐藏文件"),
              checked: ExplorerPrefs.showHiddenFiles,
              shortcut: root.shortcutDisplay("explorer.toggleHidden") },
            { id: "toggleSidebar",
              glyph: "panel",
              label: qsTr("侧栏"),
              checked: root.sidebarVisible },
            { id: "toggleSplit",
              glyph: "split",
              label: qsTr("分屏"),
              checked: ExplorerPrefs.splitEnabled },
            { id: "sort", glyph: "sort", label: qsTr("排序"), submenu: root.sortMenuItems() },
            { id: "group", glyph: "filter", label: qsTr("分组"), submenu: root.groupMenuItems() },
            { separator: true },
            { id: "filters", glyph: "filter", label: qsTr("筛选…") },
            { separator: true },
            { id: "settings", glyph: "gear", label: qsTr("设置"),
              shortcut: root.shortcutDisplay("app.openSettings") }
        ]
        if (Updater.supported)
            items.push({ id: "checkUpdates", glyph: "downloads",
                         label: Updater.available ? qsTr("更新可用") : qsTr("检查更新") })
        else if (Updater.packaged)
            // A packaged install cannot replace itself, but hiding the entry
            // leaves the user with no way to learn that. The dialog it opens
            // says where updates come from instead.
            items.push({ id: "checkUpdates", glyph: "downloads",
                         label: qsTr("由安装包更新") })
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
            key("created", qsTr("创建时间")),
            key("type", qsTr("类型")),
            key("path", qsTr("路径")),
            key("tag", qsTr("标签")),
            { separator: true },
            order("asc", qsTr("升序")),
            order("desc", qsTr("降序")),
            { separator: true },
            { id: "folderOrder:foldersFirst", label: qsTr("文件夹优先"),
              checked: ExplorerPrefs.folderOrder === "foldersFirst" },
            { id: "folderOrder:filesFirst", label: qsTr("文件优先"),
              checked: ExplorerPrefs.folderOrder === "filesFirst" },
            { id: "folderOrder:mixed", label: qsTr("混合排列"),
              checked: ExplorerPrefs.folderOrder === "mixed" }
        ]
    }

    /// The grouping submenu: one key at a time, and the two date keys carry a
    /// precision. `group:` with nothing after it turns grouping off.
    function groupMenuItems() {
        function key(id, label) {
            return { id: "group:" + id, label: label,
                     checked: ExplorerPrefs.groupKey === id }
        }
        function date(id, label) {
            return { id: "groupDate:" + id, label: label,
                     checked: ExplorerPrefs.groupDate === id }
        }
        const items = [
            key("", qsTr("不分组")),
            key("name", qsTr("按名称")),
            key("modified", qsTr("按修改时间")),
            key("created", qsTr("按创建时间")),
            key("size", qsTr("按大小")),
            key("type", qsTr("按类型")),
            key("tag", qsTr("按标签"))
        ]
        if (ExplorerPrefs.groupKey === "modified" || ExplorerPrefs.groupKey === "created") {
            items.push({ separator: true })
            items.push(date("year", qsTr("按年")))
            items.push(date("month", qsTr("按月")))
            items.push(date("day", qsTr("按日")))
        }
        return items
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
        } else if (id.indexOf("folderOrder:") === 0) {
            ExplorerPrefs.setString("folderOrder", id.substring(12))
        } else if (id.indexOf("groupDate:") === 0) {
            ExplorerPrefs.setString("groupDate", id.substring(10))
        } else if (id.indexOf("group:") === 0) {
            ExplorerPrefs.setString("groupKey", id.substring(6))
        } else if (id.indexOf("sort:") === 0) {
            const value = id.substring(5)
            ExplorerPrefs.setString("sortKey", value)
            ExplorerPrefs.setString("sortOrder",
                                    value === "modified" || value === "size" ? "desc" : "asc")
        } else if (id.indexOf("order:") === 0) {
            ExplorerPrefs.setString("sortOrder", id.substring(6))
        } else if (id === "filters") {
            root.openFiltersMenu()
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
            toast.show(Spaces.error !== ""
                ? root.localizeBackend(Spaces.error) : qsTr("无法添加到空间"), true)
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
                toast.show(Spaces.error !== ""
                    ? root.localizeBackend(Spaces.error) : qsTr("无法创建空间"), true)
            else {
                if (path !== "")
                    Spaces.addItem(id, path)
                root.openSpace(id)
                toast.show(qsTr("已创建空间"), false)
            }
        } else if (mode === "spaceRename") {
            if (!Spaces.renameSpace(path, text))
                toast.show(Spaces.error !== ""
                    ? root.localizeBackend(Spaces.error) : qsTr("无法重命名空间"), true)
        } else if (mode === "saveSearch") {
            // The spec is one JSON object rather than ten arguments: the
            // preference setter takes a `SavedSearch`, and a spec that grows a
            // field later does not change the call.
            const spec = JSON.stringify({
                query: root.searchQuery.trim(),
                mode: root.searchMode,
                scope: root.searchScope,
                typeId: root.searchType,
                kind: root.searchKind,
                modified: root.searchModified,
                size: root.searchSize,
                regex: root.searchRegex,
                caseSensitive: root.searchCase,
                filter: root.searchFilter
            })
            if (ExplorerPrefs.saveSearch(text, spec))
                toast.show(qsTr("已保存搜索“%1”").arg(text), false)
            else
                toast.show(qsTr("无法保存搜索"), true)
        }
    }

    /// The row menu's space submenu is built from the shared singleton, so it
    /// never lists a stale rename. The final entry creates a space and pins the
    /// row in one prompt.
    function spaceSubmenu() {
        const entries = []
        for (let index = 0; index < Spaces.count; index += 1)
            entries.push({ id: "addSpace:" + Spaces.idAt(index),
                           glyph: "diamond",
                           label: Spaces.nameAt(index) })
        entries.push({ separator: true })
        entries.push({ id: "newSpaceAdd", glyph: "plus", label: qsTr("新建空间并添加…") })
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
        root.rowMenuMore = false
        // Discovery runs off the Qt thread. The rows already in the model are
        // drawn at once, and `onShellCommandRevisionChanged` swaps in the fresh
        // answer without moving the panel.
        shellCommands.load(root.currentSelection(), path)
        menu.openAt(x, y, root.rowMenuItems())
    }

    /// Whether the right-clicked row lives on a remote listing.
    function rowMenuRemote() {
        return root.contextTab !== null
            ? (root.contextTab.listing.remote || root.isRemotePath(root.contextTab.listing.path))
            : root.remoteLocation
    }

    /// The row menu's first page: the actions a right-click is actually for.
    /// Everything rarer sits on the second page behind "更多…", because twenty
    /// rows in one panel made the two actions people use most the two hardest
    /// to find.
    function rowMenuItems() {
        const path = root.contextPath
        const isDir = root.contextIsDir
        const many = (root.contextList !== null
            ? root.contextList.selectionCount : fileList.selectionCount) > 1
        const hasPath = path !== ""
        const hasClipboard = Workspace.clipboardMode !== ""
        const remote = root.rowMenuRemote()
        return [
            { id: "open", glyph: isDir ? "folder" : "external",
              label: isDir ? qsTr("打开") : qsTr("用默认应用打开"),
              enabled: hasPath && (!remote || isDir), shortcut: root.chord("Enter") },
            { id: "openTab", glyph: "plus", label: qsTr("在新标签页打开"),
              enabled: hasPath && isDir },
            { id: "openWindow", glyph: "window", label: qsTr("在新窗口打开"),
              enabled: hasPath && isDir },
            { separator: true },
            { id: "copy", glyph: "copy", label: qsTr("复制"),
              shortcut: root.shortcutDisplay("explorer.copy"), enabled: hasPath },
            { id: "cut", glyph: "cut", label: qsTr("剪切"),
              shortcut: root.shortcutDisplay("explorer.cut"), enabled: hasPath },
            { id: "paste", glyph: "paste", label: qsTr("粘贴到此处"),
              shortcut: root.shortcutDisplay("explorer.paste"),
              enabled: hasClipboard && isDir },
            { separator: true },
            { id: many ? "bulkRename" : "rename",
              glyph: "pencil",
              label: many ? qsTr("批量重命名…") : qsTr("重命名"),
              enabled: hasPath,
              shortcut: many ? "" : root.shortcutDisplay("explorer.rename") },
            { id: "trash", glyph: "trash", label: qsTr("移到回收站"),
              shortcut: root.shortcutDisplay("explorer.trash"),
              enabled: hasPath && !remote, danger: true },
            { separator: true },
            { id: "properties", glyph: "info", label: qsTr("属性"),
              shortcut: root.shortcutDisplay("explorer.properties"),
              enabled: hasPath && !many },
            { id: "tags", glyph: "star", label: qsTr("标签"),
              enabled: hasPath, submenu: root.tagSubmenu() },
            { id: "moreMenu", glyph: "more", label: qsTr("更多…") }
        ]
    }

    /// The tag rows, rebuilt from the singleton every time the menu opens so a
    /// definition created a moment ago is already there. Clicking toggles the
    /// tag on the whole selection.
    function tagSubmenu() {
        let tags = []
        try {
            tags = JSON.parse(TagStore.tagsJson)
        } catch (error) {
            tags = []
        }
        const entries = []
        for (let index = 0; index < tags.length; index += 1)
            entries.push({ id: "tag:" + tags[index].id, label: tags[index].name })
        if (entries.length === 0)
            entries.push({ id: "tag:none", label: qsTr("尚无标签"), enabled: false })
        entries.push({ separator: true })
        entries.push({ id: "tag:__clear", label: qsTr("清除标签") })
        return entries
    }

    /// The user's `[[actions]]` rows that apply to the current selection.
    /// Availability (platform, program on PATH, selection kind) is decided in
    /// Rust; the menu only paints what it gets.
    function customActionSubmenu() {
        let entries = []
        try {
            entries = JSON.parse(Workspace.customActionsFor(root.currentSelection(),
                                                           root.contextDirectory()))
        } catch (error) {
            entries = []
        }
        const items = []
        for (let index = 0; index < entries.length; index += 1)
            items.push({ id: "customAction:" + entries[index].id, label: entries[index].name })
        return items
    }

    /// The row menu's second page. It swaps the panel's rows in place instead
    /// of opening a flyout, so the submenus here — installed apps, archive
    /// formats, spaces — stay one level deep, which is what the menu overlay
    /// supports.
    function rowMoreMenuItems() {
        const path = root.contextPath
        const isDir = root.contextIsDir
        const hasPath = path !== ""
        const hasClipboard = Workspace.clipboardMode !== ""
        const remote = root.rowMenuRemote()
        // Archiving streams remote sources into a local container, so the
        // sources may be remote but the destination (here) may not.
        const canExtract = hasPath && !isDir && !remote && root.isArchivePath(path)
        const items = [
            { id: "menuBack", glyph: "chevronLeft", label: qsTr("返回") },
            { separator: true },
            { id: "reveal", glyph: "folder", label: qsTr("在资源管理器中显示"),
              enabled: hasPath && !remote },
            { id: "openWith", glyph: "exe", label: qsTr("选择打开方式…"),
              enabled: hasPath && !remote },
            { id: "terminal", glyph: "terminal", label: qsTr("在终端中打开"),
              enabled: hasPath && isDir && !remote },
            { id: "preview", glyph: "eye",
              label: root.previewShows(path) ? qsTr("隐藏预览") : qsTr("预览"),
              enabled: hasPath && !manySelection(), shortcut: root.shortcutDisplay("explorer.preview") },
            { id: "copyPath", glyph: "copy", label: qsTr("复制路径"), enabled: hasPath },
            { id: "share", glyph: "external", label: qsTr("共享…"),
              enabled: hasPath && !remote && Workspace.shareSupported },
            { id: "measureSize", glyph: "info", label: qsTr("计算大小"),
              enabled: hasPath && (isDir || manySelection()) },
            { id: "compareHashes", glyph: "check", label: qsTr("比较 SHA-256"),
              enabled: root.currentSelection().length === 2 && !remote }
        ]

        const extensions = root.shellCommandSubmenu()
        if (extensions.length > 0) {
            items.push({ separator: true })
            items.push({ id: "shellCommands", glyph: "external",
                         label: qsTr("应用扩展"), submenu: extensions })
        }

        const custom = root.customActionSubmenu()
        if (custom.length > 0) {
            items.push({ id: "customActions", glyph: "exe",
                         label: qsTr("自定义动作"), submenu: custom })
        }

        items.push({ separator: true })
        items.push({ id: "pasteShortcut", glyph: "paste", label: qsTr("粘贴快捷方式"),
                     enabled: hasClipboard && isDir && !remote })
        items.push({ id: "duplicate", glyph: "copy", label: qsTr("创建副本"), enabled: hasPath })
        items.push({ id: "createShortcut", glyph: "link", label: qsTr("创建快捷方式"),
                     enabled: hasPath && !remote })
        items.push({ id: "compress", glyph: "archive", label: qsTr("压缩为…"),
                     enabled: hasPath && !remote, submenu: [
            { id: "compressZip", label: qsTr("ZIP 压缩包") },
            { id: "compressZipEncrypted", label: qsTr("ZIP 压缩包（加密）") },
            { id: "compressTar", label: qsTr("TAR 归档") },
            { id: "compressTarGz", label: qsTr("TAR.GZ 压缩包") },
            { id: "compress7z", label: qsTr("7Z 压缩包") },
            { id: "compress7zEncrypted", label: qsTr("7Z 压缩包（加密）") }
        ] })
        items.push({ id: "extractHere", glyph: "extract", label: qsTr("解压到此处"),
                     enabled: canExtract })
        items.push({ id: "extractTo", glyph: "folder", label: qsTr("解压到新文件夹…"),
                     enabled: canExtract })
        items.push({ id: "moveTo", glyph: "arrowRight", label: qsTr("移动到…"),
                     enabled: hasPath })
        items.push({ separator: true })
        items.push({ id: "favorite", glyph: "star", label: qsTr("加入收藏"),
                     enabled: hasPath && isDir })
        items.push({ id: "addToSpace", glyph: "diamond", label: qsTr("添加到空间"),
                     enabled: hasPath, submenu: root.spaceSubmenu() })
        items.push({ separator: true })
        items.push({ id: "delete", glyph: "trash", label: qsTr("永久删除"),
                     shortcut: root.shortcutDisplay("explorer.deletePermanent"),
                     enabled: hasPath, danger: true })
        return items
    }

    /// The row menu opens on the right-clicked selection; the second page can
    /// be reached with a multi-selection too, so it asks the same question.
    function manySelection() {
        return (root.contextList !== null
            ? root.contextList.selectionCount : fileList.selectionCount) > 1
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

    /// Applies a measured folder total to whichever listing shows that path.
    /// Both panes are told; a model ignores paths it does not list.
    function applyFolderSize(path, size) {
        if (root.activeTab !== null)
            root.activeTab.listing.setDirectorySize(path, size)
        if (root.splitTab !== null)
            root.splitTab.listing.setDirectorySize(path, size)
    }

    function togglePreview(path) {
        if (root.previewShows(path)) {
            root.previewVisible = false
            previewPanel.showPaths([])
            return
        }
        // A pane that is closed, too narrow to dock, or on another surface has
        // no room to reflow into — Space opens the overlay instead of changing
        // the window's layout behind the user's back.
        if (!root.previewVisible || !root.folderActive || root.width < 840) {
            quickPreview.show([path])
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

    /// A right-click anywhere on the breadcrumb bar. The trail is the folder
    /// itself, so the menu acts on that path rather than on an entry inside it:
    /// copy its path or name, open a terminal there, or turn the bar into the
    /// path editor. `split` says which pane's trail was clicked; only the
    /// primary bar has an editor to open.
    function openBreadcrumbBarMenu(x, y, split) {
        const tab = split ? root.splitTab : root.activeTab
        if (tab === null)
            return
        const path = tab.listing.path
        if (path === "")
            return
        const names = tab.listing.breadcrumbNames
        root.contextKind = "breadcrumbBar"
        root.contextTab = split ? tab : null
        root.contextList = null
        root.contextPath = path
        root.contextName = names.length > 0 ? names[names.length - 1] : path
        root.contextIsDir = true
        const remote = tab.listing.remote || root.isRemotePath(path)
        const items = [
            { id: "copyPath", glyph: "copy", label: qsTr("复制路径") },
            { id: "copyName", glyph: "text", label: qsTr("复制名称") },
            { id: "terminal", glyph: "terminal", label: qsTr("在终端中打开"), enabled: !remote }
        ]
        if (!split) {
            items.push({ separator: true })
            items.push({ id: "editPath", glyph: "pencil", label: qsTr("编辑路径") })
        }
        menu.openAt(x, y, items)
    }

    function showTrashRowMenu(id, originalParent, x, y) {
        root.contextKind = "trash"
        root.contextTrashId = id
        root.contextTrashParent = originalParent
        menu.openAt(x, y, [
            { id: "trashRestore", glyph: "undo", label: qsTr("恢复"),
              shortcut: root.chord("Enter") },
            { id: "trashOpenOriginal", glyph: "external", label: qsTr("打开原位置"),
              enabled: originalParent !== "" },
            { separator: true },
            { id: "trashPurge", glyph: "trash", label: qsTr("永久删除"),
              shortcut: root.chord("Delete"), danger: true }
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
            { id: "newFolder", glyph: "folder", label: qsTr("新建文件夹"),
              shortcut: root.chord("Mod+Shift+N") },
            { id: "newFile", glyph: "file", label: qsTr("新建文件") },
            { separator: true },
            { id: "paste", glyph: "paste", label: qsTr("粘贴"),
              shortcut: root.shortcutDisplay("explorer.paste"),
              enabled: Workspace.clipboardMode !== "" },
            { id: "pasteShortcut", glyph: "link", label: qsTr("粘贴快捷方式"),
              enabled: Workspace.clipboardMode !== "" && !remote },
            { id: "selectAll", glyph: "check", label: qsTr("全选"),
              shortcut: root.shortcutDisplay("explorer.selectAll") },
            { id: "refresh", glyph: "refresh", label: qsTr("刷新"),
              shortcut: root.shortcutDisplay("explorer.refresh") },
            { separator: true },
            { id: "terminal", glyph: "terminal", label: qsTr("在终端中打开"),
              enabled: !remote }
        ])
    }

    /// What the drive menu may offer, read once from the shell. Every row is
    /// gated on this and on `removable`, so an unsupported action is absent
    /// rather than present and broken.
    readonly property var driveCapabilities: {
        try {
            return JSON.parse(Workspace.driveCapabilities)
        } catch (error) {
            return {}
        }
    }

    function showSidebarMenu(kind, path, removable, x, y) {
        if (kind === "volume") {
            const caps = root.driveCapabilities
            const items = [
                { id: "open", glyph: "drive", label: qsTr("打开") },
                { id: "openTab", glyph: "plus", label: qsTr("在新标签页打开") },
                { id: "openWindow", glyph: "window", label: qsTr("在新窗口打开") },
                { separator: true }
            ]
            if (caps.eject)
                items.push({ id: "ejectDrive", glyph: "drive", label: qsTr("弹出"),
                             enabled: removable === true })
            if (caps.unmount)
                items.push({ id: "unmountDrive", glyph: "drive", label: qsTr("卸载"),
                             enabled: removable === true })
            if (caps.mount)
                items.push({ id: "mountDrive", glyph: "drive", label: qsTr("挂载") })
            if (caps.format)
                items.push({ id: "formatDrive", glyph: "archive", label: qsTr("格式化…"),
                             enabled: removable === true, danger: true })
            if (caps.cleanup)
                items.push({ id: "cleanupDrive", glyph: "refresh", label: qsTr("存储清理…") })
            if (caps.encryption)
                items.push({ id: "encryptionDrive", glyph: "info", label: qsTr("磁盘加密状态") })
            if (caps.previousVersions)
                items.push({ id: "previousVersionsDrive", glyph: "clock",
                             label: qsTr("以前的版本…") })
            items.push({ separator: true })
            items.push({ id: "copyPath", glyph: "copy", label: qsTr("复制路径") })
            menu.openAt(x, y, items)
            root.contextPath = path
            root.contextIsDir = true
            root.contextKind = "sidebar"
            root.contextTab = null
            root.contextList = null
            return
        }
        root.contextKind = "sidebar"
        root.contextTab = null
        root.contextList = null
        root.contextPath = path
        root.contextIsDir = true
        if (kind === "connection" || kind === "cloud") {
            menu.openAt(x, y, [
                { id: "open", glyph: kind === "connection" ? "network" : "cloud",
                  label: qsTr("打开") },
                { id: "openTab", glyph: "plus", label: qsTr("在新标签页打开") },
                { id: "openWindow", glyph: "window", label: qsTr("在新窗口打开") },
                { id: "copyPath", glyph: "copy", label: qsTr("复制路径") },
                { separator: true },
                { id: kind === "connection" ? "removeConnection" : "removeCloud",
                  glyph: "trash",
                  label: kind === "connection" ? qsTr("移除连接") : qsTr("移除云盘账户"),
                  danger: true }
            ])
            return
        }
        // Place rows carry their icon name after `place:`; the menu needs the
        // kind itself so it can offer "hide" and the model can resolve it back.
        const placeKind = kind.indexOf("place:") === 0
            ? kind.substring("place:".length) : ""
        const rowKind = placeKind !== "" ? "place" : kind
        const items = []
        if (rowKind === "favorite")
            items.push({ id: "removeFavorite", glyph: "star", label: qsTr("从收藏移除"),
                         danger: true })
        if (rowKind === "recent" && !root.isRemotePath(path))
            items.push({ id: "reveal", glyph: "folder", label: qsTr("在资源管理器中显示") })
        items.push({ id: "openTab", glyph: "plus", label: qsTr("在新标签页打开") })
        items.push({ id: "openWindow", glyph: "window", label: qsTr("在新窗口打开") })
        if (rowKind !== "favorite")
            items.push({ id: "favorite", glyph: "star", label: qsTr("加入收藏") })
        items.push({ separator: true })
        items.push({ id: "copyPath", glyph: "copy", label: qsTr("复制路径") })
        if (placeKind !== "") {
            items.push({ separator: true })
            items.push({ id: "hidePlace:" + placeKind, glyph: "eye",
                         label: qsTr("从侧栏隐藏"), danger: true })
        }
        menu.openAt(x, y, items)
    }

    /// Forgets which entry the menu was opened on. Every action that reaches
    /// the window through the menu ends here, including the ones that only
    /// needed the path for a moment.
    function clearContext() {
        root.contextTab = null
        root.contextList = null
        root.contextKind = ""
        root.contextName = ""
    }

    /// The action bar's verbs, on the live selection rather than on a menu's
    /// captured target: the bar describes what is selected right now, and a
    /// click that opened a menu a moment ago must not redirect it. The long
    /// tail stays in the context menu; this runs only what the footer shows.
    function runActionBarAction(id) {
        const list = root.actionList
        if (list === null)
            return
        const paths = list.selectedPathList()
        if (paths.length === 0)
            return
        const model = list.model
        const first = paths[0]
        const firstIndex = model !== null ? model.indexOfPath(first) : -1
        const firstIsDir = firstIndex >= 0 && model.isDirectoryAt(firstIndex)
        const many = paths.length > 1
        const directory = list.tab !== null && list.tab !== undefined
            ? list.tab.listing.path : root.activePath
        switch (id) {
        case "open":
            // One row: the same act as Enter. Several: open every file and
            // leave folders alone — navigating has no meaning for a set.
            if (!many && firstIndex >= 0) {
                list.handleActivate(firstIndex)
                break
            }
            for (let index = 0; index < paths.length; index += 1) {
                const at = model !== null ? model.indexOfPath(paths[index]) : -1
                if (at >= 0 && !model.isDirectoryAt(at))
                    Workspace.openPath(paths[index])
            }
            break
        case "cut":
            Workspace.setClipboard("cut", paths)
            break
        case "copy":
            Workspace.setClipboard("copy", paths)
            break
        case "paste":
            // A single folder is its own destination; anything else pastes
            // into the folder the bar belongs to.
            Workspace.pasteInto(!many && firstIsDir ? first : directory)
            break
        case "rename":
            if (many)
                bulkRenameDialog.open(paths)
            else
                root.promptRename(first, first.split(/[\\/]/).pop())
            break
        case "trash":
            if (root.remoteLocation) {
                root.requestRemoteDelete(paths)
                break
            }
            Workspace.trashEntries(paths)
            root.announceUndoable(many
                ? qsTr("已将 %1 项移到回收站").arg(paths.length)
                : qsTr("已移到回收站"))
            break
        case "moveTo":
            moveToDialog.open(!many && firstIsDir ? first : directory, paths)
            break
        case "compress":
            // The format picker lives in the menu; the bar's one verb is the
            // default archive, which is what most compressions are.
            Workspace.compressEntries(paths, directory, "zip", "")
            break
        }
    }

    /// The action bar's two menu buttons. 标签 reuses the row menu's tag rows;
    /// 更多 opens the row menu's second page. Either way the menu's target is
    /// seeded from the live selection first, because `runMenuAction` resolves
    /// against the captured context, not against the bar.
    function openActionBarMenu(kind, x, y) {
        const list = root.actionList
        if (list === null)
            return
        const paths = list.selectedPathList()
        if (paths.length === 0)
            return
        const model = list.model
        const index = model !== null ? model.indexOfPath(paths[0]) : -1
        root.contextTab = list === splitList ? root.splitTab : root.activeTab
        root.contextList = list
        root.contextPath = paths[0]
        root.contextIsDir = index >= 0 && model.isDirectoryAt(index)
        root.contextKind = "row"
        root.rowMenuMore = kind !== "tags"
        if (kind === "tags") {
            menu.openAt(x, y, root.tagSubmenu())
            return
        }
        shellCommands.load(root.currentSelection(), paths[0])
        menu.openAt(x, y, root.rowMoreMenuItems())
    }

    function runMenuAction(id) {
        if (id.indexOf("customAction:") === 0) {
            Workspace.runCustomAction(id.substring("customAction:".length),
                                      root.currentSelection(), root.contextDirectory())
            root.clearContext()
            return
        }
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
        if (id.indexOf("hidePlace:") === 0) {
            Workspace.hidePlace(id.substring("hidePlace:".length))
            root.clearContext()
            return
        }
        // The tag submenu toggles one tag over the captured selection.
        if (id.indexOf("tag:") === 0) {
            const tag = id.substring("tag:".length)
            const paths = root.currentSelection()
            if (tag === "__clear")
                TagStore.clearTags(paths)
            else if (tag !== "none")
                TagStore.toggleTag(paths, tag)
            root.clearContext()
            return
        }
        // The row menu's second page: the same panel, different rows, at the
        // coordinates it already occupies.
        if (id === "moreMenu") {
            root.rowMenuMore = true
            menu.openAt(menu.openX, menu.openY, root.rowMoreMenuItems())
            return
        }
        if (id === "menuBack") {
            root.rowMenuMore = false
            menu.openAt(menu.openX, menu.openY, root.rowMenuItems())
            return
        }
        if (id === "newSpaceAdd") {
            root.promptSpaceCreate(path)
            root.clearContext()
            return
        }
        switch (id) {
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
        case "share":
            root.shareSelection()
            break
        case "copyName":
            if (Workspace.copyText(root.contextName))
                toast.show(qsTr("已复制名称"), false)
            else
                toast.show(qsTr("无法写入剪贴板"), true)
            break
        case "editPath":
            root.beginPathEditing()
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
        case "compressZipEncrypted":
            root.startCompress("zip", true)
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
        case "extractTo":
            root.startExtractTo(path)
            break
        case "rename":
            root.promptRename(path, path.split(/[\\/]/).pop())
            break
        case "ejectDrive":
            Workspace.ejectDrive(path)
            break
        case "unmountDrive":
            Workspace.unmountDrive(path)
            break
        case "mountDrive":
            Workspace.mountDrive(path)
            break
        case "formatDrive":
            Workspace.formatDrive(path)
            break
        case "cleanupDrive":
            Workspace.openStorageCleanup()
            break
        case "encryptionDrive":
            confirmDialog.open(qsTr("磁盘加密状态"),
                               Workspace.driveEncryptionStatus(path), qsTr("关闭"), false)
            root.confirmMode = ""
            break
        case "previousVersionsDrive":
            Workspace.openPreviousVersions(path)
            break
        case "measureSize":
            Workspace.measureFolders(root.currentSelection())
            break
        case "compareHashes":
            Workspace.compareHashes(root.currentSelection())
            break
        case "tabDuplicate":
            root.duplicateTab(root.tabMenuIndex)
            break
        case "tabPin":
            root.toggleTabPin(root.tabMenuIndex)
            break
        case "tabClose":
            root.closeTab(root.tabMenuIndex)
            break
        case "tabCloseOthers":
            root.closeOtherTabs(root.tabMenuIndex)
            break
        case "tabCloseLeft":
            root.closeTabsBeside(root.tabMenuIndex, -1)
            break
        case "tabCloseRight":
            root.closeTabsBeside(root.tabMenuIndex, 1)
            break
        case "tabReopen":
            root.reopenClosedTab()
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
        if (focus !== "") {
            fileList.focusPath(focus)
            // §7.1.7: the row a creation just landed on keeps one beat of
            // highlight. A rename is not a new row, so it only takes focus.
            if (kind === "create")
                fileList.flashRow(focus)
        }
    }

    function modalClosed() {
        Qt.callLater(function () {
            if (root.modalOpen)
                return
            // "Edit path" opened the editor from the bar's menu. The menu is
            // gone by now, so the field can take the caret that would otherwise
            // fall through to the listing below.
            if (root.pathEditing) {
                locator.focusAddress()
                return
            }
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
        if (root.terminalActive) {
            root.terminalLoaded = true
            Qt.callLater(root.focusTerminalPanel)
        } else if (root.trashActive) {
            trashView.takeFocus()
        } else {
            fileList.takeFocus()
        }
    }

    function focusTerminalPanel() {
        const panel = terminalLoader.item as TerminalPanel
        if (panel !== null)
            panel.focusTerminal()
    }

    Connections {
        target: Spaces

        function onChanged() {
            root.reconcileSpaces()
            // The landing page lists spaces too; refresh it in place so a
            // rename or a new space is not a stale row.
            overviewModelInstance.refresh()
            if (commandPalette.shown)
                commandPalette.rebuild()
        }
    }

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            overviewModelInstance.refresh()
        }

        function onPlacesChanged() {
            overviewModelInstance.refresh()
        }

        function onOperationFinished(kind, focus) {
            if (kind === "openWith" && openWithDialog.shown) {
                openWithDialog.close()
                root.modalClosed()
            }
            if (kind === "compress" || kind === "extract") {
                root.resetArchiveState()
                if (passwordDialog.shown) {
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

        function onCompareFinished(equal) {
            toast.show(equal
                ? qsTr("SHA-256 相同：两个文件内容一致")
                : qsTr("SHA-256 不同：两个文件内容不一致"), !equal)
        }

        function onFolderSizesMeasured(sizes) {
            let entries = []
            try {
                entries = JSON.parse(sizes)
            } catch (error) {
                return
            }
            for (let index = 0; index < entries.length; ++index)
                root.applyFolderSize(entries[index].path, entries[index].size)
        }

        function onOperationFailed(kind, message) {
            if (kind === "openWith" && openWithDialog.shown) {
                openWithDialog.showError(message)
                return
            }
            if (passwordDialog.shown && kind === root.archiveMode) {
                passwordDialog.showError(message)
                return
            }
            if (kind === "compress" || kind === "extract")
                root.resetArchiveState()
            toast.show(root.localizeBackend(message), true)
        }

        function onArchivePasswordRequired(target, message) {
            root.requestArchivePassword(target, message)
        }

        function onConnectionTestFinished(ok, message) {
            if (connectDialog.shown)
                connectDialog.applyTestResult(ok, message)
        }

        function onConnectionSaveFinished(ok, id, message) {
            if (!ok) {
                if (connectDialog.shown)
                    connectDialog.applySaveError(message)
                else
                    toast.show(root.localizeBackend(message), true)
                return
            }
            if (connectDialog.shown) {
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
                if (cloudDialog.shown)
                    cloudDialog.applyError(message)
                else
                    toast.show(root.localizeBackend(message), true)
                return
            }
            if (cloudDialog.shown) {
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
            if (operation === "create" && gitPanel.shown && gitPanel.creating)
                return
            toast.show(root.localizeBackend(message), true)
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

    /// A user image behind the listing. The listing surfaces switch to
    /// `Theme.content` (a translucent canvas) while this is set, so the image
    /// shows through at the contrast guard the settings page exposes. Chrome
    /// stays solid; that is what keeps the frame and its labels readable over
    /// an arbitrary photograph.
    Image {
        anchors.fill: parent
        visible: ExplorerPrefs.backgroundImage !== ""
        source: root.backgroundImageSource(ExplorerPrefs.backgroundImage)
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        cache: false
        smooth: true
    }

    Column {
        anchors.fill: parent

        // The chrome bar is one row: mark, tabs, window buttons. 38 dp is the
        // number `kTitleBarDp` in crates/dae-qt/cpp/window.h hard-codes for the
        // native hit test, so the two move together.
        Item {
            id: chromeBar
            width: parent.width
            height: Theme.shellTitle

            TopBar {
                id: topBar
                anchors.fill: parent
                window: root
                controller: chrome
                dark: root.dark
                material: root.material
                title: root.title
                tabs: root.tabs
                activeIndex: root.activeIndex
                hostWindow: root
                accent: root.accent

                // Native hit test input. Without this the chips sit inside the
                // caption and a click on a tab starts a window drag instead of
                // activating it. The bar is full-width, so its own numbers are
                // already window coordinates; the native setter clamps the
                // right edge against the button columns.
                function pushCaptionBand() {
                    if (chrome.attached)
                        chrome.setCaptionClient(clientLeft, clientRight)
                }
                onClientRightChanged: pushCaptionBand()

                onTabActivated: function (index) {
                    root.activeIndex = index
                }
                onTabClosed: function (index) {
                    root.closeTab(index)
                }
                onNewTabRequested: root.addTabHere()
                onTabContextRequested: function (index, x, y) {
                    root.showTabMenu(index, x, y)
                }
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

            // The toolbar is a model, not a fixed row: every action is a
            // Loader over `toolbarLeadingItems`/`toolbarTrailingItems`, so a
            // drag reorders it and the order survives a restart. The nav
            // buttons keep their box when they cannot act, so the address bar
            // never shifts under the pointer.
            Row {
                id: leadingToolbar
                anchors.left: parent.left
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                // A positioner does not take its implicit width as `width`;
                // the path field anchors to this row's right edge.
                width: implicitWidth
                spacing: 2

                Repeater {
                    id: leadingToolbarRepeater
                    model: root.toolbarLeadingItems
                    delegate: Loader {
                        id: leadingToolbarLoader
                        required property int index
                        required property string modelData
                        // The components this picks from are declared later in
                        // the path bar; a one-time binding evaluated during
                        // creation would see them as undefined.
                        Component.onCompleted: Qt.callLater(function () {
                            sourceComponent = pathBar.toolbarComponent(modelData)
                        })

                        DragHandler {
                            target: null
                            xAxis.enabled: true
                            yAxis.enabled: false
                            property real pressX: 0
                            onActiveChanged: {
                                if (active) {
                                    pressX = centroid.scenePosition.x
                                } else {
                                    root.finishToolbarDrag("leading",
                                                           leadingToolbarLoader.index,
                                                           pressX + activeTranslation.x)
                                }
                            }
                        }
                    }
                }
            }

            // Separates navigation from the path itself, the way the
            // instrument's control cluster is set apart from its readout.
            Rectangle {
                id: navSeparator
                anchors.left: leadingToolbar.right
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
                anchors.right: trailingToolbar.left
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                // §7.2: the one input control in the row is 32 dp tall.
                height: Theme.controlLg

                // One frame, three modes. The trail navigates, a click on its
                // empty part or Mod+L becomes the path editor, Mod+F becomes
                // search; the shell owns which mode is on.
                Locator {
                    id: locator
                    anchors.fill: parent
                    visible: root.folderActive
                    mode: root.searchOpen ? "search"
                        : (root.pathEditing ? "address" : "breadcrumb")
                    interactive: root.folderActive
                    names: root.activeTab !== null
                        ? root.activeTab.listing.breadcrumbNames : []
                    paths: root.activeTab !== null
                        ? root.activeTab.listing.breadcrumbPaths : []
                    dark: root.dark
                    accent: root.accent
                    textColor: root.textColor
                    mutedColor: root.mutedColor
                    addressPath: root.activePath
                    searchMode: root.searchMode
                    searchBusy: root.searchBusy
                    searchText: root.searchQuery

                    onCrumbActivated: function (path) {
                        root.navigate(path)
                    }
                    onAncestorsRequested: function (x, y, names, paths) {
                        root.openBreadcrumbMenu(x, y, names, paths, false)
                    }
                    onContextMenuRequested: function (x, y) {
                        root.openBreadcrumbBarMenu(x, y, false)
                    }
                    onAddressRequested: root.beginPathEditing()
                    onAddressAccepted: function (path) {
                        root.navigate(path)
                    }
                    onAddressCanceled: root.pathEditing = false
                    onSearchEdited: function (value) {
                        root.acceptSearchText(value)
                    }
                    onSearchCleared: root.acceptSearchText("")
                    onSearchDismissed: root.closeSearch()
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
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                }
            }

            // The trailing actions are the same model as the leading ones,
            // mirrored on the other side of the address bar.
            Row {
                id: trailingToolbar
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                width: implicitWidth
                spacing: 2

                Repeater {
                    id: trailingToolbarRepeater
                    model: root.toolbarTrailingItems
                    delegate: Loader {
                        id: trailingToolbarLoader
                        required property int index
                        required property string modelData
                        // The components this picks from are declared later in
                        // the path bar; a one-time binding evaluated during
                        // creation would see them as undefined.
                        Component.onCompleted: Qt.callLater(function () {
                            sourceComponent = pathBar.toolbarComponent(modelData)
                        })

                        DragHandler {
                            target: null
                            xAxis.enabled: true
                            yAxis.enabled: false
                            property real pressX: 0
                            onActiveChanged: {
                                if (active) {
                                    pressX = centroid.scenePosition.x
                                } else {
                                    root.finishToolbarDrag("trailing",
                                                           trailingToolbarLoader.index,
                                                           pressX + activeTranslation.x)
                                }
                            }
                        }
                    }
                }
            }

            /// One component per action id. The stored order may name an id a
            /// future build removed; `toolbarIds` drops it on the next sync.
            function toolbarComponent(id) {
                switch (id) {
                case "back": return backToolbarComponent
                case "forward": return forwardToolbarComponent
                case "up": return upToolbarComponent
                case "search": return searchToolbarComponent
                case "trash": return trashToolbarComponent
                case "view": return viewToolbarComponent
                case "more": return moreToolbarComponent
                }
                return null
            }

            Component {
                id: backToolbarComponent
                Rectangle {
                    width: 28
                    height: Theme.controlMd
                    implicitWidth: width
                    implicitHeight: height
                    radius: Theme.radiusSm
                    color: root.canGoBack && backMouse.containsMouse
                        ? Theme.hover : "transparent"
                    opacity: root.canGoBack ? 1 : 0.32

                    readonly property bool canGo: root.canGoBack

                    Glyph {
                        anchors.centerIn: parent
                        width: 14
                        height: 14
                        kind: "arrowLeft"
                        tint: backMouse.containsMouse && parent.canGo
                            ? root.textColor : root.mutedColor
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("后退")

                    /// A nav button is a tab stop, and Enter is its click. The
                    /// ring is drawn inside the 28 px box so the button does
                    /// not move when it takes focus.
                    activeFocusOnTab: canGo
                    border.width: activeFocus ? 1 : 0
                    border.color: Theme.accent
                    Keys.onReturnPressed: {
                        if (canGo)
                            root.goBack()
                    }
                    Keys.onSpacePressed: {
                        if (canGo)
                            root.goBack()
                    }

                    MouseArea {
                        id: backMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: parent.canGo
                        onClicked: root.goBack()
                    }
                }
            }

            Component {
                id: forwardToolbarComponent
                Rectangle {
                    width: 28
                    height: Theme.controlMd
                    implicitWidth: width
                    implicitHeight: height
                    radius: Theme.radiusSm
                    color: root.canGoForward && forwardMouse.containsMouse
                        ? Theme.hover : "transparent"
                    opacity: root.canGoForward ? 1 : 0.32

                    readonly property bool canGo: root.canGoForward

                    Glyph {
                        anchors.centerIn: parent
                        width: 14
                        height: 14
                        kind: "arrowRight"
                        tint: forwardMouse.containsMouse && parent.canGo
                            ? root.textColor : root.mutedColor
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("前进")

                    activeFocusOnTab: canGo
                    border.width: activeFocus ? 1 : 0
                    border.color: Theme.accent
                    Keys.onReturnPressed: {
                        if (canGo)
                            root.goForward()
                    }
                    Keys.onSpacePressed: {
                        if (canGo)
                            root.goForward()
                    }

                    MouseArea {
                        id: forwardMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: parent.canGo
                        onClicked: root.goForward()
                    }
                }
            }

            Component {
                id: upToolbarComponent
                Rectangle {
                    width: 28
                    height: Theme.controlMd
                    implicitWidth: width
                    implicitHeight: height
                    radius: Theme.radiusSm
                    color: root.folderActive && upMouse.containsMouse
                        ? Theme.hover : "transparent"
                    opacity: root.folderActive ? 1 : 0.32

                    Glyph {
                        anchors.centerIn: parent
                        width: 14
                        height: 14
                        kind: "arrowUp"
                        tint: upMouse.containsMouse && root.folderActive
                            ? root.textColor : root.mutedColor
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("上一级")

                    activeFocusOnTab: root.folderActive
                    border.width: activeFocus ? 1 : 0
                    border.color: Theme.accent
                    Keys.onReturnPressed: {
                        if (root.folderActive)
                            root.goUp()
                    }
                    Keys.onSpacePressed: {
                        if (root.folderActive)
                            root.goUp()
                    }

                    MouseArea {
                        id: upMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: root.folderActive
                        onClicked: root.goUp()
                    }
                }
            }

            // ── Search ────────────────────────────────────────────────────
            // The field lives in the locator now (§7.2): this button switches
            // that one control into search mode, so the bar never carries two
            // text fields that could disagree about the query.
            Component {
                id: searchToolbarComponent
                Item {
                    id: searchSlot

                    height: Theme.controlMd
                    width: 28
                    implicitWidth: width
                    implicitHeight: height
                    visible: root.folderActive

                    /// Search walks the local disk only. Keep the affordance in
                    /// place so it does not flicker in and out while crossing
                    /// locations, but dim and disable it on network paths.
                    opacity: root.remoteLocation ? 0.35 : 1

                    activeFocusOnTab: !root.remoteLocation
                    Keys.onReturnPressed: root.openSearch()
                    Keys.onSpacePressed: root.openSearch()

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusSm
                        color: root.searchOpen || searchIconMouse.containsMouse
                            ? Theme.hover : "transparent"
                        border.width: searchSlot.activeFocus ? 1 : 0
                        border.color: Theme.accent

                        Glyph {
                            anchors.centerIn: parent
                            kind: "search"
                            tint: root.searchOpen ? Theme.accent
                                : (searchIconMouse.containsMouse && !root.remoteLocation
                                    ? root.textColor : root.mutedColor)
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("搜索")

                        MouseArea {
                            id: searchIconMouse
                            anchors.fill: parent
                            enabled: !root.remoteLocation
                            hoverEnabled: true
                            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                            onClicked: root.openSearch()
                        }
                    }
                }
            }

            // ── Trash ─────────────────────────────────────────────────────
            // Emptying the bin is a text action that only exists here, and only
            // while there is something to empty. The hover is the only place it
            // turns dangerous.
            Component {
                id: trashToolbarComponent
                Rectangle {
                    id: trashClear
                    width: visible ? 44 : 0
                    height: Theme.controlSm
                    implicitWidth: width
                    implicitHeight: height
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

                    activeFocusOnTab: trashClear.visible
                    border.width: trashClear.activeFocus ? 1 : 0
                    border.color: Theme.accent
                    Keys.onReturnPressed: root.requestEmptyTrash()
                    Keys.onSpacePressed: root.requestEmptyTrash()

                    MouseArea {
                        id: trashClearMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.requestEmptyTrash()
                    }
                }
            }

            // ── View ──────────────────────────────────────────────────────
            // One button showing the view that is on, opening the one display
            // menu.
            Component {
                id: viewToolbarComponent
                Item {
                    id: viewSlot
                    width: 28
                    height: Theme.controlMd
                    implicitWidth: width
                    implicitHeight: height
                    visible: root.folderActive

                    /// The slot takes focus, the plate inside draws the ring: the
                    /// 28 px box is the button even though the visible plate is
                    /// one rectangle deeper.
                    activeFocusOnTab: root.folderActive
                    Keys.onReturnPressed: viewSlot.openMenu()
                    Keys.onSpacePressed: viewSlot.openMenu()

                    function openMenu() {
                        root.openDisplayMenu()
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusSm
                        color: viewMouse.containsMouse || displayMenu.visible
                            ? Theme.hover : "transparent"
                        border.width: viewSlot.activeFocus ? 1 : 0
                        border.color: Theme.accent

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
                        onClicked: viewSlot.openMenu()
                    }
                }
            }

            // ── More ──────────────────────────────────────────────────────
            // Refresh, the visibility toggles, filters and settings live here
            // now that the sidebar gives up its hamburger and the status bar
            // gives up its settings text.
            Component {
                id: moreToolbarComponent
                Item {
                    id: moreSlot
                    width: 28
                    height: Theme.controlMd
                    implicitWidth: width
                    implicitHeight: height

                    /// Same split as the view button: the slot is the tab stop,
                    /// the plate draws the ring.
                    activeFocusOnTab: true
                    Keys.onReturnPressed: moreSlot.openMenu()
                    Keys.onSpacePressed: moreSlot.openMenu()

                    function openMenu() {
                        root.openMoreMenu()
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusSm
                        color: moreMouse.containsMouse || moreMenu.visible
                            ? Theme.hover : "transparent"
                        border.width: moreSlot.activeFocus ? 1 : 0
                        border.color: Theme.accent

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
                        onClicked: moreSlot.openMenu()
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
            visible: root.folderActive && !root.remoteLocation
                && (root.searchOpen || root.searchQuery.length > 0)

            mode: root.searchMode
            contentEnabled: !root.remoteLocation
            caseSensitive: root.searchCase
            regex: root.searchRegex
            typeId: root.searchType
            scope: root.searchScope
            hasHistory: root.searchHistoryEntries.length > 0
            busy: root.searchBusy
            matchCount: root.searchMatchCount
            query: root.searchQuery.trim()

            onModeRequested: function (mode) { root.setSearchMode(mode) }
            onHistoryRequested: function (x, y) {
                searchHistoryMenu.openAt(x, y, root.searchHistoryItems())
            }
            onSaveRequested: root.requestSaveSearch()
            onFiltersRequested: function (x, y) {
                filterMenu.openAbove(x, y)
            }
            onScopeRequested: function (scope) {
                if (root.searchScope === scope)
                    return
                root.searchScope = scope
                if (root.searchActive)
                    searchTimer.restart()
            }
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
                - root.footerHeight - searchFilterBar.height

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
                    Accessible.ignored: true
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
                onNavigateRequested: function (path) {
                    root.navigate(path)
                }
                onTrashRequested: root.openTrash()
                onTagActivated: function (name) {
                    root.searchTag(name)
                }
                onSavedSearchActivated: function (id) {
                    root.applySavedSearch(id)
                }
                onSavedSearchRemoveRequested: function (id) {
                    ExplorerPrefs.deleteSavedSearch(id)
                }
                onAddRequested: function (kind) {
                    if (kind === "cloud")
                        root.openCloudDialog()
                    else
                        root.openConnectDialog()
                }
                onRestorePlacesRequested: Workspace.showAllPlaces()
                onContextRequested: function (kind, path, removable, x, y) {
                    root.showSidebarMenu(kind, path, removable, x, y)
                }
                onDiscoveredActivated: function (protocol, host, port) {
                    connectDialog.openWith(protocol, host, port)
                }
                onErrorRequested: function (message) {
                    toast.show(root.localizeBackend(message), true)
                }
            }

            Column {
                width: parent.width - sidebar.width - sidebarHandle.width
                    - previewPanel.width - previewHandle.width
                height: parent.height

                Item {
                    width: parent.width
                    height: parent.height - terminalLoader.height

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
                            height: Theme.controlLg
                            color: root.chromeColor

                            Text {
                                id: splitClose
                                anchors.right: parent.right
                                anchors.rightMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                text: "\u00D7"
                                color: splitCloseMouse.containsMouse
                                    ? root.textColor : root.mutedColor
                                font.pixelSize: Theme.typeBody
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
                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("路径栏")
                                anchors.fill: parent
                                hoverEnabled: true
                                acceptedButtons: Qt.LeftButton | Qt.RightButton
                                onClicked: function (mouse) {
                                    if (mouse.button === Qt.RightButton) {
                                        const point = mapToItem(null, mouse.x, mouse.y)
                                        root.openBreadcrumbBarMenu(point.x, point.y, true)
                                    } else if (splitCloseMouse.containsMouse) {
                                        ExplorerPrefs.setBool("splitEnabled", false)
                                    } else {
                                        splitList.takeFocus()
                                    }
                                }
                            }

                            Locator {
                                anchors.left: parent.left
                                anchors.leftMargin: 8
                                anchors.right: splitClose.left
                                anchors.rightMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                height: Theme.controlSm
                                // The split pane keeps a read-only trail: the
                                // left click still belongs to the pane's focus
                                // target.
                                framed: false
                                interactive: false
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
                                onContextMenuRequested: function (x, y) {
                                    root.openBreadcrumbBarMenu(x, y, true)
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

                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("关闭分屏")
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
                            /// The split divider is a drag strip between the two panes. There is no
                            /// keyboard command for the ratio, so the strip stays out of the tree.
                            Accessible.ignored: true
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

                        onPreviewRequested: function (path) {
                            previewPanel.showPaths([path])
                        }
                    }

                    LazySurface {
                        id: trashView
                        anchors.fill: parent
                        // A view, not a dialog: created the first time the
                        // content stack shows it, and kept afterwards.
                        visible: root.trashActive
                        loadWhenShown: root.trashActive
                        surface: Component {
                            TrashView {
                                anchors.fill: parent
                                model: trashModel
                                headerColor: Theme.canvas

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
                        }

                        // Reached from keyboard handlers that can fire before
                        // the surface has ever been shown.
                        readonly property int selectionCount: content !== null ? content.selectionCount : 0
                        function selectedIdList() {
                            return content !== null ? content.selectedIdList() : [];
                        }
                        function clearSelection() {
                            if (content !== null)
                                content.clearSelection();
                        }
                        function takeFocus() {
                            if (content !== null)
                                content.takeFocus();
                        }
                    }

                    OverviewView {
                        id: overviewView
                        anchors.fill: parent
                        visible: root.overviewActive
                        model: overviewModelInstance
                        accent: root.accent
                        textColor: root.textColor
                        mutedColor: root.mutedColor
                        hairline: root.hairline
                        commandShortcut: root.shortcutDisplay("app.commandBar")

                        onOpenRequested: function (kind, path) {
                            if (kind === "space")
                                root.openSpace(path)
                            else if (kind === "tag")
                                root.searchTag(path)
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
                            toast.show(root.localizeBackend(message), true)
                        }
                    }
                }

                // Built on first use, not with the window: see `terminalLoaded`.
                // Kept alive afterwards, like the shell it shows.
                Loader {
                    id: terminalLoader
                    width: parent.width
                    active: root.terminalLoaded
                    sourceComponent: terminalPanelComponent
                }

                Component {
                    id: terminalPanelComponent

                    TerminalPanel {
                        width: parent ? parent.width : 0
                        active: root.folderActive && root.terminalActive
                        tab: root.activeTab
                        dark: root.dark
                        material: root.material
                        onExited: {
                            fileList.takeFocus()
                        }
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
                    && root.width >= 840

                width: live ? 6 : 0
                height: parent.height

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 1
                    height: parent.height
                    color: handleMouse.containsMouse ? Theme.accent : Theme.hairline
                }

                MouseArea {
                    id: handleMouse
                    Accessible.ignored: true
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
                    && root.width >= 840
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
            // A selection swaps this strip for the action bar at the same edge.
            // The column still stacks both, so the hidden one takes no height
            // and the 36 dp one lands exactly where the 28 dp strip was.
            //
            // The strip stays `visible` at height 0 instead of being hidden:
            // a positioner skips invisible items entirely, and the body above
            // still reserves the strip's height, which would leave a dead band
            // at the bottom of the window. Clipping at zero height paints
            // nothing and keeps the column arithmetic exact.
            height: root.actionActive ? 0 : Theme.shellStatus
            clip: true
            // A clipped strip has no focusable children: without this the
            // transfer chip and the pane toggles would still answer Tab while
            // the action bar is the footer on screen.
            enabled: !root.actionActive
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
                        return overviewModelInstance.projectBusy
                            ? qsTr("查找项目…")
                            : (overviewModelInstance.loaded
                                ? overviewModelInstance.count + qsTr(" 个入口")
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
                color: Theme.inkSecondary
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeCaption
                font.features: { "tnum": 1 }
                elide: Text.ElideRight
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }

            /// What is on the clipboard, while something is. The in-app
            /// clipboard outlives the menu that filled it, so the footer is the
            /// only place that says a paste is still possible.
            Row {
                id: clipboardChip
                anchors.left: statusText.right
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                visible: Workspace.clipboardMode !== "" && root.width > 900
                spacing: 5

                Rectangle {
                    width: 6
                    height: 12
                    color: Theme.accent
                }

                Text {
                    /// QStringList exposes `.length` to QML; qmllint's type
                    /// model does not (same caveat as the transfer chip).
                    // qmllint disable missing-property
                    text: qsTr("剪贴板") + " " + Workspace.clipboardPaths.length
                        + " · " + (Workspace.clipboardMode === "cut"
                                   ? qsTr("剪切") : qsTr("复制"))
                    // qmllint enable missing-property
                    color: Theme.inkSecondary
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeCaption
                    font.features: { "tnum": 1 }
                }
            }

            // ── Centre: the running transfer ──────────────────────────────
            // The capsule is shared with the action bar (TransferChip.qml):
            // both footer states must keep the queue's only door visible, and
            // a paste is exactly what a selection leads to.
            TransferChip {
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                onOpenRequested: function (x, y) { transferQueue.openAbove(x, y) }
            }

            /// The block cursor at the end of the strip. drift ends its footer
            /// with one, and it is the only moving thing in the frame: a
            /// reminder that the listing takes typing (type-ahead, the search
            /// bar) without spending a word on saying so.
            Rectangle {
                id: statusCursor
                anchors.right: statusRight.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 7
                height: 13
                color: Theme.accent

                /// Reduced motion turns the blink off rather than shortening
                /// it: a cursor that fades in 110 ms still blinks.
                SequentialAnimation on opacity {
                    running: Theme.motion
                    loops: Animation.Infinite
                    NumberAnimation { to: 0; duration: 1 }
                    PauseAnimation { duration: 550 }
                    NumberAnimation { to: 1; duration: 1 }
                    PauseAnimation { duration: 550 }
                }
            }

            // ── Right: repository and pane switches ───────────────────────
            Row {
                id: statusRight
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                // The status center's door. The badge is the unread failure
                // count; opening the panel clears it, the rows stay until the
                // user clears the history.
                Item {
                    id: historyChip
                    width: historyRow.implicitWidth + 12
                    height: Theme.controlSm
                    visible: Workspace.operationsJson !== "[]"

                    Accessible.role: Accessible.Button
                    Accessible.name: Workspace.failureCount > 0
                        ? qsTr("状态中心，%1 项失败").arg(Workspace.failureCount)
                        : qsTr("状态中心")
                    Accessible.onPressAction: historyChip.openPanel()

                    activeFocusOnTab: historyChip.visible
                    Keys.onReturnPressed: historyChip.openPanel()
                    Keys.onSpacePressed: historyChip.openPanel()

                    function openPanel() {
                        const point = historyChip.mapToItem(null, 0, 0)
                        statusCenter.openAbove(point.x, point.y)
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: historyMouse.containsMouse || statusCenter.visible
                            ? Theme.hover : "transparent"
                    }

                    Row {
                        id: historyRow
                        anchors.centerIn: parent
                        spacing: 4

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            kind: "clock"
                            implicitWidth: 12
                            implicitHeight: 12
                            tint: Workspace.failureCount > 0 ? Theme.danger : Theme.inkSecondary
                        }

                        Text {
                            visible: Workspace.failureCount > 0
                            anchors.verticalCenter: parent.verticalCenter
                            text: Workspace.failureCount
                            color: Theme.danger
                            font.pixelSize: Theme.typeMicro
                            font.family: Theme.monoFamily
                        }
                    }

                    MouseArea {
                        id: historyMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: historyChip.openPanel()
                    }
                }

                // The branch is a name, not a pill: the colour in this bar
                // belongs to the signal and to nothing else.
                Item {
                    id: gitStatus
                    width: gitStatusRow.implicitWidth + 8
                    height: Theme.controlSm
                    visible: root.gitVisible

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("分支 %1，打开分支面板")
                        .arg(root.gitListing !== null ? root.gitListing.gitBranch : "")
                    Accessible.onPressAction: gitStatus.openPanel()

                    activeFocusOnTab: gitStatus.visible
                    Keys.onReturnPressed: gitStatus.openPanel()
                    Keys.onSpacePressed: gitStatus.openPanel()

                    Rectangle {
                        anchors.fill: parent
                        visible: gitStatus.activeFocus
                        color: "transparent"
                        border.width: 1
                        border.color: Theme.accent
                        radius: Theme.radiusXs
                    }

                    function openPanel() {
                        const point = gitStatus.mapToItem(null, 0, 0)
                        gitPanel.openAbove(point.x, point.y)
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: gitStatusMouse.containsMouse || gitPanel.shown
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
                            tint: Theme.inkSecondary
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: Math.min(implicitWidth, 120)
                            text: root.gitListing !== null ? root.gitListing.gitBranch : ""
                            color: root.gitListing !== null && root.gitListing.gitDetached
                                ? Theme.inkTertiary : Theme.inkSecondary
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
                    height: Theme.controlSm
                    visible: root.folderActive || root.trashActive

                    Accessible.role: Accessible.Button
                    Accessible.name: root.previewVisible ? qsTr("隐藏预览") : qsTr("显示预览")

                    /// The status bar closes the tab chain, so its two pane
                    /// switches are the last stops: Enter flips them the way a
                    /// click does.
                    activeFocusOnTab: previewToggle.visible
                    Keys.onReturnPressed: previewToggle.flip()
                    Keys.onSpacePressed: previewToggle.flip()

                    function flip() {
                        root.previewVisible = !root.previewVisible
                        if (root.previewVisible)
                            return
                        if (root.trashActive)
                            trashView.takeFocus()
                        else
                            fileList.takeFocus()
                    }

                    Rectangle {
                        anchors.fill: parent
                        visible: previewToggle.activeFocus
                        color: "transparent"
                        border.width: 1
                        border.color: Theme.accent
                        radius: Theme.radiusXs
                    }

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
                            tint: root.previewVisible ? Theme.ink : Theme.inkTertiary
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("预览")
                            color: root.previewVisible ? Theme.ink : Theme.inkTertiary
                            font.pixelSize: Theme.typeMicro
                        }
                    }

                    MouseArea {
                        id: previewToggleMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: previewToggle.flip()
                    }
                }

                Item {
                    id: terminalToggle
                    width: terminalToggleRow.implicitWidth + 12
                    height: Theme.controlSm
                    visible: root.folderActive
                    opacity: root.remoteLocation ? 0.35 : 1

                    Accessible.role: Accessible.Button
                    Accessible.name: root.terminalActive ? qsTr("隐藏终端") : qsTr("显示终端")

                    activeFocusOnTab: terminalToggle.visible && !root.remoteLocation
                    Keys.onReturnPressed: root.toggleTerminal()
                    Keys.onSpacePressed: root.toggleTerminal()

                    Rectangle {
                        anchors.fill: parent
                        visible: terminalToggle.activeFocus
                        color: "transparent"
                        border.width: 1
                        border.color: Theme.accent
                        radius: Theme.radiusXs
                    }

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
                            tint: root.terminalActive ? Theme.ink : Theme.inkTertiary
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("终端")
                            color: root.terminalActive ? Theme.ink : Theme.inkTertiary
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

        // The same edge, the other state: with a selection the 28 dp strip is
        // gone and this 36 dp bar stands exactly where it was. Both are
        // children of the column, so `root.footerHeight` decides how much of
        // the window the body above them may use.
        ActionBar {
            id: actionBar
            width: parent.width
            height: root.actionActive ? Theme.actionBar : 0
            visible: root.actionActive
            background: root.chromeColor
            count: root.actionCount
            remote: root.remoteLocation
            clipboardReady: Workspace.clipboardMode !== ""

            onActionRequested: function (id) { root.runActionBarAction(id) }
            onMenuRequested: function (kind, x, y) { root.openActionBarMenu(kind, x, y) }
            onTransferRequested: function (x, y) { transferQueue.openAbove(x, y) }
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
                "onStrip": drag.y >= topBar.y
                    && drag.y < topBar.y + topBar.height
            }
        }

        onEntered: function (drag) {
            const tab = draggedTab(drag)
            if (tab === null) {
                drag.accepted = false
                return
            }
            drag.accept(Qt.MoveAction)
            topBar.showDrop(tab.onStrip ? drag.x : -1)
        }
        onPositionChanged: function (drag) {
            const tab = draggedTab(drag)
            if (tab === null) {
                drag.accepted = false
                return
            }
            drag.accept(Qt.MoveAction)
            topBar.showDrop(tab.onStrip ? drag.x : -1)
        }
        onExited: topBar.clearDrop()
        onDropped: function (drop) {
            const tab = draggedTab(drop)
            topBar.clearDrop()
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
        id: searchHistoryMenu
        anchors.fill: parent
        dark: root.dark

        onActionTriggered: function (id) {
            root.runHistoryEntry(id)
        }
    }

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
        sequence: root.shortcutSequence("explorer.displayOptions")
        enabled: sequence !== ""
        onActivated: root.openDisplayMenu()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.filters")
        enabled: sequence !== ""
        onActivated: {
            // The filter menu lost its status-bar button; it now opens under
            // the nav bar's overflow button, where the rest of the listing
            // switches live.
            root.openFiltersMenu()
        }
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.toggleSplit")
        enabled: sequence !== ""
        onActivated: ExplorerPrefs.setBool("splitEnabled", !ExplorerPrefs.splitEnabled)
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.properties")
        enabled: !root.modalOpen && !root.pathEditing && !root.trashActive
                  && sequence !== ""
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
            menu.replaceActions(root.rowMenuMore
                ? root.rowMoreMenuItems() : root.rowMenuItems())
    }

    Connections {
        target: TagStore
        function onTagsChanged(paths) {
            root.syncTagsToLists(paths)
        }
        function onDefinitionsChanged() {
            root.syncTagDefinitions()
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

    /// The installed apps' right-click commands. One model per window: the
    /// rows follow this window's selection, and a second window has its own.
    ShellCommandModel {
        id: shellCommands
    }

    LazySurface {
        id: propertiesDialog
        anchors.fill: parent
        z: 1005
        surface: Component {
            PropertiesDialog {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent

                onChanged: root.reloadAfterOperation("properties", "")
                onClosed: root.modalClosed()
            }
        }
        function open(path) {
            instantiate().open(path);
        }
        function close() {
            if (content !== null)
                content.close();
        }
    }

    LazySurface {
        id: bulkRenameDialog
        anchors.fill: parent
        z: 1006
        surface: Component {
            BulkRenameDialog {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent

                onClosed: root.modalClosed()
            }
        }
        function open(paths) {
            instantiate().open(paths);
        }
    }

    LazySurface {
        id: moveToDialog
        anchors.fill: parent
        z: 1007
        surface: Component {
            MoveToDialog {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent

                onClosed: root.modalClosed()
            }
        }
        function open(destination, moving) {
            instantiate().open(destination, moving);
        }
        function openExtract(destination, source) {
            const dialog = instantiate()
            dialog.extractSource = source
            dialog.open(destination, [])
        }
    }

    LazySurface {
        id: namePrompt
        anchors.fill: parent
        z: 1001
        surface: Component {
            NamePrompt {
                anchors.fill: parent
                dark: root.dark

                onAccepted: function (text) {
                    root.acceptName(text)
                    root.modalClosed()
                }
                onCanceled: root.modalClosed()
            }
        }
        function open(title, initial, placeholder) {
            instantiate().open(title, initial, placeholder);
        }
        function close() {
            if (content !== null)
                content.close();
        }
    }

    LazySurface {
        id: confirmDialog
        anchors.fill: parent
        z: 1002
        surface: Component {
            ConfirmDialog {
                anchors.fill: parent
                dark: root.dark

                onAccepted: root.acceptConfirm()
                onCanceled: {
                    root.confirmMode = ""
                    root.confirmTrashIds = []
                    root.modalClosed()
                }
            }
        }
        function open(title, text, label, isDanger) {
            instantiate().open(title, text, label, isDanger);
        }
        function close() {
            if (content !== null)
                content.close();
        }
    }

    LazySurface {
        id: passwordDialog
        anchors.fill: parent
        z: 1003
        surface: Component {
            ArchivePasswordDialog {
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
        }
        function open(mode, archiveName, error) {
            instantiate().open(mode, archiveName, error);
        }
        function close() {
            if (content !== null)
                content.close();
        }
        // A failed open arrives while the dialog is up, so the surface exists.
        function showError(message) {
            if (content !== null)
                content.showError(message);
        }
    }

    LazySurface {
        id: settingsPanel
        anchors.fill: parent
        z: 1001
        surface: Component {
            SettingsPanel {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent
                isMac: root.isMac
                shortcutMap: root.shortcutMap
                blurSupported: chrome.blurSupported

                onClosed: root.modalClosed()
            }
        }
        // Read from the Escape shortcut's `enabled`, which has to answer while
        // the panel does not exist.
        readonly property bool capturing: content !== null && content.capturing
        function open() {
            instantiate().open();
        }
        function close() {
            if (content !== null)
                content.close();
        }
    }

    LazySurface {
        id: commandPalette
        anchors.fill: parent
        z: 1200
        surface: Component {
            CommandPalette {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent
                textColor: root.textColor
                mutedColor: root.mutedColor
                hairline: root.hairline
                overviewModel: overviewModelInstance
                folderActive: root.folderActive
                remoteLocation: root.remoteLocation
                scopePath: root.folderActive ? root.activePath : Workspace.homeDirectory
                shortcutMap: root.shortcutMap
                isMac: root.isMac

                onActivated: function (kind, value) {
                    commandPalette.closePalette()
                    root.handlePaletteActivation(kind, value)
                }
                onClosed: root.modalClosed()
            }
        }
        function openPalette() {
            instantiate().openPalette();
        }
        function closePalette() {
            if (content !== null)
                content.closePalette();
        }
        function rebuild() {
            if (content !== null)
                content.rebuild();
        }
    }

    LazySurface {
        id: connectDialog
        anchors.fill: parent
        z: 1001
        surface: Component {
            ConnectDialog {
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
        }
        function open() {
            instantiate().open();
        }
        /// A discovered server opens the dialog with protocol, host and port
        /// already filled; the surface is created on this first use.
        function openWith(protocol, host, port) {
            instantiate().openWith(protocol, host, port);
        }
        function close() {
            if (content !== null)
                content.close();
        }
        function dismiss() {
            if (content !== null)
                content.dismiss();
        }
        // Replies from the connection test land after `open()`, so the surface
        // already exists; the guards cover a reply racing the first open.
        function setTesting(busy) {
            if (content !== null)
                content.setTesting(busy);
        }
        function setSaving(busy) {
            if (content !== null)
                content.setSaving(busy);
        }
        function applyTestResult(ok, message) {
            if (content !== null)
                content.applyTestResult(ok, message);
        }
        function applySaveError(message) {
            if (content !== null)
                content.applySaveError(message);
        }
    }

    LazySurface {
        id: cloudDialog
        anchors.fill: parent
        z: 1005
        surface: Component {
            CloudAccountDialog {
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
        }
        function open() {
            instantiate().open();
        }
        function close() {
            if (content !== null)
                content.close();
        }
        function dismiss() {
            if (content !== null)
                content.dismiss();
        }
        function setAuthorizing(busy) {
            if (content !== null)
                content.setAuthorizing(busy);
        }
        function applyError(message) {
            if (content !== null)
                content.applyError(message);
        }
    }

    LazySurface {
        id: openWithDialog
        anchors.fill: parent
        z: 1004
        surface: Component {
            OpenWithDialog {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent

                onCanceled: root.modalClosed()
            }
        }
        function open(targetPath) {
            instantiate().open(targetPath);
        }
        function close() {
            if (content !== null)
                content.close();
        }
        function dismiss() {
            if (content !== null)
                content.dismiss();
        }
        function showError(message) {
            if (content !== null)
                content.showError(message);
        }
    }

    LazySurface {
        id: updateDialog
        anchors.fill: parent
        z: 1005
        surface: Component {
            UpdateDialog {
                anchors.fill: parent
                dark: root.dark
                accent: root.accent

                onCanceled: root.modalClosed()
            }
        }
        function open() {
            instantiate().open();
        }
        function close() {
            if (content !== null)
                content.close();
        }
        function dismiss() {
            if (content !== null)
                content.dismiss();
        }
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

    QuickPreview {
        id: quickPreview
        anchors.fill: parent
        dark: root.dark
        accent: root.accent
    }

    StatusCenter {
        id: statusCenter
        anchors.fill: parent
        dark: root.dark
        accent: root.accent

        onVisibleChanged: {
            if (!visible)
                root.modalClosed()
        }
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

    LazySurface {
        id: gitPanel
        anchors.fill: parent
        z: 1002
        surface: Component {
            GitPanel {
                anchors.fill: parent
                repo: root.gitListing
                dark: root.dark
                accent: root.accent

                onVisibleChanged: {
                    if (!visible)
                        root.modalClosed()
                }
            }
        }
        readonly property bool creating: content !== null && content.creating
        function openAbove(x, y) {
            instantiate().openAbove(x, y);
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
            if (Updater.available && !updateDialog.shown)
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
        sequence: root.shortcutSequence("app.newTab")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.addTabHere()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.newWindow")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.newWindowAt(root.activePath)
    }

    Shortcut {
        sequence: root.shortcutSequence("app.commandBar")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: commandPalette.openPalette()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.closeTab")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.closeTab(root.activeIndex)
    }

    /// The browser gesture for a tab closed by mistake. It is in the action
    /// table like every other binding, so the settings panel can change it.
    Shortcut {
        sequence: root.shortcutSequence("app.reopenTab")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.reopenClosedTab()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.nextTab")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.cycleTab(1)
    }

    Shortcut {
        sequence: root.shortcutSequence("app.previousTab")
        enabled: !root.modalOpen && sequence !== ""
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
        enabled: !root.modalOpen && root.folderActive
            && !root.remoteLocation && sequence !== ""
        onActivated: root.openSearch()
    }

    Shortcut {
        sequence: root.shortcutSequence("app.toggleSidebar")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.sidebarVisible = !root.sidebarVisible
    }

    Shortcut {
        sequence: root.shortcutSequence("app.openSettings")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: settingsPanel.open()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.newFolder")
        enabled: !root.modalOpen && root.folderActive && sequence !== ""
        onActivated: root.promptCreate(true)
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.refresh")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: {
            if (root.activeTab !== null)
                root.activeTab.listing.reload()
        }
    }


    // Rows that used to be menu-only. The dispatchers reuse the same bridge
    // calls the context menu makes; the guards keep a shortcut from acting on
    // an empty selection or a modal.
    Shortcut {
        sequence: root.shortcutSequence("explorer.copyPath")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: {
            const path = root.shortcutPath()
            if (path === "")
                return
            if (Workspace.copyText(path))
                toast.show(qsTr("已复制路径"), false)
            else
                toast.show(qsTr("无法写入剪贴板"), true)
        }
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.copyName")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: {
            const path = root.shortcutPath()
            if (path === "")
                return
            const name = path.split(/[\\/]/).pop()
            if (Workspace.copyText(name))
                toast.show(qsTr("已复制名称"), false)
            else
                toast.show(qsTr("无法写入剪贴板"), true)
        }
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.openWith")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length === 1
        onActivated: {
            const path = root.shortcutPath()
            if (Workspace.openWithPickerSupported)
                openWithDialog.open(path)
            else
                Workspace.openWith(path)
        }
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.compressZip")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: root.startCompress("zip", false)
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.extractHere")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length === 1
        onActivated: root.startExtract(root.shortcutPath())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.measureSize")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: Workspace.measureFolders(root.shortcutSelection())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.compareHashes")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length === 2
        onActivated: Workspace.compareHashes(root.shortcutSelection())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.duplicate")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: Workspace.duplicateEntries(root.shortcutSelection())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.createShortcut")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: Workspace.createShortcuts(root.shortcutSelection(),
                                                root.shortcutDirectory())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.moveTo")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: moveToDialog.open(root.shortcutDirectory(), root.shortcutSelection())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.bulkRename")
        enabled: !root.modalOpen && sequence !== "" && root.shortcutSelection().length > 0
        onActivated: bulkRenameDialog.open(root.shortcutSelection())
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.cycleView")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.cycleViewMode()
    }

    Shortcut {
        sequence: root.shortcutSequence("explorer.toggleSizeUnits")
        enabled: !root.modalOpen && sequence !== ""
        onActivated: root.toggleSizeUnits()
    }

    /// One Escape handler for whichever overlay is on top. Per-overlay
    /// shortcuts would race each other when two are visible.
    Shortcut {
        sequence: "Escape"
        enabled: root.modalOpen && !settingsPanel.capturing
        onActivated: {
            if (quickPreview.open)
                quickPreview.close()
            else if (menu.visible)
                menu.close()
            else if (commandPalette.shown)
                commandPalette.closePalette()
            else if (updateDialog.shown)
                updateDialog.dismiss()
            else if (namePrompt.shown)
                namePrompt.close()
            else if (passwordDialog.shown) {
                passwordDialog.close()
                root.resetArchiveState()
            } else if (cloudDialog.shown)
                cloudDialog.dismiss()
            else if (connectDialog.shown)
                connectDialog.dismiss()
            else if (settingsPanel.shown)
                settingsPanel.close()
            else if (confirmDialog.shown) {
                confirmDialog.close()
                root.confirmMode = ""
                root.confirmTrashIds = []
            }
        }
    }

    Component.onCompleted: {
        // First statement: everything above is instantiation, everything below
        // is the startup work this mark splits from it. See startup.rs.
        Startup.mark("qml-root-completed")
        ShellState.register(root.token, root)

        if (!chrome.attach())
            console.warn("dae: native window attach failed")
        // The tab band is a client area of the caption. It can only be declared
        // once the window exists natively, so this runs after `attach` rather
        // than from the bar's own completion.
        topBar.pushCaptionBand()

        overviewModelInstance.setHiddenSections(ExplorerPrefs.hiddenHomeSections)
        root.syncToolbarOrder()

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
        } else if (root.firstWindow && root.restoreSessionWindows()) {
            // Restored above: tabs, geometry and the split preference.
        } else {
            // The landing surface is the overview. A deep link or an explicit
            // command-line path is a request to open that folder, so only the
            // "recent"/"home" fallbacks land on the overview instead.
            const explicit = root.config !== null
                && (root.config.startupSource === "deepLink"
                    || root.config.startupSource === "commandLine")
            const path = explicit ? root.startupPath : ""
            const tab = root.createTab(path)
            if (tab !== null) {
                if (path === "") {
                    tab.openOverview()
                    // `openOverview` is the shortcut that also rebuilds the
                    // rows; the startup tab bypasses it, so refresh here too.
                    overviewModelInstance.refresh()
                }
                root.tabs = [tab]
                root.activeIndex = 0
            }
        }

        // A tab created with its path already set never fires the path-change
        // signal the layout sync listens for; without this a launch straight
        // into a folder started from the defaults and ignored that folder's
        // saved layout until the next navigation. Restored session tabs take
        // the same path.
        root.syncLayoutForActiveTab()

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
        // The record is built while the tabs still exist: `disposeTabs` below
        // kills their models, and a closed tab in the record would restore an
        // empty folder.
        Session.saveWindow(root.token, JSON.stringify(root.sessionRecord()))
        if (ShellState.count() <= 1)
            Session.markClean()
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
