pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The toolbar's one display menu: view mode, density, icon set, row
// thumbnails, sort key/direction, folder pinning and the split layout.
//
// It reads and writes `ExplorerPrefs` directly, so the menu has no second copy
// of the state and a second window's menu is already showing the same ticks.
// The panel itself is `Menu`, shared with the context and filter menus; this
// file owns the rows and the icon-size band pinned under them.
Menu {
    id: root

    property color accent: Theme.accent
    /// The shell's pane visibility. These are window state, not preferences,
    /// so the menu asks the window to flip them instead of writing prefs.
    property bool sidebarVisible: true
    /// The docked inspector (§11.5). Off by default, so a window that never
    /// opens it keeps the full width for the listing.
    property bool inspectorVisible: false
    property bool terminalVisible: false

    signal toggleRequested(string property)
    signal actionRequested(string id)

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
        { kind: "shellToggle", property: "inspectorVisible", glyph: "panel",
          label: qsTr("检查器") },
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
          label: qsTr("按内容自动选择布局") }
    ]

    rows: root.normalized()

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.above = true
        root.show()
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
                    || entry.kind === "column")
            })
        }
        return mapped
    }

    onRowTriggered: function (index) { root.choose(root.items[index]) }

    /// The icon size is a continuous value, so it is a slider rather than
    /// five radio rows at the foot of the panel: the rows above already carry
    /// enough discrete ticks, and the band stays put while they scroll.
    footer: Component {
        Item {
            implicitHeight: 30

            Row {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                height: parent.height
                spacing: 10

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("图标大小")
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeMicro
                }

                M3Slider {
                    id: iconSlider
                    width: Math.max(80, parent.width - 120)
                    anchors.verticalCenter: parent.verticalCenter
                    value: (ExplorerPrefs.iconSize - 24) / 72
                    step: 1 / 72
                    onMoved: function (value) {
                        ExplorerPrefs.setNumber("iconSize", Math.round(24 + value * 72))
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
        }
    }
}
