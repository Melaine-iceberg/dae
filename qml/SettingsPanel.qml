pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import dae 1.0
import "Shortcut.js" as ShortcutJS
import "DialogFocus.js" as DialogFocus

// The settings document.
//
// The panel intentionally has no global "save" step: appearance, language and
// shortcuts apply immediately, while the terminal typography page keeps its
// own save button because a half-typed font size should not resize the live
// terminal. Appearance, language, terminal typography and shortcuts live in
// the shared `settings.toml`. The default-file-manager page changes an OS
// association, so its buttons call the
// platform immediately and its state is re-read whenever the page opens.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.accent
    property bool isMac: false
    property var shortcutMap: ({})
    /// Whether this session can blur a window behind it (Wayland only). The
    /// Windows materials are resolved per build by the native side.
    property bool blurSupported: true

    signal closed()

    Keys.onTabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, true)
    }
    Keys.onBacktabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, false)
    }

    visible: false
    z: 1001
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("设置")

    property string page: "appearance"
    property string captureId: ""
    property string captureLabel: ""
    property string shortcutError: ""
    property string terminalError: ""
    property string terminalStatus: ""
    property string fontError: ""
    property string fontStatus: ""
    property string tagError: ""
    /// Which of the eight fixed tag colours the next tag gets. A picked option
    /// is stored as its light hex, so a tag keeps its identity across themes.
    property int draftTagIndex: 0
    /// Outcome of the last export/import/rebuild. `tagTaskFailed` only picks
    /// the colour; the text says what happened, including the skipped
    /// definitions an import refused.
    property string tagTaskStatus: ""
    property bool tagTaskFailed: false
    /// Bumped by the tag store; the definitions page reads it so a write made
    /// in another window shows up here too.
    property int tagRevision: 0

    function createTag(name, color) {
        root.tagError = ""
        if (TagStore.createTag(name, color) === "") {
            root.tagError = TagStore.note !== "" ? TagStore.note
                : qsTr("无法创建标签（名称可能重复）")
            return
        }
        draftName.text = ""
    }

    function updateTag(id, name, color) {
        root.tagError = ""
        const error = TagStore.updateTag(id, name, color)
        if (error !== "")
            root.tagError = error
    }

    function removeTag(id) {
        root.tagError = ""
        const error = TagStore.removeTag(id)
        if (error !== "")
            root.tagError = error
    }

    /// The tag store's one reply shape: JSON, with `ok` deciding whether the
    /// other keys are counters or an `error`.
    function tagReply(raw) {
        try {
            return JSON.parse(String(raw))
        } catch (error) {
            return { ok: false, error: String(raw) }
        }
    }

    function exportTags(path) {
        const reply = root.tagReply(TagStore.exportDefinitions(path))
        root.tagTaskFailed = !reply.ok
        root.tagTaskStatus = reply.ok
            ? qsTr("已导出到 %1").arg(path)
            : String(reply.error)
    }

    function importTags(path) {
        const reply = root.tagReply(TagStore.importDefinitions(path))
        if (!reply.ok) {
            root.tagTaskFailed = true
            root.tagTaskStatus = String(reply.error)
            return
        }
        root.tagTaskFailed = false
        let text = qsTr("新增 %1，更新 %2").arg(reply.added).arg(reply.updated)
        const skipped = reply.skipped || []
        if (skipped.length > 0) {
            text += qsTr("；跳过 %1 条：").arg(skipped.length)
                    + skipped.slice(0, 3).join("；")
                    + (skipped.length > 3 ? "…" : "")
        }
        root.tagTaskStatus = text
    }

    function rebuildTagIndex() {
        const reply = root.tagReply(TagStore.rebuildIndex())
        if (!reply.ok) {
            root.tagTaskFailed = true
            root.tagTaskStatus = String(reply.error)
            return
        }
        root.tagTaskFailed = false
        let text = qsTr("索引已重建：%1 条记录，移除 %2 条失效、%3 条重复、%4 个损坏分片")
                .arg(reply.entries)
                .arg(reply.droppedMissing)
                .arg(reply.droppedDuplicate)
                .arg(reply.corruptShards)
        if (reply.adoptedFromFile > 0)
            text += qsTr("；%1 条以文件流为准").arg(reply.adoptedFromFile)
        root.tagTaskStatus = text
    }

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color faintColor: Theme.inkTertiary
    readonly property color hairline: Theme.hairline
    readonly property color fieldColor: Theme.field
    readonly property color panelColor: Theme.raised
    readonly property color sidebarColor: Theme.chrome
    readonly property color hoverFill: Theme.hover
    readonly property color selectedFill: Theme.selection

    readonly property bool defaultManagerCanSet:
        Workspace.defaultManagerSupported && !Workspace.defaultManagerBusy
        && !Workspace.defaultManagerIsDefault
    readonly property bool defaultManagerCanUnset:
        Workspace.defaultManagerSupported && !Workspace.defaultManagerBusy
        && (Workspace.defaultManagerIsDefault || Workspace.defaultManagerIsRegistered)
    readonly property string defaultManagerState: !Workspace.defaultManagerSupported
        ? qsTr("当前平台不支持默认文件管理器注册")
        : Workspace.defaultManagerBusy && !Workspace.defaultManagerIsDefault
            ? qsTr("正在处理…")
            : Workspace.defaultManagerIsDefault
                ? qsTr("dae 已是默认文件管理器")
                : Workspace.defaultManagerIsRegistered
                    ? qsTr("dae 已注册，等待系统确认")
                    : qsTr("dae 不是默认文件管理器")
    readonly property color defaultManagerStateColor: Workspace.defaultManagerIsDefault
        ? Theme.success
        : Workspace.defaultManagerIsRegistered
            ? Theme.warning
            : Theme.inkSecondary

    readonly property var themeOptions: [
        { value: "system", label: qsTr("跟随系统") },
        { value: "light", label: qsTr("浅色") },
        { value: "dark", label: qsTr("深色") }
    ]
    readonly property var accentOptions: [
        { value: "system", label: qsTr("系统强调色") },
        { value: "brand", label: qsTr("青蓝") },
        { value: "custom", label: qsTr("自定义") }
    ]
    /// The seed palette offered when the accent source is "custom". Mid-tone
    /// saturated hues: the M3 derivation guards every container/ink pair for
    /// contrast, so any of these stays readable in both themes.
    readonly property var accentPalette: [
        "#007a7b", "#2563eb", "#4f46e5", "#7c3aed",
        "#c026d3", "#db2777", "#dc2626", "#ea580c",
        "#d97706", "#65a30d", "#16a34a", "#0891b2"
    ]
    readonly property var languageOptions: [
        { value: "system", label: qsTr("跟随系统") },
        { value: "zh-CN", label: qsTr("简体中文") },
        { value: "zh-TW", label: qsTr("繁體中文") },
        { value: "en", label: "English" }
    ]
    readonly property var materialOptions: [
        { value: "none", label: qsTr("不透明") },
        { value: "mica", label: qsTr("云母 (Mica)") },
        { value: "mica-alt", label: qsTr("云母 Alt (Mica Alt)") },
        { value: "acrylic", label: qsTr("亚克力 (Acrylic)") }
    ]
    /// The material choice is a Windows concept; other platforms either have
    /// a single effect or none, so the section would only mislead.
    readonly property bool showMaterialChoices: Qt.platform.os === "windows"
    readonly property string backgroundNote: {
        if (Qt.platform.os === "linux")
            return root.blurSupported
                ? qsTr("当前 Wayland 合成器支持 ext-background-effect-v1，背景模糊可用。")
                : qsTr("当前 Wayland 合成器未提供 ext-background-effect-v1；窗口会保持实色背景。")
        if (Qt.platform.os === "osx")
            return root.blurSupported
                ? qsTr("macOS 使用系统毛玻璃效果，跟随窗口的深浅色。")
                : qsTr("当前系统不支持窗口背景模糊。")
        return ""
    }

    /// Grouping mirrors the Rust action catalogue. Everything here is edited
    /// by id, so a missing translation never changes the stored key.
    readonly property var shortcutGroups: [
        {
            title: qsTr("全局"),
            actions: [
                { id: "app.commandBar", label: qsTr("命令面板"),
                  detail: qsTr("打开文件、空间与命令的快速搜索") },
                { id: "app.pathJump", label: qsTr("跳转路径"),
                  detail: qsTr("在地址栏中直接输入路径") },
                { id: "app.toggleTerminal", label: qsTr("切换终端"),
                  detail: qsTr("显示或隐藏内嵌终端") },
                { id: "app.openSettings", label: qsTr("打开设置"),
                  detail: qsTr("打开这个设置面板") },
                { id: "app.newWindow", label: qsTr("新建窗口"),
                  detail: qsTr("在相同目录打开一个新窗口") },
                { id: "app.newTab", label: qsTr("新建标签页"),
                  detail: qsTr("在当前标签页之后新建一个") },
                { id: "app.closeTab", label: qsTr("关闭标签页"),
                  detail: qsTr("关闭当前标签页") },
                { id: "app.reopenTab", label: qsTr("重新打开标签页"),
                  detail: qsTr("恢复最近关闭的标签页") },
                { id: "app.nextTab", label: qsTr("下一个标签页"),
                  detail: qsTr("按顺序切换到下一个标签页") },
                { id: "app.previousTab", label: qsTr("上一个标签页"),
                  detail: qsTr("按顺序切换到上一个标签页") },
                { id: "app.toggleSidebar", label: qsTr("切换侧栏"),
                  detail: qsTr("显示或隐藏侧栏") },
                { id: "app.toggleInspector", label: qsTr("切换检查器"),
                  detail: qsTr("显示或隐藏右侧检查器") }
            ]
        },
        {
            title: qsTr("浏览与选择"),
            actions: [
                { id: "explorer.clearSelection", label: qsTr("清除选择"),
                  detail: qsTr("取消当前所有选中项") },
                { id: "explorer.goBack", label: qsTr("后退"),
                  detail: qsTr("回到上一个访问过的文件夹") },
                { id: "explorer.goForward", label: qsTr("前进"),
                  detail: qsTr("回到后退之前所在的文件夹") },
                { id: "explorer.focusSearch", label: qsTr("搜索"),
                  detail: qsTr("把焦点移到搜索框") },
                { id: "explorer.selectAll", label: qsTr("全选"),
                  detail: qsTr("选中当前目录的全部条目") },
                { id: "explorer.toggleHidden", label: qsTr("显示隐藏文件"),
                  detail: qsTr("切换隐藏文件和目录的可见性") },
                { id: "explorer.rename", label: qsTr("重命名"),
                  detail: qsTr("重命名当前选中项") },
                { id: "explorer.preview", label: qsTr("预览"),
                  detail: qsTr("打开或关闭当前项的预览") },
                { id: "explorer.switchPane", label: qsTr("切换窗格"),
                  detail: qsTr("在分屏的两个窗格之间移动焦点") },
                { id: "explorer.properties", label: qsTr("属性"),
                  detail: qsTr("打开选中项的属性") },
                { id: "explorer.refresh", label: qsTr("刷新"),
                  detail: qsTr("重新读取当前目录") },
                { id: "explorer.newFolder", label: qsTr("新建文件夹"),
                  detail: qsTr("在当前位置新建文件夹") },
                { id: "explorer.toggleSplit", label: qsTr("切换分屏"),
                  detail: qsTr("打开或关闭双窗格") },
                { id: "explorer.displayOptions", label: qsTr("显示选项"),
                  detail: qsTr("打开视图、排序与列菜单") },
                { id: "explorer.filters", label: qsTr("过滤"),
                  detail: qsTr("打开类型、时间与大小过滤") },
                { id: "explorer.cycleView", label: qsTr("切换视图"),
                  detail: qsTr("在列表、网格与分栏之间循环") },
                { id: "explorer.viewList", label: qsTr("列表视图"),
                  detail: qsTr("直接切换到详细列表") },
                { id: "explorer.viewGrid", label: qsTr("网格视图"),
                  detail: qsTr("直接切换到图标网格") },
                { id: "explorer.viewColumn", label: qsTr("分栏视图"),
                  detail: qsTr("直接切换到 Miller 分栏") },
                { id: "explorer.toggleSizeUnits", label: qsTr("切换大小单位"),
                  detail: qsTr("在 KB/MB 与 KiB/MiB 之间切换") },
                { id: "explorer.terminalJumpFolder", label: qsTr("终端跳转当前目录"),
                  detail: qsTr("让内嵌终端 cd 到当前标签的文件夹") }
            ]
        },
        {
            title: qsTr("文件操作"),
            actions: [
                { id: "explorer.copy", label: qsTr("复制"),
                  detail: qsTr("把选中项复制到剪贴板") },
                { id: "explorer.cut", label: qsTr("剪切"),
                  detail: qsTr("把选中项剪切到剪贴板") },
                { id: "explorer.paste", label: qsTr("粘贴"),
                  detail: qsTr("把剪贴板内容粘贴到当前目录") },
                { id: "explorer.undo", label: qsTr("撤销"),
                  detail: qsTr("撤销上一次文件操作") },
                { id: "explorer.redo", label: qsTr("重做"),
                  detail: qsTr("重做上一次撤销的操作") },
                { id: "explorer.redoAlt", label: qsTr("重做（备用键）"),
                  detail: qsTr("重做的第二个快捷键") },
                { id: "explorer.trash", label: qsTr("移到回收站"),
                  detail: qsTr("把选中项移到回收站") },
                { id: "explorer.deletePermanent", label: qsTr("永久删除"),
                  detail: qsTr("不经过回收站直接删除") },
                { id: "explorer.copyPath", label: qsTr("复制路径"),
                  detail: qsTr("把选中项的完整路径复制到剪贴板") },
                { id: "explorer.copyName", label: qsTr("复制名称"),
                  detail: qsTr("只复制选中项的名称") },
                { id: "explorer.openWith", label: qsTr("打开方式"),
                  detail: qsTr("选择用哪个程序打开选中项") },
                { id: "explorer.compressZip", label: qsTr("压缩为 ZIP"),
                  detail: qsTr("把选中项压缩成一个 ZIP") },
                { id: "explorer.extractHere", label: qsTr("解压到此处"),
                  detail: qsTr("就地解开选中的压缩包") },
                { id: "explorer.measureSize", label: qsTr("计算大小"),
                  detail: qsTr("统计选中文件夹占用的空间") },
                { id: "explorer.compareHashes", label: qsTr("比较 SHA-256"),
                  detail: qsTr("比较两个本地文件的 SHA-256") },
                { id: "explorer.duplicate", label: qsTr("创建副本"),
                  detail: qsTr("在同目录创建选中项的副本") },
                { id: "explorer.createShortcut", label: qsTr("创建快捷方式"),
                  detail: qsTr("为选中项创建快捷方式") },
                { id: "explorer.moveTo", label: qsTr("移动到…"),
                  detail: qsTr("把选中项移动到另一个文件夹") },
                { id: "explorer.bulkRename", label: qsTr("批量重命名"),
                  detail: qsTr("用规则重命名当前选择") },
                { id: "explorer.openSystemTerminal", label: qsTr("在终端中打开"),
                  detail: qsTr("在系统终端中打开当前目录") }
            ]
        }
    ]

    /// Flattened once so the flickable list can use a single Repeater while
    /// still drawing section headers. `qsTr` in this binding is retranslated
    /// with the rest of the panel.
    readonly property var shortcutRows: {
        const rows = []
        for (let groupIndex = 0; groupIndex < shortcutGroups.length; ++groupIndex) {
            const group = shortcutGroups[groupIndex]
            rows.push({ header: true, title: group.title })
            for (let actionIndex = 0; actionIndex < group.actions.length; ++actionIndex) {
                const action = group.actions[actionIndex]
                rows.push({
                    header: false,
                    id: action.id,
                    label: action.label,
                    detail: action.detail
                })
            }
        }
        return rows
    }

    function open() {
        root.visible = true
        root.page = "appearance"
        root.captureId = ""
        root.shortcutError = ""
        root.terminalError = ""
        root.terminalStatus = ""
        root.syncTerminalFields()
        root.fontError = ""
        root.fontStatus = ""
        root.syncFontFields()
        Workspace.refreshDefaultManager()
        root.forceActiveFocus()
        Qt.callLater(function () {
            if (root.visible)
                root.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
        root.captureId = ""
        root.closed()
    }

    function syncTerminalFields() {
        sizeInput.text = Workspace.terminalFontSize
        heightInput.text = Workspace.terminalLineHeight.toFixed(2)
        familyInput.text = Workspace.terminalFontFamily
        shellInput.text = ExplorerPrefs.terminalShell
    }

    function syncFontFields() {
        uiFontInput.text = Appearance.uiFontFamily
        monoFontInput.text = Appearance.monoFontFamily
    }

    function saveFonts() {
        const ui = uiFontInput.text.trim()
        const mono = monoFontInput.text.trim()
        const installed = Theme.installedFamilies
        if (ui !== "" && installed.indexOf(ui) < 0) {
            root.fontError = qsTr("未找到界面字体“%1”").arg(ui)
            return
        }
        if (mono !== "" && installed.indexOf(mono) < 0) {
            root.fontError = qsTr("未找到等宽字体“%1”").arg(mono)
            return
        }
        const uiChanged = ui !== Appearance.uiFontFamily
        Appearance.chooseFonts(ui, mono)
        root.fontError = ""
        root.fontStatus = uiChanged
            ? qsTr("已保存；界面字体在重新启动后生效")
            : qsTr("已保存")
        fontStatusTimer.restart()
    }

    /// The stored actions, unfiltered: a row whose program is missing must
    /// still be editable, and the menu is what hides it.
    property var actionDefinitions: []
    property string newActionName: ""
    property string newActionCommand: "code"
    property string newActionArgs: "%d"
    property string newActionSelection: "directory"

    /// True when the landing page hides `section`. The stored value is a
    /// comma-separated list of hidden names, so an empty string means all four
    /// widgets are on.
    function homeSectionHidden(section) {
        const parts = ExplorerPrefs.hiddenHomeSections.split(",")
        for (let index = 0; index < parts.length; index += 1) {
            if (parts[index].trim() === section)
                return true
        }
        return false
    }

    function toggleHomeSection(section) {
        const parts = []
        const stored = ExplorerPrefs.hiddenHomeSections.split(",")
        for (let index = 0; index < stored.length; index += 1) {
            const value = stored[index].trim()
            if (value !== "" && value !== section)
                parts.push(value)
        }
        if (!root.homeSectionHidden(section))
            parts.push(section)
        ExplorerPrefs.setString("hiddenHomeSections", parts.join(","))
    }

    function refreshActions() {
        try {
            root.actionDefinitions = JSON.parse(Workspace.customActionDefinitions())
        } catch (error) {
            root.actionDefinitions = []
        }
    }

    function bindingText(binding) {
        return binding ? ShortcutJS.display(binding, root.isMac) : qsTr("未绑定")
    }

    /// Recognise fixed backend messages so the panel can translate them; a
    /// save error with a path suffix is kept verbatim.
    function backendError(text) {
        switch (text) {
        case "Unknown shortcut action": return qsTr("未知的快捷键操作")
        case "Invalid shortcut format": return qsTr("快捷键格式无效")
        case "Could not set the default file manager": return qsTr("无法设置默认文件管理器")
        case "Could not read the default file manager status": return qsTr("无法读取默认文件管理器状态")
        }
        const log = /^Could not create log directory: (.+)$/.exec(text)
        if (log)
            return qsTr("无法创建日志目录：%1").arg(log[1])
        return text
    }

    /// The platform layer reports the manual follow-up as a fixed sentence.
    function localizeDetail(text) {
        switch (text) {
        case "Confirm dae on the Default apps page in Windows Settings.":
            return qsTr("Windows 需要你在系统设置的「默认应用」页确认 dae。")
        case "macOS does not allow apps to remove their default status. Choose another app in Finder settings.":
            return qsTr("macOS 不允许应用自行取消默认身份，请在访达的设置中选择其他应用。")
        default: return text
        }
    }

    function bindingFor(id) {
        return root.shortcutMap && root.shortcutMap[id] ? root.shortcutMap[id] : ""
    }

    function conflictFor(id, binding) {
        if (!binding)
            return ""
        for (let groupIndex = 0; groupIndex < shortcutGroups.length; ++groupIndex) {
            const actions = shortcutGroups[groupIndex].actions
            for (let actionIndex = 0; actionIndex < actions.length; ++actionIndex) {
                const action = actions[actionIndex]
                if (action.id !== id
                        && ShortcutJS.bindingsEqual(binding, root.bindingFor(action.id)))
                    return action.label
            }
        }
        return ""
    }

    readonly property bool capturing: root.captureId !== ""

    function beginCapture(id, label) {
        root.captureId = id
        root.captureLabel = label
        root.shortcutError = ""
        captureLayer.forceActiveFocus()
    }

    function cancelCapture() {
        root.captureId = ""
        root.forceActiveFocus()
    }

    function commitCapture(binding) {
        const id = root.captureId
        if (id === "")
            return
        const conflict = root.conflictFor(id, binding)
        if (conflict !== "") {
            root.shortcutError = qsTr("“%1”已分配给“%2”").arg(root.bindingText(binding)).arg(conflict)
            root.captureId = ""
            root.forceActiveFocus()
            return
        }
        Workspace.setShortcut(id, binding)
        if (Workspace.lastError !== "")
            root.shortcutError = root.backendError(Workspace.lastError)
        root.captureId = ""
        root.forceActiveFocus()
    }

    function handleCaptureKey(event) {
        if (event.key === Qt.Key_Escape) {
            root.cancelCapture()
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Backspace && event.modifiers === Qt.NoModifier) {
            root.commitCapture("")
            event.accepted = true
            return
        }
        const binding = ShortcutJS.record(event, root.isMac)
        if (binding === "") {
            event.accepted = true
            return
        }
        root.commitCapture(binding)
        event.accepted = true
    }

    function saveTerminal() {
        const size = parseInt(sizeInput.text, 10)
        const lineHeight = parseFloat(heightInput.text)
        if (!(size >= 8 && size <= 24)) {
            root.terminalError = qsTr("字号需要在 8 到 24 之间")
            return
        }
        if (!(lineHeight >= 0.8 && lineHeight <= 3.0)) {
            root.terminalError = qsTr("行高需要在 0.80 到 3.00 之间")
            return
        }
        Workspace.saveTerminalSettings(size, lineHeight, familyInput.text)
        ExplorerPrefs.setString("terminalShell", shellInput.text)
        root.terminalError = ""
        root.terminalStatus = qsTr("已保存")
        terminalStatusTimer.restart()
    }

    // Escape is owned by ShellWindow's single modal handler; while a key is
    // being recorded that handler is disabled and the capture layer cancels
    // instead of closing the whole panel.

    /// §11.7: settings is a page, and every key-value row carries the control
    /// on the right and a per-row default beside it.
    component SettingsRow: Item {
        id: setting

        property string label
        property string description
        property bool showReset: true
        default property alias control: controlHost.data
        signal reset()

        width: parent ? parent.width : 0
        /// A row with a second line needs the extra 14 dp: at 40 dp the 12 px
        /// note sat two pixels under the 14 px label and the pair read as one
        /// smudged line, with the separator touching the descenders.
        height: setting.description !== "" ? 54 : 44

        HoverHandler {
            id: rowHover
        }

        Text {
            id: rowLabel
            x: 0
            y: setting.description !== ""
                ? 7 : Math.round((setting.height - rowLabel.height) / 2)
            width: Math.max(80, controlHost.x - 12)
            text: setting.label
            color: root.textColor
            font.pixelSize: Theme.typeBody
            textFormat: Text.PlainText
            elide: Text.ElideRight
        }

        Text {
            visible: setting.description !== ""
            x: 0
            y: rowLabel.y + rowLabel.height + 3
            width: Math.max(80, controlHost.x - 12)
            text: setting.description
            color: root.faintColor
            font.pixelSize: Theme.typeCaption
            textFormat: Text.PlainText
            elide: Text.ElideRight
        }

        Item {
            id: controlHost
            anchors.right: setting.showReset ? resetButton.left : parent.right
            anchors.rightMargin: setting.showReset ? 8 : 0
            anchors.verticalCenter: parent.verticalCenter
            width: childrenRect.width
            height: childrenRect.height
        }

        /// The reset is a hint, not a column of its own: a 28 dp undo mark
        /// that fades in on hover or focus. Fourteen "恢复默认" labels stacked
        /// at the right edge turned the page into a wall of grey text and
        /// pushed every control 80 dp away from the edge it belonged to.
        Rectangle {
            id: resetButton
            visible: setting.showReset
            anchors.right: parent.right
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            width: 28
            height: 28
            radius: Theme.radiusFull
            color: resetMouse.containsMouse ? Theme.hover : "transparent"
            opacity: rowHover.hovered || resetMouse.containsMouse
                || resetButton.activeFocus ? 1 : 0
            Behavior on opacity {
                NumberAnimation {
                    duration: 110
                    easing.type: Easing.OutCubic
                }
            }

            activeFocusOnTab: true
            Keys.onReturnPressed: setting.reset()
            Keys.onSpacePressed: setting.reset()

            Rectangle {
                anchors.fill: parent
                anchors.margins: -2
                radius: Theme.radiusFull
                color: "transparent"
                border.width: 2
                border.color: Theme.accent
                visible: resetButton.activeFocus
            }

            Glyph {
                anchors.centerIn: parent
                width: 14
                height: 14
                kind: "undo"
                tint: resetMouse.containsMouse ? root.textColor : root.mutedColor
            }

            ToolTip {
                text: qsTr("恢复默认")
            }

            MouseArea {
                id: resetMouse
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("恢复%1的默认值").arg(setting.label)
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: setting.reset()
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: root.hairline
            opacity: 0.7
        }
    }

    Timer {
        id: terminalStatusTimer
        interval: 2200
        onTriggered: root.terminalStatus = ""
    }

    Timer {
        id: fontStatusTimer
        interval: 3200
        onTriggered: root.fontStatus = ""
    }

    // A full page, not a dialog: the chrome bar stays live so the window can
    // still be moved and closed; everything below it belongs to settings.
    Rectangle {
        id: panel
        anchors.fill: parent
        anchors.topMargin: Theme.shellTitle
        color: "transparent"
        clip: true

        Glass { radius: 0 }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
        }

        // ---------------------------------------------------------------- nav
        Rectangle {
            id: sidebar
            width: 200
            height: parent.height
            color: root.sidebarColor

            Rectangle {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: Theme.ruleWidth(Screen.devicePixelRatio)
                color: root.hairline
            }

            Text {
                x: 20
                y: 20
                text: qsTr("设置")
                color: root.textColor
                font.pixelSize: Theme.typeHeading
                font.weight: Font.Medium
            }

            Text {
                x: 20
                y: 46
                text: qsTr("dae · Qt 桌面壳")
                color: root.faintColor
                font.pixelSize: Theme.typeCaption
            }

            Column {
                x: 8
                y: 80
                width: parent.width - 16
                spacing: 4

                NavItem { label: qsTr("外观与语言"); value: "appearance" }
                NavItem { label: qsTr("字体设置"); value: "fonts" }
                NavItem { label: qsTr("快捷键"); value: "shortcuts" }
                NavItem { label: qsTr("动作"); value: "actions" }
                NavItem { label: qsTr("终端"); value: "terminal" }
                NavItem { label: qsTr("标签"); value: "tags" }
                NavItem { label: qsTr("默认文件管理器"); value: "defaultManager" }
                NavItem { label: qsTr("日志"); value: "logs" }
            }
        }

        // ------------------------------------------------------------ content
        Item {
            id: content
            x: sidebar.width + 1
            width: parent.width - sidebar.width - 1
            height: parent.height - footer.height

            // Each page positions its own content from the top, so pages with
            // different heights share one frame without a layout dependency.
            // Appearance outgrew the frame once background effects joined it,
            // so this page is the one that scrolls.
            Flickable {
                id: appearancePage

                anchors.fill: parent
                visible: root.page === "appearance"
                contentWidth: width
                contentHeight: appearanceColumn.implicitHeight + 48
                clip: true
                flickableDirection: Flickable.VerticalFlick
                boundsBehavior: Flickable.StopAtBounds

                Column {
                    id: appearanceColumn

                    x: 24
                    y: 20
                    /// A settings page keeps a readable measure instead of
                    /// stretching rows to a 1200 dp window: the label and its
                    /// control stay one glance apart.
                    width: Math.min(760, Math.max(320, content.width - 48))
                    spacing: 0

                    Text {
                        text: qsTr("外观与语言")
                        color: root.textColor
                        font.pixelSize: Theme.typeHeading
                        font.weight: Font.Medium
                    }

                    Text {
                        y: 6
                        width: Math.min(parent.width, 620)
                        text: qsTr("颜色、密度、材质与背景都在这里；改动立即生效。")
                        color: root.mutedColor
                        font.pixelSize: Theme.typeCaption
                        wrapMode: Text.WordWrap
                    }

                    Item { width: 1; height: 18 }

                    // ── 颜色 ────────────────────────────────────────────────
                    Text {
                        height: 34
                        verticalAlignment: Text.AlignBottom
                        text: qsTr("颜色")
                        color: Theme.inkSecondary
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.DemiBold
                    }

                    SettingsRow {
                        label: qsTr("主题")
                        description: qsTr("跟随系统时，深浅色会随系统切换。")
                        onReset: Appearance.chooseTheme("system")

                        Row {
                            spacing: 8

                            Repeater {
                                model: root.themeOptions
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: Appearance.theme === modelData.value
                                    onClicked: Appearance.chooseTheme(modelData.value)
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("强调色")
                        description: qsTr("“系统”读取桌面强调色，“青蓝”用自带色，“自定义”从调色盘选种色。")
                        onReset: Appearance.chooseAccent("system")

                        Row {
                            spacing: 8

                            Repeater {
                                model: root.accentOptions
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: Appearance.accent === modelData.value
                                    swatch: modelData.value === "brand" ? Theme.brandAccent
                                        : modelData.value === "custom"
                                            ? (Appearance.customAccent !== ""
                                                ? Appearance.customAccent : Theme.accent)
                                            : Workspace.accent
                                    onClicked: Appearance.chooseAccent(modelData.value)
                                }
                            }
                        }
                    }

                    /// The seed palette, revealed only while the accent source
                    /// is "custom" so the row ladder stays 40 dp elsewhere.
                    /// Picking a swatch re-derives every M3 role live.
                    Item {
                        visible: Appearance.accent === "custom"
                        width: parent.width
                        height: visible ? paletteBody.implicitHeight + 12 : 0

                        Column {
                            id: paletteBody
                            width: parent.width
                            spacing: 10

                            Flow {
                                width: parent.width
                                spacing: 8

                                Repeater {
                                    model: root.accentPalette
                                    delegate: Rectangle {
                                        required property var modelData
                                        property color hue: modelData
                                        readonly property bool picked:
                                            Appearance.customAccent === modelData
                                        width: 26
                                        height: 26
                                        radius: 13
                                        color: hue
                                        border.width: picked ? 2 : 1
                                        border.color: picked ? Theme.ink : Theme.outlineVariant

                                        Accessible.role: Accessible.RadioButton
                                        Accessible.name: modelData
                                        Accessible.checkable: true
                                        Accessible.checked: picked
                                        Accessible.onPressAction:
                                            Appearance.chooseCustomAccent(modelData)

                                        Rectangle {
                                            anchors.centerIn: parent
                                            visible: picked
                                            width: 8
                                            height: 8
                                            radius: 4
                                            color: Theme.readableInk(hue)
                                        }

                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: Appearance.chooseCustomAccent(modelData)
                                        }
                                    }
                                }
                            }

                            Row {
                                spacing: 8

                                ChoiceChip {
                                    label: qsTr("取色…")
                                    onClicked: accentPicker.open()
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: Appearance.customAccent !== ""
                                        ? Appearance.customAccent : qsTr("未选择")
                                    color: root.mutedColor
                                    font.pixelSize: Theme.typeCaption
                                    font.family: Theme.monoFamily
                                }
                            }
                        }

                        ColorDialog {
                            id: accentPicker
                            onAccepted: Appearance.chooseCustomAccent(selectedColor)
                        }
                    }

                    SettingsRow {
                        label: qsTr("界面语言")
                        description: qsTr("切换后整个窗口立即重新翻译。")
                        onReset: Appearance.chooseLanguage("system")

                        Row {
                            spacing: 8

                            Repeater {
                                model: root.languageOptions
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: Appearance.language === modelData.value
                                    onClicked: Appearance.chooseLanguage(modelData.value)
                                }
                            }
                        }
                    }

                    Item { width: 1; height: 14 }

                    // ── 网格与密度 ──────────────────────────────────────────
                    Text {
                        height: 34
                        verticalAlignment: Text.AlignBottom
                        text: qsTr("网格与密度")
                        color: Theme.inkSecondary
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.DemiBold
                    }

                    SettingsRow {
                        label: qsTr("界面密度")
                        description: qsTr("影响文件列表的行高与网格间距。")
                        onReset: ExplorerPrefs.setString("density", "comfortable")

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: "compact", label: qsTr("紧凑") },
                                    { value: "comfortable", label: qsTr("标准") },
                                    { value: "spacious", label: qsTr("宽松") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: ExplorerPrefs.density === modelData.value
                                    onClicked: ExplorerPrefs.setString("density",
                                                                       modelData.value)
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("缩略图形状")
                        description: qsTr("网格视图的缩略图比例；1:1 更适合图标。")
                        onReset: ExplorerPrefs.setString("gridThumb", "portrait")

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: "portrait", label: qsTr("3:4") },
                                    { value: "square", label: qsTr("1:1") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: ExplorerPrefs.gridThumb === modelData.value
                                    onClicked: ExplorerPrefs.setString("gridThumb",
                                                                       modelData.value)
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("文件大小单位")
                        description: qsTr("只影响显示；排序与比较仍用字节数。")
                        onReset: ExplorerPrefs.setString("sizeUnits", "binary")

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: "binary", label: qsTr("二进制（KiB）") },
                                    { value: "decimal", label: qsTr("十进制（KB）") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: ExplorerPrefs.sizeUnits === modelData.value
                                    onClicked: ExplorerPrefs.setString("sizeUnits",
                                                                       modelData.value)
                                }
                            }
                        }
                    }

                    Item { width: 1; height: 14 }

                    // ── 界面 ────────────────────────────────────────────────
                    Text {
                        height: 34
                        verticalAlignment: Text.AlignBottom
                        text: qsTr("界面")
                        color: Theme.inkSecondary
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.DemiBold
                    }

                    SettingsRow {
                        label: qsTr("动效方案")
                        description: qsTr("生动方案带弹簧过冲；标准方案平稳落入，不回弹。")
                        onReset: ExplorerPrefs.setString("motionScheme", "expressive")

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: "expressive", label: qsTr("生动") },
                                    { value: "standard", label: qsTr("标准") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: ExplorerPrefs.motionScheme === modelData.value
                                    onClicked: ExplorerPrefs.setString("motionScheme",
                                                                       modelData.value)
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("减少动效")
                        description: qsTr("开启后所有过渡改为瞬时切换。")
                        onReset: ExplorerPrefs.setBool("reduceMotion", false)

                        M3Switch {
                            checked: ExplorerPrefs.reduceMotion
                            onClicked: ExplorerPrefs.setBool("reduceMotion",
                                                             !ExplorerPrefs.reduceMotion)
                        }
                    }

                    SettingsRow {
                        label: qsTr("窗口材质")
                        description: qsTr("Mica 从壁纸取色，Mica Alt 色调更重、适合标签栏，Acrylic 实时模糊；仅作用于顶部标签栏。选择材质会开启背景效果，默认不透明。")
                        visible: root.showMaterialChoices
                        onReset: Appearance.chooseWindowMaterial("none")

                        Row {
                            spacing: 8

                            Repeater {
                                model: root.materialOptions
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: Appearance.windowMaterial === modelData.value
                                    onClicked: {
                                        Appearance.chooseWindowMaterial(modelData.value)
                                        // A material cannot composite while the
                                        // background effect is off; picking one
                                        // is the request to turn it on.
                                        if (modelData.value !== "none"
                                                && !Appearance.blurEnabled)
                                            Appearance.chooseBlur(true)
                                    }
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("背景效果")
                        description: root.backgroundNote !== ""
                            ? root.backgroundNote : qsTr("关闭则使用实色，适合远程桌面环境。")
                        onReset: Appearance.chooseBlur(false)

                        M3Switch {
                            checked: Appearance.blurEnabled
                            onClicked: Appearance.chooseBlur(!Appearance.blurEnabled)
                        }
                    }

                    SettingsRow {
                        label: qsTr("浮层毛玻璃")
                        description: qsTr("菜单、对话框等浮层模糊背后内容；关闭改为不透明表面，适合性能较弱的设备。")
                        onReset: Appearance.chooseFrostedGlass(true)

                        M3Switch {
                            checked: Appearance.frostedGlass
                            onClicked: Appearance.chooseFrostedGlass(!Appearance.frostedGlass)
                        }
                    }

                    SettingsRow {
                        label: qsTr("启动")
                        description: qsTr("先显示纯色窗口，界面在它后面加载；下次启动生效。")
                        onReset: Appearance.chooseShellFirst(true)

                        M3Switch {
                            checked: Appearance.shellFirst
                            onClicked: Appearance.chooseShellFirst(!Appearance.shellFirst)
                        }
                    }

                    SettingsRow {
                        label: qsTr("主页小部件")
                        description: qsTr("关闭一项后，概览页对应的分组整段隐藏。")
                        onReset: ExplorerPrefs.setString("hiddenHomeSections", "")

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: "Projects", label: qsTr("项目") },
                                    { value: "Favorites", label: qsTr("收藏") },
                                    { value: "Recent", label: qsTr("最近") },
                                    { value: "Spaces", label: qsTr("空间") },
                                    { value: "Tags", label: qsTr("标签") },
                                    { value: "Drives", label: qsTr("驱动器") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: !root.homeSectionHidden(modelData.value)
                                    onClicked: root.toggleHomeSection(modelData.value)
                                }
                            }
                        }
                    }

                    Item { width: 1; height: 14 }

                    // ── 背景与工具栏 ────────────────────────────────────────
                    Text {
                        height: 34
                        verticalAlignment: Text.AlignBottom
                        text: qsTr("背景与工具栏")
                        color: Theme.inkSecondary
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.DemiBold
                    }

                    SettingsRow {
                        label: qsTr("背景图")
                        description: qsTr("图片铺在列表后面，列表用半透明画布盖住它。")
                        onReset: ExplorerPrefs.setString("backgroundImage", "")

                        Row {
                            spacing: 8

                            Item {
                                width: 220
                                height: Theme.controlMd

                                FieldFrame {
                                    anchors.fill: parent
                                }

                                TextInput {
                                    Accessible.role: Accessible.EditableText
                                    Accessible.name: qsTr("背景图片路径")
                                    anchors.fill: parent
                                    anchors.leftMargin: 8
                                    anchors.rightMargin: 8
                                    verticalAlignment: TextInput.AlignVCenter
                                    text: ExplorerPrefs.backgroundImage
                                    color: root.textColor
                                    selectionColor: root.accent
                                    selectedTextColor: Theme.inkOnAccent
                                    font.pixelSize: Theme.typeCaption
                                    selectByMouse: true
                                    clip: true
                                    onEditingFinished:
                                        ExplorerPrefs.setString("backgroundImage", text)
                                }
                            }

                            Rectangle {
                                width: 68
                                height: Theme.controlMd
                                radius: Theme.radiusSm
                                color: chooseMouse.containsMouse
                                    ? root.hoverFill : "transparent"
                                border.width: 1
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: qsTr("选择…")
                                    color: root.accent
                                    font.pixelSize: Theme.typeCaption
                                    font.weight: Theme.weightStrong
                                }

                                MouseArea {
                                    id: chooseMouse
                                    Accessible.role: Accessible.Button
                                    Accessible.name: qsTr("选择背景图片")
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: backgroundDialog.open()
                                }
                            }

                            Rectangle {
                                width: 68
                                height: Theme.controlMd
                                radius: Theme.radiusSm
                                color: clearMouse.containsMouse
                                    ? root.hoverFill : "transparent"
                                border.width: 1
                                border.color: Theme.outlineVariant
                                enabled: ExplorerPrefs.backgroundImage !== ""
                                opacity: enabled ? 1 : 0.45

                                Text {
                                    anchors.centerIn: parent
                                    text: qsTr("清除")
                                    color: root.mutedColor
                                    font.pixelSize: Theme.typeCaption
                                }

                                MouseArea {
                                    id: clearMouse
                                    Accessible.role: Accessible.Button
                                    Accessible.name: qsTr("清除背景图片")
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    enabled: parent.enabled
                                    onClicked:
                                        ExplorerPrefs.setString("backgroundImage", "")
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("遮罩")
                        description: qsTr("遮罩越高，背景图上的文字越清楚。")
                        onReset: ExplorerPrefs.setNumber("backgroundDim", 70)

                        Row {
                            spacing: 8

                            Repeater {
                                model: [
                                    { value: 0, label: qsTr("无遮罩") },
                                    { value: 40, label: qsTr("轻") },
                                    { value: 70, label: qsTr("中") },
                                    { value: 90, label: qsTr("强") }
                                ]
                                delegate: ChoiceChip {
                                    required property var modelData
                                    label: modelData.label
                                    selected: ExplorerPrefs.backgroundDim === modelData.value
                                    onClicked: ExplorerPrefs.setNumber("backgroundDim",
                                                                      modelData.value)
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: qsTr("工具栏")
                        description: qsTr("拖动地址栏两侧的按钮调整顺序，重启后保留。")
                        showReset: false

                        Rectangle {
                            width: 96
                            height: Theme.controlMd
                            radius: Theme.radiusSm
                            color: toolbarResetMouse.containsMouse
                                ? root.hoverFill : "transparent"
                            border.width: 1
                            border.color: Theme.outlineVariant

                            Text {
                                anchors.centerIn: parent
                                text: qsTr("恢复默认")
                                color: root.accent
                                font.pixelSize: Theme.typeCaption
                                font.weight: Theme.weightStrong
                            }

                            MouseArea {
                                id: toolbarResetMouse
                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("恢复默认工具栏")
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: {
                                    ExplorerPrefs.setString("toolbarLeading",
                                                            "back,forward,up")
                                    ExplorerPrefs.setString("toolbarTrailing",
                                                            "search,trash,view,more")
                                }
                            }
                        }
                    }

                    Item { width: 1; height: 18 }

                    Rectangle {
                        width: parent.width
                        height: Theme.ruleWidth(Screen.devicePixelRatio)
                        color: root.hairline
                    }

                    Text {
                        y: 12
                        width: parent.width
                        text: qsTr("外观、语言、背景效果、启动方式与终端排版保存在 settings.toml；大小单位与视图偏好保存在 explorer-qt.ini。")
                        color: root.faintColor
                        font.pixelSize: Theme.typeCaption
                        wrapMode: Text.WordWrap
                    }
                }
            }

            ScrollBar {
                target: root.page === "appearance" ? appearancePage : null
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.top: parent.top
                anchors.bottom: parent.bottom
            }

            // ----------------------------------------------------------- fonts
            Item {
                anchors.fill: parent
                visible: root.page === "fonts"

                Text {
                    x: 24; y: 20
                    text: qsTr("字体设置")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 52
                    width: Math.min(content.width - 48, 620)
                    text: qsTr("输入系统已安装的字体族名；留空使用内置字体。界面字体在重新启动后生效，等宽字体立即生效。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                    lineHeight: Theme.leadingBody
                }

                Text {
                    x: 24; y: 93
                    text: qsTr("界面字体")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: uiFontFrame
                    x: 130; y: 84
                    width: Math.min(420, content.width - 178)
                    height: Theme.controlLg
                }
                TextInput {
                    id: uiFontInput
                    anchors.fill: uiFontFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveFonts()
                }
                Text {
                    x: 130; y: 120
                    width: Math.min(420, content.width - 178)
                    text: qsTr("留空使用内置 Google Sans Flex；中文回退到系统字体。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 159
                    text: qsTr("等宽字体")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: monoFontFrame
                    x: 130; y: 150
                    width: Math.min(420, content.width - 178)
                    height: Theme.controlLg
                }
                TextInput {
                    id: monoFontInput
                    anchors.fill: monoFontFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveFonts()
                }
                Text {
                    x: 130; y: 186
                    width: Math.min(420, content.width - 178)
                    text: qsTr("留空使用内置 Google Sans Code；终端、大小、速率与代码预览共用。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 236
                    width: Math.min(content.width - 48, 620)
                    text: qsTr("已安装的字体族可在此输入；切换界面字体不会改变等宽区域。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 268
                    width: Math.min(content.width - 48, 620)
                    visible: text !== ""
                    text: root.fontError
                    color: Theme.danger
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24
                    y: 300
                    spacing: 12

                    PrimaryButton {
                        label: qsTr("保存字体")
                        onClicked: root.saveFonts()
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.fontStatus
                        color: Theme.success
                        font.pixelSize: Theme.typeCaption
                    }
                }
            }

            // ------------------------------------------------------- shortcuts
            Item {
                anchors.fill: parent
                visible: root.page === "shortcuts"

                Text {
                    x: 24; y: 20
                    text: qsTr("快捷键")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 50
                    width: content.width - 160
                    text: qsTr("点击右侧按键重新录制。按 Backspace 清除绑定，按 Esc 取消；重复的绑定不会被保存。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                SmallButton {
                    x: content.width - 24 - width
                    y: 24
                    label: qsTr("全部恢复默认")
                    onClicked: {
                        root.shortcutError = ""
                        Workspace.resetShortcuts()
                    }
                }

                Text {
                    x: 24; y: 68
                    width: content.width - 48
                    visible: text !== ""
                    text: root.shortcutError
                    color: Theme.warning
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Flickable {
                    id: shortcutFlick
                    x: 24
                    y: root.shortcutError !== "" ? 92 : 76
                    width: content.width - 48
                    height: content.height - y - 8
                    contentWidth: width
                    contentHeight: shortcutColumn.implicitHeight
                    clip: true
                    flickableDirection: Flickable.VerticalFlick
                    boundsBehavior: Flickable.StopAtBounds

                    Column {
                        id: shortcutColumn
                        width: parent.width
                        spacing: 2

                        Repeater {
                            model: root.shortcutRows

                            delegate: Item {
                                id: shortcutRow

                                required property var modelData
                                width: shortcutColumn.width
                                height: modelData.header ? 34 : 40

                                Text {
                                    visible: shortcutRow.modelData.header
                                    anchors.left: parent.left
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: 8
                                    text: shortcutRow.modelData.header
                                        ? shortcutRow.modelData.title : ""
                                    color: Theme.inkSecondary
                                    font.pixelSize: Theme.typeCaption
                                    font.weight: Font.DemiBold
                                }

                                Rectangle {
                                    visible: !shortcutRow.modelData.header
                                    anchors.fill: parent
                                    anchors.margins: 0
                                    radius: Theme.radiusSm
                                    color: rowMouse.containsMouse ? root.hoverFill : "transparent"

                                    Text {
                                        x: 0
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: shortcutRow.modelData.header
                                            ? "" : shortcutRow.modelData.label
                                        color: root.textColor
                                        font.pixelSize: Theme.typeRow
                                    }
                                    Text {
                                        x: Math.min(190, parent.width * 0.35)
                                        width: parent.width - x - 180
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: shortcutRow.modelData.header
                                            ? "" : shortcutRow.modelData.detail
                                        color: root.faintColor
                                        font.pixelSize: Theme.typeCaption
                                        elide: Text.ElideRight
                                    }

                                    Rectangle {
                                        id: bindingKey
                                        anchors.right: resetKey.left
                                        anchors.rightMargin: 8
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: Math.min(156, bindingLabel.implicitWidth + 22)
                                        height: Theme.controlSm + 2
                                        radius: Theme.radiusSm
                                        color: keyMouse.containsMouse ? root.selectedFill : root.fieldColor
                                        border.width: 1
                                        border.color: keyMouse.containsMouse ? root.accent : root.hairline

                                        Text {
                                            id: bindingLabel
                                            anchors.centerIn: parent
                                            width: parent.width - 12
                                            horizontalAlignment: Text.AlignHCenter
                                            text: shortcutRow.modelData.header ? ""
                                                : root.bindingText(root.bindingFor(shortcutRow.modelData.id))
                                            color: root.bindingFor(shortcutRow.modelData.id)
                                                ? root.textColor : root.faintColor
                                            font.pixelSize: Theme.typeCaption
                                            font.family: Theme.monoFamily
                                            elide: Text.ElideRight
                                        }

                                        MouseArea {
                                            id: keyMouse
                                            Accessible.role: Accessible.Button
                                            Accessible.name: qsTr("设置快捷键：%1").arg(shortcutRow.modelData.label)
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            onClicked: root.beginCapture(shortcutRow.modelData.id,
                                                shortcutRow.modelData.label)
                                        }
                                    }

                                    Rectangle {
                                        id: resetKey
                                        anchors.right: parent.right
                                        anchors.rightMargin: 4
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 28
                                        height: Theme.controlSm
                                        radius: Theme.radiusSm
                                        visible: !shortcutRow.modelData.header
                                        color: resetMouse.containsMouse ? root.hoverFill : "transparent"

                                        /// A glyph-only button: the arrow is the
                                        /// picture, the name is the meaning.
                                        Accessible.role: Accessible.Button
                                        Accessible.name: qsTr("恢复默认快捷键")
                                        Accessible.onPressAction: {
                                            root.shortcutError = ""
                                            Workspace.resetShortcut(shortcutRow.modelData.id)
                                        }

                                        Glyph {
                                            anchors.centerIn: parent
                                            width: 14
                                            height: 14
                                            kind: "undo"
                                            tint: resetMouse.containsMouse
                                                ? root.textColor : root.faintColor
                                        }

                                        ToolTip {
                                            text: qsTr("恢复默认快捷键")
                                        }

                                        MouseArea {
                                            id: resetMouse
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            onClicked: {
                                                root.shortcutError = ""
                                                Workspace.resetShortcut(shortcutRow.modelData.id)
                                            }
                                        }
                                    }
                                }

                                MouseArea {
                                    id: rowMouse
                                    /// The row repeats the key pill's action on the empty space beside it,
                                    /// so the pill stays the one control the row announces.
                                    Accessible.ignored: true
                                    z: -1
                                    anchors.fill: parent
                                    enabled: !shortcutRow.modelData.header
                                    hoverEnabled: true
                                    onClicked: {
                                        if (!shortcutRow.modelData.header)
                                            root.beginCapture(shortcutRow.modelData.id,
                                                shortcutRow.modelData.label)
                                    }
                                }
                            }
                        }
                    }
                }

                /// The shortcut table is its own scroller under the page
                /// header; its bar is the shared capsule, not a thinner
                /// special case.
                ScrollBar {
                    target: shortcutFlick
                    x: shortcutFlick.x + shortcutFlick.width + 2
                    y: shortcutFlick.y
                    height: shortcutFlick.height
                }
            }

            // --------------------------------------------------------- actions
            Flickable {
                id: actionsPage
                anchors.fill: parent
                visible: root.page === "actions"
                contentWidth: width
                contentHeight: actionsColumn.height + 48
                clip: true
                flickableDirection: Flickable.VerticalFlick
                boundsBehavior: Flickable.StopAtBounds

                Column {
                    id: actionsColumn
                    x: 24
                    y: 20
                    width: actionsPage.width - 48
                    spacing: 10

                    Text {
                        text: qsTr("动作")
                        color: root.textColor
                        font.pixelSize: Theme.typeHeading
                        font.weight: Font.Medium
                    }

                    Text {
                        width: parent.width
                        text: qsTr("动作把选中的文件或文件夹交给外部程序，参数逐个传递，不经过 shell。占位符：%f 首个选中项，%F 每一项（各成一个参数），%d 当前目录，%u 首个选中项的 file: 地址。参数每行一个。只列出程序在 PATH 上的动作。")
                        color: root.mutedColor
                        font.pixelSize: Theme.typeCaption
                        wrapMode: Text.WordWrap
                    }

                    Repeater {
                        model: root.actionDefinitions

                        delegate: Column {
                            id: actionRow
                            required property var modelData
                            required property int index

                            width: actionsColumn.width
                            spacing: 6

                            property string editName: modelData.name
                            property string editCommand: modelData.command
                            property string editArgs: modelData.args
                            property string editSelection: modelData.selection

                            Rectangle {
                                width: parent.width
                                height: Theme.ruleWidth(Screen.devicePixelRatio)
                                color: root.hairline
                            }

                            Row {
                                spacing: 8

                                FieldFrame {
                                    width: 160
                                    height: Theme.controlMd
                                }
                                TextInput {
                                    Accessible.role: Accessible.EditableText
                                    Accessible.name: qsTr("动作名称")
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 152
                                    height: Theme.controlMd
                                    text: actionRow.editName
                                    onTextEdited: actionRow.editName = text
                                    verticalAlignment: TextInput.AlignVCenter
                                    color: root.textColor
                                    selectionColor: root.accent
                                    selectedTextColor: Theme.inkOnAccent
                                    font.pixelSize: Theme.typeCaption
                                    selectByMouse: true
                                    clip: true
                                }

                                FieldFrame {
                                    width: 180
                                    height: Theme.controlMd
                                }
                                TextInput {
                                    Accessible.role: Accessible.EditableText
                                    Accessible.name: qsTr("动作命令")
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 172
                                    height: Theme.controlMd
                                    text: actionRow.editCommand
                                    onTextEdited: actionRow.editCommand = text
                                    verticalAlignment: TextInput.AlignVCenter
                                    color: root.textColor
                                    selectionColor: root.accent
                                    selectedTextColor: Theme.inkOnAccent
                                    font.pixelSize: Theme.typeCaption
                                    font.family: Theme.monoFamily
                                    selectByMouse: true
                                    clip: true
                                }
                            }

                            Row {
                                spacing: 6

                                Repeater {
                                    model: [
                                        { value: "any", label: qsTr("任意") },
                                        { value: "file", label: qsTr("文件") },
                                        { value: "directory", label: qsTr("文件夹") },
                                        { value: "multiple", label: qsTr("多选") }
                                    ]
                                    delegate: ChoiceChip {
                                        required property var modelData
                                        label: modelData.label
                                        selected: actionRow.editSelection === modelData.value
                                        onClicked: actionRow.editSelection = modelData.value
                                    }
                                }
                            }

                            FieldFrame {
                                width: parent.width
                                height: 46

                                TextInput {
                                    Accessible.role: Accessible.EditableText
                                    Accessible.name: qsTr("动作参数")
                                    anchors.fill: parent
                                    anchors.leftMargin: 8
                                    anchors.rightMargin: 8
                                    anchors.topMargin: 6
                                    anchors.bottomMargin: 6
                                    text: actionRow.editArgs
                                    onTextEdited: actionRow.editArgs = text
                                    color: root.textColor
                                    selectionColor: root.accent
                                    selectedTextColor: Theme.inkOnAccent
                                    font.pixelSize: Theme.typeCaption
                                    font.family: Theme.monoFamily
                                    selectByMouse: true
                                    clip: true
                                    wrapMode: TextInput.WrapAnywhere
                                }
                            }

                            Row {
                                spacing: 10

                                Text {
                                    id: saveAction
                                    text: qsTr("保存")
                                    color: saveActionMouse.containsMouse
                                        ? root.accent : root.textColor
                                    font.pixelSize: Theme.typeCaption

                                    MouseArea {
                                        id: saveActionMouse
                                        Accessible.role: Accessible.Button
                                        Accessible.name: qsTr("保存动作")
                                        anchors.fill: parent
                                        anchors.margins: -4
                                        hoverEnabled: true
                                        onClicked: {
                                            Workspace.saveCustomAction(actionRow.modelData.id,
                                                actionRow.editName, actionRow.editCommand,
                                                actionRow.editArgs, actionRow.editSelection)
                                            root.refreshActions()
                                        }
                                    }
                                }

                                Text {
                                    text: qsTr("删除")
                                    color: deleteActionMouse.containsMouse
                                        ? Theme.danger : root.faintColor
                                    font.pixelSize: Theme.typeCaption

                                    MouseArea {
                                        id: deleteActionMouse
                                        Accessible.role: Accessible.Button
                                        Accessible.name: qsTr("删除动作")
                                        anchors.fill: parent
                                        anchors.margins: -4
                                        hoverEnabled: true
                                        onClicked: {
                                            Workspace.removeCustomAction(actionRow.modelData.id)
                                            root.refreshActions()
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Rectangle {
                        width: parent.width
                        height: Theme.ruleWidth(Screen.devicePixelRatio)
                        color: root.hairline
                    }

                    Text {
                        text: qsTr("新增动作")
                        color: root.textColor
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.Medium
                    }

                    Text {
                        text: qsTr("名称 · 程序 · 参数（每行一个）")
                        color: root.faintColor
                        font.pixelSize: Theme.typeCaption
                    }

                    Row {
                        spacing: 8

                        FieldFrame {
                            width: 160
                            height: Theme.controlMd
                        }
                        TextInput {
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("新动作名称")
                            anchors.verticalCenter: parent.verticalCenter
                            width: 152
                            height: Theme.controlMd
                            text: root.newActionName
                            onTextEdited: root.newActionName = text
                            verticalAlignment: TextInput.AlignVCenter
                            color: root.textColor
                            selectionColor: root.accent
                            selectedTextColor: Theme.inkOnAccent
                            font.pixelSize: Theme.typeCaption
                            selectByMouse: true
                            clip: true
                        }

                        FieldFrame {
                            width: 180
                            height: Theme.controlMd
                        }
                        TextInput {
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("新动作命令")
                            anchors.verticalCenter: parent.verticalCenter
                            width: 172
                            height: Theme.controlMd
                            text: root.newActionCommand
                            onTextEdited: root.newActionCommand = text
                            verticalAlignment: TextInput.AlignVCenter
                            color: root.textColor
                            selectionColor: root.accent
                            selectedTextColor: Theme.inkOnAccent
                            font.pixelSize: Theme.typeCaption
                            font.family: Theme.monoFamily
                            selectByMouse: true
                            clip: true
                        }
                    }

                    Row {
                        spacing: 6

                        Repeater {
                            model: [
                                { value: "any", label: qsTr("任意") },
                                { value: "file", label: qsTr("文件") },
                                { value: "directory", label: qsTr("文件夹") },
                                { value: "multiple", label: qsTr("多选") }
                            ]
                            delegate: ChoiceChip {
                                required property var modelData
                                label: modelData.label
                                selected: root.newActionSelection === modelData.value
                                onClicked: root.newActionSelection = modelData.value
                            }
                        }
                    }

                    FieldFrame {
                        width: parent.width
                        height: 46

                        TextInput {
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("新动作参数")
                            anchors.fill: parent
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            anchors.topMargin: 6
                            anchors.bottomMargin: 6
                            text: root.newActionArgs
                            onTextEdited: root.newActionArgs = text
                            color: root.textColor
                            selectionColor: root.accent
                            selectedTextColor: Theme.inkOnAccent
                            font.pixelSize: Theme.typeCaption
                            font.family: Theme.monoFamily
                            selectByMouse: true
                            clip: true
                            wrapMode: TextInput.WrapAnywhere
                        }
                    }

                    Row {
                        spacing: 10

                        Text {
                            text: qsTr("添加")
                            color: addActionMouse.containsMouse
                                ? root.accent : root.textColor
                            font.pixelSize: Theme.typeCaption

                            MouseArea {
                                id: addActionMouse
                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("添加动作")
                                anchors.fill: parent
                                anchors.margins: -4
                                hoverEnabled: true
                                onClicked: {
                                    if (root.newActionName.trim() === ""
                                            || root.newActionCommand.trim() === "")
                                        return
                                    Workspace.saveCustomAction("", root.newActionName,
                                        root.newActionCommand, root.newActionArgs,
                                        root.newActionSelection)
                                    root.newActionName = ""
                                    root.newActionArgs = "%d"
                                    root.refreshActions()
                                }
                            }
                        }

                        Text {
                            text: qsTr("恢复内置动作")
                            color: resetActionsMouse.containsMouse
                                ? root.accent : root.faintColor
                            font.pixelSize: Theme.typeCaption

                            MouseArea {
                                id: resetActionsMouse
                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("恢复内置动作")
                                anchors.fill: parent
                                anchors.margins: -4
                                hoverEnabled: true
                                onClicked: {
                                    Workspace.resetCustomActions()
                                    root.refreshActions()
                                }
                            }
                        }
                    }
                }
            }

            ScrollBar {
                target: root.page === "actions" ? actionsPage : null
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.top: parent.top
                anchors.bottom: parent.bottom
            }

            // -------------------------------------------------------- terminal
            Item {
                anchors.fill: parent
                visible: root.page === "terminal"

                Text {
                    x: 24; y: 20
                    text: qsTr("终端")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 52
                    width: Math.min(content.width - 48, 620)
                    text: qsTr("外观写入 settings.toml，两个桌面壳共用同一组值。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 90
                    text: qsTr("字号")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: sizeFrame
                    x: 130; y: 84
                    width: 90; height: Theme.controlLg
                }
                TextInput {
                    id: sizeInput
                    anchors.fill: sizeFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    inputMethodHints: Qt.ImhDigitsOnly
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 234
                    anchors.verticalCenter: sizeFrame.verticalCenter
                    text: qsTr("8 – 24")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                }

                Text {
                    x: 24; y: 138
                    text: qsTr("行高")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: heightFrame
                    x: 130; y: 132
                    width: 90; height: Theme.controlLg
                }
                TextInput {
                    id: heightInput
                    anchors.fill: heightFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 234
                    anchors.verticalCenter: heightFrame.verticalCenter
                    text: qsTr("0.80 – 3.00")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                }

                Text {
                    x: 24; y: 186
                    text: qsTr("字体")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: familyFrame
                    x: 130; y: 180
                    width: Math.min(420, content.width - 178)
                    height: Theme.controlLg
                }
                TextInput {
                    id: familyInput
                    anchors.fill: familyFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 130; y: 216
                    width: Math.min(420, content.width - 178)
                    text: qsTr("终端字体覆盖；留空使用「字体设置」页的全局等宽字体")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideRight
                }

                Text {
                    x: 24; y: 250
                    text: qsTr("Shell")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }
                FieldFrame {
                    id: shellFrame
                    x: 130; y: 244
                    width: Math.min(420, content.width - 178)
                    height: Theme.controlLg
                }
                TextInput {
                    id: shellInput
                    anchors.fill: shellFrame
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.inkOnAccent
                    font.pixelSize: Theme.typeRow
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 130; y: 280
                    width: Math.min(420, content.width - 178)
                    text: qsTr("可执行文件名或完整路径；留空自动选择（Windows: pwsh / PowerShell，其他平台: $SHELL）")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 321
                    text: qsTr("选中即复制")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                }

                Row {
                    x: 130; y: 312
                    spacing: 8

                    Repeater {
                        model: [
                            { value: false, label: qsTr("关闭") },
                            { value: true, label: qsTr("开启") }
                        ]
                        delegate: ChoiceChip {
                            required property var modelData
                            label: modelData.label
                            selected: ExplorerPrefs.copyOnSelect === modelData.value
                            onClicked: ExplorerPrefs.setBool("copyOnSelect",
                                                             modelData.value)
                        }
                    }
                }

                Text {
                    x: 24; y: 362
                    text: qsTr("字号与行高立即应用；Shell 在下次启动终端时生效。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                }

                Text {
                    id: terminalErrorText
                    x: 24; y: 392
                    width: Math.min(content.width - 48, 620)
                    visible: text !== ""
                    text: root.terminalError
                    color: Theme.danger
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24
                    y: 420
                    spacing: 12

                    PrimaryButton {
                        label: qsTr("保存终端设置")
                        onClicked: root.saveTerminal()
                    }

                    Text {
                        id: terminalStatusText
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.terminalStatus
                        color: Theme.success
                        font.pixelSize: Theme.typeCaption
                    }
                }
            }

            // ------------------------------------------------------------ tags
            // Definitions only: which files carry a tag is edited in the
            // listing, and the storage boundary (a stream on NTFS/APFS/ext4,
            // an index elsewhere) is stated once, here, rather than in every
            // row that writes a tag.
            Item {
                anchors.fill: parent
                visible: root.page === "tags"

                readonly property var tags: {
                    if (root.tagRevision >= 0) {
                        try {
                            return JSON.parse(TagStore.tagsJson)
                        } catch (error) {
                            return []
                        }
                    }
                    return []
                }

                Text {
                    x: 24; y: 20
                    text: qsTr("标签")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 52
                    width: Math.min(content.width - 48, 620)
                    text: qsTr("标签会写进文件本身（NTFS 数据流、xattr）；网络位置、压缩包以及不支持的文件系统改为保存在 dae 的索引里，不会随文件移动。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                    lineHeight: Theme.leadingBody
                }

                Row {
                    x: 24; y: 108
                    height: Theme.controlLg
                    spacing: 10

                    Rectangle {
                        width: 180
                        height: parent.height
                        radius: Theme.radiusSm
                        color: Theme.field
                        border.width: 1
                        border.color: draftName.activeFocus ? Theme.accent : Theme.line

                        TextInput {
                            id: draftName
                            anchors.fill: parent
                            anchors.leftMargin: 10
                            anchors.rightMargin: 10
                            verticalAlignment: TextInput.AlignVCenter
                            color: Theme.ink
                            font.pixelSize: Theme.typeBody
                            selectByMouse: true
                            activeFocusOnTab: true
                            Accessible.name: qsTr("新标签名称")
                            onAccepted: root.createTag(draftName.text,
                                Theme.tagPaletteLight[root.draftTagIndex])
                        }
                    }

                    Row {
                        height: parent.height
                        spacing: 3

                        Repeater {
                            model: [0, 1, 2, 3, 4, 5, 6, 7]

                            TagSwatch {
                                required property int modelData
                                anchors.verticalCenter: parent.verticalCenter
                                option: modelData
                                selected: root.draftTagIndex === modelData
                                onClicked: root.draftTagIndex = modelData
                            }
                        }
                    }

                    PrimaryButton {
                        label: qsTr("添加")
                        onClicked: root.createTag(draftName.text,
                            Theme.tagPaletteLight[root.draftTagIndex])
                    }
                }

                Text {
                    x: 24; y: 148
                    width: Math.min(content.width - 48, 620)
                    visible: root.tagError !== ""
                    text: root.tagError
                    color: Theme.danger
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.Wrap
                }

                /// The list is empty until the first tag exists; a bare blank
                /// column reads as a failed load rather than as a fresh store.
                Text {
                    x: 24; y: 152
                    width: Math.min(content.width - 48, 480)
                    visible: parent.tags.length === 0 && root.tagError === ""
                    text: qsTr("还没有标签。输入名称、挑一个颜色，然后点「添加」——之后就能在文件列表和检查器里给文件打标签。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                    lineHeight: Theme.leadingBody
                }

                ListView {
                    x: 24; y: 172
                    width: content.width - 48
                    height: content.height - 256
                    clip: true
                    model: parent.tags
                    spacing: 2

                    delegate: Rectangle {
                        id: tagRow
                        required property var modelData

                        width: ListView.view.width
                        height: Theme.controlLg
                        radius: Theme.radiusSm
                        color: "transparent"

                        Rectangle {
                            x: 0
                            width: 8
                            height: 8
                            radius: 4
                            anchors.verticalCenter: parent.verticalCenter
                            color: Theme.tagColorFor(tagRow.modelData.color)
                        }

                        Rectangle {
                            id: nameField
                            x: 20
                            width: 200
                            height: Theme.controlMd
                            anchors.verticalCenter: parent.verticalCenter
                            radius: Theme.radiusSm
                            color: Theme.field
                            border.width: nameInput.activeFocus ? 2 : 1
                            border.color: nameInput.activeFocus ? Theme.accent : Theme.outlineVariant

                            TextInput {
                                id: nameInput
                                Accessible.role: Accessible.EditableText
                                Accessible.name: qsTr("标签名称")
                                anchors.fill: parent
                                anchors.leftMargin: 8
                                anchors.rightMargin: 8
                                verticalAlignment: TextInput.AlignVCenter
                                color: Theme.ink
                                font.pixelSize: Theme.typeBody
                                selectByMouse: true
                                activeFocusOnTab: true
                                Component.onCompleted: text = tagRow.modelData.name
                            }
                        }

                        Row {
                            x: nameField.x + nameField.width + 8
                            height: parent.height
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 3

                            Repeater {
                                model: [0, 1, 2, 3, 4, 5, 6, 7]

                                TagSwatch {
                                    required property int modelData
                                    anchors.verticalCenter: parent.verticalCenter
                                    option: modelData
                                    selected: Theme.tagColorIndex(tagRow.modelData.color)
                                        === modelData
                                    onClicked: root.updateTag(tagRow.modelData.id,
                                        nameInput.text,
                                        Theme.tagPaletteLight[modelData])
                                }
                            }
                        }

                        Text {
                            x: nameField.x + nameField.width + 8 + 8 * 14 + 7 * 3 + 12
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("%1 项").arg(tagRow.modelData.count)
                            color: Theme.inkTertiary
                            font.pixelSize: Theme.typeCaption
                        }

                        Row {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 14

                            SmallButton {
                                label: qsTr("保存")
                                onClicked: root.updateTag(tagRow.modelData.id,
                                                          nameInput.text,
                                                          tagRow.modelData.color)
                            }

                            SmallButton {
                                label: qsTr("删除")
                                onClicked: root.removeTag(tagRow.modelData.id)
                            }
                        }
                    }
                }

                Text {
                    x: 24
                    y: content.height - 82
                    width: content.width - 48
                    height: 34
                    visible: root.tagTaskStatus !== ""
                    text: root.tagTaskStatus
                    color: root.tagTaskFailed ? Theme.danger : root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }

                Row {
                    x: 24
                    y: content.height - 42
                    spacing: 8

                    SecondaryButton {
                        label: qsTr("导出…")
                        onClicked: tagExportDialog.open()
                    }
                    SecondaryButton {
                        label: qsTr("导入…")
                        onClicked: tagImportDialog.open()
                    }
                    SecondaryButton {
                        label: qsTr("重建索引")
                        onClicked: root.rebuildTagIndex()
                    }
                }
            }

            // ------------------------------------------------ default manager
            Item {
                anchors.fill: parent
                visible: root.page === "defaultManager"

                Text {
                    x: 24; y: 20
                    text: qsTr("默认文件管理器")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 50
                    width: content.width - 48
                    text: qsTr("把 dae 注册为系统打开文件夹的应用。这个设置修改操作系统关联，不写入 settings.toml。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 100
                    height: Theme.controlXs
                    spacing: 7

                    Glyph {
                        visible: Workspace.defaultManagerBusy
                        anchors.verticalCenter: parent.verticalCenter
                        width: 12
                        height: 12
                        kind: "refresh"
                        tint: root.faintColor

                        RotationAnimator on rotation {
                            running: Workspace.defaultManagerBusy
                            loops: Animation.Infinite
                            from: 0
                            to: 360
                            duration: 900
                        }
                    }

                    Glyph {
                        visible: !Workspace.defaultManagerBusy && Workspace.defaultManagerIsDefault
                        anchors.verticalCenter: parent.verticalCenter
                        width: 12
                        height: 12
                        kind: "check"
                        tint: Theme.success
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.defaultManagerState
                        color: root.defaultManagerStateColor
                        font.pixelSize: Theme.typeCaption
                    }
                }

                Text {
                    x: 24; y: 126
                    width: content.width - 48
                    visible: text !== ""
                    text: root.localizeDetail(Workspace.defaultManagerDetail)
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 160
                    spacing: 8

                    PrimaryButton {
                        label: qsTr("设为默认")
                        enabled: root.defaultManagerCanSet
                        onClicked: Workspace.setDefaultFileManager()
                    }
                    SecondaryButton {
                        label: qsTr("取消默认")
                        enabled: root.defaultManagerCanUnset
                        onClicked: Workspace.unsetDefaultFileManager()
                    }
                }

                Text {
                    x: 24; y: 204
                    width: content.width - 48
                    visible: text !== ""
                    text: root.backendError(Workspace.defaultManagerError)
                    color: Theme.danger
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }
            }

            // ------------------------------------------------------------ logs
            Item {
                anchors.fill: parent
                visible: root.page === "logs"

                Text {
                    x: 24; y: 20
                    text: qsTr("日志")
                    color: root.textColor
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }
                Text {
                    x: 24; y: 52
                    width: Math.min(content.width - 48, 620)
                    text: qsTr("两个桌面壳把运行日志写入同一个目录，方便一起排查。日志可能包含文件路径，分享前请先检查内容。")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                    lineHeight: Theme.leadingBody
                }

                Text {
                    x: 24; y: 96
                    text: qsTr("日志目录")
                    color: root.textColor
                    font.pixelSize: Theme.typeRow
                    font.weight: Font.Medium
                }

                Rectangle {
                    x: 24; y: 120
                    width: Math.min(560, content.width - 48)
                    height: Theme.controlLg
                    radius: Theme.radiusSm
                    color: root.fieldColor
                    border.width: 1
                    border.color: Theme.outlineVariant

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        verticalAlignment: Text.AlignVCenter
                        text: Workspace.logDirectory
                        color: root.textColor
                        font.pixelSize: Theme.typeRow
                        font.family: Theme.monoFamily
                        elide: Text.ElideMiddle
                    }
                }

                Row {
                    x: 24; y: 166
                    spacing: 8

                    PrimaryButton {
                        label: qsTr("打开日志目录")
                        onClicked: Workspace.openLogDirectory()
                    }
                }

                Text {
                    x: 24; y: 202
                    width: content.width - 48
                    text: qsTr("日志页只负责定位文件，不会上传或汇总任何内容。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                    wrapMode: Text.WordWrap
                }
            }
        }

        // ------------------------------------------------------------- footer
        Rectangle {
            id: footer
            anchors.left: sidebar.right
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 52
            color: "transparent"

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: Theme.ruleWidth(Screen.devicePixelRatio)
                color: root.hairline
            }

            Text {
                x: 24
                anchors.verticalCenter: parent.verticalCenter
                text: root.page === "fonts"
                    ? qsTr("等宽字体立即生效，界面字体下次启动生效")
                    : qsTr("更改会立即生效")
                color: root.faintColor
                font.pixelSize: Theme.typeCaption
            }

            Rectangle {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 76
                height: Theme.controlLg
                radius: Theme.radiusSm
                color: doneMouse.containsMouse
                    ? Theme.accentHover : root.accent

                Text {
                    anchors.centerIn: parent
                    text: qsTr("完成")
                    color: Theme.inkOnAccent
                    font.pixelSize: Theme.typeCaption
                    font.weight: Theme.weightStrong
                }

                MouseArea {
                    id: doneMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.close()
                }
            }
        }

        // ------------------------------------------------------------ capture
        FocusScope {
            id: captureLayer
            anchors.fill: parent
            visible: root.captureId !== ""
            focus: visible
            z: 50

            Keys.onPressed: function (event) {
                root.handleCaptureKey(event)
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.AllButtons
            }

            Rectangle {
                anchors.fill: parent
                color: Theme.scrim
            }

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(420, parent.width - 64)
                height: 164
                radius: Theme.radiusXl
                color: "transparent"
                border.width: 0

                Glass { radius: Theme.radiusXl }

                Text {
                    x: 22; y: 20
                    width: parent.width - 44
                    text: qsTr("为“%1”录制快捷键").arg(root.captureLabel)
                    color: root.textColor
                    font.pixelSize: Theme.typeBody
                    font.weight: Font.Medium
                    elide: Text.ElideRight
                }

                Rectangle {
                    x: 22; y: 54
                    width: parent.width - 44
                    height: 46
                    radius: Theme.radiusSm
                    color: root.fieldColor
                    border.width: 2
                    border.color: root.accent

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("按下新的组合键…")
                        color: root.mutedColor
                        font.pixelSize: Theme.typeCaption
                    }
                }

                Text {
                    x: 22; y: 114
                    width: parent.width - 44
                    text: qsTr("Backspace 清除绑定 · Esc 取消")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                }
            }
        }
    }

    // ------------------------------------------------------------ components
    component NavItem: Rectangle {
        id: nav
        property string label: ""
        property string value: ""

        width: parent.width
        height: 36
        radius: Theme.radiusSm
        color: root.page === nav.value
            ? root.selectedFill
            : (navMouse.containsMouse ? root.hoverFill : "transparent")

        /// The settings pages are a tab list: the label is the name and the
        /// current page is the selection.
        Accessible.role: Accessible.PageTab
        Accessible.name: nav.label
        Accessible.selectable: true
        Accessible.selected: root.page === nav.value
        Accessible.onPressAction: {
            root.page = nav.value
            root.shortcutError = ""
            if (nav.value === "terminal")
                root.syncTerminalFields()
            else if (nav.value === "defaultManager")
                Workspace.refreshDefaultManager()
            else if (nav.value === "actions")
                root.refreshActions()
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 14
            anchors.verticalCenter: parent.verticalCenter
            text: nav.label
            color: root.page === nav.value ? Theme.selectionInk : root.mutedColor
            font.pixelSize: Theme.typeRow
            font.weight: root.page === nav.value
                ? Theme.weightEmphasis : Font.Normal
        }

        MouseArea {
            id: navMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: {
                root.page = nav.value
                root.shortcutError = ""
                if (nav.value === "terminal")
                    root.syncTerminalFields()
                else if (nav.value === "defaultManager")
                    Workspace.refreshDefaultManager()
                else if (nav.value === "actions")
                    root.refreshActions()
            }
        }
    }

    component ChoiceChip: Rectangle {
        id: chip
        property string label: ""
        property bool selected: false
        property color swatch: "transparent"
        signal clicked()

        // 24px for the swatch and 16px right padding, plus 2px of slack so the
        // last glyph never lands under the elide threshold.
        width: chipText.implicitWidth + (chip.swatch.a > 0 ? 45 : 32)
        height: Theme.controlLg
        radius: Theme.radiusSm
        color: chip.selected
            ? Theme.selection
            : (chipMouse.containsMouse ? root.hoverFill : "transparent")
        border.width: chip.selected ? 0 : 1
        border.color: Theme.outlineVariant

        /// A chip is one choice in a mutually exclusive set, which is a radio
        /// button however it is drawn.
        Accessible.role: Accessible.RadioButton
        Accessible.name: chip.label
        Accessible.checkable: true
        Accessible.checked: chip.selected
        Accessible.onPressAction: chip.clicked()

        Rectangle {
            visible: chip.swatch.a > 0
            x: 9
            anchors.verticalCenter: parent.verticalCenter
            width: 11
            height: 11
            radius: width / 2
            color: chip.swatch
            border.width: 1
            border.color: chip.selected ? Theme.selectionInk : Theme.outlineVariant
        }

        Text {
            id: chipText
            anchors.left: parent.left
            anchors.leftMargin: chip.swatch.a > 0 ? 28 : 16
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            text: chip.label
            color: chip.selected ? Theme.selectionInk : root.mutedColor
            font.pixelSize: Theme.typeCaption
            font.weight: chip.selected ? Theme.weightStrong : Font.Normal
            elide: Text.ElideRight
        }

        MouseArea {
            id: chipMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: chip.clicked()
        }
    }

    component PrimaryButton: Rectangle {
        id: button
        property string label: ""
        signal clicked()

        width: buttonText.implicitWidth + 32
        height: Theme.controlLg
        radius: Theme.radiusSm
        opacity: button.enabled ? 1 : 0.45
        color: button.enabled && buttonMouse.containsMouse
            ? Theme.accentHover : Theme.accent

        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.onPressAction: {
            if (button.enabled)
                button.clicked()
        }

        Text {
            id: buttonText
            anchors.centerIn: parent
            text: button.label
            color: Theme.inkOnAccent
            font.pixelSize: Theme.typeCaption
            font.weight: Theme.weightStrong
        }

        MouseArea {
            id: buttonMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: button.clicked()
        }
    }

    component SecondaryButton: Rectangle {
        id: button
        property string label: ""
        signal clicked()

        width: buttonText.implicitWidth + 32
        height: Theme.controlLg
        radius: Theme.radiusSm
        opacity: button.enabled ? 1 : 0.45
        color: button.enabled && buttonMouse.containsMouse
            ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
            : "transparent"
        border.width: 1
        border.color: Theme.outline

        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.onPressAction: {
            if (button.enabled)
                button.clicked()
        }

        Text {
            id: buttonText
            anchors.centerIn: parent
            text: button.label
            color: Theme.accent
            font.pixelSize: Theme.typeCaption
            font.weight: Theme.weightStrong
        }

        MouseArea {
            id: buttonMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: button.clicked()
        }
    }

    component SmallButton: Rectangle {
        id: button
        property string label: ""
        signal clicked()

        width: smallText.implicitWidth + 24
        height: Theme.controlMd
        radius: Theme.radiusSm
        color: smallMouse.containsMouse
            ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
            : "transparent"

        Text {
            id: smallText
            anchors.centerIn: parent
            text: button.label
            color: Theme.accent
            font.pixelSize: Theme.typeCaption
            font.weight: Theme.weightStrong
        }

        MouseArea {
            id: smallMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: button.clicked()
        }
    }

    component TagSwatch: Rectangle {
        id: swatch

        property int option: 0
        property bool selected: false

        signal clicked()

        width: 14
        height: 14
        radius: width / 2
        color: Theme.tagColor(option)
        border.width: selected ? 2 : 1
        border.color: selected ? Theme.ink : Theme.hairline

        Accessible.role: Accessible.RadioButton
        Accessible.name: qsTr("标签颜色 %1").arg(swatch.option + 1)
        Accessible.checkable: true
        Accessible.checked: swatch.selected
        Accessible.onPressAction: swatch.clicked()

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: swatch.clicked()
        }
    }

    component FieldFrame: Rectangle {
        radius: Theme.radiusSm
        color: root.fieldColor
        border.width: 1
        border.color: Theme.outlineVariant
    }

    /// The dialog hands back a URL; the preference stores a native path so the
    /// INI stays readable and `Image.source` can rebuild the URL.
    function localPath(url) {
        let text = String(url)
        if (text.startsWith("file:///"))
            text = text.substring(8)
        else if (text.startsWith("file://"))
            text = text.substring(7)
        return decodeURIComponent(text)
    }

    FileDialog {
        id: backgroundDialog
        title: qsTr("选择背景图片")
        nameFilters: [qsTr("图片") + " (*.png *.jpg *.jpeg *.webp *.bmp *.gif)"]
        onAccepted: ExplorerPrefs.setString("backgroundImage",
                                            root.localPath(selectedFile))
    }

    /// Tag definitions are the same `tags.toml` document the config directory
    /// holds, so both dialogs offer exactly one filter.
    FileDialog {
        id: tagImportDialog
        title: qsTr("导入标签定义")
        nameFilters: [qsTr("标签定义") + " (*.toml)"]
        onAccepted: root.importTags(root.localPath(selectedFile))
    }

    FileDialog {
        id: tagExportDialog
        title: qsTr("导出标签定义")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "toml"
        nameFilters: [qsTr("标签定义") + " (*.toml)"]
        onAccepted: root.exportTags(root.localPath(selectedFile))
    }
}
