pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Shortcut.js" as ShortcutJS

// The Mod+K palette: commands, spaces, favorites, recents, projects and a
// progressive filename search in one keyboard-first surface. Filename and
// content search are local-only, so a network scope keeps commands and jumps
// but never starts a search walk.
//
// The row list is rebuilt from the shared models rather than kept in a second
// store: `Spaces` owns the space list, `OverviewModel` owns the landing page's
// rows, and the palette's own `SearchModel` owns the filename hits. A rebuild
// re-reads those models, ranks the flat candidate set, then buckets the matches
// back into sections so a section header never floats over an unrelated row.
//
// Geometry follows the prototype: 560 wide, 64 down from the top of the
// window, 8 px corners, a 44 px input band and 30 px rows. The older version
// was 680 wide with 40 px rows and a tinted plate behind every mark, which made
// a list of sixty one-line commands read like a grid of tiles.
Item {
    id: root

    property var overviewModel: null
    property bool folderActive: false
    property bool remoteLocation: false
    property string scopePath: ""
    property bool dark: true
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline
    /// The bindings from `settings.toml`, so a hint in the list is the key the
    /// user actually has to press. Commands without an entry keep their
    /// built-in hint because those keys are not part of the shared registry.
    property var shortcutMap: ({})
    property bool isMac: false

    readonly property color panelColor: Theme.raised

    readonly property string trimmedQuery: query.trim()
    readonly property bool pathLike: root.isPathLike(root.trimmedQuery)
    readonly property bool searchingFiles: visible && !root.remoteLocation
        && root.trimmedQuery.length >= 2 && !root.pathLike
        && paletteSearchModel.busy

    visible: false
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        NumberAnimation { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1200

    /// One dialog, one name: everything inside it is announced relative to the
    /// surface, so the scrim's click target and the keycaps never turn up as
    /// controls of their own.
    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("命令面板")

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

    /// Rows print the chord the user actually has, through the same formatter
    /// the menus use, so `Mod+K` reads `⌘K` on macOS and never `Meta+K`.
    function shortcutHint(id, fallback) {
        const binding = root.shortcutMap ? root.shortcutMap[id] : undefined
        if (binding === undefined)
            return fallback === "" ? "" : ShortcutJS.display(fallback, root.isMac)
        return ShortcutJS.display(binding, root.isMac)
    }

    /// Chords the shell decides for itself instead of reading from
    /// `settings.toml` — new tab, close tab, new folder — still print through
    /// the shared formatter, so a macOS palette reads `⌘T` and not `Ctrl+T`.
    function chord(binding) {
        return ShortcutJS.display(binding, root.isMac)
    }

    function command(id, label, hint, glyph, group) {
        return {
            group: group,
            label: label,
            hint: hint,
            glyph: glyph,
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
            root.command("overview", qsTr("概览"), "", "home", qsTr("导航")),
            root.command("trash", qsTr("回收站"), "", "trash", qsTr("导航")),
            root.command("new-tab", qsTr("新建标签页"), root.chord("Mod+T"), "plus", qsTr("导航")),
            // This one lost its button when the chrome bar became one row: the
            // strip is the window's drag handle now, so a permanent button in it
            // costs the only empty space there is.
            root.command("new-window", qsTr("新建窗口"), "", "", qsTr("导航")),
            root.command("close-tab", qsTr("关闭标签页"), root.chord("Mod+W"), "close", qsTr("导航")),
            root.command("new-space", qsTr("新建空间…"), "", "diamond", qsTr("导航")),
            root.command("settings", qsTr("设置"),
                         root.shortcutHint("app.openSettings", "Ctrl+,"),
                         "gear", qsTr("导航"))
        ]
        if (root.folderActive)
            items.push(root.command("path-jump", qsTr("跳到路径…"),
                                    root.shortcutHint("app.pathJump", "Ctrl+L"),
                                    "chevron", qsTr("导航")))
        return items
    }

    function fileCommands() {
        const items = [
            root.command("new-folder", qsTr("新建文件夹"), root.chord("Mod+Shift+N"), "folder", qsTr("文件")),
            root.command("new-file", qsTr("新建文件"), "", "file", qsTr("文件")),
            root.command("rename", qsTr("重命名"), root.shortcutHint("explorer.rename", "F2"),
                         "", qsTr("文件")),
            root.command("trash-selection", qsTr("移到回收站"),
                         root.shortcutHint("explorer.trash", "Delete"), "trash", qsTr("文件")),
            root.command("copy", qsTr("复制"), root.shortcutHint("explorer.copy", "Ctrl+C"),
                         "", qsTr("文件")),
            root.command("cut", qsTr("剪切"), root.shortcutHint("explorer.cut", "Ctrl+X"),
                         "", qsTr("文件")),
            root.command("paste", qsTr("粘贴"), root.shortcutHint("explorer.paste", "Ctrl+V"),
                         "", qsTr("文件")),
            root.command("copy-path", qsTr("复制路径"), "", "link", qsTr("文件")),
            root.command("select-all", qsTr("全选"),
                         root.shortcutHint("explorer.selectAll", "Ctrl+A"),
                         "check", qsTr("文件")),
            root.command("refresh", qsTr("刷新"), "F5", "", qsTr("文件")),
            root.command("go-back", qsTr("后退"),
                         root.shortcutHint("explorer.goBack", "Alt+Left"),
                         "", qsTr("文件")),
            root.command("go-forward", qsTr("前进"),
                         root.shortcutHint("explorer.goForward", "Alt+Right"),
                         "", qsTr("文件")),
            root.command("go-up", qsTr("上一级"), "Alt+Up", "", qsTr("文件")),
            root.command("open-terminal", qsTr("在终端中打开"),
                         root.shortcutHint("explorer.openSystemTerminal", "Ctrl+`"),
                         "terminal", qsTr("文件")),
            root.command("toggle-favorite", qsTr("加入收藏"), "", "star", qsTr("文件")),
            root.command("toggle-split", qsTr("切换分栏"), "", "column", qsTr("文件")),
            root.command("toggle-preview", qsTr("切换预览"),
                         root.shortcutHint("explorer.preview", "Space"),
                         "panel", qsTr("文件")),
            root.command("toggle-sidebar", qsTr("切换侧栏"), root.chord("Mod+B"), "panel", qsTr("文件"))
        ]
        // Trash and terminal have no remote implementation, so neither row
        // should be offered on a network tab.
        return root.remoteLocation
            ? items.filter(function (item) {
                return item.value !== "trash-selection" && item.value !== "open-terminal"
            })
            : items
    }

    function viewCommands() {
        return [
            root.command("view-list", qsTr("列表视图"), "", "list", qsTr("视图")),
            root.command("view-grid", qsTr("网格视图"), "", "grid", qsTr("视图")),
            root.command("view-column", qsTr("分栏视图"), "", "column", qsTr("视图")),
            root.command("sort-name", qsTr("按名称排序"), "", "", qsTr("视图")),
            root.command("sort-modified", qsTr("按修改时间排序"), "", "", qsTr("视图")),
            root.command("sort-type", qsTr("按类型排序"), "", "", qsTr("视图")),
            root.command("sort-size", qsTr("按大小排序"), "", "", qsTr("视图")),
            root.command("sort-order", qsTr("切换升降序"), "", "", qsTr("视图")),
            root.command("folders-first", qsTr("文件夹优先"), "", "folder", qsTr("视图")),
            root.command("hidden-files", qsTr("显示隐藏文件"),
                         root.shortcutHint("explorer.toggleHidden", "Ctrl+H"),
                         "", qsTr("视图")),
            root.command("filters-reset", qsTr("清除过滤"), "", "close", qsTr("视图"))
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
                glyph: "diamond",
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
                    glyph: "star",
                    kind: "navigate",
                    value: path,
                    keywords: qsTr("收藏 favorite ") + path
                })
            } else if (kind === "recent") {
                items.push({
                    group: qsTr("最近"),
                    label: name,
                    hint: path,
                    glyph: isDir ? "folder" : "file",
                    kind: isDir ? "navigate" : "file",
                    value: path,
                    keywords: qsTr("最近 recent ") + path
                })
            } else if (kind === "project") {
                items.push({
                    group: qsTr("项目"),
                    label: name,
                    hint: path,
                    glyph: "folder",
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
        if (root.remoteLocation || root.trimmedQuery.length < 2 || root.pathLike)
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
                glyph: isDir ? "folder" : "file",
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
            glyph: "chevron",
            kind: "path",
            value: target,
            keywords: qsTr("跳转 path jump ") + target
        }
    }

    /// The last row of a query that found nothing. Filename search answers
    /// "no such name"; this offers the one cheaper question left, which is
    /// whether the text is inside a file rather than in its name.
    function contentSearchCandidate() {
        if (root.remoteLocation || root.trimmedQuery.length < 2
                || root.pathLike || !root.folderActive)
            return null
        return {
            group: qsTr("搜索"),
            label: qsTr("在文件内容中搜索 “") + root.trimmedQuery + "”",
            hint: qsTr("内容搜索"),
            glyph: "search",
            kind: "content-search",
            value: root.trimmedQuery,
            keywords: qsTr("内容 content grep 搜索 ") + root.trimmedQuery
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
        const fallback = root.contentSearchCandidate()
        if (fallback !== null)
            candidates.push(fallback)

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
                for (let row = 0; row < bucket.entries.length; row += 1)
                    ordered.push({ item: bucket.entries[row].item,
                                   indices: bucket.entries[row].indices })
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
                glyph: item.glyph,
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
        if (!root.remoteLocation && q.length >= 2 && !root.pathLike) {
            searchTimer.restart()
        } else {
            searchTimer.stop()
            paletteSearchModel.cancel()
        }
    }

    function runFileSearch() {
        if (!root.visible || root.remoteLocation)
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
        y: 64
        width: Math.min(560, root.width - 32)
        height: Math.min(430, root.height - y - 96)
        radius: Theme.radiusMd
        color: root.panelColor
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: function (wheel) { wheel.accepted = true }
        }

        // Search field: a mark, the query, and the way out. The palette's own
        // chord used to sit here as text; it is stated by the footer now, so
        // the band belongs to the query.
        Item {
            id: field

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 44

            Glyph {
                x: 12
                width: 14
                height: 14
                anchors.verticalCenter: parent.verticalCenter
                kind: "search"
                tint: Theme.inkMuted
            }

            TextInput {
                id: input

                anchors.left: parent.left
                anchors.leftMargin: 37
                anchors.right: escChip.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                height: 24
                color: root.textColor
                selectionColor: root.accent
                selectedTextColor: Theme.onInteractive
                font.pixelSize: Theme.typeEmphasis
                verticalAlignment: TextInput.AlignVCenter
                focus: true
                clip: true

                // The palette's field is its only text input, and it has no
                // visible label: the name and the hint have to come from here
                // or the field is announced as "text".
                Accessible.role: Accessible.EditableText
                Accessible.name: qsTr("命令面板搜索")
                Accessible.description: qsTr("搜索文件、空间或命令")

                // `textEdited` covers keystrokes only: a paste, or any other
                // programmatic write into the field, leaves the query at its
                // old value and the list unfiltered while the text sits in
                // the box. The guard keeps the two from chasing each other
                // when the palette clears itself on close.
                onTextChanged: {
                    if (root.query !== text)
                        root.query = text
                }

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
                anchors.leftMargin: 37
                anchors.verticalCenter: parent.verticalCenter
                visible: input.text.length === 0
                text: root.pathLike ? qsTr("跳到路径…") : qsTr("搜索文件、空间或命令")
                color: Theme.inkFaint
                font.pixelSize: Theme.typeEmphasis
            }

            // 20 × 18, the one keycap in the surface: there is no button to
            // close the palette with, so the way out has to be stated.
            Rectangle {
                id: escChip

                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 24
                height: 18
                radius: Theme.radiusXs
                color: "transparent"
                border.width: 1
                border.color: root.hairline

                Text {
                    anchors.centerIn: parent
                    text: "Esc"
                    color: Theme.inkFaint
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                }
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
            anchors.leftMargin: 4
            anchors.rightMargin: 4
            anchors.topMargin: 4
            anchors.bottomMargin: 4
            clip: true
            focus: false
            model: results
            boundsBehavior: Flickable.StopAtBounds
            cacheBuffer: 200

            /// The rows are the palette's answer to the query; the section
            /// headers painted between them are labels on a list, not entries.
            Accessible.role: Accessible.List
            Accessible.name: qsTr("命令面板结果")

            section.property: "groupLabel"
            section.criteria: ViewSection.FullString
            section.delegate: Item {
                required property string section

                width: list.width
                height: 22

                Text {
                    x: 10
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 3
                    text: section
                    color: Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.weight: Font.Medium
                    font.letterSpacing: 0.6
                }
            }

            delegate: Item {
                id: row

                required property int index
                required property string label
                required property string styledLabel
                required property string hint
                required property string glyph
                required property string kind
                required property string value

                width: list.width
                height: 30

                readonly property bool current: list.currentIndex === row.index

                /// The chord is already drawn in the row, but it is text; the
                /// description carries it to a reader that never sees the plate.
                Accessible.role: Accessible.ListItem
                Accessible.name: row.label
                Accessible.description: row.hint
                Accessible.selectable: true
                Accessible.selected: row.current
                Accessible.onPressAction: {
                    list.currentIndex = row.index
                    root.activeIndex = row.index
                    root.activateCurrent()
                }

                // The active row is the shell's selection wash plus a 2 px
                // rail, the same pair the listing uses: hover alone is too
                // weak for the row Enter is about to run.
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: row.current ? Theme.signalSoft
                        : (rowMouse.containsMouse ? Theme.hover : "transparent")

                    Rectangle {
                        visible: row.current
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        anchors.topMargin: 6
                        anchors.bottomMargin: 6
                        width: 2
                        radius: Theme.radiusXs
                        color: Theme.signal
                    }
                }

                Glyph {
                    x: 9
                    width: 14
                    height: 14
                    anchors.verticalCenter: parent.verticalCenter
                    kind: row.glyph
                    tint: row.current ? Theme.inkMuted : Theme.inkFaint
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 32
                    anchors.right: hintText.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.styledLabel
                    textFormat: Text.StyledText
                    color: root.textColor
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                }

                Text {
                    id: hintText

                    anchors.right: parent.right
                    anchors.rightMargin: 9
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(220, implicitWidth)
                    text: row.hint
                    color: Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.family: Theme.monoFamily
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
        // because "no results" while the scan continues would be a lie, and
        // names the two things that still work when it has stopped.
        Column {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: fieldRule.bottom
            anchors.bottom: footerBar.top
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            anchors.topMargin: 16
            spacing: 6
            visible: results.count === 0

            Text {
                width: parent.width
                text: root.searchingFiles
                    ? qsTr("正在搜索文件…")
                    : (root.trimmedQuery.length === 0
                        ? qsTr("没有可用的入口")
                        : qsTr("没有匹配 “") + root.trimmedQuery + "”")
                color: Theme.inkMuted
                font.pixelSize: Theme.typeBody
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: !root.searchingFiles
                text: root.trimmedQuery.length === 0
                    ? qsTr("试试命令名、文件路径，或一个空间的名字。")
                    : qsTr("试试其他关键词，或输入一个路径直接跳转。")
                color: Theme.inkFaint
                font.pixelSize: Theme.typeCaption
                wrapMode: Text.Wrap
            }
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
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("↑↓ 选择") + " · " + qsTr("↵ 执行") + " · " + qsTr("Esc 关闭")
                color: Theme.inkFaint
                font.pixelSize: Theme.typeMicro
            }

            Text {
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: root.searchingFiles ? qsTr("搜索文件…")
                    : qsTr("%1 个结果").arg(root.resultCount)
                color: Theme.inkFaint
                font.pixelSize: Theme.typeMicro
            }
        }
    }
}
