pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The toolbar's one display menu: view mode, density, icon set, row
// thumbnails, sort key/direction, folder pinning and the split layout.
//
// It reads and writes `ExplorerPrefs` directly, so the menu has no second copy
// of the state and a second window's menu is already showing the same ticks.
// The rows themselves come from `MenuList`, which is also what the context
// menu draws: one row height, one group label, one tick mark in the shell.
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
    signal actionRequested(string id)

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

    readonly property var items: [
        { kind: "label", label: qsTr("视图") },
        { kind: "radio", group: "viewMode", value: "list", label: qsTr("列表") },
        { kind: "radio", group: "viewMode", value: "grid", label: qsTr("网格") },
        { kind: "radio", group: "viewMode", value: "column", label: qsTr("分栏") },
        { kind: "sep" },
        // The overview lost its sidebar header row; the view menu and the
        // command palette are now the two ways in.
        { kind: "action", id: "overview", label: qsTr("概览") },
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
        list.resetScroll()
        root.visible = true
        Qt.callLater(list.takeFocus)
    }

    function close() {
        root.visible = false
    }

    function checked(item) {
        if (item.kind === "shellToggle")
            return root[item.property] === true
        if (item.kind === "toggle")
            return ExplorerPrefs[item.property] === true
        if (item.kind === "radio")
            return ExplorerPrefs[item.group] === item.value
        // Actions and labels never carry a tick; `undefined === undefined`
        // would otherwise mark the overview row.
        return false
    }

    function choose(item) {
        if (item === undefined)
            return
        if (item.kind === "action") {
            root.close()
            root.actionRequested(item.id)
            return
        }
        if (item.kind === "shellToggle") {
            root.toggleRequested(item.property)
            return
        }
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
                checkable: kind === "item" && (entry.kind === "radio"
                    || entry.kind === "toggle" || entry.kind === "shellToggle")
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
        // Twenty-two rows do not fit a window; the list scrolls in what is
        // left of it.
        height: Math.min(list.contentHeight + 8, root.height - 16)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: root.above
            ? Math.max(8, root.openY - height - 6)
            : Math.min(root.openY, root.height - height - 8)
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
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            maxHeight: height
            items: root.normalized()

            onTriggered: function (index) { root.choose(root.items[index]) }
            onEscapeRequested: root.close()
        }
    }
}
