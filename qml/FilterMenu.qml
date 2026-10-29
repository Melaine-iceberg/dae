pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// Entry filters: which rows exist, as opposed to how they are arranged.
//
// The menu carries the hidden-files switch alongside the three buckets because
// all four narrow the same listing, and keeps a clear-all action at the bottom
// so an accidental combination is one click away from undo. The rows come from
// `MenuList`, shared with every other menu in the shell.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    /// The binding from `settings.toml`; the view only prints it.
    property string hiddenShortcut: "Ctrl+H"
    property real openX: 0
    property real openY: 0

    visible: false
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        NumberAnimation { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1002

    readonly property bool active: ExplorerPrefs.kindFilter !== "all"
        || ExplorerPrefs.modifiedFilter !== "any"
        || ExplorerPrefs.sizeFilter !== "any"
        || !ExplorerPrefs.showHiddenFiles

    readonly property var items: [
        { kind: "label", label: qsTr("可见性") },
        { kind: "toggle", property: "showHiddenFiles", label: qsTr("显示隐藏文件"),
          shortcut: root.hiddenShortcut },
        { kind: "sep" },
        { kind: "label", label: qsTr("类型") },
        { kind: "radio", group: "kindFilter", value: "all", label: qsTr("全部") },
        { kind: "radio", group: "kindFilter", value: "folders", label: qsTr("仅文件夹") },
        { kind: "radio", group: "kindFilter", value: "files", label: qsTr("仅文件") },
        { kind: "radio", group: "kindFilter", value: "images", label: qsTr("仅图片") },
        { kind: "sep" },
        { kind: "label", label: qsTr("修改时间") },
        { kind: "radio", group: "modifiedFilter", value: "any", label: qsTr("不限") },
        { kind: "radio", group: "modifiedFilter", value: "today", label: qsTr("今天") },
        { kind: "radio", group: "modifiedFilter", value: "week", label: qsTr("最近 7 天") },
        { kind: "radio", group: "modifiedFilter", value: "month", label: qsTr("最近 30 天") },
        { kind: "sep" },
        { kind: "label", label: qsTr("大小") },
        { kind: "radio", group: "sizeFilter", value: "any", label: qsTr("不限") },
        { kind: "radio", group: "sizeFilter", value: "small", label: qsTr("小（< 1 MB）") },
        { kind: "radio", group: "sizeFilter", value: "medium", label: qsTr("中（1 – 100 MB）") },
        { kind: "radio", group: "sizeFilter", value: "large", label: qsTr("大（> 100 MB）") }
    ]

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        list.resetScroll()
        root.visible = true
        Qt.callLater(list.takeFocus)
    }

    function close() {
        root.visible = false
    }

    function checked(item) {
        if (item.kind === "toggle")
            return ExplorerPrefs[item.property] === true
        return ExplorerPrefs[item.group] === item.value
    }

    function choose(item) {
        if (item === undefined)
            return
        if (item.kind === "toggle") {
            ExplorerPrefs.setBool(item.property, !ExplorerPrefs[item.property])
            return
        }
        if (item.kind === "radio")
            ExplorerPrefs.setString(item.group, item.value)
    }

    function normalized() {
        const mapped = []
        for (let index = 0; index < root.items.length; index += 1) {
            const entry = root.items[index]
            const kind = entry.kind === "label" ? "header"
                : (entry.kind === "sep" ? "separator" : "item")
            mapped.push({
                kind: kind,
                label: entry.label,
                checked: kind === "item" && root.checked(entry),
                shortcut: entry.shortcut || ""
            })
        }
        return mapped
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel
        width: list.naturalWidth
        height: Math.min(list.contentHeight + 8 + (root.active ? 29 : 0),
                         root.height - 16)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, root.openY - height - 6)
        radius: Theme.radiusMd
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MenuList {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 4
            // The clear row keeps its place at the bottom; the rows scroll
            // above it rather than sliding under it.
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4 + (root.active ? 29 : 0)
            maxHeight: height
            items: root.normalized()

            onTriggered: function (index) { root.choose(root.items[index]) }
            onEscapeRequested: root.close()
        }

        Rectangle {
            visible: root.active
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 29
            color: "transparent"

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: Theme.hairline
            }

            Text {
                anchors.centerIn: parent
                text: qsTr("清除筛选")
                color: root.accent
                font.pixelSize: Theme.typeMicro
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: ExplorerPrefs.resetFilters()
            }
        }
    }
}
