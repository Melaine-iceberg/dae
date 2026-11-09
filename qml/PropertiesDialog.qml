pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "DialogFocus.js" as DialogFocus
import "Localize.js" as Localize

// One entry's metadata: identity and timestamps, the platform's permission
// editor, and an on-demand checksum pass.
//
// The state lives in `PropertiesController` (crates/dae-qt/src/properties.rs);
// this file draws it and owns only the draft — what the user has typed but not
// yet written back. Opening a second entry re-syncs the draft from the freshly
// loaded snapshot, so a half-finished edit never leaks across entries.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    /// `general`, `permissions` (unix), `attributes` (windows) or `checksum`.
    property string tab: "general"

    // The editable draft. Integers and booleans rather than the controller's
    // properties, because a binding would write back the moment the user
    // toggles something.
    property int draftMode: 0
    property string draftUser: ""
    property string draftGroup: ""
    property bool draftReadOnly: false
    property bool draftHidden: false
    property bool draftArchive: false
    property bool draftSystem: false
    property bool recursive: false
    /// Which checksum row was copied last, for the one-line confirmation.
    property string copiedAlgorithm: ""
    /// A checksum pasted to compare against. The dialog hashes the file and
    /// answers here rather than shelling out to another tool.
    property string expectedHash: ""
    readonly property string expectedMatch: {
        const known = [controller.hashMd5, controller.hashSha1, controller.hashSha256]
        const labels = ["MD5", "SHA-1", "SHA-256"]
        const expected = root.expectedHash.trim().toLowerCase()
        if (expected === "" || !controller.hashDone)
            return ""
        for (let index = 0; index < known.length; index += 1) {
            if (known[index] !== "" && known[index] === expected)
                return labels[index]
        }
        return ""
    }
    readonly property bool expectedMatches: root.expectedMatch !== ""

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color fieldColor: Theme.field
    readonly property color hairline: Theme.hairline
    readonly property color dangerColor: Theme.danger
    readonly property bool canSave: controller.editable && !controller.saving
                                     && controller.error === ""

    signal closed()
    /// The write went through. The window reloads the listing so an attribute
    /// change (a hidden file, a new mode) is reflected in the row.
    signal changed()

    visible: false
    z: 1005
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("属性")

    function open(path) {
        root.tab = "general"
        root.recursive = false
        root.copiedAlgorithm = ""
        root.expectedHash = ""
        controller.open(path)
        root.visible = true
        root.forceActiveFocus()
        // Opening from a click leaves the release event still to be
        // delivered to whatever is underneath; asking once more on the next
        // turn is what actually lands the focus here, and Escape only works
        // when it does.
        Qt.callLater(function () {
            if (root.visible)
                root.forceActiveFocus()
        })
    }

    /// The controller's stable token as the one sentence the tab shows.
    function signatureStatusText() {
        switch (controller.signatureStatus) {
        case "valid": return qsTr("有效")
        case "unsigned": return qsTr("未签名")
        case "untrusted": return qsTr("不受信任")
        case "invalid": return qsTr("无效")
        }
        return controller.signatureStatus
    }

    function close() {
        controller.close()
        root.visible = false
        root.closed()
    }

    function save() {
        if (!root.canSave)
            return
        controller.save(root.draftMode, root.draftUser, root.draftGroup,
                        root.draftReadOnly, root.draftHidden, root.draftArchive,
                        root.draftSystem, root.recursive)
    }

    function copyValue(text) {
        if (text !== "" && Workspace.copyText(text))
            root.copiedAlgorithm = text
        copyTimer.restart()
    }

    Timer {
        id: copyTimer
        interval: 1600
        onTriggered: root.copiedAlgorithm = ""
    }

    PropertiesController {
        id: controller
    }

    Connections {
        target: controller

        // `loading` is cleared once the snapshot is complete; that is the one
        // moment the draft can be taken from it.
        function onLoadingChanged() {
            if (controller.loading)
                return
            root.draftMode = controller.mode
            root.draftUser = controller.userName
            root.draftGroup = controller.groupName
            root.draftReadOnly = controller.readOnly
            root.draftHidden = controller.hidden
            root.draftArchive = controller.archive
            root.draftSystem = controller.system
            userField.reset(controller.userName)
            groupField.reset(controller.groupName)
        }

        function onSaved() {
            root.changed()
            root.close()
        }
    }

    Keys.onEscapePressed: root.close()
    Keys.onTabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, true)
    }
    Keys.onBacktabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, false)
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.close()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(600, root.width - 40)
        height: Math.min(620, root.height - 48)
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: layout
            anchors.fill: parent
            anchors.margins: 18
            spacing: 0

            DialogHeader {
                width: parent.width
                title: qsTr("属性")
                subtitle: controller.loading ? qsTr("正在读取…") : controller.name
                onCloseRequested: root.close()
            }

            Item { width: 1; height: 12 }

            // Tabs. The set depends on what the entry can actually do: a file
            // has no enclosed items, a remote path has no attributes to edit,
            // and a folder has no checksum of its own.
            Row {
                id: tabBar
                width: parent.width
                height: Theme.controlMd
                spacing: 18

                Repeater {
                    model: {
                        const tabs = [{ key: "general", label: qsTr("常规") }]
                        if (controller.platformKind === "unix" && controller.editable)
                            tabs.push({ key: "permissions", label: qsTr("权限") })
                        if (controller.platformKind === "windows" && controller.editable)
                            tabs.push({ key: "attributes", label: qsTr("属性") })
                        if (controller.signatureSupported && !controller.remote
                                && !controller.isDirectory)
                            tabs.push({ key: "signature", label: qsTr("签名") })
                        if (!controller.isDirectory && !controller.remote)
                            tabs.push({ key: "checksum", label: qsTr("校验和") })
                        return tabs
                    }

                    delegate: Item {
                        id: tabItem
                        required property var modelData
                        width: tabLabel.width
                        height: tabBar.height

                        Text {
                            id: tabLabel
                            anchors.centerIn: parent
                            text: tabItem.modelData.label
                            color: root.tab === tabItem.modelData.key
                                ? root.textColor : root.mutedColor
                            font.pixelSize: Theme.typeCaption
                            font.weight: root.tab === tabItem.modelData.key
                                ? Font.Medium : Font.Normal
                        }

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 2
                            radius: Theme.radiusSm
                            // The current tab is a position, not a control: it
                            // keeps the same signal track as the window's tabs
                            // instead of following the system accent.
                            color: Theme.signal
                            visible: root.tab === tabItem.modelData.key
                        }

                        /// The tabs are a page list for a reader too; until
                        /// now they were pointer-only.
                        Accessible.role: Accessible.PageTab
                        Accessible.name: tabItem.modelData.label
                        Accessible.selectable: true
                        Accessible.selected: root.tab === tabItem.modelData.key
                        Accessible.onPressAction: root.tab = tabItem.modelData.key

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.tab = tabItem.modelData.key
                        }
                    }
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: root.hairline
            }

            Item { width: 1; height: 12 }

            Item {
                id: content
                width: parent.width
                // Whatever the rows above and the footer leave over. Reading
                // `y` back is safe: the Column owns it, this only measures it.
                height: Math.max(120, parent.height - y - 32 - 6 - 16)
                clip: true

                // ---- General -------------------------------------------------
                Column {
                    width: parent.width
                    visible: root.tab === "general"
                    spacing: 0

                    InfoRow { label: qsTr("名称"); value: controller.name }
                    InfoRow { label: qsTr("类型"); value: Localize.message(controller.kindText) }
                    InfoRow { label: qsTr("位置"); value: controller.location }
                    InfoRow {
                        label: qsTr("大小")
                        value: controller.isDirectory
                            ? (controller.sizeScanText === "" ? "—" : controller.sizeScanText)
                            : controller.sizeText
                    }

                    // A folder's size is a scan, not a number `stat` can give,
                    // so it is asked for explicitly and streamed while it runs.
                    Row {
                        width: parent.width
                        height: controller.isDirectory && !controller.remote ? 30 : 0
                        visible: height > 0
                        spacing: 8

                        Rectangle {
                            width: 96
                            height: Theme.controlSm
                            radius: Theme.radiusSm
                            color: scanMouse.containsMouse
                                ? Theme.fill : "transparent"
                            border.width: 1
                            border.color: root.hairline

                            Text {
                                anchors.centerIn: parent
                                text: controller.sizeScanning ? qsTr("取消计算") : qsTr("计算大小")
                                color: root.textColor
                                font.pixelSize: Theme.typeMicro
                            }

                            MouseArea {
                                id: scanMouse
                                Accessible.role: Accessible.Button
                                Accessible.name: controller.sizeScanning ? qsTr("取消计算") : qsTr("计算大小")
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: controller.sizeScanning
                                    ? controller.cancelSizeScan()
                                    : controller.startSizeScan()
                            }
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: controller.sizeScanning
                                ? qsTr("计算中… ") + controller.sizeScanText
                                : (controller.sizeScanDone ? qsTr("已统计 ") + controller.sizeScanText : "")
                            color: root.mutedColor
                            font.pixelSize: Theme.typeMicro
                        }
                    }

                    Item { width: 1; height: controller.isDirectory && !controller.remote ? 4 : 0 }

                    InfoRow { label: qsTr("创建时间"); value: controller.createdText }
                    InfoRow { label: qsTr("修改时间"); value: controller.modifiedText }
                    InfoRow { label: qsTr("访问时间"); value: controller.accessedText }
                    InfoRow {
                        label: qsTr("链接目标")
                        value: controller.targetText
                        visible: controller.isSymlink && controller.targetText !== ""
                    }
                }

                // ---- POSIX permissions ---------------------------------------
                Column {
                    width: parent.width
                    visible: root.tab === "permissions"
                    spacing: 0

                    Row {
                        width: parent.width
                        height: Theme.controlSm

                        Item { width: 70; height: 1 }
                        Text { width: 90; text: qsTr("用户"); color: root.mutedColor; font.pixelSize: Theme.typeMicro }
                        Text { width: 90; text: qsTr("组"); color: root.mutedColor; font.pixelSize: Theme.typeMicro }
                        Text { width: 90; text: qsTr("其他"); color: root.mutedColor; font.pixelSize: Theme.typeMicro }
                    }

                    PermissionRow {
                        label: qsTr("读取")
                        bit: 0o400
                    }
                    PermissionRow {
                        label: qsTr("写入")
                        bit: 0o200
                    }
                    PermissionRow {
                        label: qsTr("执行")
                        bit: 0o100
                    }

                    Item { width: 1; height: 6 }

                    Row {
                        width: parent.width
                        height: Theme.controlSm
                        spacing: 14

                        Toggle {
                            label: "setuid"
                            checked: (root.draftMode & 0o4000) !== 0
                            onToggled: root.draftMode = root.draftMode ^ 0o4000
                        }
                        Toggle {
                            label: "setgid"
                            checked: (root.draftMode & 0o2000) !== 0
                            onToggled: root.draftMode = root.draftMode ^ 0o2000
                        }
                        Toggle {
                            label: "sticky"
                            checked: (root.draftMode & 0o1000) !== 0
                            onToggled: root.draftMode = root.draftMode ^ 0o1000
                        }
                    }

                    Item { width: 1; height: 10 }

                    Row {
                        width: parent.width
                        height: Theme.controlLg
                        spacing: 10

                        Text {
                            width: 70
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("所有者")
                            color: root.mutedColor
                            font.pixelSize: Theme.typeMicro
                        }

                        Field {
                            id: userField
                            width: 150
                            placeholder: qsTr("用户名或 uid")
                            onEdited: function (value) { root.draftUser = value }
                        }

                        Field {
                            id: groupField
                            width: 150
                            placeholder: qsTr("组名或 gid")
                            onEdited: function (value) { root.draftGroup = value }
                        }
                    }
                }

                // ---- DOS attributes ------------------------------------------
                Column {
                    width: parent.width
                    visible: root.tab === "attributes"
                    spacing: 8

                    Toggle {
                        label: qsTr("只读")
                        checked: root.draftReadOnly
                        onToggled: root.draftReadOnly = !root.draftReadOnly
                    }
                    Toggle {
                        label: qsTr("隐藏")
                        checked: root.draftHidden
                        onToggled: root.draftHidden = !root.draftHidden
                    }
                    Toggle {
                        label: qsTr("存档")
                        checked: root.draftArchive
                        onToggled: root.draftArchive = !root.draftArchive
                    }
                    Toggle {
                        label: qsTr("系统")
                        checked: root.draftSystem
                        onToggled: root.draftSystem = !root.draftSystem
                    }

                    // The ACL is a read-only view: inheritance and deny
                    // ordering are a different editor, and the DOS flags above
                    // stay the writable surface. Empty when the platform has no
                    // reader or the file has no DACL.
                    Rectangle {
                        width: parent.width
                        height: 1
                        color: root.hairline
                        visible: controller.aclSupported && controller.aclEntries !== ""
                    }
                    Text {
                        width: parent.width
                        visible: controller.aclSupported && controller.aclOwner !== ""
                        text: qsTr("所有者：") + controller.aclOwner
                            + (controller.aclGroup !== ""
                                ? qsTr("　组：") + controller.aclGroup : "")
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.WordWrap
                    }
                    Text {
                        width: parent.width
                        visible: controller.aclEntries !== ""
                        text: controller.aclEntries
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                        font.family: Theme.monoFamily
                        wrapMode: Text.WordWrap
                    }
                }

                // ---- Authenticode -------------------------------------------
                Column {
                    width: parent.width
                    visible: root.tab === "signature"
                    spacing: 8

                    InfoRow {
                        label: qsTr("状态")
                        value: root.signatureStatusText()
                    }
                    InfoRow {
                        label: qsTr("签名者")
                        value: controller.signatureSigner
                        visible: controller.signatureSigner !== ""
                    }
                    InfoRow {
                        label: qsTr("颁发者")
                        value: controller.signatureIssuer
                        visible: controller.signatureIssuer !== ""
                    }
                    Text {
                        width: parent.width
                        visible: controller.signatureDetail !== ""
                        text: Localize.message(controller.signatureDetail)
                        color: controller.signatureStatus === "valid"
                            ? root.mutedColor : root.dangerColor
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.WordWrap
                    }
                }

                // ---- Checksum -------------------------------------------------
                Column {
                    width: parent.width
                    visible: root.tab === "checksum"
                    spacing: 12

                    Text {
                        width: parent.width
                        text: controller.hashDone
                            ? qsTr("校验值已计算完成。")
                            : (controller.hashing ? qsTr("正在计算校验值…") : qsTr("一次读取文件，同时给出三种校验值。"))
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.WordWrap
                    }

                    ChecksumRow {
                        algorithm: "MD5"
                        value: controller.hashMd5
                    }
                    ChecksumRow {
                        algorithm: "SHA-1"
                        value: controller.hashSha1
                    }
                    ChecksumRow {
                        algorithm: "SHA-256"
                        value: controller.hashSha256
                    }

                    Rectangle {
                        width: parent.width
                        height: Theme.controlMd
                        radius: Theme.radiusSm
                        color: root.fieldColor
                        border.width: 1
                        border.color: Theme.hairline

                        TextInput {
                            id: expectedInput
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("期望的校验值")
                            anchors.fill: parent
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            verticalAlignment: TextInput.AlignVCenter
                            color: root.textColor
                            selectionColor: root.accent
                            selectedTextColor: Theme.onInteractive
                            font.pixelSize: Theme.typeCaption
                            font.family: Theme.monoFamily
                            selectByMouse: true
                            clip: true
                            text: root.expectedHash
                            onTextEdited: root.expectedHash = text
                        }

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            verticalAlignment: Text.AlignVCenter
                            visible: expectedInput.text === ""
                            text: qsTr("粘贴已知的 MD5 / SHA-1 / SHA-256 以比对")
                            color: root.mutedColor
                            font.pixelSize: Theme.typeCaption
                            elide: Text.ElideRight
                        }
                    }

                    Text {
                        width: parent.width
                        visible: root.expectedHash.trim() !== ""
                        text: {
                            if (root.expectedHash.trim() === "")
                                return ""
                            if (!controller.hashDone)
                                return qsTr("点击“开始校验”后比对")
                            if (root.expectedMatches)
                                return qsTr("与 %1 校验值相同").arg(root.expectedMatch)
                            return qsTr("与任何校验值都不匹配")
                        }
                        color: root.expectedMatches ? Theme.success
                            : (controller.hashDone ? root.dangerColor : root.mutedColor)
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.WordWrap
                    }

                    Rectangle {
                        width: parent.width
                        height: 3
                        // Capsule: half the height, not one of the four corner
                        // steps — a bar has no corners to round.
                        radius: height / 2
                        visible: controller.hashing
                        color: Theme.hairline

                        Rectangle {
                            height: parent.height
                            radius: parent.radius
                            width: parent.width * controller.hashProgress
                            color: root.accent
                        }
                    }

                    Text {
                        width: parent.width
                        visible: controller.hashError !== ""
                        text: controller.hashError
                        color: root.dangerColor
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.WordWrap
                    }

                    Row {
                        width: parent.width
                        height: Theme.controlMd
                        spacing: 8

                        Rectangle {
                            width: 96
                            height: Theme.controlMd
                            radius: Theme.radiusSm
                            color: hashMouse.containsMouse ? Theme.interactiveHover : Theme.interactive

                            Text {
                                anchors.centerIn: parent
                                text: controller.hashing ? qsTr("取消") : qsTr("开始校验")
                                color: Theme.onInteractive
                                font.pixelSize: Theme.typeCaption
                            }

                            MouseArea {
                                id: hashMouse
                                Accessible.role: Accessible.Button
                                Accessible.name: controller.hashing ? qsTr("取消校验") : qsTr("开始校验")
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: controller.hashing
                                    ? controller.cancelHashing()
                                    : controller.startHashing()
                            }
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: root.copiedAlgorithm !== "" ? qsTr("已复制") : ""
                            color: root.mutedColor
                            font.pixelSize: Theme.typeMicro
                        }
                    }
                }
            }

            // ---- Footer ------------------------------------------------------
            Row {
                width: parent.width
                height: Theme.controlLg
                spacing: 8

                Toggle {
                    width: 180
                    height: parent.height
                    label: qsTr("应用于所有子项目")
                    checked: root.recursive
                    visible: controller.isDirectory && controller.editable
                    enabled: visible
                    onToggled: root.recursive = !root.recursive
                }

                Item {
                    width: Math.max(0, parent.width - 180 - 76 - 84 - 16)
                    height: 1
                }

                Rectangle {
                    width: 76
                    height: Theme.controlMd
                    anchors.verticalCenter: parent.verticalCenter
                    radius: Theme.radiusSm
                    color: cancelMouse.containsMouse
                        ? Theme.fill : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        // A read-only entry has nothing to write, so the button
                        // that is always available is a plain close.
                        text: controller.editable ? qsTr("取消") : qsTr("关闭")
                        color: Theme.inkMuted
                        font.pixelSize: Theme.typeCaption
                    }

                    MouseArea {
                        id: cancelMouse
                        Accessible.role: Accessible.Button
                        Accessible.name: controller.editable ? qsTr("取消") : qsTr("关闭")
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.close()
                    }
                }

                Rectangle {
                    width: 84
                    height: Theme.controlMd
                    anchors.verticalCenter: parent.verticalCenter
                    visible: controller.editable
                    radius: Theme.radiusSm
                    color: !root.canSave ? Theme.canvasAlt
                        : (saveMouse.containsMouse ? Theme.interactiveHover : Theme.interactive)

                    Text {
                        anchors.centerIn: parent
                        text: controller.saving ? qsTr("保存中…") : qsTr("确定")
                        color: root.canSave ? Theme.onInteractive : root.mutedColor
                        font.pixelSize: Theme.typeCaption
                    }

                    MouseArea {
                        id: saveMouse
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("确定")
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: root.canSave
                        onClicked: root.save()
                    }
                }
            }

            Item { width: 1; height: 4 }

            Text {
                width: parent.width
                height: root.errorMessage !== "" ? 16 : 0
                visible: height > 0
                text: root.errorMessage
                color: root.dangerColor
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideRight
            }
        }
    }

    /// The first failure worth showing, whichever half produced it.
    readonly property string errorMessage: controller.error !== ""
        ? Localize.message(controller.error) : Localize.message(controller.saveError)

    // ---- Inline components ------------------------------------------------

    /// `标签   值`, the whole general tab's row.
    component InfoRow: Item {
        id: row

        property string label: ""
        property string value: ""

        width: parent ? parent.width : 0
        height: row.visible ? 26 : 0

        Text {
            x: 0
            width: 70
            anchors.verticalCenter: parent.verticalCenter
            text: row.label
            // Labels carry the field name only, so they sit back at `inkFaint`
            // while the value keeps full `ink`: the pair reads as one line,
            // scanned left to right, instead of as two competing columns.
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 70
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            text: row.value
            color: root.textColor
            font.pixelSize: Theme.typeBody
            elide: Text.ElideMiddle
        }
    }

    /// One permission bit for the user, the group and everyone else at once.
    /// `bit` is the user triplet's value; the other two columns shift it.
    component PermissionRow: Item {
        id: permission

        property string label: ""
        property int bit: 0

        width: parent ? parent.width : 0
        height: Theme.controlMd

        Text {
            width: 70
            anchors.verticalCenter: parent.verticalCenter
            text: permission.label
            color: root.mutedColor
            font.pixelSize: Theme.typeMicro
        }

        Toggle {
            x: 70
            anchors.verticalCenter: parent.verticalCenter
            width: 90
            showLabel: false
            checked: (root.draftMode & permission.bit) !== 0
            onToggled: root.draftMode = root.draftMode ^ permission.bit
        }
        Toggle {
            x: 160
            anchors.verticalCenter: parent.verticalCenter
            width: 90
            showLabel: false
            checked: (root.draftMode & (permission.bit >> 3)) !== 0
            onToggled: root.draftMode = root.draftMode ^ (permission.bit >> 3)
        }
        Toggle {
            x: 250
            anchors.verticalCenter: parent.verticalCenter
            width: 90
            showLabel: false
            checked: (root.draftMode & (permission.bit >> 6)) !== 0
            onToggled: root.draftMode = root.draftMode ^ (permission.bit >> 6)
        }
    }

    /// A checkbox with a label. `showLabel` off leaves just the box, which is
    /// how the permission matrix keeps its three columns aligned.
    component Toggle: Item {
        id: toggle

        property string label: ""
        property bool checked: false
        property bool showLabel: true

        signal toggled()

        width: box.width + (showLabel ? 8 + toggleLabel.width : 0)
        height: Theme.controlSm
        opacity: enabled ? 1 : 0.45

        Rectangle {
            id: box
            width: 15
            height: 15
            anchors.verticalCenter: parent.verticalCenter
            radius: Theme.radiusSm
            color: toggle.checked ? root.accent : "transparent"
            border.width: 1
            border.color: toggle.checked ? root.accent : Theme.scrim

            Text {
                anchors.centerIn: parent
                visible: toggle.checked
                text: "\u2713"
                color: Theme.onInteractive
                font.pixelSize: Theme.typeMicro
            }
        }

        Text {
            id: toggleLabel
            anchors.left: box.right
            anchors.leftMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            visible: toggle.showLabel
            text: toggle.label
            color: root.textColor
            font.pixelSize: Theme.typeCaption
        }

        MouseArea {
            anchors.fill: parent
            enabled: toggle.enabled
            cursorShape: Qt.PointingHandCursor
            onClicked: toggle.toggled()
        }
    }

    /// A one-line text field for the owner and group names.
    component Field: Rectangle {
        id: field

        property string text: ""
        property string placeholder: ""

        signal edited(string value)

        /// Replaces the live text. Typing breaks a `text:` binding, so a
        /// reload after loading another entry goes through this instead.
        function reset(value) {
            fieldInput.text = value
        }

        width: 150
        height: Theme.controlMd
        radius: Theme.radiusSm
        color: root.fieldColor
        border.width: 1
        border.color: fieldInput.activeFocus
            ? Theme.focusRing
            : root.hairline

        TextInput {
            id: fieldInput
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            color: root.textColor
            selectionColor: root.accent
            selectedTextColor: Theme.onInteractive
            font.pixelSize: Theme.typeCaption
            selectByMouse: true
            clip: true

            Accessible.role: Accessible.EditableText
            Accessible.name: field.placeholder

            onTextEdited: field.edited(text)
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 9
            anchors.verticalCenter: parent.verticalCenter
            visible: fieldInput.text.length === 0
            text: field.placeholder
            color: root.mutedColor
            font.pixelSize: Theme.typeCaption
        }
    }

    /// One algorithm's digest, with the copy affordance next to it.
    component ChecksumRow: Item {
        id: checksum

        property string algorithm: ""
        property string value: ""

        width: parent ? parent.width : 0
        height: Theme.rowSpacious

        Text {
            x: 0
            width: 64
            anchors.verticalCenter: parent.verticalCenter
            text: checksum.algorithm
            color: root.mutedColor
            font.pixelSize: Theme.typeMicro
        }

        Rectangle {
            anchors.left: parent.left
            anchors.leftMargin: 64
            anchors.right: copyButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            height: Theme.controlMd
            radius: Theme.radiusSm
            color: root.fieldColor
            border.width: 1
            border.color: root.hairline

            Text {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                verticalAlignment: Text.AlignVCenter
                text: checksum.value === "" ? "—" : checksum.value
                color: root.textColor
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideMiddle
            }
        }

        Rectangle {
            id: copyButton
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            width: 56
            height: Theme.controlMd
            radius: Theme.radiusSm
            color: copyMouse.containsMouse
                ? Theme.fill : "transparent"
            border.width: 1
            border.color: root.hairline
            visible: checksum.value !== ""

            Text {
                anchors.centerIn: parent
                text: qsTr("复制")
                color: root.textColor
                font.pixelSize: Theme.typeMicro
            }

            MouseArea {
                id: copyMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.copyValue(checksum.value)
            }
        }
    }
}
