pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The toolbar's one display menu: view mode, density, icon set, row
// thumbnails, sort key/direction, folder pinning and the split layout.
//
// It reads and writes `ExplorerPrefs` directly, so the menu has no second copy
// of the state and a second window's menu is already showing the same ticks.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    /// The shell's pane visibility. These are window state, not preferences,
    /// so the menu asks the window to flip them instead of writing prefs.
    property bool sidebarVisible: true
    property bool previewVisible: true
    property bool terminalVisible: false
    property real openX: 0
    property real openY: 0
    property bool above: true

    signal toggleRequested(string property)

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

    readonly property color panelColor: Theme.raised
    readonly property color borderColor: Theme.hairlineStrong
    readonly property color labelColor: Theme.inkFaint
    readonly property color textColor: Theme.ink

    readonly property var items: [
        { kind: "label", label: qsTr("视图") },
        { kind: "radio", group: "viewMode", value: "list", label: qsTr("列表") },
        { kind: "radio", group: "viewMode", value: "grid", label: qsTr("网格") },
        { kind: "radio", group: "viewMode", value: "column", label: qsTr("分栏") },
        { kind: "sep" },
        { kind: "shellToggle", property: "sidebarVisible", label: qsTr("侧栏") },
        { kind: "shellToggle", property: "previewVisible", label: qsTr("预览") },
        { kind: "shellToggle", property: "terminalVisible", label: qsTr("终端") },
        { kind: "sep" },
        { kind: "label", label: qsTr("密度") },
        { kind: "radio", group: "density", value: "compact", label: qsTr("紧凑") },
        { kind: "radio", group: "density", value: "comfortable", label: qsTr("舒适") },
        { kind: "radio", group: "density", value: "spacious", label: qsTr("宽松") },
        { kind: "sep" },
        { kind: "label", label: qsTr("图标") },
        { kind: "radio", group: "iconStyle", value: "system", label: qsTr("系统图标") },
        { kind: "radio", group: "iconStyle", value: "themed", label: qsTr("内置图标") },
        { kind: "toggle", property: "rowThumbnails", label: qsTr("行内显示缩略图") },
        { kind: "sep" },
        { kind: "toggle", property: "splitEnabled", label: qsTr("分屏") }
    ]

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.above = true
        root.visible = true
        root.forceActiveFocus()
    }

    function close() {
        root.visible = false
    }

    function checked(item) {
        if (item.kind === "shellToggle")
            return root[item.property] === true
        if (item.kind === "toggle")
            return ExplorerPrefs[item.property] === true
        return ExplorerPrefs[item.group] === item.value
    }

    function choose(item) {
        if (item.kind === "shellToggle") {
            root.toggleRequested(item.property)
            return
        }
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
        height: column.implicitHeight + 10
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: root.above
            ? Math.max(8, root.openY - height - 6)
            : Math.min(root.openY, root.height - height - 8)
        radius: 12
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
                        color: Theme.fill
                    }

                    Text {
                        visible: entry.modelData.kind === "label"
                        anchors.left: parent.left
                        anchors.leftMargin: 13
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.modelData.kind === "label" ? entry.modelData.label : ""
                        color: root.labelColor
                        font.pixelSize: 11
                        font.weight: Font.DemiBold
                    }

                    Rectangle {
                        visible: entry.modelData.kind !== "label"
                            && entry.modelData.kind !== "sep"
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        radius: 6
                        color: rowMouse.containsMouse ? Theme.fill
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
    }
}
