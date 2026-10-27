pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// Entry filters: which rows exist, as opposed to how they are arranged.
//
// The menu carries the hidden-files switch alongside the three buckets because
// all four narrow the same listing, and keeps a clear-all action at the bottom
// so an accidental combination is one click away from undo.
Item {
    id: root

    property bool dark: true
    property color accent: "#2f6fdf"
    property real openX: 0
    property real openY: 0

    visible: false
    focus: visible
    z: 1002

    readonly property bool active: ExplorerPrefs.kindFilter !== "all"
        || ExplorerPrefs.modifiedFilter !== "any"
        || ExplorerPrefs.sizeFilter !== "any"
        || !ExplorerPrefs.showHiddenFiles

    readonly property color panelColor: dark ? "#202329" : "#ffffff"
    readonly property color borderColor: dark ? "#33ffffff" : "#18000000"
    readonly property color labelColor: dark ? "#7f8797" : "#8a929d"
    readonly property color textColor: dark ? "#e3e7ee" : "#24292f"

    readonly property var items: [
        { kind: "label", label: "可见性" },
        { kind: "toggle", property: "showHiddenFiles", label: "显示隐藏文件", shortcut: "Ctrl+H" },
        { kind: "sep" },
        { kind: "label", label: "类型" },
        { kind: "radio", group: "kindFilter", value: "all", label: "全部" },
        { kind: "radio", group: "kindFilter", value: "folders", label: "仅文件夹" },
        { kind: "radio", group: "kindFilter", value: "files", label: "仅文件" },
        { kind: "radio", group: "kindFilter", value: "images", label: "仅图片" },
        { kind: "sep" },
        { kind: "label", label: "修改时间" },
        { kind: "radio", group: "modifiedFilter", value: "any", label: "不限" },
        { kind: "radio", group: "modifiedFilter", value: "today", label: "今天" },
        { kind: "radio", group: "modifiedFilter", value: "week", label: "最近 7 天" },
        { kind: "radio", group: "modifiedFilter", value: "month", label: "最近 30 天" },
        { kind: "sep" },
        { kind: "label", label: "大小" },
        { kind: "radio", group: "sizeFilter", value: "any", label: "不限" },
        { kind: "radio", group: "sizeFilter", value: "small", label: "小（< 1 MB）" },
        { kind: "radio", group: "sizeFilter", value: "medium", label: "中（1 – 100 MB）" },
        { kind: "radio", group: "sizeFilter", value: "large", label: "大（> 100 MB）" }
    ]

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.visible = true
        root.forceActiveFocus()
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
        if (item.kind === "toggle") {
            ExplorerPrefs.setBool(item.property, !ExplorerPrefs[item.property])
            return
        }
        ExplorerPrefs.setString(item.group, item.value)
    }

    Keys.onEscapePressed: root.close()

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel
        width: 252
        height: column.implicitHeight + (root.active ? 37 : 10)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, root.openY - height - 6)
        radius: 9
        color: root.panelColor
        border.width: 1
        border.color: root.borderColor

        Column {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 5

            Repeater {
                model: root.items

                delegate: Item {
                    id: entry

                    required property var modelData

                    width: column.width
                    height: entry.modelData.kind === "label" ? 22
                        : entry.modelData.kind === "sep" ? 9 : 27

                    Rectangle {
                        visible: entry.modelData.kind === "sep"
                        anchors.verticalCenter: parent.verticalCenter
                        x: 10
                        width: parent.width - 20
                        height: 1
                        color: root.dark ? "#1affffff" : "#12000000"
                    }

                    Text {
                        visible: entry.modelData.kind === "label"
                        anchors.left: parent.left
                        anchors.leftMargin: 13
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.kind === "label" ? entry.modelData.label : ""
                        color: root.labelColor
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                    }

                    Rectangle {
                        visible: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep"
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 5
                        color: rowMouse.containsMouse ? (root.dark ? "#22ffffff" : "#0f000000")
                                                      : "transparent"
                    }

                    Text {
                        visible: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep"
                        anchors.left: parent.left
                        anchors.leftMargin: 13
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep" ? entry.modelData.label : ""
                        color: root.textColor
                        font.pixelSize: 12
                    }

                    Text {
                        visible: entry.modelData.shortcut !== undefined
                        anchors.right: parent.right
                        anchors.rightMargin: 34
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.shortcut || ""
                        color: root.labelColor
                        font.pixelSize: 10
                    }

                    Text {
                        visible: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep" && root.checked(entry.modelData)
                        anchors.right: parent.right
                        anchors.rightMargin: 13
                        anchors.verticalCenter: parent.verticalCenter
                        text: "\u2713"
                        color: root.accent
                        font.pixelSize: 12
                        font.weight: Font.DemiBold
                    }

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep"
                        onClicked: root.choose(entry.modelData)
                    }
                }
            }
        }

        Rectangle {
            visible: root.active
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 27
            color: "transparent"

            Text {
                anchors.centerIn: parent
                text: "清除筛选"
                color: root.accent
                font.pixelSize: 11
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                onClicked: ExplorerPrefs.resetFilters()
            }
        }
    }
}
