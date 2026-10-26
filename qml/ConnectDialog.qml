pragma ComponentBehavior: Bound
import QtQuick

// The add/edit dialog for SMB and SFTP servers.
//
// Validation, testing and storage all live on the bridge; this file only
// collects the fields, shows the outcome, and emits intent, so the same dialog
// would work if a server is later edited instead of created.
Item {
    id: root

    property bool dark: true
    property color accent: "#2f6fdf"
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

    readonly property color textColor: root.dark ? "#e7ebf2" : "#20252b"
    readonly property color mutedColor: root.dark ? "#8d94a2" : "#657080"
    readonly property color fieldColor: root.dark ? "#16181c" : "#f4f6f9"
    readonly property color hairline: root.dark ? "#26ffffff" : "#18000000"
    readonly property color dangerColor: root.dark ? "#f08a8a" : "#b42323"
    readonly property color okColor: root.dark ? "#8fd18f" : "#2c7a2c"

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
            root.errorText = "端口必须是 1-65535"
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
            root.errorText = "请输入服务器地址"
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
            root.errorText = "请输入服务器地址"
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

    Rectangle {
        anchors.fill: parent
        color: "#73000000"

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
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        // The dimmed backdrop must not scroll the list behind the dialog, and
        // clicks inside the card must never reach that backdrop.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Text {
            x: 20
            y: 18
            text: "连接网络存储"
            color: root.textColor
            font.pixelSize: 14
            font.weight: Font.DemiBold
        }

        Text {
            x: 20
            y: 42
            width: parent.width - 40
            text: "连接 SMB 服务器（NAS、Windows 共享）或 SFTP 服务器（Linux、NAS）。"
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Text {
            x: 20
            y: 72
            text: "协议"
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        Row {
            x: 20
            y: 92
            spacing: 8

            ProtocolButton {
                protocol: "smb"
                label: "SMB / Windows 共享"
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
                    text: "FTP / WebDAV 即将支持"
                    color: root.mutedColor
                    font.pixelSize: 10
                }
            }
        }

        Field {
            id: hostField
            x: 20
            y: 138
            width: card.width - 40 - 104 - 12
            label: "服务器地址"
            placeholder: "例如 nas.local 或 192.168.1.10"
            onEdited: root.clearStatus()
        }

        Field {
            id: portField
            x: hostField.x + hostField.width + 12
            y: 138
            width: 104
            label: "端口"
            placeholder: root.defaultPort()
            inputHints: Qt.ImhDigitsOnly
            onEdited: root.clearStatus()
        }

        Field {
            id: userField
            x: 20
            y: 202
            width: (card.width - 40 - 12) / 2
            label: "账号（可选）"
            onEdited: root.clearStatus()
        }

        Field {
            id: passField
            x: userField.x + userField.width + 12
            y: 202
            width: userField.width
            label: "密码（可选）"
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
                radius: 4
                color: root.remember ? root.accent : "transparent"
                border.width: 1
                border.color: root.remember ? root.accent : root.hairline

                Text {
                    anchors.centerIn: parent
                    visible: root.remember
                    text: "✓"
                    color: "#ffffff"
                    font.pixelSize: 10
                }
            }

            Text {
                anchors.left: checkBox.right
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                text: "记住密码（保存在系统钥匙串中）"
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
            height: 26
            radius: 6
            color: testMouse.containsMouse && testButton.enabled
                ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"
            border.width: 1
            border.color: root.hairline
            enabled: !root.testing && !root.saving

            Text {
                anchors.centerIn: parent
                text: root.testing ? "测试中…" : "测试连接"
                color: testButton.enabled ? (root.dark ? "#c9cfd9" : "#3a4048") : root.mutedColor
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
            height: 26
            radius: 6
            color: submitButton.enabled
                ? (submitMouse.containsMouse ? "#3f7ce0" : root.accent)
                : (root.dark ? "#2b2f36" : "#e2e6eb")

            Text {
                anchors.centerIn: parent
                text: root.saving ? "保存中…" : "连接"
                color: "#ffffff"
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
            height: 26
            radius: 6
            color: cancelMouse.containsMouse
                ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"
            border.width: 1
            border.color: root.hairline

            Text {
                anchors.centerIn: parent
                text: "取消"
                color: root.dark ? "#c9cfd9" : "#3a4048"
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
        height: 30
        radius: 6
        color: selected
            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.20)
            : (hovered ? (root.dark ? "#26ffffff" : "#12000000") : "transparent")
        border.width: 1
        border.color: selected ? root.accent : root.hairline

        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.onPressAction: root.setProtocol(button.protocol)

        Text {
            id: labelText
            anchors.centerIn: parent
            text: button.label
            color: button.selected ? root.textColor : (root.dark ? "#c3cad5" : "#3a4048")
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
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        Rectangle {
            id: frame
            y: 20
            width: parent.width
            height: 30
            radius: 6
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
                selectedTextColor: "#ffffff"
                font.pixelSize: 12
                echoMode: field.password ? TextInput.Password : TextInput.Normal
                inputMethodHints: field.inputHints
                selectByMouse: true
                clip: true

                onTextEdited: field.edited()
                onAccepted: root.submit()
            }
        }
    }
}
