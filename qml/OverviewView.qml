pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import dae 1.0

// The landing page (§11.1): a quiet, grouped list, not a board.
//
// Titles and instructions are gone. The page opens on 快速访问, then the
// groups the model publishes — 项目, 最近, 磁盘, 标签 — each on the same
// 40 dp row the file list and the trash mount. A section heading is 11 px
// tertiary type, not a card; a destination is never a sticky note.
Item {
    id: root

    property var model: null
    property color accent: Theme.accent
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkSecondary
    property color hairline: Theme.hairline
    property string commandShortcut: "Ctrl+K"

    signal openRequested(string kind, string path)
    signal commandPaletteRequested()

    readonly property color contentColor: Theme.content

    readonly property int sideMargin: 24
    /// The list keeps the file list's readable measure: home is a column of
    /// names, and a name column stretched past 880 stops being scannable.
    readonly property int contentMax: 880
    readonly property int contentWidth: Math.max(320,
        Math.min(width - sideMargin * 2, contentMax))
    readonly property int sectionGap: 26

    /// `[{ name, label, notes: [{ row, kind, name, path, … }] }]`
    property var sections: []
    property var quickAccess: []
    property var focusables: []

    function rebuild() {
        if (root.model === null) {
            root.sections = [];
            root.quickAccess = [];
            return;
        }
        const order = ["Projects", "Recent", "Drives", "Tags", "Favorites"]
        const buckets = ({})
        const total = root.model.count
        for (let row = 0; row < total; ++row) {
            const section = root.model.sectionAt(row)
            if (buckets[section] === undefined)
                buckets[section] = []
            buckets[section].push(row)
        }

        const names = []
        for (let index = 0; index < order.length; ++index) {
            if (buckets[order[index]] !== undefined)
                names.push(order[index])
        }
        for (const name in buckets) {
            if (order.indexOf(name) < 0)
                names.push(name)
        }

        let quick = []
        const sections = []
        for (let index = 0; index < names.length; ++index) {
            const name = names[index]
            const rows = buckets[name]
            if (rows === undefined || rows.length === 0)
                continue
            const notes = []
            for (let at = 0; at < rows.length; ++at) {
                const row = rows[at]
                notes.push({
                    row: row,
                    kind: root.model.kindAt(row),
                    name: root.model.nameAt(row),
                    path: root.model.pathAt(row),
                    detail: root.model.detailAt(row),
                    icon: root.model.iconAt(row),
                    isDir: root.model.isDirAt(row),
                    branch: root.model.branchAt(row),
                    changeText: root.model.changeTextAt(row),
                    timeText: root.model.timeTextAt(row)
                })
            }
            if (name === "Favorites")
                quick = notes.slice(0, 6)
            else
                sections.push({ name: name, label: root.sectionLabel(name), notes: notes })
        }

        root.quickAccess = quick
        root.sections = sections
    }

    function takeFocus() {
        if (root.focusables.length > 0 && root.focusables[0] !== null)
            root.focusables[0].forceActiveFocus()
    }

    function registerFocusable(item) {
        if (root.focusables.indexOf(item) < 0)
            root.focusables.push(item)
    }

    function unregisterFocusable(item) {
        const at = root.focusables.indexOf(item)
        if (at >= 0)
            root.focusables.splice(at, 1)
    }

    function glyphKind(icon) {
        switch (icon) {
        case "home": return "home"
        case "desktop": return "desktop"
        case "documents": return "documents"
        case "downloads": return "downloads"
        case "pictures": return "pictures"
        case "music": return "music"
        case "videos": return "videos"
        case "star":
        case "favorite": return "star"
        case "project": return "folder"
        case "tag": return "tag"
        case "drive": return "drive"
        case "file": return "file"
        default: return "folder"
        }
    }

    function openNote(note) {
        if (note.kind === "tag")
            root.openRequested("tag", note.path)
        else if (note.kind === "recent" && !note.isDir)
            root.openRequested("file", note.path)
        else
            root.openRequested("navigate", note.path)
    }

    function sectionLabel(value) {
        switch (value) {
        case "Projects": return qsTr("项目")
        case "Favorites": return qsTr("已固定")
        case "Recent": return qsTr("最近")
        case "Tags": return qsTr("标签")
        case "Drives": return qsTr("磁盘")
        default: return value
        }
    }

    function placeLabel(icon) {
        switch (icon) {
        case "home": return qsTr("主目录")
        case "desktop": return qsTr("桌面")
        case "documents": return qsTr("文档")
        case "downloads": return qsTr("下载")
        case "pictures": return qsTr("图片")
        case "music": return qsTr("音乐")
        case "videos": return qsTr("视频")
        default: return ""
        }
    }

    function detailLabel(value) {
        const count = /^(\d+) items?$/.exec(value)
        if (count)
            return qsTr("%1 项").arg(count[1])
        return value
    }

    function changeLabel(value) {
        if (value === "Clean")
            return qsTr("干净")
        if (value === "Untracked")
            return qsTr("未跟踪")
        const count = /^(\d+) changes?$/.exec(value)
        if (count)
            return qsTr("%1 项变更").arg(count[1])
        return value
    }

    function timeLabel(value) {
        if (value === "Today")
            return qsTr("今天")
        if (value === "Yesterday")
            return qsTr("昨天")
        const days = /^(\d+) days ago$/.exec(value)
        if (days)
            return qsTr("%1 天前").arg(days[1])
        return value
    }

    /// §11.1 reads a recent row as name · relative path · time. The row shows
    /// the item's own path with the home prefix dropped (`code\dae`); anything
    /// outside home stays absolute (`K:\backup`). Recents spell separators as
    /// `/` even on Windows, so the comparison normalizes them.
    function relativePath(value) {
        const home = String(Workspace.homeDirectory)
        if (home === "" || value.length <= home.length)
            return value
        const slashed = function(text) { return text.replace(/\\/g, "/") }
        const separator = value.charAt(home.length)
        if (separator !== "/" && separator !== "\\")
            return value
        const head = slashed(value.slice(0, home.length))
        if (Qt.platform.os === "windows"
                ? head.toLowerCase() !== slashed(home).toLowerCase()
                : head !== slashed(home))
            return value
        return value.slice(home.length + 1)
    }

    /// `NTFS · 265 GB / 476 GB` → the filled part of the capacity bar.
    function driveParts(note) {
        const chunks = note.detail.split(" · ")
        const filesystem = chunks.length > 0 ? chunks[0] : ""
        const capacity = chunks.length > 1 ? chunks[1] : ""
        const sizes = capacity.split(" / ")
        return {
            filesystem: filesystem,
            free: sizes.length > 0 ? sizes[0] : "",
            total: sizes.length > 1 ? sizes[1] : ""
        }
    }

    function sizeBytes(text) {
        // Sidebar models spell binary units as `GiB`, so the optional `i`
        // before the `B` is what makes these figures parse at all.
        const match = /^([\d.]+)\s*([KMGTP]?)i?B$/i.exec(text.trim())
        if (match === null)
            return 0
        const exponent = ["", "K", "M", "G", "T", "P"].indexOf(match[2].toUpperCase())
        return Number(match[1]) * Math.pow(1024, exponent)
    }

    function driveRatio(note) {
        const parts = root.driveParts(note)
        const free = root.sizeBytes(parts.free)
        const total = root.sizeBytes(parts.total)
        if (total <= 0)
            return 0
        return Math.max(0, Math.min(1, 1 - free / total))
    }

    onModelChanged: root.rebuild()
    Component.onCompleted: root.rebuild()

    Connections {
        target: root.model
        function onModelReset() { root.rebuild() }
        function onCountChanged() { root.rebuild() }
        function onLoadedChanged() { root.rebuild() }
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    Flickable {
        id: board

        anchors.fill: parent
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        contentWidth: width
        contentHeight: stack.height + 48

        Column {
            id: stack

            x: Math.round((board.width - root.contentWidth) / 2)
            y: 24
            width: root.contentWidth
            spacing: root.sectionGap

            /// ── 快速访问 ─────────────────────────────────────────────────
            Column {
                width: parent.width
                spacing: 8
                visible: root.quickAccess.length > 0

                Text {
                    text: qsTr("快速访问")
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeMicro
                    font.weight: Font.Medium
                }

                Flow {
                    width: parent.width
                    spacing: 8

                    Repeater {
                        model: root.quickAccess

                        delegate: Rectangle {
                            id: chip

                            required property var modelData
                            readonly property bool hovered: chipHover.containsMouse

                            implicitWidth: chipRow.implicitWidth + 24
                            height: 32
                            radius: Theme.radiusSm
                            color: chipHover.containsMouse ? Theme.hover : Theme.inset
                            border.width: 1
                            border.color: chip.activeFocus ? Theme.accent : Theme.outlineVariant

                            activeFocusOnTab: true
                            Accessible.role: Accessible.Button
                            Accessible.name: root.placeLabel(chip.modelData.icon) !== ""
                                ? root.placeLabel(chip.modelData.icon) : chip.modelData.name
                            Accessible.onPressAction: root.openNote(chip.modelData)
                            Keys.onReturnPressed: root.openNote(chip.modelData)
                            Keys.onEnterPressed: root.openNote(chip.modelData)
                            Keys.onSpacePressed: root.openNote(chip.modelData)

                            Component.onCompleted: root.registerFocusable(chip)
                            Component.onDestruction: root.unregisterFocusable(chip)

                            RowLayout {
                                id: chipRow
                                anchors.centerIn: parent
                                spacing: 8

                                Glyph {
                                    width: 16
                                    height: 16
                                    kind: root.glyphKind(chip.modelData.icon)
                                    tint: Theme.inkSecondary
                                }

                                Text {
                                    text: root.placeLabel(chip.modelData.icon) !== ""
                                        ? root.placeLabel(chip.modelData.icon)
                                        : chip.modelData.name
                                    color: Theme.ink
                                    font.pixelSize: Theme.typeBody
                                    textFormat: Text.PlainText
                                }
                            }

                            MouseArea {
                                id: chipHover
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.openNote(chip.modelData)
                            }
                        }
                    }
                }
            }

            /// ── 项目 / 最近 / 磁盘 / 标签 ────────────────────────────────
            Repeater {
                model: root.sections

                delegate: Column {
                    id: section

                    required property var modelData
                    required property int index

                    width: stack.width
                    spacing: 6

                    Text {
                        text: section.modelData.label
                        color: Theme.inkTertiary
                        font.pixelSize: Theme.typeMicro
                        font.weight: Font.Medium
                    }

                    Repeater {
                        model: section.modelData.notes

                        delegate: ListRow {
                            id: entry

                            required property var modelData
                            required property int index

                            width: section.width
                            rowHeight: Theme.rowSpacious
                            contentInset: 8
                            focusable: true
                            keyboardCursor: entry.activeFocus

                            title: entry.modelData.name
                            subtitle: entry.modelData.kind === "drive"
                                ? root.driveParts(entry.modelData).filesystem
                                : (entry.modelData.kind === "recent"
                                    ? root.relativePath(entry.modelData.path) : "")
                            subtitleInline: entry.modelData.kind === "recent"
                            titleMonospace: entry.modelData.kind === "recent"
                            iconCategory: entry.modelData.isDir ? "folder" : "file"
                            showIcon: entry.modelData.kind === "project"
                                || entry.modelData.kind === "recent"
                                || entry.modelData.kind === "favorite"
                            showLeading: entry.modelData.kind === "tag"
                                || entry.modelData.kind === "drive"
                            iconEdge: 18
                            meta: entry.modelData.kind === "recent"
                                ? root.timeLabel(entry.modelData.timeText)
                                : (entry.modelData.kind === "tag"
                                    ? root.detailLabel(entry.modelData.detail) : "")
                            metaWidth: entry.modelData.kind === "recent" ? 84 : 120
                            accessibleName: entry.title
                            accessibleDescription: entry.modelData.kind === "recent"
                                ? entry.modelData.detail : entry.meta

                            leading: Item {
                                width: 18
                                height: 18
                                visible: entry.modelData.kind === "tag"
                                    || entry.modelData.kind === "drive"

                                Glyph {
                                    anchors.centerIn: parent
                                    width: 18
                                    height: 18
                                    kind: root.glyphKind(entry.modelData.icon)
                                    tint: Theme.inkSecondary
                                }
                            }

                            /// A project row reads name · branch · changes. The
                            /// branch and the diff count are one sentence, not
                            /// two chips: the row is a destination, not a card.
                            /// Drives end in their free/total figures.
                            trailing: Item {
                                width: entry.modelData.kind === "project"
                                    ? projectFacts.implicitWidth
                                    : (entry.modelData.kind === "drive" ? 150 : 0)
                                height: 18

                                Text {
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: entry.modelData.kind === "drive"
                                    text: {
                                        const parts = root.driveParts(entry.modelData)
                                        return parts.total === ""
                                            ? "" : parts.free + " / " + parts.total
                                    }
                                    color: Theme.inkTertiary
                                    font.family: Theme.monoFamily
                                    font.pixelSize: Theme.typeCaption
                                    horizontalAlignment: Text.AlignRight
                                }

                                Row {
                                    id: projectFacts
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 12

                                    Text {
                                        text: entry.modelData.branch
                                        visible: entry.modelData.branch !== ""
                                        color: Theme.inkSecondary
                                        font.family: Theme.monoFamily
                                        font.pixelSize: Theme.typeCaption
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        text: root.changeLabel(entry.modelData.changeText)
                                        visible: entry.modelData.changeText !== ""
                                        color: Theme.inkTertiary
                                        font.family: Theme.monoFamily
                                        font.pixelSize: Theme.typeCaption
                                    }
                                }
                            }

                            /// Drives carry a 160 × 3 capacity bar between the
                            /// name and the figures (§11.1).
                            Item {
                                width: entry.modelData.kind === "drive" ? 160 : 0
                                height: 22
                                visible: entry.modelData.kind === "drive"
                                Layout.alignment: Qt.AlignVCenter

                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 160
                                    height: 3
                                    color: Theme.line
                                    Accessible.ignored: true

                                    Rectangle {
                                        width: Math.round(parent.width
                                            * root.driveRatio(entry.modelData))
                                        height: parent.height
                                        color: Theme.accent
                                        Accessible.ignored: true
                                    }
                                }
                            }

                            onClicked: root.openNote(entry.modelData)
                            onDoubleClicked: root.openNote(entry.modelData)
                            onActivated: root.openNote(entry.modelData)

                            Component.onCompleted: root.registerFocusable(entry)
                            Component.onDestruction: root.unregisterFocusable(entry)
                        }
                    }
                }
            }
        }
    }

    /// The board is the page's only scroller; one bar covers it.
    ScrollBar {
        target: board
        anchors.right: parent.right
        anchors.rightMargin: 4
        anchors.top: parent.top
        anchors.bottom: parent.bottom
    }
}
