pragma ComponentBehavior: Bound
import QtQuick
import "DialogFocus.js" as DialogFocus

// The add/edit dialog for SMB and SFTP servers.
//
// Validation, testing and storage all live on the bridge; this file only
// collects the fields, shows the outcome, and emits intent, so the same dialog
// would work if a server is later edited instead of created.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    property string protocol: "smb"
    property bool remember: true
    property bool testing: false
    property bool saving: false
    property bool testOk: false
    property string testMessage: ""
    property string errorText: ""

    signal testRequested(string protocol, string host, string port, string username, string password)
    signal saveRequested(string protocol, string host, string port, string username,
                         string password, bool remember)
    signal canceled()

    visible: false
    z: 1001
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("连接网络存储")

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color fieldColor: Theme.field
    readonly property color hairline: Theme.hairline
    readonly property color dangerColor: Theme.danger
    readonly property color okColor: Theme.success

    function defaultPort() {
        return root.protocol === "sftp" ? "22" : "445"
    }

    function open() {
        root.protocol = "smb"
        hostField.value = ""
        portField.value = ""
        userField.value = ""
        passField.value = ""
        root.remember = true
        root.testing = false
        root.saving = false
        root.testOk = false
        root.testMessage = ""
        root.errorText = ""
        root.visible = true
        Qt.callLater(function () {
            if (root.visible)
                hostField.inputItem.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
    }

    function dismiss() {
        if (root.saving)
            return
        root.visible = false
        root.canceled()
    }

    function setProtocol(value) {
        if (root.protocol === value)
            return
        root.protocol = value
        root.clearStatus()
    }

    function clearStatus() {
        root.testOk = false
        root.testMessage = ""
        root.errorText = ""
    }

    function setTesting(value) {
        root.testing = value
        if (value) {
            root.testOk = false
            root.testMessage = ""
            root.errorText = ""
        }
    }

    function applyTestResult(ok, message) {
        root.testing = false
        root.testOk = ok
        root.testMessage = message
        if (ok)
            root.errorText = ""
    }

    function setSaving(value) {
        root.saving = value
        if (value)
            root.errorText = ""
    }

    function applySaveError(message) {
        root.saving = false
        root.errorText = message
    }

    /// Returns the trimmed port text, or sets `errorText` and returns null.
    function validatedPort() {
        const raw = portField.value.trim()
        if (raw === "")
            return ""
        const port = parseInt(raw, 10)
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
            root.errorText = qsTr("端口必须是 1-65535")
            return null
        }
        return raw
    }

    function requestTest() {
        if (root.testing || root.saving)
            return
        root.clearStatus()
        const host = hostField.value.trim()
        if (host === "") {
            root.errorText = qsTr("请输入服务器地址")
            return
        }
        const port = root.validatedPort()
        if (port === null)
            return
        root.setTesting(true)
        root.testRequested(root.protocol, host, port, userField.value.trim(), passField.value)
    }

    function submit() {
        if (root.testing || root.saving)
            return
        root.clearStatus()
        const host = hostField.value.trim()
        if (host === "") {
            root.errorText = qsTr("请输入服务器地址")
            return
        }
        const port = root.validatedPort()
        if (port === null)
            return
        root.setSaving(true)
        root.saveRequested(root.protocol, host, port, userField.value.trim(),
                           passField.value, root.remember)
    }

    Keys.onEscapePressed: root.dismiss()
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
            onClicked: root.dismiss()
        }
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(470, root.width - 48)
        height: 380
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        // The dimmed backdrop must not scroll the list behind the dialog, and
        // clicks inside the card must never reach that backdrop.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        // The card positions its fields absolutely, so the head is the one
        // element that has to agree with every other dialog: 44 tall, title
        // and its one-line note stacked, dismiss on the right.
        DialogHeader {
            width: parent.width
            inset: 20
            title: qsTr("连接网络存储")
            subtitle: qsTr("连接 SMB 服务器（NAS、Windows 共享）或 SFTP 服务器（Linux、NAS）。")
            onCloseRequested: root.dismiss()
        }

        Text {
            x: 20
            y: 72
            text: qsTr("协议")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
        }

        Row {
            x: 20
            y: 92
            spacing: 8

            ProtocolButton {
                protocol: "smb"
                label: qsTr("SMB / Windows 共享")
            }

            ProtocolButton {
                protocol: "sftp"
                label: "SFTP / SSH"
            }

            Item {
                width: note.implicitWidth
                height: 30

                Text {
                    id: note
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("FTP / WebDAV 即将支持")
                    color: root.mutedColor
                    font.pixelSize: 11
                }
            }
        }

        Field {
            id: hostField
            x: 20
            y: 138
            width: card.width - 40 - 104 - 12
            label: qsTr("服务器地址")
            placeholder: qsTr("例如 nas.local 或 192.168.1.10")
            onEdited: root.clearStatus()
        }

        Field {
            id: portField
            x: hostField.x + hostField.width + 12
            y: 138
            width: 104
            label: qsTr("端口")
            placeholder: root.defaultPort()
            inputHints: Qt.ImhDigitsOnly
            onEdited: root.clearStatus()
        }

        Field {
            id: userField
            x: 20
            y: 202
            width: (card.width - 40 - 12) / 2
            label: qsTr("账号（可选）")
            onEdited: root.clearStatus()
        }

        Field {
            id: passField
            x: userField.x + userField.width + 12
            y: 202
            width: userField.width
            label: qsTr("密码（可选）")
            password: true
            onEdited: root.clearStatus()
        }

        Item {
            id: rememberRow
            x: 20
            y: 266
            width: card.width - 40
            height: 18

            Rectangle {
                id: checkBox
                anchors.verticalCenter: parent.verticalCenter
                width: 16
                height: 16
                radius: Theme.radiusSm
                color: root.remember ? root.accent : "transparent"
                border.width: 1
                border.color: root.remember ? root.accent : root.hairline

                Glyph {
                    anchors.centerIn: parent
                    visible: root.remember
                    width: 10
                    height: 10
                    kind: "check"
                    tint: Theme.onInteractive
                }
            }

            Text {
                anchors.left: checkBox.right
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("记住密码（保存在系统钥匙串中）")
                color: root.mutedColor
                font.pixelSize: 11
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.remember = !root.remember
            }
        }

        Text {
            id: statusText
            x: 20
            y: 292
            width: card.width - 40
            height: 34
            text: root.testMessage !== "" ? root.testMessage : root.errorText
            color: root.errorText !== "" ? root.dangerColor
                : (root.testOk ? root.okColor : root.mutedColor)
            font.pixelSize: 11
            wrapMode: Text.Wrap
            elide: Text.ElideRight
        }

        Rectangle {
            id: testButton
            x: 20
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 90
            height: 28
            radius: Theme.radiusSm
            color: testMouse.containsMouse && testButton.enabled
                ? Theme.fill : "transparent"
            border.width: 1
            border.color: root.hairline
            enabled: !root.testing && !root.saving

            Text {
                anchors.centerIn: parent
                text: root.testing ? qsTr("测试中…") : qsTr("测试连接")
                color: testButton.enabled ? Theme.inkMuted : root.mutedColor
                font.pixelSize: 12
            }

            MouseArea {
                id: testMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.requestTest()
            }
        }

        Rectangle {
            id: submitButton
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 82
            height: 28
            radius: Theme.radiusSm
            color: submitButton.enabled
                ? (submitMouse.containsMouse ? Theme.interactiveHover : root.accent)
                : Theme.canvasAlt

            Text {
                anchors.centerIn: parent
                text: root.saving ? qsTr("保存中…") : qsTr("连接")
                color: Theme.onInteractive
                font.pixelSize: 12
            }

            MouseArea {
                id: submitMouse
                anchors.fill: parent
                hoverEnabled: true
                enabled: !root.testing && !root.saving
                onClicked: root.submit()
            }
        }

        Rectangle {
            anchors.right: submitButton.left
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 72
            height: 28
            radius: Theme.radiusSm
            color: cancelMouse.containsMouse
                ? Theme.fill : "transparent"
            border.width: 1
            border.color: root.hairline

            Text {
                anchors.centerIn: parent
                text: qsTr("取消")
                color: Theme.inkMuted
                font.pixelSize: 12
            }

            MouseArea {
                id: cancelMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.dismiss()
            }
        }
    }

    component ProtocolButton: Rectangle {
        id: button

        property string protocol: ""
        property string label: ""

        readonly property bool selected: root.protocol === button.protocol
        readonly property bool hovered: protocolMouse.containsMouse

        width: Math.max(120, labelText.implicitWidth + 28)
        height: 28
        radius: Theme.radiusSm
        color: selected
            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.20)
            : (hovered ? Theme.fill : "transparent")
        border.width: 1
        border.color: selected ? root.accent : root.hairline

        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.onPressAction: root.setProtocol(button.protocol)

        Text {
            id: labelText
            anchors.centerIn: parent
            text: button.label
            color: button.selected ? root.textColor : Theme.inkMuted
            font.pixelSize: 11
        }

        MouseArea {
            id: protocolMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.setProtocol(button.protocol)
        }
    }

    component Field: Item {
        id: field

        property string label: ""
        property string placeholder: ""
        property bool password: false
        property int inputHints: Qt.ImhNone
        property alias value: input.text
        property alias inputItem: input

        signal edited()

        height: 52

        Text {
            x: 0
            y: 0
            text: field.label
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
        }

        Rectangle {
            id: frame
            y: 20
            width: parent.width
            height: 28
            radius: Theme.radiusSm
            color: root.fieldColor
            border.width: 1
            border.color: input.activeFocus ? root.accent : root.hairline

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 9
                anchors.verticalCenter: parent.verticalCenter
                visible: input.text.length === 0
                text: field.placeholder
                color: root.mutedColor
                font.pixelSize: 12
                elide: Text.ElideRight
                width: parent.width - 18
            }

            TextInput {
                id: input
                anchors.fill: parent
                anchors.leftMargin: 9
                anchors.rightMargin: 9
                verticalAlignment: TextInput.AlignVCenter
                color: root.textColor
                selectionColor: root.accent
                selectedTextColor: Theme.onInteractive
                font.pixelSize: 12
                echoMode: field.password ? TextInput.Password : TextInput.Normal
                inputMethodHints: field.inputHints
                selectByMouse: true

                Accessible.role: Accessible.EditableText
                Accessible.name: field.label
                clip: true

                onTextEdited: field.edited()
                onAccepted: root.submit()
            }
        }
    }
}
