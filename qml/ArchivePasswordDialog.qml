import QtQuick

// Asks for an archive password. `compress` mode protects a new 7Z archive and
// asks twice; `extract` mode retries a protected archive after the core
// answered with a wrong-password error. The caller keeps the dialog open until
// the operation succeeds, so a wrong retry can show the new message in place.
FocusScope {
    id: root

    property bool dark: true
    property string mode: "extract"
    property string archiveName: ""

    readonly property bool compressMode: root.mode === "compress"
    readonly property string heading: root.compressMode
        ? qsTr("设置压缩包密码")
        : qsTr("解压加密压缩包")
    readonly property string description: root.compressMode
        ? qsTr("为新的 7Z 压缩包设置打开密码。")
        : "“" + root.archiveName + qsTr("”已加密，请输入密码后解压。")

    signal accepted(string password)
    signal canceled()

    visible: false
    z: 1003
    focus: visible

    function open(mode, archiveName, error) {
        root.mode = mode
        root.archiveName = archiveName || ""
        passwordInput.text = ""
        confirmInput.text = ""
        errorText.text = error || ""
        root.visible = true
        Qt.callLater(function () {
            if (root.visible)
                passwordInput.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
    }

    function showError(message) {
        errorText.text = message || ""
    }

    function submit() {
        if (passwordInput.text.length === 0) {
            errorText.text = qsTr("请输入密码")
            return
        }
        if (root.compressMode && passwordInput.text !== confirmInput.text) {
            errorText.text = qsTr("两次输入的密码不一致")
            return
        }
        errorText.text = ""
        root.accepted(passwordInput.text)
    }

    Keys.onEscapePressed: {
        root.close()
        root.canceled()
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: {
                root.close()
                root.canceled()
            }
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(420, root.width - 48)
        height: Math.max(236, contentColumn.implicitHeight + 78)
        radius: 16
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: contentColumn
            x: 18
            y: 18
            width: parent.width - 36
            spacing: 8

            Text {
                width: contentColumn.width
                text: root.heading
                color: Theme.ink
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }

            Text {
                width: contentColumn.width
                text: root.description
                color: Theme.inkMuted
                font.pixelSize: 12
                lineHeight: 1.2
                wrapMode: Text.Wrap
            }

            Text {
                width: contentColumn.width
                text: qsTr("密码")
                color: Theme.inkMuted
                font.pixelSize: 11
            }

            Rectangle {
                width: contentColumn.width
                height: 30
                radius: 6
                color: Theme.field
                border.width: 1
                border.color: passwordInput.activeFocus
                    ? Theme.focusRing
                    : Theme.hairline

                TextInput {
                    id: passwordInput
                    anchors.fill: parent
                    anchors.leftMargin: 9
                    anchors.rightMargin: 9
                    verticalAlignment: TextInput.AlignVCenter
                    color: Theme.ink
                    selectionColor: Theme.interactive
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 13
                    echoMode: TextInput.Password
                    selectByMouse: true
                    clip: true
                    KeyNavigation.tab: confirmInput

                    onAccepted: {
                        if (root.compressMode && confirmInput.text.length === 0) {
                            confirmInput.forceActiveFocus()
                            return
                        }
                        root.submit()
                    }
                }
            }

            Text {
                visible: root.compressMode
                width: contentColumn.width
                text: qsTr("确认密码")
                color: Theme.inkMuted
                font.pixelSize: 11
            }

            Rectangle {
                visible: root.compressMode
                width: contentColumn.width
                height: 30
                radius: 6
                color: Theme.field
                border.width: 1
                border.color: confirmInput.activeFocus
                    ? Theme.focusRing
                    : Theme.hairline

                TextInput {
                    id: confirmInput
                    anchors.fill: parent
                    anchors.leftMargin: 9
                    anchors.rightMargin: 9
                    verticalAlignment: TextInput.AlignVCenter
                    color: Theme.ink
                    selectionColor: Theme.interactive
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 13
                    echoMode: TextInput.Password
                    selectByMouse: true
                    clip: true
                    KeyNavigation.tab: passwordInput

                    onAccepted: root.submit()
                }
            }

            Text {
                id: errorText
                width: contentColumn.width
                height: Math.max(16, implicitHeight)
                text: ""
                color: Theme.danger
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }
        }

        Rectangle {
            id: cancelButton
            anchors.right: submitButton.left
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 72
            height: 26
            radius: 6
            color: cancelMouse.containsMouse
                ? Theme.fill : "transparent"
            border.width: 1
            border.color: Theme.hairline

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("取消")
            Accessible.onPressAction: {
                root.close()
                root.canceled()
            }

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
                onClicked: {
                    root.close()
                    root.canceled()
                }
            }
        }

        Rectangle {
            id: submitButton
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 78
            height: 26
            radius: 6
            color: submitMouse.containsMouse ? Theme.interactiveHover : Theme.interactive

            Accessible.role: Accessible.Button
            Accessible.name: submitLabel.text
            Accessible.onPressAction: root.submit()

            Text {
                id: submitLabel
                anchors.centerIn: parent
                text: root.compressMode ? qsTr("压缩") : qsTr("解压")
                color: Theme.onInteractive
                font.pixelSize: 12
            }

            MouseArea {
                id: submitMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.submit()
            }
        }
    }
}
