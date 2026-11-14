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
    property color accent: Theme.accent
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
        { kind: "radio", group: "viewMode", value: "list", glyph: "list",
          label: qsTr("列表") },
        { kind: "radio", group: "viewMode", value: "grid", glyph: "grid",
          label: qsTr("网格") },
        { kind: "radio", group: "viewMode", value: "column", glyph: "column",
          label: qsTr("分栏") },
        { kind: "sep" },
        // The overview lost its sidebar header row; the view menu and the
        // command palette are now the two ways in.
        { kind: "action", id: "overview", label: qsTr("概览") },
        { kind: "sep" },
        { kind: "shellToggle", property: "sidebarVisible", glyph: "panel",
          label: qsTr("侧栏") },
        { kind: "shellToggle", property: "previewVisible", glyph: "eye",
          label: qsTr("预览") },
        { kind: "shellToggle", property: "terminalVisible", glyph: "terminal",
          label: qsTr("终端") },
        { kind: "sep" },
        { kind: "label", label: qsTr("排序") },
        { kind: "radio", group: "sortKey", value: "name", label: qsTr("名称") },
        { kind: "radio", group: "sortKey", value: "modified", label: qsTr("修改时间") },
        { kind: "radio", group: "sortKey", value: "created", label: qsTr("创建时间") },
        { kind: "radio", group: "sortKey", value: "size", label: qsTr("大小") },
        { kind: "radio", group: "sortKey", value: "type", label: qsTr("类型") },
        { kind: "radio", group: "sortKey", value: "path", label: qsTr("路径") },
        { kind: "radio", group: "sortKey", value: "tag", label: qsTr("标签") },
        { kind: "sep" },
        { kind: "label", label: qsTr("分组") },
        { kind: "radio", group: "groupKey", value: "", label: qsTr("不分组") },
        { kind: "radio", group: "groupKey", value: "name", label: qsTr("按名称") },
        { kind: "radio", group: "groupKey", value: "modified", label: qsTr("按修改时间") },
        { kind: "radio", group: "groupKey", value: "created", label: qsTr("按创建时间") },
        { kind: "radio", group: "groupKey", value: "size", label: qsTr("按大小") },
        { kind: "radio", group: "groupKey", value: "type", label: qsTr("按类型") },
        { kind: "radio", group: "groupKey", value: "tag", label: qsTr("按标签") },
        { kind: "sep" },
        { kind: "label", label: qsTr("文件夹") },
        { kind: "radio", group: "folderOrder", value: "foldersFirst", label: qsTr("文件夹优先") },
        { kind: "radio", group: "folderOrder", value: "filesFirst", label: qsTr("文件优先") },
        { kind: "radio", group: "folderOrder", value: "mixed", label: qsTr("混合排列") },
        { kind: "sep" },
        { kind: "label", label: qsTr("列") },
        { kind: "column", value: "type", label: qsTr("类型") },
        { kind: "column", value: "size", label: qsTr("大小") },
        { kind: "column", value: "modified", label: qsTr("修改时间") },
        { kind: "column", value: "created", label: qsTr("创建时间") },
        { kind: "column", value: "tags", label: qsTr("标签") },
        { kind: "toggle", property: "showExtensions", label: qsTr("显示扩展名") },
        { kind: "sep" },
        { kind: "label", label: qsTr("密度") },
        { kind: "radio", group: "density", value: "compact", label: qsTr("紧凑") },
        { kind: "radio", group: "density", value: "comfortable", label: qsTr("舒适") },
        { kind: "radio", group: "density", value: "spacious", label: qsTr("宽松") },
        { kind: "sep" },
        { kind: "label", label: qsTr("图标") },
        { kind: "radio", group: "iconStyle", value: "themed", label: qsTr("内置图标") },
        { kind: "radio", group: "iconStyle", value: "system", label: qsTr("系统图标") },
        { kind: "toggle", property: "rowThumbnails", glyph: "image",
          label: qsTr("行内显示缩略图") },
        { kind: "sep" },
        { kind: "toggle", property: "splitEnabled", glyph: "split",
          label: qsTr("分屏") },
        { kind: "sep" },
        { kind: "toggle", property: "syncLayout", glyph: "refresh",
          label: qsTr("所有目录使用同一布局") },
        { kind: "toggle", property: "adaptiveLayout", glyph: "grid",
          label: qsTr("按内容自动选择布局") },
        { kind: "sep" },
        { kind: "label", label: qsTr("图标大小") },
        { kind: "size", value: 24, label: "24" },
        { kind: "size", value: 32, label: "32" },
        { kind: "size", value: 48, label: "48" },
        { kind: "size", value: 64, label: "64" },
        { kind: "size", value: 96, label: "96" }
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
        if (item.kind === "column")
            return (ExplorerPrefs.columns || "").split(",").indexOf(item.value) >= 0
        if (item.kind === "size")
            return ExplorerPrefs.iconSize === item.value
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
        if (item.kind === "column") {
            const configured = (ExplorerPrefs.columns || "").split(",").filter(function (value) {
                return value.length > 0
            })
            const index = configured.indexOf(item.value)
            if (index >= 0)
                configured.splice(index, 1)
            else
                configured.push(item.value)
            ExplorerPrefs.setString("columns", configured.join(","))
        }
        if (item.kind === "size")
            ExplorerPrefs.setNumber("iconSize", item.value)
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
                glyph: entry.glyph || "",
                checked: kind === "item" && root.checked(entry),
                checkable: kind === "item" && (entry.kind === "radio"
                    || entry.kind === "toggle" || entry.kind === "shellToggle"
                    || entry.kind === "column" || entry.kind === "size")
            })
        }
        return mapped
    }

    MouseArea {
        Accessible.ignored: true
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Shadow {
        target: panel
    }

    Rectangle {
        id: panel
        width: list.naturalWidth
        // Twenty-two rows do not fit a window; the list scrolls in what is
        // left of it.
        height: Math.min(list.contentHeight + 42, root.height - 16)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: root.above
            ? Math.max(8, root.openY - height - 6)
            : Math.min(root.openY, root.height - height - 8)
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.line

        // The panel itself is not an input item, so a press on its padding or
        // on a non-selectable row (a separator, a group label) would otherwise
        // reach the window-wide scrim behind it and read as a click outside:
        // the menu closed on release, and a drag that started there closed it
        // too. Swallow presses and wheel here; the rows above still get theirs.
        MouseArea {
            anchors.fill: parent
            onWheel: {}
        }

        /// The icon size is a continuous value, so it is a slider rather than
        /// five radio rows: the menu already lists enough discrete ticks.
        Row {
            id: iconSliderRow
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            height: 30
            spacing: 10

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("图标大小")
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
            }

            Item {
                id: iconSlider
                width: Math.max(80, iconSliderRow.width - 120)
                height: 20
                anchors.verticalCenter: parent.verticalCenter

                readonly property real ratio: (ExplorerPrefs.iconSize - 24) / 72

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    height: 2
                    radius: 1
                    color: Theme.line
                }

                Rectangle {
                    x: Math.max(0, Math.min(iconSlider.width - 10,
                                            iconSlider.ratio * iconSlider.width - 5))
                    anchors.verticalCenter: parent.verticalCenter
                    width: 10
                    height: 10
                    radius: 5
                    color: Theme.accent
                }

                MouseArea {
                    /// A drag dot for the pointer. Its value is set from the menu around
                    /// it, so the dot would only add a nameless target to the tree.
                    Accessible.ignored: true
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onPositionChanged: function (mouse) {
                        if (!pressed)
                            return
                        const ratio = Math.max(0, Math.min(1, mouse.x / iconSlider.width))
                        ExplorerPrefs.setNumber("iconSize", Math.round(24 + ratio * 72))
                    }
                    onPressed: function (mouse) {
                        const ratio = Math.max(0, Math.min(1, mouse.x / iconSlider.width))
                        ExplorerPrefs.setNumber("iconSize", Math.round(24 + ratio * 72))
                    }
                }
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: ExplorerPrefs.iconSize
                color: Theme.inkSecondary
                font.pixelSize: Theme.typeMicro
                font.features: { "tnum": 1 }
                width: 24
                horizontalAlignment: Text.AlignRight
            }
        }

        MenuList {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 4
            anchors.bottom: iconSliderRow.top
            anchors.bottomMargin: 4
            maxHeight: height
            items: root.normalized()

            onTriggered: function (index) { root.choose(root.items[index]) }
            onEscapeRequested: root.close()
        }
    }
}
