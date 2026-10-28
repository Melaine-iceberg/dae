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

    function glyph(icon) {
        if (icon === "home")
            return "\u2302"
        if (icon === "desktop")
            return "\u25a3"
        if (icon === "documents")
            return "\u2261"
        if (icon === "downloads")
            return "\u2193"
        if (icon === "pictures")
            return "\u25a4"
        if (icon === "music")
            return "\u266b"
        if (icon === "videos")
            return "\u25b6"
        if (icon === "star")
            return "\u2605"
        if (icon === "space")
            return "\u25c8"
        if (icon === "project")
            return "\u2325"
        if (icon === "file")
            return "\u25a1"
        return "\u25b1"
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
        boundsBehavior: Flickable.StopAtBounds
        model: root.model
        cacheBuffer: 400
        section.property: "section"
        section.criteria: ViewSection.FullString
        section.delegate: Item {
            required property string section
            width: list.width
            height: 36

            Text {
                x: 24
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 8
                text: root.sectionLabel(section)
                color: root.mutedColor
                font.pixelSize: 11
                font.weight: Font.DemiBold
                font.letterSpacing: 0.4
            }
        }

        header: Item {
            width: list.width
            height: 150

            Column {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                anchors.topMargin: 24
                spacing: 6

                Text {
                    text: qsTr("概览")
                    color: root.textColor
                    font.pixelSize: 24
                    font.weight: Font.DemiBold
                }

                Text {
                    width: parent.width
                    text: qsTr("从收藏、最近访问与空间继续。")
                    color: root.mutedColor
                    font.pixelSize: 12
                    wrapMode: Text.Wrap
                }
            }

            Rectangle {
                id: launcher

                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.topMargin: 84
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                height: 40
                radius: 12
                color: Theme.canvasAlt
                border.width: 1
                border.color: launcherMouse.containsMouse ? root.accent : root.hairline

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: "\u2315"
                    color: root.mutedColor
                    font.pixelSize: 14
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 34
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("搜索文件、空间或命令")
                    color: root.mutedColor
                    font.pixelSize: 12
                }

                Rectangle {
                    id: shortcutChip

                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: shortcutText.width + 12
                    height: 18
                    radius: 4
                    color: Theme.hover

                    Text {
                        id: shortcutText
                        anchors.centerIn: parent
                        text: root.commandShortcut
                        color: root.mutedColor
                        font.pixelSize: 11
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
                anchors.leftMargin: 24
                anchors.top: launcher.bottom
                anchors.topMargin: 8
                visible: root.model !== null && root.model.projectBusy
                text: qsTr("正在查找最近的项目…")
                color: root.mutedColor
                font.pixelSize: 11
            }
        }

        delegate: Item {
            id: row

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
            height: 58

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                radius: 6
                color: rowMouse.containsMouse ? root.hoverColor : "transparent"
            }

            Rectangle {
                x: 22
                anchors.verticalCenter: parent.verticalCenter
                width: 30
                height: 30
                radius: 12
                color: Theme.canvasAlt

                Text {
                    anchors.centerIn: parent
                    text: root.glyph(row.icon)
                    color: row.kind === "space" ? root.accent : root.mutedColor
                    font.pixelSize: 14
                }
            }

            Column {
                anchors.left: parent.left
                anchors.leftMargin: 64
                anchors.right: trailing.left
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 3

                Text {
                    width: parent.width
                    text: row.kind === "place"
                        ? root.placeLabel(row.icon) : row.name
                    color: root.textColor
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }

                Text {
                    width: parent.width
                    text: root.detailLabel(row.detail)
                    color: root.mutedColor
                    font.pixelSize: 11
                    elide: Text.ElideMiddle
                }
            }

            Row {
                id: trailing

                anchors.right: parent.right
                anchors.rightMargin: 22
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                Rectangle {
                    visible: row.kind === "project" && row.branch !== ""
                    height: 19
                    radius: 6
                    color: Theme.canvasAlt
                    width: branchText.width + 14

                    Text {
                        id: branchText
                        anchors.centerIn: parent
                        text: "\u2325 " + row.branch
                        color: root.mutedColor
                        font.pixelSize: 11
                    }
                }

                Text {
                    visible: row.kind === "project" && row.changeText !== ""
                    text: root.changeLabel(row.changeText)
                    color: row.changeText === "Clean" ? root.mutedColor
                        : (row.changeText === "Untracked" ? "#c98b2e" : root.accent)
                    font.pixelSize: 11
                }

                Text {
                    visible: row.timeText !== ""
                    text: root.timeLabel(row.timeText)
                    color: root.mutedColor
                    font.pixelSize: 11
                }
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    if (row.kind === "space")
                        root.openRequested("space", row.path)
                    else if (row.kind === "recent" && !row.isDir)
                        root.openRequested("file", row.path)
                    else
                        root.openRequested("navigate", row.path)
                }
            }
        }
    }
}
