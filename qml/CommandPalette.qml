pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Shortcut.js" as Shortcut

// The Mod+K palette: commands, spaces, favorites, recents, projects and a
// progressive filename search in one keyboard-first surface.
//
// The row list is rebuilt from the shared models rather than kept in a second
// store: `Spaces` owns the space list, `OverviewModel` owns the landing page's
// rows, and the palette's own `SearchModel` owns the filename hits. A rebuild
// re-reads those models, ranks the flat candidate set, then buckets the matches
// back into sections so a section header never floats over an unrelated row.
Item {
    id: root

    property var overviewModel: null
    property bool folderActive: false
    property string scopePath: ""
    property bool dark: true
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline
    property color headerColor: Theme.chrome
    property string commandShortcut: "Ctrl+K"
    property string pathShortcut: "Ctrl+P"
    /// The bindings from `settings.toml`, so a hint in the list is the key the
    /// user actually has to press. Commands without an entry keep their
    /// built-in hint because those keys are not part of the shared registry.
    property var shortcutMap: ({})
    property bool isMac: false

    readonly property color panelColor: Theme.raised
    readonly property color activeColor: Qt.rgba(accent.r, accent.g, accent.b,
                                                 dark ? 0.26 : 0.14)
    readonly property color hoverColor: Theme.hover
    readonly property color plateColor: Theme.canvasAlt

    readonly property string trimmedQuery: query.trim()
    readonly property bool pathLike: root.isPathLike(root.trimmedQuery)
    readonly property bool searchingFiles: visible
        && root.trimmedQuery.length >= 2 && !root.pathLike
        && paletteSearchModel.busy

    visible: false
    focus: visible
    z: 1200

    signal activated(string kind, string value)
    signal closed()

    property string query: ""
    property int activeIndex: -1
    property int resultCount: 0

    function openPalette() {
        if (root.visible) {
            input.forceActiveFocus()
            return
        }
        root.query = ""
        root.activeIndex = -1
        input.text = ""
        paletteSearchModel.cancel()
        searchTimer.stop()
        // The palette is the second entry point to Spaces and the overview, so
        // it refreshes both rather than trusting that their page was visited.
        Spaces.refresh()
        if (root.overviewModel !== null)
            root.overviewModel.refresh()
        root.visible = true
        root.rebuild()
        input.forceActiveFocus()
        Qt.callLater(function () {
            if (root.visible)
                input.forceActiveFocus()
        })
    }

    function closePalette() {
        if (!root.visible)
            return
        root.visible = false
        root.query = ""
        root.activeIndex = -1
        input.text = ""
        paletteSearchModel.cancel()
        searchTimer.stop()
        root.closed()
    }

    function escapeHtml(text) {
        return String(text)
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;")
    }

    /// Basic `<b>` highlighting for the characters fuzzyMatch consumed. The
    /// label stays StyledText so QML's own eliding still applies.
    function styledLabel(label, indices) {
        const text = String(label)
        if (!indices || indices.length === 0)
            return root.escapeHtml(text)
        const marked = ({})
        for (let index = 0; index < indices.length; index += 1)
            marked[indices[index]] = true
        let output = ""
        for (let index = 0; index < text.length; index += 1) {
            const char = root.escapeHtml(text.charAt(index))
            output += marked[index] === true ? "<b>" + char + "</b>" : char
        }
        return output
    }

    /// Drive letter, home alias, UNC share or POSIX root. A path-shaped query
    /// is a jump target, not a filename to search for.
    function isPathLike(text) {
        return /^([a-zA-Z]:[\\/]|~(?=$|[\\/])|\\\\|\/)/.test(text)
    }

    function isWordStart(text, index) {
        if (index === 0)
            return true
        const previous = text.charAt(index - 1)
        if (/[\s\-_./\\:]/.test(previous))
            return true
        return previous === previous.toLowerCase()
            && text.charAt(index) !== text.charAt(index).toLowerCase()
    }

    /// Ordered subsequence matching, scored the way the React shell scored its
    /// palette: earlier matches, word starts and consecutive runs rank higher;
    /// shorter targets win otherwise-equal ties.
    function fuzzyMatch(queryText, target) {
        const q = queryText.toLowerCase()
        if (q.length === 0)
            return { score: 0, indices: [] }
        const source = String(target)
        const lower = source.toLowerCase()
        const indices = []
        let score = 0
        let from = 0
        let previous = -1
        for (let index = 0; index < q.length; index += 1) {
            const found = lower.indexOf(q.charAt(index), from)
            if (found < 0)
                return null
            indices.push(found)
            score += 2
            if (found === 0)
                score += 8
            else if (root.isWordStart(source, found))
                score += 6
            if (previous >= 0)
                score += found === previous + 1 ? 4 : -1
            from = found + 1
            previous = found
        }
        score -= Math.floor(lower.length / 8)
        return { score: score, indices: indices }
    }

    function shortcutHint(id, fallback) {
        const binding = root.shortcutMap ? root.shortcutMap[id] : undefined
        return binding === undefined ? fallback : Shortcut.display(binding, root.isMac)
    }

    function command(id, label, hint, icon, group) {
        return {
            group: group,
            label: label,
            hint: hint,
            icon: icon,
            kind: "command",
            value: id,
            keywords: label + " " + id
        }
    }

    /// Commands that are always reachable. The file and view groups are
    /// appended only while a folder surface is active, because their ids are
    /// no-ops on the overview, a space or the recycle bin.
    function navigationCommands() {
        const items = [
            root.command("overview", qsTr("概览"), "", "\u2302", qsTr("导航")),
            root.command("trash", qsTr("回收站"), "", "\u232b", qsTr("导航")),
            root.command("new-tab", qsTr("新建标签页"), "Ctrl+T", "+", qsTr("导航")),
            root.command("close-tab", qsTr("关闭标签页"), "Ctrl+W", "\u00d7", qsTr("导航")),
            root.command("new-space", qsTr("新建空间…"), "", "\u25c8", qsTr("导航")),
            root.command("settings", qsTr("设置"), root.shortcutHint("app.openSettings", "Ctrl+,"),
                         "\u2699", qsTr("导航"))
        ]
        if (root.folderActive)
            items.push(root.command("path-jump", qsTr("跳到路径…"), root.pathShortcut, "\u2192", qsTr("导航")))
        return items
    }

    function fileCommands() {
        return [
            root.command("new-folder", qsTr("新建文件夹"), "Ctrl+Shift+N", "\u25a3", qsTr("文件")),
            root.command("new-file", qsTr("新建文件"), "", "\u25a4", qsTr("文件")),
            root.command("rename", qsTr("重命名"), root.shortcutHint("explorer.rename", "F2"),
                         "\u270e", qsTr("文件")),
            root.command("trash-selection", qsTr("移到回收站"),
                         root.shortcutHint("explorer.trash", "Delete"), "\u232b", qsTr("文件")),
            root.command("copy", qsTr("复制"), root.shortcutHint("explorer.copy", "Ctrl+C"),
                         "\u29c9", qsTr("文件")),
            root.command("cut", qsTr("剪切"), root.shortcutHint("explorer.cut", "Ctrl+X"),
                         "\u2702", qsTr("文件")),
            root.command("paste", qsTr("粘贴"), root.shortcutHint("explorer.paste", "Ctrl+V"),
                         "\u2398", qsTr("文件")),
            root.command("copy-path", qsTr("复制路径"), "", "\u2310", qsTr("文件")),
            root.command("select-all", qsTr("全选"),
                         root.shortcutHint("explorer.selectAll", "Ctrl+A"),
                         "\u2713", qsTr("文件")),
            root.command("refresh", qsTr("刷新"), "F5", "\u21bb", qsTr("文件")),
            root.command("go-up", qsTr("上一级"), "Alt+Up", "\u2191", qsTr("文件")),
            root.command("open-terminal", qsTr("在终端中打开"),
                         root.shortcutHint("explorer.openSystemTerminal", "Ctrl+`"),
                         ">_", qsTr("文件")),
            root.command("toggle-favorite", qsTr("加入收藏"), "", "\u2605", qsTr("文件")),
            root.command("toggle-split", qsTr("切换分栏"), "", "\u25a5", qsTr("文件")),
            root.command("toggle-preview", qsTr("切换预览"),
                         root.shortcutHint("explorer.preview", "Space"),
                         "\u25eb", qsTr("文件")),
            root.command("toggle-sidebar", qsTr("切换侧栏"), "Ctrl+B", "\u25e7", qsTr("文件"))
        ]
    }

    function viewCommands() {
        return [
            root.command("view-list", qsTr("列表视图"), "", "\u2630", qsTr("视图")),
            root.command("view-grid", qsTr("网格视图"), "", "\u229e", qsTr("视图")),
            root.command("view-column", qsTr("分栏视图"), "", "\u25a5", qsTr("视图")),
            root.command("sort-name", qsTr("按名称排序"), "", "\u21c5", qsTr("视图")),
            root.command("sort-modified", qsTr("按修改时间排序"), "", "\u21c5", qsTr("视图")),
            root.command("sort-type", qsTr("按类型排序"), "", "\u21c5", qsTr("视图")),
            root.command("sort-size", qsTr("按大小排序"), "", "\u21c5", qsTr("视图")),
            root.command("sort-order", qsTr("切换升降序"), "", "\u21f5", qsTr("视图")),
            root.command("folders-first", qsTr("文件夹优先"), "", "\u25a4", qsTr("视图")),
            root.command("hidden-files", qsTr("显示隐藏文件"),
                         root.shortcutHint("explorer.toggleHidden", "Ctrl+H"),
                         "\u25c9", qsTr("视图")),
            root.command("filters-reset", qsTr("清除过滤"), "", "\u2a2f", qsTr("视图"))
        ]
    }

    function spaceCandidates() {
        const items = []
        for (let index = 0; index < Spaces.count; index += 1) {
            const name = Spaces.nameAt(index)
            const count = Spaces.itemCountAt(index)
            items.push({
                group: qsTr("空间"),
                label: qsTr("打开空间 “") + name + "”",
                hint: count > 0 ? count + qsTr(" 项") : qsTr("空"),
                icon: "\u25c8",
                kind: "space",
                value: Spaces.idAt(index),
                keywords: qsTr("空间 space workspace ") + name
            })
        }
        return items
    }

    function overviewCandidates() {
        const items = []
        if (root.overviewModel === null)
            return items
        for (let index = 0; index < root.overviewModel.count; index += 1) {
            const kind = root.overviewModel.kindAt(index)
            // Spaces are already listed from the singleton, which is the
            // authoritative source and does not wait for the project scan.
            if (kind === "space")
                continue
            const name = root.overviewModel.nameAt(index)
            const path = root.overviewModel.pathAt(index)
            const detail = root.overviewModel.detailAt(index)
            const isDir = root.overviewModel.isDirAt(index)
            if (kind === "favorite") {
                items.push({
                    group: qsTr("收藏"),
                    label: name,
                    hint: path,
                    icon: "\u2605",
                    kind: "navigate",
                    value: path,
                    keywords: qsTr("收藏 favorite ") + path
                })
            } else if (kind === "recent") {
                items.push({
                    group: qsTr("最近"),
                    label: name,
                    hint: path,
                    icon: isDir ? "\u25b1" : "\u25a1",
                    kind: isDir ? "navigate" : "file",
                    value: path,
                    keywords: qsTr("最近 recent ") + path
                })
            } else if (kind === "project") {
                items.push({
                    group: qsTr("项目"),
                    label: name,
                    hint: path,
                    icon: "\u2325",
                    kind: "navigate",
                    value: path,
                    keywords: qsTr("项目 project ") + detail
                })
            }
        }
        return items
    }

    function fileCandidates() {
        const items = []
        if (root.trimmedQuery.length < 2 || root.pathLike)
            return items
        if (paletteSearchModel.query !== root.trimmedQuery)
            return items
        const limit = Math.min(paletteSearchModel.loaded, 12)
        for (let index = 0; index < limit; index += 1) {
            const isDir = paletteSearchModel.isDirectoryAt(index)
            const path = paletteSearchModel.pathAt(index)
            items.push({
                group: qsTr("文件"),
                label: paletteSearchModel.nameAt(index),
                hint: paletteSearchModel.relativePathAt(index),
                icon: isDir ? "\u25b1" : "\u25a1",
                kind: isDir ? "navigate" : "file",
                value: path,
                keywords: qsTr("文件 file search ") + path
            })
        }
        return items
    }

    function pathCandidate() {
        if (!root.pathLike)
            return null
        const target = root.trimmedQuery
        return {
            group: qsTr("跳转"),
            label: qsTr("跳转到 “") + target + "”",
            hint: target,
            icon: "\u2192",
            kind: "path",
            value: target,
            keywords: qsTr("跳转 path jump ") + target
        }
    }

    /// Replaces the flat row list. Headers come from `groupLabel` and the
    /// ListView's section support, so the keyboard index space stays one row
    /// per result.
    function rebuild() {
        const candidates = []
        const direct = root.pathCandidate()
        if (direct !== null)
            candidates.push(direct)
        candidates.push.apply(candidates, root.navigationCommands())
        candidates.push.apply(candidates, root.spaceCandidates())
        candidates.push.apply(candidates, root.overviewCandidates())
        if (root.folderActive) {
            candidates.push.apply(candidates, root.fileCommands())
            candidates.push.apply(candidates, root.viewCommands())
        }
        candidates.push.apply(candidates, root.fileCandidates())

        let ordered = []
        const q = root.trimmedQuery
        if (q.length === 0) {
            // Registration order, which keeps each source's own ordering and
            // puts the four landing-page sections after the fixed commands.
            for (let index = 0; index < candidates.length; index += 1)
                ordered.push({ item: candidates[index], indices: [] })
        } else {
            const ranked = []
            for (let index = 0; index < candidates.length; index += 1) {
                const item = candidates[index]
                const match = root.fuzzyMatch(q, item.label + " " + item.hint
                                               + " " + item.keywords)
                if (match === null)
                    continue
                ranked.push({ item: item, score: match.score, indices: match.indices,
                              order: index })
            }
            ranked.sort(function (left, right) {
                if (right.score !== left.score)
                    return right.score - left.score
                return left.order - right.order
            })

            // Bucket by section, with the buckets themselves ordered by their
            // best match. Inside a bucket the global score order is kept.
            const buckets = []
            const byGroup = ({})
            for (let index = 0; index < ranked.length; index += 1) {
                const entry = ranked[index]
                let bucket = byGroup[entry.item.group]
                if (bucket === undefined) {
                    bucket = { group: entry.item.group, entries: [] }
                    byGroup[entry.item.group] = bucket
                    buckets.push(bucket)
                }
                bucket.entries.push(entry)
            }
            for (let index = 0; index < buckets.length; index += 1) {
                const bucket = buckets[index]
                for (let row = 0; row < bucket.entries.length; row += 1) {
                    ordered.push({ item: bucket.entries[row].item,
                                   indices: bucket.entries[row].indices })
                }
            }
        }

        results.clear()
        for (let index = 0; index < ordered.length; index += 1) {
            const item = ordered[index].item
            const indices = ordered[index].indices || []
            results.append({
                groupLabel: item.group,
                label: item.label,
                styledLabel: root.styledLabel(item.label, indices),
                hint: item.hint,
                icon: item.icon,
                kind: item.kind,
                value: item.value
            })
        }
        root.resultCount = results.count
        // `currentIndex` is an index into the model, not into the sectioned
        // rows: section headers are painted by the view and cost no index.
        list.currentIndex = results.count > 0 ? 0 : -1
        root.activeIndex = list.currentIndex
        if (list.currentIndex >= 0)
            list.positionViewAtBeginning()
    }

    function scheduleUpdate() {
        root.rebuild()
        const q = root.trimmedQuery
        if (q.length >= 2 && !root.pathLike) {
            searchTimer.restart()
        } else {
            searchTimer.stop()
            paletteSearchModel.cancel()
        }
    }

    function runFileSearch() {
        if (!root.visible)
            return
        const q = root.trimmedQuery
        if (q.length < 2 || root.pathLike)
            return
        const scope = root.scopePath !== "" ? root.scopePath : Workspace.homeDirectory
        if (scope === "")
            return
        paletteSearchModel.search(scope, q)
    }

    function takeFocus() {
        input.forceActiveFocus()
    }

    function moveCurrent(delta) {
        if (results.count === 0)
            return
        const next = (list.currentIndex + delta + results.count) % results.count
        list.currentIndex = next
        root.activeIndex = next
        list.positionViewAtIndex(next, ListView.Contain)
    }

    function activateCurrent() {
        if (list.currentIndex < 0 || list.currentIndex >= results.count)
            return
        const row = results.get(list.currentIndex)
        root.activated(row.kind, row.value)
    }

    SearchModel {
        id: paletteSearchModel
    }

    ListModel {
        id: results
    }

    Timer {
        id: searchTimer
        interval: 220
        repeat: false
        onTriggered: root.runFileSearch()
    }

    Connections {
        target: paletteSearchModel

        function onLoadedChanged() {
            if (root.visible)
                root.rebuild()
        }

        function onQueryChanged() {
            if (root.visible)
                root.rebuild()
        }
    }

    Connections {
        target: root.overviewModel

        function onCountChanged() {
            if (root.visible)
                root.rebuild()
        }

        function onProjectBusyChanged() {
            if (root.visible)
                root.rebuild()
        }
    }

    onQueryChanged: root.scheduleUpdate()
    onFolderActiveChanged: {
        if (root.visible)
            root.rebuild()
    }

    // The overlay dims the window, eats its own clicks and closes on the empty
    // space, like every other modal in this shell.
    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.closePalette()

            onWheel: function (wheel) { wheel.accepted = true }
        }
    }

    Rectangle {
        id: panel

        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.max(56, Math.round(root.height * 0.12))
        width: Math.min(680, root.width - 48)
        height: Math.min(470, root.height - y - 96)
        radius: 12
        color: root.panelColor
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: function (wheel) { wheel.accepted = true }
        }

        // Search field. The binding sits at the trailing edge as plain text,
        // not as a keycap: the palette is a document, not a keyboard tester.
        Item {
            id: field

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 58

            Text {
                x: 18
                anchors.verticalCenter: parent.verticalCenter
                text: "\u2315"
                color: root.mutedColor
                font.pixelSize: 17
            }

            TextInput {
                id: input

                anchors.left: parent.left
                anchors.leftMargin: 46
                anchors.right: shortcutText.left
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                height: 26
                color: root.textColor
                selectionColor: root.accent
                selectedTextColor: Theme.onInteractive
                font.pixelSize: 16
                verticalAlignment: TextInput.AlignVCenter
                focus: true
                clip: true

                onTextEdited: root.query = text

                Keys.onPressed: function (event) {
                    if (event.key === Qt.Key_Down) {
                        root.moveCurrent(1)
                        event.accepted = true
                    } else if (event.key === Qt.Key_Up) {
                        root.moveCurrent(-1)
                        event.accepted = true
                    } else if (event.key === Qt.Key_PageDown) {
                        root.moveCurrent(8)
                        event.accepted = true
                    } else if (event.key === Qt.Key_PageUp) {
                        root.moveCurrent(-8)
                        event.accepted = true
                    } else if (event.key === Qt.Key_Home) {
                        if (results.count > 0) {
                            list.currentIndex = 0
                            root.activeIndex = 0
                        }
                        event.accepted = true
                    } else if (event.key === Qt.Key_End) {
                        if (results.count > 0) {
                            list.currentIndex = results.count - 1
                            root.activeIndex = list.currentIndex
                        }
                        event.accepted = true
                    } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                        root.activateCurrent()
                        event.accepted = true
                    } else if (event.key === Qt.Key_Escape) {
                        root.closePalette()
                        event.accepted = true
                    }
                }
            }

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 46
                anchors.verticalCenter: parent.verticalCenter
                visible: input.text.length === 0
                text: root.pathLike ? qsTr("跳到路径…") : qsTr("搜索文件、空间或命令")
                color: root.mutedColor
                font.pixelSize: 15
            }

            Text {
                id: shortcutText
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                text: root.pathLike ? root.pathShortcut : root.commandShortcut
                color: root.mutedColor
                font.pixelSize: 11
            }
        }

        Rectangle {
            id: fieldRule

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: field.bottom
            height: 1
            color: root.hairline
        }

        // Results. Section headers are painted by the view; `currentIndex` is
        // still the flat result index the keyboard moves in.
        ListView {
            id: list

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: fieldRule.bottom
            anchors.bottom: footerBar.top
            anchors.margins: 6
            clip: true
            focus: false
            model: results
            boundsBehavior: Flickable.StopAtBounds
            cacheBuffer: 200
            section.property: "groupLabel"
            section.criteria: ViewSection.FullString
            section.delegate: Item {
                required property string section

                width: list.width
                height: 30

                Text {
                    x: 12
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 5
                    text: section
                    color: root.mutedColor
                    font.pixelSize: 10
                    font.weight: Font.DemiBold
                    font.letterSpacing: 0.6
                }
            }

            delegate: Item {
                id: row

                required property int index
                required property string label
                required property string styledLabel
                required property string hint
                required property string icon
                required property string kind
                required property string value

                width: list.width
                height: 40

                readonly property bool current: list.currentIndex === row.index

                Rectangle {
                    anchors.fill: parent
                    anchors.leftMargin: 6
                    anchors.rightMargin: 6
                    radius: 7
                    color: row.current ? root.activeColor
                        : (rowMouse.containsMouse ? root.hoverColor : "transparent")

                    Rectangle {
                        visible: row.current
                        anchors.left: parent.left
                        anchors.leftMargin: 1
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        anchors.topMargin: 8
                        anchors.bottomMargin: 8
                        width: 2
                        radius: 1
                        color: root.accent
                    }
                }

                Rectangle {
                    x: 18
                    anchors.verticalCenter: parent.verticalCenter
                    width: 22
                    height: 22
                    radius: 5
                    color: Theme.canvasAlt

                    Text {
                        anchors.centerIn: parent
                        text: row.icon
                        color: row.kind === "space" ? root.accent : root.mutedColor
                        font.pixelSize: 12
                    }
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 50
                    anchors.right: hintText.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.styledLabel
                    textFormat: Text.StyledText
                    color: root.textColor
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }

                Text {
                    id: hintText

                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(220, implicitWidth)
                    text: row.hint
                    color: root.mutedColor
                    font.pixelSize: 10
                    elide: Text.ElideMiddle
                    horizontalAlignment: Text.AlignRight
                }

                MouseArea {
                    id: rowMouse

                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        list.currentIndex = row.index
                        root.activeIndex = row.index
                        root.activateCurrent()
                    }
                }
            }
        }

        // Empty state: says whether the filename traversal is still running,
        // because "no results" while the scan continues would be a lie.
        Text {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: fieldRule.bottom
            anchors.bottom: footerBar.top
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            visible: results.count === 0
            text: root.searchingFiles
                ? qsTr("正在搜索文件…")
                : (root.trimmedQuery.length === 0
                    ? qsTr("没有可用的入口")
                    : qsTr("没有匹配 “") + root.trimmedQuery + "”")
            color: root.mutedColor
            font.pixelSize: 12
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            wrapMode: Text.Wrap
        }

        Rectangle {
            id: footerBar

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 32
            color: "transparent"

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: root.hairline
            }

            Text {
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                text: root.searchingFiles
                    ? qsTr("搜索文件…")
                    : (root.resultCount > 0
                        ? root.resultCount + qsTr(" 个结果 · ↑↓ 选择 · ↵ 执行")
                        : qsTr("↑↓ 选择 · ↵ 执行 · Esc 关闭"))
                color: root.mutedColor
                font.pixelSize: 10
            }

            Text {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                text: "Esc"
                color: root.mutedColor
                font.pixelSize: 10
            }
        }
    }
}
