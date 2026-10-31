pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The landing page: favorites, recent items, spaces, and the work trees behind
// the recent folders.
//
// The model flattens all four into one sectioned list so this view stays a
// single scroll. Rows differ by `kind`, but they share the same anatomy (glyph,
// name, one line of context, one trailing fact) and the delegate reads the
// roles it needs instead of switching to a nested component per section.
Item {
    id: root

    property var model: null
    property bool dark: true
    property color accent: Theme.interactive
    property color headerColor: Theme.chrome
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline
    property string commandShortcut: "Ctrl+K"

    signal openRequested(string kind, string path)
    signal commandPaletteRequested()

    readonly property color contentColor: Theme.canvas
    readonly property color hoverColor: Theme.hover

    function takeFocus() {
        list.forceActiveFocus()
    }

    /// The overview model names destinations by the same keys the sidebar uses,
    /// so the landing page draws its icons from `Glyph` too. The old text
    /// glyphs were a second icon set: the project row's "⌥" was the macOS
    /// Option key, not a branch, and it sat beside a real folder glyph.
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
        case "space": return "diamond"
        case "project": return "folder"
        case "file": return "file"
        default: return "folder"
        }
    }

    /// The one place a landing row turns into a destination, so the pointer,
    /// a screen reader's press and the keyboard cursor cannot disagree.
    function openRow(kind, path, isDir) {
        if (kind === "space")
            root.openRequested("space", path)
        else if (kind === "recent" && !isDir)
            root.openRequested("file", path)
        else
            root.openRequested("navigate", path)
    }

    function activateCurrent() {
        const index = list.currentIndex
        if (index < 0 || root.model === null)
            return
        // The model exposes the same three facts the delegate binds, so the
        // keyboard does not have to reach into a live delegate.
        root.openRow(root.model.kindAt(index), root.model.pathAt(index),
                     root.model.isDirAt(index))
    }

    function sectionLabel(value) {
        switch (value) {
        case "Projects": return qsTr("项目")
        case "Favorites": return qsTr("收藏")
        case "Recent": return qsTr("最近")
        case "Spaces": return qsTr("空间")
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
        if (value === "Empty space")
            return qsTr("空空间")
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

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    ListView {
        id: list

        anchors.fill: parent
        clip: true
        focus: true
        activeFocusOnTab: true
        boundsBehavior: Flickable.StopAtBounds
        model: root.model
        cacheBuffer: 400
        currentIndex: -1
        section.property: "section"
        section.criteria: ViewSection.FullString

        /// Startup split: the model marks its publish point in Rust (a QML
        /// handler runs after the view's own model connection, so it would
        /// report layout as data); this is the layout half, and the Rust side
        /// keeps only the first report of each stage. See
        /// `crates/dae-qt/src/startup.rs`.
        onContentHeightChanged: if (root.visible && count > 0 && contentHeight > 0)
            Startup.mark("qml-first-list")

        /// The list is a keyboard surface too: the arrow keys move the cursor,
        /// Enter and Space open the row under it. Without this the landing
        /// page was only reachable with a pointer or a screen reader.
        Accessible.role: Accessible.List
        Accessible.name: qsTr("概览")
        Keys.onReturnPressed: root.activateCurrent()
        Keys.onSpacePressed: root.activateCurrent()

        /// The reference caps the launcher column at a readable measure and
        /// leaves the rest of a wide window empty, rather than stretching a
        /// row's name and its trailing fact to opposite edges.
        readonly property int maxContent: 832
        readonly property int sideMargin: Math.max(24, Math.round((width - maxContent) / 2))
        anchors.leftMargin: sideMargin
        anchors.rightMargin: sideMargin

        section.delegate: Item {
            id: sectionRow

            required property string section
            width: list.width
            height: 24

            Text {
                x: 20
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 5
                text: root.sectionLabel(sectionRow.section)
                color: Theme.inkFaint
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.88
            }
        }

        header: Item {
            width: list.width
            height: 132

            Column {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                anchors.topMargin: 20
                spacing: 4

                Text {
                    text: qsTr("概览")
                    color: root.textColor
                    font.pixelSize: Theme.typeTitle
                    font.weight: Font.Medium
                }

                Text {
                    width: parent.width
                    text: qsTr("从收藏、最近访问与空间继续。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideRight
                }
            }

            Rectangle {
                id: launcher

                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.topMargin: 66
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                height: Theme.rowSpacious
                radius: Theme.radiusMd
                color: Theme.canvasAlt
                border.width: 1
                border.color: launcherMouse.containsMouse ? root.accent : root.hairline

                Glyph {
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    kind: "search"
                    tint: root.mutedColor
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 34
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("搜索文件、空间或命令")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeBody
                }

                Rectangle {
                    id: shortcutChip

                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: shortcutText.width + 12
                    height: 18
                    radius: Theme.radiusXs
                    color: Theme.hover

                    Text {
                        id: shortcutText
                        anchors.centerIn: parent
                        text: root.commandShortcut
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                    }
                }

                MouseArea {
                    id: launcherMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.commandPaletteRequested()
                }
            }

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 10
                anchors.top: launcher.bottom
                anchors.topMargin: 6
                visible: root.model !== null && root.model.projectBusy
                text: qsTr("正在查找最近的项目…")
                color: root.mutedColor
                font.pixelSize: Theme.typeMicro
            }
        }

        delegate: Item {
            id: row

            required property int index
            required property string kind
            required property string name
            required property string path
            required property string detail
            required property string icon
            required property bool isDir
            required property string branch
            required property string changeText
            required property string timeText

            width: list.width
            height: Theme.rowDefault

            readonly property bool project: row.kind === "project"
            /// Places and favorites carry a path, recents their parent folder,
            /// spaces a count: one trailing fact instead of a second line.
            readonly property bool hasDetail: !row.project && row.detail !== ""
            /// The keyboard cursor, drawn only while the list owns focus. A
            /// pointer click opens the row immediately, so there is no clicked
            /// selection state to confuse it with.
            readonly property bool cursor: list.activeFocus
                && list.currentIndex === row.index

            /// A landing-page row opens something. The kind decides what, so it
            /// goes in the description: "space" and "folder" look the same in a
            /// row but lead to different places.
            Accessible.role: Accessible.ListItem
            /// The reader hears what the row shows, which for a system place
            /// is its translated label, not the model's English key.
            Accessible.name: row.kind === "place" ? root.placeLabel(row.icon)
                : row.name
            Accessible.description: row.hasDetail
                ? root.detailLabel(row.detail) : ""
            Accessible.selectable: true
            Accessible.selected: row.cursor
            Accessible.onPressAction: root.openRow(row.kind, row.path, row.isDir)

            Rectangle {
                id: plate

                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                radius: Theme.radiusSm
                color: row.cursor ? Theme.signalSoft
                    : (rowMouse.containsMouse ? root.hoverColor : "transparent")
                border.width: row.cursor ? 1 : 0
                border.color: Theme.signal
            }

            Glyph {
                anchors.left: parent.left
                anchors.leftMargin: 20
                anchors.verticalCenter: parent.verticalCenter
                width: 16
                height: 16
                kind: root.glyphKind(row.icon)
                tint: root.mutedColor
            }

            Text {
                id: nameText

                anchors.left: parent.left
                anchors.leftMargin: 44
                anchors.right: detailText.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: row.kind === "place" ? root.placeLabel(row.icon) : row.name
                color: root.textColor
                font.pixelSize: Theme.typeName
                elide: Text.ElideRight
            }

            TextMetrics {
                id: detailMetrics
                font: detailText.font
                text: detailText.text
            }

            Text {
                id: detailText

                anchors.right: trailing.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                visible: row.hasDetail
                width: visible
                    ? Math.min(detailMetrics.advanceWidth, row.width * 0.45) : 0
                horizontalAlignment: Text.AlignRight
                text: root.detailLabel(row.detail)
                color: root.mutedColor
                font.pixelSize: Theme.typeMicro
                font.features: { "tnum": 1 }
                elide: Text.ElideMiddle
            }

            Row {
                id: trailing

                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                Rectangle {
                    visible: row.project && row.branch !== ""
                    height: 18
                    radius: Theme.radiusXs
                    color: Theme.canvasAlt
                    width: branchRow.implicitWidth + 12

                    Row {
                        id: branchRow

                        anchors.centerIn: parent
                        spacing: 4

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 10
                            height: 10
                            kind: "branch"
                            tint: root.mutedColor
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: row.branch
                            color: root.mutedColor
                            font.pixelSize: Theme.typeMicro
                            font.family: Theme.monoFamily
                        }
                    }
                }

                Text {
                    visible: row.project && row.changeText !== ""
                    text: root.changeLabel(row.changeText)
                    color: row.changeText === "Clean" ? root.mutedColor
                        : (row.changeText === "Untracked" ? Theme.link : Theme.warn)
                    font.pixelSize: Theme.typeMicro
                }

                Text {
                    visible: row.timeText !== ""
                    text: root.timeLabel(row.timeText)
                    color: root.mutedColor
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                }
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openRow(row.kind, row.path, row.isDir)
            }
        }
    }

    /// With every system place hidden and nothing favourited yet, the list is
    /// only a header over empty canvas. This is the landing page's own empty
    /// state, using the shared grammar rather than a second mute sentence.
    EmptyState {
        anchors.fill: parent
        visible: root.model !== null && root.model.loaded
            && root.model.count === 0
        glyph: "folder"
        title: qsTr("还没有可显示的位置")
        hint: qsTr("在地址栏打开一个文件夹，它会出现在「最近」里。")
    }
}
