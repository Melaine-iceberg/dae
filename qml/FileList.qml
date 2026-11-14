pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import QtQuick.Shapes
import dae 1.0
import "Localize.js" as Localize
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
    /// with the window. 980 dp is the widest the table needs: name floor plus
    /// every detail column in §7.1.2 (42 + 16 + 260 + 88 + 104 + 150 + 150 +
    /// 120), so enabling created and tags still leaves a readable name. Past
    /// this point the row is background, and that strip still takes clicks,
    /// marquees and drops.
    readonly property int contentMaxWidth: 980
    readonly property int contentWidth: Math.min(width, contentMaxWidth)
    /// The details columns as stored in preferences, padded to a stable length
    /// for positional widths.
    readonly property var columnIds: ["type", "size", "modified", "created", "tags"]
    /// Display order (§7.1.2): size, type, modified, created, tags. Widths stay
    /// keyed by `columnIds`, so an existing preference keeps its meaning.
    readonly property var columnOrder: ["size", "type", "modified", "created", "tags"]
    /// When the pane narrows, columns retreat in this order (§7.1.2): path
    /// (search only), created, tags, type, modified, size. The name column is
    /// the last thing standing.
    readonly property var columnRetreat: ["created", "tags", "type", "modified", "size"]
    readonly property var visibleColumns: {
        const configured = (ExplorerPrefs.columns || "").split(",")
        return root.columnOrder.filter(function (id) {
            return configured.indexOf(id) >= 0
        })
    }
    /// The name column keeps this much room (§7.1.2). Without the floor, a
    /// narrow pane squeezes the one column the table exists for into zero.
    readonly property int nameMinWidth: 260
    readonly property var shownColumns: {
        const list = visibleColumns.slice()
        while (list.length > 0) {
            let used = 0
            for (let index = 0; index < list.length; index += 1)
                used += root.effectiveColumnWidth(list[index])
            if (root.contentWidth - 16 - 50 - used >= root.nameMinWidth)
                break
            for (let index = 0; index < root.columnRetreat.length; index += 1) {
                const drop = list.indexOf(root.columnRetreat[index])
                if (drop >= 0) {
                    list.splice(drop, 1)
                    break
                }
            }
        }
        return list
    }
    function columnWidthAt(index) {
        const parts = (ExplorerPrefs.columnWidths || "").split(",")
        const width = parseInt(parts[index])
        return isNaN(width) ? 100 : width
    }
    function columnWidth(id) {
        return root.columnWidthAt(root.columnIds.indexOf(id))
    }
    /// A resize drag updates the painted width here and only writes the
    /// preference on release: a document write per pointer move is a write per
    /// pixel, and the preference file is not a render target.
    property var columnOverrides: ({})
    function effectiveColumnWidth(id) {
        const live = root.columnOverrides[id]
        return live === undefined ? root.columnWidth(id) : live
    }
    function columnVisible(id) {
        return root.shownColumns.indexOf(id) >= 0
    }
    function setColumnWidth(id, width, live) {
        const index = root.columnIds.indexOf(id)
        if (index < 0)
            return
        const clamped = Math.round(Math.max(60, Math.min(420, width)))
        if (live === true) {
            const overrides = {}
            for (const key in root.columnOverrides)
                overrides[key] = root.columnOverrides[key]
            overrides[id] = clamped
            root.columnOverrides = overrides
            return
        }
        const parts = (ExplorerPrefs.columnWidths || "").split(",")
        while (parts.length < root.columnIds.length)
            parts.push("100")
        parts[index] = String(clamped)
        root.columnOverrides = ({})
        ExplorerPrefs.setString("columnWidths", parts.join(","))
    }
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
    /// Double-click on a divider sizes the column to its heading plus the
    /// widest sample it can paint — not to the longest visible value, which
    /// would reflow the table every time a large file scrolled past.
    function autoFitColumn(id) {
        const samples = {
            "type": qsTr("PowerPoint 演示文稿"),
            "size": "888.8 MiB",
            "modified": qsTr("周三 14:32"),
            "created": qsTr("周三 14:32"),
            "tags": qsTr("工作, 重要")
        }
        const label = root.columnLabel(id) + "  "
        const sample = label.length > (samples[id] || "").length ? label : samples[id]
        const metrics = Qt.createQmlObject(
            "import QtQuick; Text { font.pixelSize: " + Theme.typeCaption + " }", root)
        metrics.text = sample || ""
        root.setColumnWidth(id, metrics.implicitWidth + 28)
        metrics.destroy()
    }
    readonly property var columnLabels: ({
        "type": qsTr("类型"),
        "size": qsTr("大小"),
        "modified": qsTr("修改时间"),
        "created": qsTr("创建时间"),
        "tags": qsTr("标签")
    })
    function columnLabel(id) {
        return root.columnLabels[id] || id
    }
    /// Section headers are a rendering of the model's `groupLabel`, translated
    /// here so the Rust side keeps language-neutral keys.
    function groupLabelText(label) {
        if (label === "")
            return qsTr("无标签")
        const key = ExplorerPrefs.groupKey
        if (key === "type") {
            const names = {
                "folder": qsTr("文件夹"), "link": qsTr("链接"),
                "image": qsTr("图片"), "video": qsTr("视频"),
                "audio": qsTr("音频"), "document": qsTr("文档"),
                "spreadsheet": qsTr("电子表格"), "presentation": qsTr("演示文稿"),
                "pdf": qsTr("PDF"), "text": qsTr("文本"), "code": qsTr("代码"),
                "archive": qsTr("压缩包"), "font": qsTr("字体"),
                "executable": qsTr("程序"), "other": qsTr("其他"),
                "file": qsTr("文件")
            }
            return names[label] || label
        }
        if (key === "modified")
            return label + qsTr(" 修改")
        if (key === "created")
            return label + qsTr(" 创建")
        return label
    }
    /// The name without its extension, for the "show extensions" preference.
    /// The model still reports the full name, so sorting, search and rename
    /// are unaffected.
    function cellText(r, id) {
        if (id === "type")
            return Localize.typeName(r.typeName)
        if (id === "size")
            return r.sizeText
        if (id === "modified")
            return root.relativeTime(r.modifiedAt)
        if (id === "created") {
            if (!r.createdAt)
                return "—"
            return root.relativeTime(r.createdAt)
        }
        if (id === "tags")
            return r.tagNames
        return ""
    }
    /// The relative stamp the modified and created columns print (§7.1.4):
    /// today's time, "yesterday", a weekday inside seven days, then the
    /// localised short date. Computed from the raw epoch stamp in the user's
    /// time zone rather than from the model's UTC text, so "today" means the
    /// user's today.
    function relativeTime(milliseconds) {
        if (!milliseconds || milliseconds <= 0)
            return "—"
        const then = new Date(milliseconds)
        const now = new Date()
        const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
        const startOfThen = new Date(then.getFullYear(), then.getMonth(), then.getDate()).getTime()
        const days = Math.round((startOfToday - startOfThen) / 86400000)
        if (days > 0 && days < 7) {
            const time = Qt.formatTime(then, "HH:mm")
            if (days === 1)
                return qsTr("昨天 %1").arg(time)
            return then.toLocaleDateString(Qt.locale(), "ddd") + " " + time
        }
        if (days === 0)
            return Qt.formatTime(then, "HH:mm")
        return then.toLocaleDateString(Qt.locale())
    }
    /// The tooltip: the full local stamp, which is what a relative label hides.
    function fullTimestamp(milliseconds) {
        if (!milliseconds || milliseconds <= 0)
            return "—"
        return Qt.formatDateTime(new Date(milliseconds), "yyyy-MM-dd HH:mm")
    }
    /// The filename's middle elision (§7.1.3): front 8 characters, an ellipsis,
    /// then the last 6 characters, which keeps the extension. A longer
    /// extension widens the tail so it is never cut; a stem shorter than the
    /// rule's 14 characters falls back to eliding the whole string, because
    /// keeping two fragments at that length reads worse than one.
    function middleElide(name, stem, extensionSuffix) {
        const tailLength = Math.max(6, extensionSuffix.length)
        return stem.slice(0, 8) + "…" + name.slice(-tailLength)
    }
    function displayName(name, extension) {
        if (ExplorerPrefs.showExtensions || extension === "")
            return name
        const suffix = "." + extension
        return name.endsWith(suffix) ? name.slice(0, -suffix.length) : name
    }
    /// The printed heading is gone (§6.7.1): the shell's path is the heading,
    /// and the count lives in the status bar. Only the table remains.
    readonly property int listingTop: listHeaderVisible ? headerHeight : 0
    readonly property color contentColor: Theme.content
    readonly property color textColor: Theme.ink
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
    readonly property string emptyHint: emptyIsError ? Localize.message(model.status)
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

    /// Selected rows, read from whichever model is showing. The count is a
    /// property the model maintains as flags move, so this binding updates on
    /// the signal instead of walking the selection every time the status bar
    /// asks; the column view is the one surface that spreads its rows over
    /// several models, so it answers for the panes.
    readonly property int selectionCount: root.viewMode === "column"
        ? (columnView !== null ? columnView.selectionCount : 0)
        : (root.model !== null ? root.model.selectedCount : 0)

    /// The deferred views. Null until their mode has been shown once, which is
    /// also the only time they exist at all (see the `LazySurface`s below).
    readonly property var gridView: gridSurface.content
    readonly property var columnView: columnSurface.content

    property string pendingFocus: ""
    /// The path whose row paints the one-shot paste/create highlight, and the
    /// timer that fades it: §7.1.7's "just pasted" state lasts one glance, not
    /// until the next selection.
    property string flashPath: ""
    Timer {
        id: flashTimer
        interval: 480
        onTriggered: root.flashPath = ""
    }
    /// Cut rows are drawn at 40% (§7.1.7). The clipboard is one list for the
    /// window, so it is turned into a lookup once per change instead of being
    /// scanned by every visible delegate.
    readonly property var cutSet: {
        const paths = Workspace.clipboardMode === "cut" ? Workspace.clipboardPaths : []
        const set = ({})
        for (let index = 0; index < paths.length; index += 1)
            set[paths[index]] = true
        return set
    }
    /// A folder already has a size while the shell is measuring the selection,
    /// so selected directories paint a progress ring rather than a stale
    /// number (§7.1.7). `progressLabel` is the live field the shell sets when
    /// an operation starts and clears when it settles; `operationsJson` only
    /// lists finished operations, so it never showed the ring.
    readonly property bool measuringSizes: Workspace.progressLabel.indexOf("Calculating sizes") >= 0
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
        if (root.viewMode === "column")
            return columnView !== null ? columnView.selectedPathList() : []
        return root.model !== null ? root.model.selectedPaths() : []
    }

    function clearSelection() {
        if (root.viewMode === "column") {
            if (columnView !== null)
                columnView.clearSelection()
        } else if (root.model !== null) {
            root.model.clearSelection()
        }
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
                                  ExplorerPrefs.folderOrder)
        root.tab.listing.setGroup(ExplorerPrefs.groupKey, ExplorerPrefs.groupDate)
        root.tab.listing.setFilters(ExplorerPrefs.showHiddenFiles, ExplorerPrefs.kindFilter,
                                    ExplorerPrefs.modifiedFilter, ExplorerPrefs.sizeFilter)
    }

    onTabChanged: applyViewPreferences()
    Component.onCompleted: applyViewPreferences()

    Connections {
        target: root.model
        enabled: root.model !== null && root.tab !== null
        function onLoadedChanged() { tagTimer.restart() }
        function onFinishedChanged() {
            tagTimer.restart()
            root.syncSearchSelection()
        }
        function onSortApplied() { tagTimer.restart() }
    }

    Connections {
        target: ExplorerPrefs
        function onSortKeyChanged() { root.applyViewPreferences() }
        function onSortOrderChanged() { root.applyViewPreferences() }
        function onFolderOrderChanged() { root.applyViewPreferences() }
        function onGroupKeyChanged() { root.applyViewPreferences() }
        function onGroupDateChanged() { root.applyViewPreferences() }
        function onShowHiddenFilesChanged() { root.applyViewPreferences() }
        function onKindFilterChanged() { root.applyViewPreferences() }
        function onModifiedFilterChanged() { root.applyViewPreferences() }
        function onSizeFilterChanged() { root.applyViewPreferences() }
    }

    /// Asks the model for the tags of the rows currently on screen. The
    /// listing itself never reads tags while enumerating, so this is the only
    /// moment a tag is fetched for display.
    function updateVisibleTags() {
        if (root.model === null || root.searchModel !== null || root.tab === null)
            return
        const view = root.viewMode === "grid" ? root.gridView
            : (root.viewMode === "list" ? list : null)
        if (view === null || view.height <= 0 || root.model.loaded <= 0)
            return
        const top = view.contentY + 2
        const bottom = view.contentY + view.height - 2
        // `indexAt` is exact but returns -1 over a section header or past the
        // last row. The arithmetic fallback over-approximates by the section
        // heights, which is fine: reading tags for a row just below the fold
        // is cheaper than a second request when it scrolls in.
        let first = view.indexAt(4, top)
        if (first < 0)
            first = view.indexAt(4, top + root.headerHeight + 2)
        if (first < 0)
            first = Math.max(0, Math.floor(top / root.rowHeight) - 2)
        let last = view.indexAt(4, bottom)
        if (last < 0)
            last = Math.min(root.model.loaded - 1,
                            Math.floor(bottom / root.rowHeight) + 1)
        if (last < first)
            last = first
        root.model.requestTags(first, last)
    }

    Timer {
        id: tagTimer
        interval: 60
        repeat: false
        onTriggered: root.updateVisibleTags()
    }

    /// Keyboard focus lives on the view that is showing, not on this container.
    function takeFocus() {
        if (viewMode === "grid" && gridView !== null)
            gridView.takeFocus()
        else if (viewMode === "column" && columnView !== null)
            columnView.takeFocus()
        else
            list.forceActiveFocus()
    }

    function cursorIndex() {
        if (viewMode === "grid" && gridView !== null)
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
        // The scan and the case folding happen in Rust: asking per row would
        // cross the bridge, build a QString and allocate a lowercased copy for
        // every name in the listing on a single keystroke.
        const index = root.model.indexOfNamePrefix(needle, start)
        if (index < 0)
            return
        const path = root.pathAtIndex(index)
        if (path === "")
            return
        if (root.viewMode === "grid" && gridView !== null) {
            gridView.setCursor(index)
            gridView.positionCursor(index)
        } else {
            list.currentIndex = index
            list.positionViewAtIndex(index, ListView.Contain)
        }
        root.model.selectOnly(index)
        if (root.tab !== null)
            root.tab.selectionAnchor = path
        root.selectionChanged()
    }

    /// A running query rebuilds its rows on every keystroke and every filter
    /// change, and the model drops the selection with them. The row the reader
    /// had chosen is kept when it survived the new query; otherwise the first
    /// visible result takes the anchor, so the preview can never describe a
    /// file the results no longer show.
    function syncSearchSelection() {
        const results = root.searchModel
        if (results === null || !results.finished || results.loaded === 0)
            return
        if (results.selectedCount > 0)
            return
        const anchor = root.tab !== null ? root.tab.selectionAnchor : ""
        let index = anchor === "" ? -1 : results.indexOfPath(anchor)
        if (index < 0)
            index = 0
        results.selectOnly(index)
        const path = results.pathAt(index)
        if (root.tab !== null && path !== "")
            root.tab.selectionAnchor = path
        list.currentIndex = index
        list.positionViewAtIndex(index, ListView.Contain)
        root.selectionChanged()
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
        if (root.model === null)
            return
        root.takeFocus()
        if (viewMode === "grid" && gridView !== null)
            gridView.setCursor(index)
        else
            list.currentIndex = index
        const path = root.pathAtIndex(index)
        if (path === "")
            return

        if (shift && root.tab !== null && root.tab.selectionAnchor !== "") {
            const anchor = root.model.indexOfPath(root.tab.selectionAnchor)
            if (anchor >= 0) {
                // The whole range in one call: the model sets the flags and
                // repaints the rows that moved, in Rust.
                root.model.selectRange(Math.min(anchor, index), Math.max(anchor, index))
                root.selectionChanged()
                return
            }
        }

        if (ctrl)
            root.model.toggleAt(index)
        else
            root.model.selectOnly(index)
        if (root.tab !== null)
            root.tab.selectionAnchor = path
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
        if (root.model === null)
            return
        root.model.selectAll()
        root.selectionChanged()
    }

    /// Reloads and, once the listing finishes, selects `path` and scrolls to
    /// it. Used after create/rename so the new row is visible and selected.
    function focusPath(path) {
        root.pendingFocus = path
        if (root.model !== null && root.model.finished)
            root.applyFocus()
    }

    /// The shell calls this after a paste or a creation lands: the new row is
    /// selected, scrolled to, and highlighted for one beat (§7.1.7).
    function flashRow(path) {
        root.flashPath = path
        flashTimer.restart()
    }

    function applyFocus() {
        const path = root.pendingFocus
        if (path === "" || root.model === null)
            return
        const index = root.model.indexOfPath(path)
        root.pendingFocus = ""
        if (index < 0)
            return
        if (viewMode === "grid" && gridView !== null)
            gridView.setCursor(index)
        else
            list.currentIndex = index
        root.model.selectOnly(index)
        if (root.tab !== null)
            root.tab.selectionAnchor = path
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
    function prepareDrag(index, selected, mouse) {
        root.suppressClick = false
        root.dragArmed = false
        root.dragStarted = false
        root.dragPaths = []

        if (root.model === null || root.searchModel !== null
                || root.remote || index < 0)
            return

        const path = root.pathAtIndex(index)
        if (path === "")
            return

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
            Workspace.startDragOut(paths, root.dragOutMode(mouse.modifiers),
                                   Theme.accent)
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
            return gridView !== null ? gridView.dropDirectoryAt(x, y) : root.model.path
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
            if (root.model !== null) {
                const hadSelection = root.selectionCount > 0
                root.clearSelection()
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
        property string columnId: ""
        property bool alignRight: false
        readonly property bool active: ExplorerPrefs.sortKey === cell.sortKey

        signal clicked()

        /// The divider on the trailing edge. The label's own click area is
        /// inset so a drag to resize never also sorts.
        Rectangle {
            id: divider
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 1
            color: Theme.hairline
        }

        MouseArea {
            id: dividerMouse
            /// A drag strip between two columns. Resizing is a pointer gesture with
            /// no keyboard path, so a name here would announce a button that cannot
            /// be pressed from the keyboard.
            Accessible.ignored: true
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 7
            cursorShape: Qt.SplitHCursor
            hoverEnabled: true
            preventStealing: true
            property real pressX: 0
            property real pressWidth: 0
            onPressed: function (mouse) {
                dividerMouse.pressWidth = cell.width
                dividerMouse.pressX = cell.mapToItem(null, mouse.x, mouse.y).x
            }
            onPositionChanged: function (mouse) {
                if (!pressed || cell.columnId === "")
                    return
                const x = cell.mapToItem(null, mouse.x, mouse.y).x
                root.setColumnWidth(cell.columnId,
                                    dividerMouse.pressWidth + (x - dividerMouse.pressX),
                                    true)
            }
            onReleased: {
                if (cell.columnId !== "" && root.columnOverrides[cell.columnId] !== undefined)
                    root.setColumnWidth(cell.columnId,
                                        root.columnOverrides[cell.columnId], false)
            }
            onCanceled: {
                if (cell.columnId !== "" && root.columnOverrides[cell.columnId] !== undefined)
                    root.setColumnWidth(cell.columnId,
                                        root.columnOverrides[cell.columnId], false)
            }
            onDoubleClicked: {
                if (cell.columnId !== "")
                    root.autoFitColumn(cell.columnId)
            }
        }

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
                    : (headerMouse.containsMouse ? Theme.inkSecondary : Theme.inkTertiary)
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
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("按%1排序").arg(cell.label)
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.rightMargin: 7
            anchors.top: parent.top
            anchors.bottom: parent.bottom
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
                anchors.leftMargin: 16
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: Math.max(0, detailsHeader.x - 8 - x)
                label: qsTr("名称")
                sortKey: "name"
                onClicked: root.applySort("name")
            }

            /// The details columns are one right-anchored row, so their
            /// order, widths and visibility come from the directory's layout
            /// instead of four anchor rules that have to be edited together.
            Row {
                id: detailsHeader
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                spacing: 0

                Repeater {
                    model: root.shownColumns

                    SortHeader {
                        required property string modelData
                        width: root.effectiveColumnWidth(modelData)
                        height: detailsHeader.height
                        label: root.columnLabel(modelData)
                        sortKey: modelData
                        alignRight: modelData !== "type"
                        columnId: modelData
                        onClicked: root.applySort(modelData)
                    }
                }
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
            Accessible.ignored: true
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
            /// Sections come from the model's precomputed `groupLabel`; an
            /// empty property name is how a ListView is told to paint none.
            section.property: root.searchModel === null
                && ExplorerPrefs.groupKey !== "" ? "groupLabel" : ""
            section.criteria: ViewSection.FullString
            section.delegate: Item {
                id: sectionRow

                required property string section

                width: list.width
                height: ExplorerPrefs.groupKey === "" ? 0 : 28

                Rectangle {
                    anchors.fill: parent
                    color: root.contentColor
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 16
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 4
                    text: root.groupLabelText(sectionRow.section)
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeMicro
                    font.weight: Font.Medium
                    font.letterSpacing: 0.4
                    textFormat: Text.PlainText
                }
            }

            onContentYChanged: tagTimer.restart()
            onHeightChanged: tagTimer.restart()
            onCountChanged: tagTimer.restart()

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
                required property string createdText
                required property bool createdFallback
                required property string tagNames
                required property string tagIds
                required property string tagColors
                required property string groupLabel
                /// §7.1.6: the type label Rust already resolved.
                required property string typeName
                /// Epoch stamps for the relative modified/created labels.
                required property double modifiedAt
                required property double createdAt
                /// A contiguous selection block rounds only its outer corners
                /// (§6.4.2); a delegate cannot see its neighbours on its own.
                required property bool previousSelected
                required property bool nextSelected
                required property bool selected

                /// The row stops at the column track's cap; ListView leaves the
                /// rest of the viewport as background, exactly like the
                /// Tauri shell's Windows-style detail view.
                width: Math.min(ListView.view.width, root.contentMaxWidth)
                height: root.rowHeight
                /// A cut row stays legible but reads as pending (§7.1.7).
                opacity: row.isCut ? 0.4 : 1

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
                    ? Theme.warning
                    : (row.gitStatus === "added"
                        ? Theme.success
                        : Theme.accent)
                /// §7.1.7: a cut row stays in place at 40%.
                readonly property bool isCut: root.cutSet[row.filePath] === true
                /// §7.1.7: the new or just-pasted row is highlighted once.
                readonly property bool flash: root.flashPath !== ""
                    && root.flashPath === row.filePath
                readonly property bool measuring: row.isDir && row.selected
                    && root.measuringSizes
                readonly property var tagDots: row.tagColors !== ""
                    ? row.tagColors.split(",").slice(0, 3) : []
                readonly property real tagDotsWidth: row.tagDots.length === 0 ? 0
                    : row.tagDots.length * 6 + (row.tagDots.length - 1) * 3 + 8
                readonly property bool hasPath: root.searchModel !== null
                    && row.relativePath !== "" && row.relativePath !== row.fileName
                /// §7.1.3: the stem keeps the name, the extension is a second
                /// run at 12 dp/400 so the two never merge into one ellipsis.
                readonly property bool separateExtension: ExplorerPrefs.showExtensions
                    && row.extension !== ""
                readonly property string stemText: {
                    if (!row.separateExtension)
                        return root.displayName(row.fileName, row.extension)
                    const suffix = "." + row.extension
                    return row.fileName.endsWith(suffix)
                        ? row.fileName.slice(0, -suffix.length) : row.fileName
                }
                readonly property string extensionText: row.separateExtension
                    ? "." + row.extension : ""
                readonly property real nameBudget: Math.max(0,
                    nameCell.width - row.tagDotsWidth - (row.hasPath ? 104 : 0))
                readonly property real nameIntrinsic: stemMetrics.advanceWidth
                    + (row.separateExtension ? 1 + extensionMetrics.advanceWidth : 0)
                readonly property bool truncated: row.nameIntrinsic > row.nameBudget
                /// A stem below the rule's 14 characters elides as one string
                /// rather than keeping a two-character fragment of it.
                readonly property bool singleString: row.truncated
                    && row.separateExtension && row.stemText.length < 14
                readonly property string stemDisplay: {
                    if (!row.separateExtension || row.singleString)
                        return root.displayName(row.fileName, row.extension)
                    if (!row.truncated)
                        return row.stemText
                    return root.middleElide(row.fileName, row.stemText,
                                            row.extensionText)
                }
                readonly property real stemBudget: Math.max(0,
                    row.nameBudget - (row.separateExtension
                        ? 1 + extensionMetrics.advanceWidth : 0))
                /// §7.1.4: number and unit are separate runs so the decimal
                /// points can line up under a tabular figure.
                readonly property var sizeParts: {
                    const text = row.sizeText
                    const cut = text.lastIndexOf(" ")
                    return cut < 0 ? [text, ""] : [text.slice(0, cut), text.slice(cut + 1)]
                }

                TextMetrics {
                    id: stemMetrics
                    text: row.stemText
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeBody
                    font.weight: 450
                }

                TextMetrics {
                    id: extensionMetrics
                    text: row.extensionText
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeCaption
                }

                /// The row, not the four cells inside it: a listing read cell
                /// by cell is unlistenable, and the selection state lives on
                /// the row. Press is the double click, so a reader has the same
                /// one gesture a pointer does.
                Accessible.role: Accessible.ListItem
                Accessible.name: root.displayName(row.fileName, row.extension)
                Accessible.description: Localize.typeName(row.typeName)
                Accessible.selectable: true
                Accessible.selected: row.selected
                Accessible.onPressAction: root.handleActivate(row.index)

                /// Hover is a 4% ink wash, selection a 10% accent wash and the
                /// drop target 12% accent (§7.1.7). The text never inverts:
                /// the pale masks keep the name the strongest mark in the row.
                Rectangle {
                    id: rowMask
                    anchors.fill: parent
                    color: row.selected ? Theme.selection
                        : row.dropTarget ? Theme.accentSoft
                        : (rowHover.containsMouse ? Theme.hover : "transparent")
                    /// A contiguous block rounds its outer corners and stays
                    /// square along shared edges (§6.4.2).
                    topLeftRadius: row.selected && row.previousSelected ? 0 : Theme.radiusSm
                    topRightRadius: row.selected && row.previousSelected ? 0 : Theme.radiusSm
                    bottomLeftRadius: row.selected && row.nextSelected ? 0 : Theme.radiusSm
                    bottomRightRadius: row.selected && row.nextSelected ? 0 : Theme.radiusSm
                }

                /// The one-shot paste/create highlight, a 160 ms fade (§7.1.7).
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: Theme.accentSoftStrong
                    opacity: row.flash ? 1 : 0
                    visible: opacity > 0
                    Accessible.ignored: true
                    Behavior on opacity {
                        enabled: Theme.motion
                        NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
                    }
                }

                /// A drop target is a solid 1.5 dp accent border, never a
                /// dashed one (§6.7).
                Rectangle {
                    anchors.fill: parent
                    visible: row.dropTarget
                    color: "transparent"
                    radius: Theme.radiusSm
                    border.width: 1.5
                    border.color: Theme.accent
                }

                /// The keyboard cursor (§7.1.8): on a selected row the signal
                /// is a 2 dp accent track on the left; on an unselected row a
                /// 1.5 dp inner stroke. Only keyboard navigation paints it.
                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 2
                    visible: row.focusedCursor && row.selected
                    color: Theme.accent
                    Accessible.ignored: true
                }

                Rectangle {
                    anchors.fill: parent
                    visible: row.focusedCursor && !row.selected
                    color: "transparent"
                    radius: Theme.radiusSm
                    border.width: 1.5
                    border.color: Theme.accent
                }

                /// §7.1.1: 16 dp left pad, an 18 dp icon, 8 dp to the name.
                Image {
                    id: rowIcon
                    x: 16
                    width: 18
                    height: 18
                    anchors.verticalCenter: parent.verticalCenter
                    source: row.showThumbnail || ExplorerPrefs.iconStyle !== "system"
                        ? "" : row.iconSource
                    sourceSize.width: Math.round(18 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(18 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    visible: !row.showThumbnail && ExplorerPrefs.iconStyle === "system"
                        && status === Image.Ready
                    Accessible.ignored: true
                }

                Image {
                    x: 16
                    width: 18
                    height: 18
                    anchors.verticalCenter: parent.verticalCenter
                    visible: row.showThumbnail
                    source: row.showThumbnail ? row.thumbnailSource : ""
                    sourceSize.width: Math.round(36 * Screen.devicePixelRatio)
                    sourceSize.height: Math.round(36 * Screen.devicePixelRatio)
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    smooth: true
                    Accessible.ignored: true
                }

                /// The built-in glyph (§6.3.2, §12.3) is a cached raster, not
                /// a per-row canvas. The system icon wins only when the
                /// preference asks for it and the platform delivered one.
                FileIcon {
                    x: 16
                    edge: 18
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !row.showThumbnail
                        && (ExplorerPrefs.iconStyle !== "system"
                            || rowIcon.status !== Image.Ready)
                    category: row.category
                    outline: row.isDir ? Theme.inkSecondary : Theme.inkTertiary
                    Accessible.ignored: true
                }

                /// Measuring a directory's size is a background operation;
                /// the row that started it shows a 12 dp ring (§7.1.7).
                Item {
                    id: measuringRing
                    x: Math.max(0, detailsRow.x - 20)
                    width: 12
                    height: 12
                    anchors.verticalCenter: parent.verticalCenter
                    visible: row.measuring
                    Accessible.ignored: true

                    Shape {
                        anchors.fill: parent
                        ShapePath {
                            strokeColor: Theme.accent
                            strokeWidth: 1.5
                            fillColor: "transparent"
                            capStyle: ShapePath.RoundCap
                            PathAngleArc {
                                centerX: 6
                                centerY: 6
                                radiusX: 4.5
                                radiusY: 4.5
                                startAngle: -90
                                sweepAngle: 270
                            }
                        }
                        RotationAnimator on rotation {
                            from: 0
                            to: 360
                            duration: 900
                            loops: Animation.Infinite
                            running: row.measuring
                        }
                    }
                }

                /// The name cell (§7.1.3): stem at 13 dp/450 in ink, a 1 dp
                /// gap, the extension at 12 dp/400 in `inkTertiary`, then up to
                /// three tag dots and — in a search — the relative path.
                Row {
                    id: nameCell
                    x: 42
                    width: Math.max(0, detailsRow.x - 8 - x)
                    height: parent.height
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 0

                    Text {
                        id: stemLabel
                        height: nameCell.height
                        width: row.singleString
                            ? row.nameBudget
                            : Math.min(implicitWidth, row.stemBudget)
                        verticalAlignment: Text.AlignVCenter
                        text: row.stemDisplay
                        color: row.hidden ? Theme.inkTertiary : Theme.ink
                        font.family: Theme.uiFamily
                        font.pixelSize: Theme.typeBody
                        font.weight: 450
                        textFormat: Text.PlainText
                        /// Backstop only: the rule's 8 + … + 6 candidate is
                        /// usually short enough; when it is not, Qt's middle
                        /// elide keeps the same shape.
                        elide: Text.ElideMiddle
                    }

                    Text {
                        id: extensionLabel
                        visible: row.separateExtension && !row.singleString
                        height: nameCell.height
                        leftPadding: 1
                        verticalAlignment: Text.AlignVCenter
                        text: row.extensionText
                        color: Theme.inkTertiary
                        font.family: Theme.uiFamily
                        font.pixelSize: Theme.typeCaption
                        textFormat: Text.PlainText
                    }

                    Row {
                        id: tagDotsRow
                        visible: row.tagDots.length > 0
                        height: nameCell.height
                        leftPadding: 8
                        spacing: 3

                        Repeater {
                            model: row.tagDots

                            Rectangle {
                                required property string modelData
                                width: 6
                                height: 6
                                radius: 3
                                anchors.verticalCenter: parent.verticalCenter
                                color: modelData
                            }
                        }
                    }

                    Text {
                        visible: row.hasPath
                        height: nameCell.height
                        leftPadding: 8
                        width: row.hasPath ? Math.max(0, nameCell.width - stemLabel.width
                            - (extensionLabel.visible ? extensionLabel.width : 0)
                            - (tagDotsRow.visible ? tagDotsRow.width : 0)) : 0
                        verticalAlignment: Text.AlignVCenter
                        text: row.hasPath ? row.relativePath : ""
                        color: Theme.inkTertiary
                        font.pixelSize: Theme.typeCaption
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                    }
                }

                /// A symbolic link is marked at the end of the row (§6.1.6).
                /// The marker sits in the row's right padding, so it never
                /// shifts the columns beside it.
                Text {
                    visible: row.category === "link"
                    x: row.width - 16
                    width: 12
                    anchors.verticalCenter: parent.verticalCenter
                    horizontalAlignment: Text.AlignRight
                    text: "↗"
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                    Accessible.ignored: true
                }

                /// The details columns, right-anchored in the order the
                /// header paints them. A column the pane cannot afford is
                /// dropped by `shownColumns`, and the name cell grows into
                /// the space.
                Row {
                    id: detailsRow
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    height: parent.height
                    spacing: 0

                    Repeater {
                        model: root.shownColumns

                        Item {
                            id: cell

                            required property string modelData
                            width: root.effectiveColumnWidth(modelData)
                            height: detailsRow.height

                            /// Type: the human label at inkSecondary, then the
                            /// git status as one letter hugging its right edge
                            /// (§7.1.6) — not a badge.
                            Row {
                                visible: cell.modelData === "type"
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 6

                                Text {
                                    width: Math.max(0, cell.width
                                        - (row.hasGitStatus ? 18 : 0))
                                    text: Localize.typeName(row.typeName)
                                    color: Theme.inkSecondary
                                    font.pixelSize: Theme.typeCaption
                                    textFormat: Text.PlainText
                                    elide: Text.ElideRight
                                    Accessible.ignored: true
                                }

                                Text {
                                    visible: row.hasGitStatus
                                    text: row.gitLetter
                                    color: row.gitBadgeColor
                                    font.family: Theme.monoFamily
                                    font.pixelSize: Theme.typeMicro
                                    font.weight: Font.Medium
                                    textFormat: Text.PlainText
                                    Accessible.ignored: true
                                }
                            }

                            /// §7.1.4: the number is right-aligned on a
                            /// tabular figure, the unit sits in its own left
                            /// track so MiB and GiB share one baseline.
                            Row {
                                visible: cell.modelData === "size"
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 4

                                Text {
                                    width: 44
                                    horizontalAlignment: Text.AlignRight
                                    text: row.sizeParts[0]
                                    color: Theme.inkSecondary
                                    font.pixelSize: Theme.typeCaption
                                    font.features: { "tnum": 1 }
                                    textFormat: Text.PlainText
                                    Accessible.ignored: true
                                }

                                Text {
                                    width: 30
                                    text: row.sizeParts[1]
                                    color: Theme.inkTertiary
                                    font.pixelSize: Theme.typeCaption
                                    textFormat: Text.PlainText
                                    Accessible.ignored: true
                                }
                            }

                            /// Modified and created: a relative label, right
                            /// aligned, tabular (§7.1.4). The full local stamp
                            /// stays available to a screen reader.
                            Text {
                                anchors.fill: parent
                                visible: cell.modelData === "modified"
                                    || cell.modelData === "created"
                                horizontalAlignment: Text.AlignRight
                                verticalAlignment: Text.AlignVCenter
                                text: root.cellText(row, cell.modelData)
                                color: cell.modelData === "created" && row.createdFallback
                                    ? Theme.inkTertiary : Theme.inkSecondary
                                font.pixelSize: Theme.typeCaption
                                font.features: { "tnum": 1 }
                                textFormat: Text.PlainText
                                elide: Text.ElideRight
                                Accessible.description: cell.modelData === "modified"
                                    ? root.fullTimestamp(row.modifiedAt)
                                    : root.fullTimestamp(row.createdAt)
                            }

                            Row {
                                visible: cell.modelData === "tags"
                                anchors.fill: parent
                                anchors.leftMargin: 4
                                spacing: 6

                                Row {
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 3

                                    Repeater {
                                        model: row.tagColors.length > 0
                                            ? row.tagColors.split(",") : []

                                        Rectangle {
                                            required property string modelData
                                            width: 6
                                            height: 6
                                            radius: 3
                                            color: modelData
                                        }
                                    }
                                }

                                Text {
                                    width: Math.max(0, parent.width - 14)
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: row.tagNames
                                    color: Theme.inkSecondary
                                    font.pixelSize: Theme.typeCaption
                                    textFormat: Text.PlainText
                                    elide: Text.ElideRight
                                }
                            }
                        }
                    }
                }

                MouseArea {
                    id: rowHover
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onPressed: function (mouse) {
                        root.keyboardCursor = false
                        if (mouse.button === Qt.LeftButton)
                            root.prepareDrag(row.index, row.selected, mouse)
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
                            if (!row.selected)
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

        /// The grid and the column view are built the first time their mode is
        /// shown, and kept afterwards. Both used to be instantiated up front,
        /// so every completion reset destroyed and recreated 177 delegates in
        /// views nobody could see (docs/decisions/listing-shell-costs.md §9).
        LazySurface {
            id: gridSurface
            anchors.fill: parent
            visible: root.viewMode === "grid"
            loadWhenShown: root.viewMode === "grid"
            surface: Component {
                FileGrid {
                    anchors.fill: parent
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
                        Workspace.startDragOut(paths, mode, Theme.accent)
                    }
                }
            }
        }

        LazySurface {
            id: columnSurface
            anchors.fill: parent
            visible: root.viewMode === "column"
            loadWhenShown: root.viewMode === "column"
            surface: Component {
                ColumnView {
                    anchors.fill: parent
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
                        Workspace.startDragOut(paths, mode, Theme.accent)
                    }
                }
            }
        }

        /// Rubber-band selection over the detail list. The overlay sits above
        /// the rows so a drag that began on empty space keeps receiving moves
        /// over them; a press that began on a row is handed down instead.
        MouseArea {
            id: listMarquee
            /// As in the grid: the marquee is the picture of a drag, not a target.
            Accessible.ignored: true
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
            /// Ctrl was held when the drag began: the band extends the
            /// selection rather than replacing it. The model snapshots the
            /// base itself.
            property bool keptBase: false
            /// The row range the last move applied. A pointer that stays inside
            /// the same rows must not rebuild the selection: a drag emits moves
            /// far faster than the rows under it change.
            property int appliedFirstRow: -2
            property int appliedLastRow: -2
            property bool appliedInBand: false

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
                if (!active || root.model === null)
                    return
                // Rows stop at the column track's edge, so the band has to
                // cross that box horizontally as well: dragging over the blank
                // area right of the columns selects nothing.
                const left = Math.min(startX, pointerX)
                const right = Math.max(startX, pointerX)
                const inBand = right > 0 && left < root.contentWidth
                let firstRow = -1
                let lastRow = -1
                if (inBand) {
                    const currentContent = list.contentY + pointerY
                    const top = Math.min(startContentY, currentContent)
                    const bottom = Math.max(startContentY, currentContent)
                    // `indexAt` rather than row arithmetic: a section header
                    // is taller than a row, and dividing the content offset
                    // by the row height would drift one row per section.
                    firstRow = list.indexAt(4, top + 2)
                    if (firstRow < 0)
                        firstRow = list.indexAt(4, top + root.headerHeight + 2)
                    lastRow = list.indexAt(4, bottom - 2)
                    if (lastRow < 0)
                        lastRow = list.indexAt(4, bottom - root.rowHeight)
                    if (firstRow < 0)
                        firstRow = 0
                    if (lastRow < 0)
                        lastRow = root.model.loaded - 1
                    firstRow = Math.max(0, firstRow)
                    lastRow = Math.min(root.model.loaded - 1, lastRow)
                }
                if (inBand === appliedInBand && firstRow === appliedFirstRow
                        && lastRow === appliedLastRow)
                    return
                appliedInBand = inBand
                appliedFirstRow = firstRow
                appliedLastRow = lastRow

                // One call for the whole band. The model holds the selection
                // the drag started from and the rows it already covered, so
                // the move sets flags instead of shipping paths across.
                if (inBand && lastRow >= firstRow)
                    root.model.marqueeCells(firstRow, 0, lastRow, 0, 1)
                else
                    root.model.marqueeCells(-1, -1, -1, -1, 1)
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
                keptBase = (mouse.modifiers & Qt.ControlModifier) !== 0
                root.model.beginMarquee(keptBase)
                startX = mouse.x
                startY = mouse.y
                startContentY = list.contentY + mouse.y
                pointerX = mouse.x
                pointerY = mouse.y
                active = true
                appliedInBand = false
                appliedFirstRow = -2
                appliedLastRow = -2
                // A plain drag starts from an empty selection, and the press is
                // what clears it — before the first move reaches the model.
                if (!keptBase)
                    root.selectionChanged()
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
                root.model.endMarquee()
                // A drag that never travelled is a click in the empty space
                // below the rows, which clears the selection.
                if (width < 4 && height < 4 && !keptBase) {
                    root.clearSelection()
                    root.selectionChanged()
                }
            }

            onCanceled: {
                active = false
                if (root.model !== null)
                    root.model.endMarquee()
            }

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
                color: Theme.accentSoft
                border.width: 1
                border.color: Theme.accent
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
            if (root.model !== null) {
                root.clearSelection()
                root.selectionChanged()
            }
        }

        Rectangle {
            id: dropPlate

            anchors.fill: parent
            visible: dropArea.containsDrag && root.dropTargetPath !== ""
            color: Theme.accentSoft
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
                font.pixelSize: Theme.typeBody
            }
        }

        /// The plate's own edge: the same solid accent frame the row under the
        /// pointer wears, so "this is where it lands" is one mark at both
        /// scales instead of a wash here and a frame there.
        Rectangle {
            anchors.fill: dropPlate
            visible: dropPlate.visible
            color: "transparent"
            border.width: 1.5
            border.color: Theme.accent
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

        /// A worker sort applies after `aboutToReload` saved the position just
        /// before its reset, and this is the matching restore. Without it the
        /// list jumps to the top when a large listing finishes sorting, or
        /// when a column header starts a re-sort.
        function onSortApplied() {
            root.restoreScroll()
        }
    }
}
