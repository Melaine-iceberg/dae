pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Shortcut.js" as Shortcut

// The settings document.
//
// The panel intentionally has no global "save" step: appearance, language and
// shortcuts apply immediately, while the terminal typography page keeps its
// own save button because a half-typed font size should not resize the live
// terminal. Appearance, language, terminal typography and shortcuts live in
// the shared `settings.toml`. The default-file-manager page changes an OS
// association, so its buttons call the
// platform immediately and its state is re-read whenever the page opens.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    property bool isMac: false
    property var shortcutMap: ({})

    signal closed()

    visible: false
    z: 1001
    focus: visible

    property string page: "appearance"
    property string captureId: ""
    property string captureLabel: ""
    property string shortcutError: ""
    property string terminalError: ""
    property string terminalStatus: ""

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color faintColor: Theme.inkFaint
    readonly property color hairline: Theme.hairline
    readonly property color fieldColor: Theme.field
    readonly property color panelColor: Theme.raised
    readonly property color sidebarColor: Theme.chrome
    readonly property color hoverFill: Theme.hover
    readonly property color selectedFill: Theme.fill

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
            ? (root.dark ? "#d8b45a" : "#9a6b10")
            : (root.dark ? "#9aa2af" : "#6a727c")

    readonly property var themeOptions: [
        { value: "system", label: qsTr("跟随系统") },
        { value: "light", label: qsTr("浅色") },
        { value: "dark", label: qsTr("深色") }
    ]
    readonly property var accentOptions: [
        { value: "system", label: qsTr("系统强调色") },
        { value: "brand", label: qsTr("dae 蓝") }
    ]
    readonly property var languageOptions: [
        { value: "system", label: qsTr("跟随系统") },
        { value: "zh-CN", label: qsTr("简体中文") },
        { value: "en", label: "English" }
    ]

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
                  detail: qsTr("打开这个设置面板") }
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
                  detail: qsTr("在分屏的两个窗格之间移动焦点") }
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
        Workspace.refreshDefaultManager()
        root.forceActiveFocus()
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
    }

    function bindingText(binding) {
        return binding ? Shortcut.display(binding, root.isMac) : qsTr("未绑定")
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
                        && Shortcut.bindingsEqual(binding, root.bindingFor(action.id)))
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
        const binding = Shortcut.record(event, root.isMac)
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
        root.terminalError = ""
        root.terminalStatus = qsTr("已保存")
        terminalStatusTimer.restart()
    }

    // Escape is owned by ShellWindow's single modal handler; while a key is
    // being recorded that handler is disabled and the capture layer cancels
    // instead of closing the whole panel.

    Timer {
        id: terminalStatusTimer
        interval: 2200
        onTriggered: root.terminalStatus = ""
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: {
                if (root.captureId !== "")
                    root.cancelCapture()
                else
                    root.close()
            }
        }
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: Math.min(820, parent.width - 64)
        height: Math.min(580, parent.height - 64)
        radius: 16
        color: root.panelColor
        border.width: 1
        border.color: Theme.hairlineStrong
        clip: true

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
        }

        // ---------------------------------------------------------------- nav
        Rectangle {
            id: sidebar
            width: 176
            height: parent.height
            color: root.sidebarColor
            radius: 16

            // Square off the inner edge so the sidebar reads as a column, not
            // as a floating card.
            Rectangle {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 10
                color: parent.color
            }

            Rectangle {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 1
                color: root.hairline
            }

            Text {
                x: 18
                y: 18
                text: qsTr("设置")
                color: root.textColor
                font.pixelSize: 15
                font.weight: Font.DemiBold
            }

            Text {
                x: 18
                y: 38
                text: qsTr("dae · Qt 桌面壳")
                color: root.faintColor
                font.pixelSize: 11
            }

            Column {
                x: 8
                y: 66
                width: parent.width - 16
                spacing: 2

                NavItem { label: qsTr("外观与语言"); value: "appearance" }
                NavItem { label: qsTr("快捷键"); value: "shortcuts" }
                NavItem { label: qsTr("终端排版"); value: "terminal" }
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
            Item {
                anchors.fill: parent
                visible: root.page === "appearance"

                Text {
                    x: 24; y: 22
                    text: qsTr("外观与语言")
                    color: root.textColor
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 48
                    text: qsTr("主题、强调色和界面语言对本 shell 立即生效。")
                    color: root.mutedColor
                    font.pixelSize: 11
                }

                Text {
                    x: 24; y: 88
                    text: qsTr("主题")
                    color: root.textColor
                    font.pixelSize: 12
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 108
                    width: content.width - 48
                    text: qsTr("跟随系统时，系统切换深浅色会同步更新窗口、标题栏与面板。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 132
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

                Text {
                    x: 24; y: 192
                    text: qsTr("强调色")
                    color: root.textColor
                    font.pixelSize: 12
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 212
                    width: content.width - 48
                    text: qsTr("“系统”读取桌面强调色，“dae 蓝”使用应用自带的品牌色。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 236
                    spacing: 8

                    Repeater {
                        model: root.accentOptions
                        delegate: ChoiceChip {
                            required property var modelData
                            label: modelData.label
                            selected: Appearance.accent === modelData.value
                            swatch: modelData.value === "brand"
                                ? Theme.brand
                                : Workspace.accent
                            onClicked: Appearance.chooseAccent(modelData.value)
                        }
                    }
                }

                Text {
                    x: 24; y: 296
                    text: qsTr("界面语言")
                    color: root.textColor
                    font.pixelSize: 12
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 316
                    width: content.width - 48
                    text: qsTr("切换后整个窗口立即重新翻译。菜单、对话框和本面板共用同一份词条。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 340
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

                Rectangle {
                    x: 24; y: 400
                    width: content.width - 48
                    height: 1
                    color: root.hairline
                }

                Text {
                    x: 24; y: 420
                    width: content.width - 48
                    text: qsTr("外观、语言、快捷键与终端排版统一保存在 settings.toml，并与 Tauri 壳共享。")
                    color: root.faintColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }
            }

            // ------------------------------------------------------- shortcuts
            Item {
                anchors.fill: parent
                visible: root.page === "shortcuts"

                Text {
                    x: 24; y: 22
                    text: qsTr("快捷键")
                    color: root.textColor
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 48
                    width: content.width - 160
                    text: qsTr("点击右侧按键重新录制。按 Backspace 清除绑定，按 Esc 取消；重复的绑定不会被保存。")
                    color: root.mutedColor
                    font.pixelSize: 11
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
                    color: root.dark ? "#f0a15a" : "#a85a0a"
                    font.pixelSize: 11
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
                                required property var modelData
                                width: shortcutColumn.width
                                height: modelData.header ? 30 : 38

                                Text {
                                    visible: modelData.header
                                    anchors.left: parent.left
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: 8
                                    text: modelData.header ? modelData.title : ""
                                    color: root.faintColor
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }

                                Rectangle {
                                    visible: !modelData.header
                                    anchors.fill: parent
                                    anchors.margins: 0
                                    radius: 6
                                    color: rowMouse.containsMouse ? root.hoverFill : "transparent"

                                    Text {
                                        x: 4
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: modelData.header ? "" : modelData.label
                                        color: root.textColor
                                        font.pixelSize: 12
                                    }
                                    Text {
                                        x: Math.min(190, parent.width * 0.35)
                                        width: parent.width - x - 170
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: modelData.header ? "" : modelData.detail
                                        color: root.faintColor
                                        font.pixelSize: 11
                                        elide: Text.ElideRight
                                    }

                                    Rectangle {
                                        id: bindingKey
                                        anchors.right: resetKey.left
                                        anchors.rightMargin: 6
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 100
                                        height: 24
                                        radius: 6
                                        color: keyMouse.containsMouse ? root.selectedFill : root.fieldColor
                                        border.width: 1
                                        border.color: keyMouse.containsMouse ? root.accent : root.hairline

                                        Text {
                                            anchors.centerIn: parent
                                            text: modelData.header ? ""
                                                : root.bindingText(root.bindingFor(modelData.id))
                                            color: root.bindingFor(modelData.id)
                                                ? root.textColor : root.faintColor
                                            font.pixelSize: 11
                                            font.family: Theme.monoFamily
                                            elide: Text.ElideRight
                                        }

                                        MouseArea {
                                            id: keyMouse
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            onClicked: root.beginCapture(modelData.id, modelData.label)
                                        }
                                    }

                                    Rectangle {
                                        id: resetKey
                                        anchors.right: parent.right
                                        anchors.rightMargin: 4
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 28
                                        height: 24
                                        radius: 6
                                        visible: !modelData.header
                                        color: resetMouse.containsMouse ? root.hoverFill : "transparent"

                                        Text {
                                            anchors.centerIn: parent
                                            text: "↺"
                                            color: resetMouse.containsMouse ? root.textColor : root.faintColor
                                            font.pixelSize: 12
                                        }

                                        MouseArea {
                                            id: resetMouse
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            onClicked: {
                                                root.shortcutError = ""
                                                Workspace.resetShortcut(modelData.id)
                                            }
                                        }
                                    }
                                }

                                MouseArea {
                                    id: rowMouse
                                    z: -1
                                    anchors.fill: parent
                                    enabled: !modelData.header
                                    hoverEnabled: true
                                    onClicked: {
                                        if (!modelData.header)
                                            root.beginCapture(modelData.id, modelData.label)
                                    }
                                }
                            }
                        }
                    }
                }

                Rectangle {
                    visible: shortcutFlick.contentHeight > shortcutFlick.height
                    x: shortcutFlick.x + shortcutFlick.width + 4
                    width: 3
                    radius: width / 2
                    height: Math.max(20, shortcutFlick.height
                        * shortcutFlick.height / shortcutFlick.contentHeight)
                    y: shortcutFlick.y + (shortcutFlick.contentHeight
                        - shortcutFlick.height > 0
                        ? shortcutFlick.contentY / (shortcutFlick.contentHeight
                            - shortcutFlick.height) * (shortcutFlick.height - height)
                        : 0)
                    color: Theme.hairlineStrong
                }
            }

            // -------------------------------------------------------- terminal
            Item {
                anchors.fill: parent
                visible: root.page === "terminal"

                Text {
                    x: 24; y: 22
                    text: qsTr("终端排版")
                    color: root.textColor
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 48
                    width: content.width - 48
                    text: qsTr("终端排版写入 settings.toml，两个桌面壳共用同一组值。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 92
                    text: qsTr("字号")
                    color: root.textColor
                    font.pixelSize: 12
                }
                FieldFrame {
                    id: sizeFrame
                    x: 130; y: 86
                    width: 90; height: 28
                }
                TextInput {
                    id: sizeInput
                    anchors.fill: sizeFrame
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    inputMethodHints: Qt.ImhDigitsOnly
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 12
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 234
                    anchors.verticalCenter: sizeFrame.verticalCenter
                    text: qsTr("8 – 24")
                    color: root.faintColor
                    font.pixelSize: 11
                }

                Text {
                    x: 24; y: 142
                    text: qsTr("行高")
                    color: root.textColor
                    font.pixelSize: 12
                }
                FieldFrame {
                    id: heightFrame
                    x: 130; y: 136
                    width: 90; height: 28
                }
                TextInput {
                    id: heightInput
                    anchors.fill: heightFrame
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 12
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 234
                    anchors.verticalCenter: heightFrame.verticalCenter
                    text: qsTr("0.80 – 3.00")
                    color: root.faintColor
                    font.pixelSize: 11
                }

                Text {
                    x: 24; y: 192
                    text: qsTr("字体")
                    color: root.textColor
                    font.pixelSize: 12
                }
                FieldFrame {
                    id: familyFrame
                    x: 130; y: 186
                    width: content.width - 130 - 48
                    height: 28
                }
                TextInput {
                    id: familyInput
                    anchors.fill: familyFrame
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 12
                    selectByMouse: true
                    clip: true
                    onAccepted: root.saveTerminal()
                }
                Text {
                    x: 130; y: 222
                    text: qsTr("留空使用 Cascadia Mono / Consolas")
                    color: root.faintColor
                    font.pixelSize: 11
                }

                Text {
                    x: 24; y: 256
                    text: qsTr("字号与行高在保存后立即应用到已打开的终端。")
                    color: root.faintColor
                    font.pixelSize: 11
                }

                Text {
                    id: terminalErrorText
                    x: 24; y: 292
                    width: content.width - 48
                    visible: text !== ""
                    text: root.terminalError
                    color: Theme.danger
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24
                    y: 320
                    spacing: 10

                    PrimaryButton {
                        label: qsTr("保存终端排版")
                        onClicked: root.saveTerminal()
                    }

                    Text {
                        id: terminalStatusText
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.terminalStatus
                        color: Theme.success
                        font.pixelSize: 11
                    }
                }
            }

            // ------------------------------------------------ default manager
            Item {
                anchors.fill: parent
                visible: root.page === "defaultManager"

                Text {
                    x: 24; y: 22
                    text: qsTr("默认文件管理器")
                    color: root.textColor
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 48
                    width: content.width - 48
                    text: qsTr("把 dae 注册为系统打开文件夹的应用。这个设置修改操作系统关联，不写入 settings.toml。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Row {
                    x: 24; y: 100
                    height: 18
                    spacing: 7

                    Text {
                        visible: Workspace.defaultManagerBusy
                        anchors.verticalCenter: parent.verticalCenter
                        text: "↻"
                        color: root.faintColor
                        font.pixelSize: 12

                        RotationAnimator on rotation {
                            running: Workspace.defaultManagerBusy
                            loops: Animation.Infinite
                            from: 0
                            to: 360
                            duration: 900
                        }
                    }

                    Text {
                        visible: !Workspace.defaultManagerBusy && Workspace.defaultManagerIsDefault
                        anchors.verticalCenter: parent.verticalCenter
                        text: "✓"
                        color: Theme.success
                        font.pixelSize: 12
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.defaultManagerState
                        color: root.defaultManagerStateColor
                        font.pixelSize: 12
                    }
                }

                Text {
                    x: 24; y: 126
                    width: content.width - 48
                    visible: text !== ""
                    text: root.localizeDetail(Workspace.defaultManagerDetail)
                    color: root.mutedColor
                    font.pixelSize: 11
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
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }
            }

            // ------------------------------------------------------------ logs
            Item {
                anchors.fill: parent
                visible: root.page === "logs"

                Text {
                    x: 24; y: 22
                    text: qsTr("日志")
                    color: root.textColor
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                Text {
                    x: 24; y: 48
                    width: content.width - 48
                    text: qsTr("两个桌面壳把运行日志写入同一个目录，方便一起排查。日志可能包含文件路径，分享前请先检查内容。")
                    color: root.mutedColor
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                }

                Text {
                    x: 24; y: 92
                    text: qsTr("日志目录")
                    color: root.textColor
                    font.pixelSize: 12
                    font.weight: Font.DemiBold
                }

                Rectangle {
                    x: 24; y: 114
                    width: content.width - 48
                    height: 34
                    radius: 6
                    color: root.fieldColor
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        verticalAlignment: Text.AlignVCenter
                        text: Workspace.logDirectory
                        color: root.textColor
                        font.pixelSize: 11
                        font.family: Theme.monoFamily
                        elide: Text.ElideMiddle
                    }
                }

                Row {
                    x: 24; y: 162
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
                    font.pixelSize: 11
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
                height: 1
                color: root.hairline
            }

            Text {
                x: 24
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("更改会立即生效")
                color: root.faintColor
                font.pixelSize: 11
            }

            Rectangle {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 78
                height: 28
                radius: 6
                color: doneMouse.containsMouse
                    ? Qt.lighter(root.accent, 1.08) : root.accent

                Text {
                    anchors.centerIn: parent
                    text: qsTr("完成")
                    color: Theme.onInteractive
                    font.pixelSize: 12
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
                color: "#99000000"
            }

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(420, parent.width - 64)
                height: 164
                radius: 12
                color: root.panelColor
                border.width: 1
                border.color: root.hairline

                Text {
                    x: 22; y: 20
                    width: parent.width - 44
                    text: qsTr("为“%1”录制快捷键").arg(root.captureLabel)
                    color: root.textColor
                    font.pixelSize: 13
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }

                Rectangle {
                    x: 22; y: 54
                    width: parent.width - 44
                    height: 46
                    radius: 6
                    color: root.fieldColor
                    border.width: 1
                    border.color: root.accent

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("按下新的组合键…")
                        color: root.mutedColor
                        font.pixelSize: 12
                    }
                }

                Text {
                    x: 22; y: 114
                    width: parent.width - 44
                    text: qsTr("Backspace 清除绑定 · Esc 取消")
                    color: root.faintColor
                    font.pixelSize: 11
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
        height: 32
        radius: 6
        color: root.page === nav.value
            ? root.selectedFill
            : (navMouse.containsMouse ? root.hoverFill : "transparent")

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: nav.label
            color: root.page === nav.value ? root.textColor : root.mutedColor
            font.pixelSize: 12
            font.weight: root.page === nav.value ? Font.DemiBold : Font.Normal
        }

        // A thin accent marker makes the active page readable at a glance
        // without tinting the whole row.
        Rectangle {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: 2
            height: 16
            radius: 4
            color: root.accent
            visible: root.page === nav.value
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
            }
        }
    }

    component ChoiceChip: Rectangle {
        id: chip
        property string label: ""
        property bool selected: false
        property color swatch: "transparent"
        signal clicked()

        // 24px for the swatch and 12px right padding, plus 2px of slack so the
        // last glyph never lands under the elide threshold.
        width: chipText.implicitWidth + (chip.swatch.a > 0 ? 38 : 24)
        height: 28
        radius: 6
        color: chip.selected
            ? (root.dark ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.22)
                         : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.12))
            : (chipMouse.containsMouse ? root.hoverFill : "transparent")
        border.width: 1
        border.color: chip.selected ? root.accent : root.hairline

        Rectangle {
            visible: chip.swatch.a > 0
            x: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 10
            height: 10
            radius: 6
            color: chip.swatch
            border.width: 1
            border.color: root.hairline
        }

        Text {
            id: chipText
            anchors.left: parent.left
            anchors.leftMargin: chip.swatch.a > 0 ? 24 : 12
            anchors.right: parent.right
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            text: chip.label
            color: chip.selected ? root.textColor : root.mutedColor
            font.pixelSize: 11
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

        width: buttonText.implicitWidth + 26
        height: 28
        radius: 6
        opacity: button.enabled ? 1 : 0.45
        color: button.enabled && buttonMouse.containsMouse
            ? Qt.lighter(root.accent, 1.08) : root.accent

        Text {
            id: buttonText
            anchors.centerIn: parent
            text: button.label
            color: Theme.onInteractive
            font.pixelSize: 11
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

        width: buttonText.implicitWidth + 26
        height: 28
        radius: 6
        opacity: button.enabled ? 1 : 0.45
        color: button.enabled && buttonMouse.containsMouse ? root.hoverFill : "transparent"
        border.width: 1
        border.color: root.hairline

        Text {
            id: buttonText
            anchors.centerIn: parent
            text: button.label
            color: root.mutedColor
            font.pixelSize: 11
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

        width: smallText.implicitWidth + 20
        height: 24
        radius: 6
        color: smallMouse.containsMouse ? root.hoverFill : "transparent"
        border.width: 1
        border.color: root.hairline

        Text {
            id: smallText
            anchors.centerIn: parent
            text: button.label
            color: root.mutedColor
            font.pixelSize: 11
        }

        MouseArea {
            id: smallMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: button.clicked()
        }
    }

    component FieldFrame: Rectangle {
        radius: 6
        color: root.fieldColor
        border.width: 1
        border.color: root.hairline
    }
}
