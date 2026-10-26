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
        ? "设置压缩包密码"
        : "解压加密压缩包"
    readonly property string description: root.compressMode
        ? "为新的 7Z 压缩包设置打开密码。"
        : "“" + root.archiveName + "”已加密，请输入密码后解压。"

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
            errorText.text = "请输入密码"
            return
        }
        if (root.compressMode && passwordInput.text !== confirmInput.text) {
            errorText.text = "两次输入的密码不一致"
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
        color: "#73000000"

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
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

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
                color: root.dark ? "#e7ebf2" : "#20252b"
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }

            Text {
                width: contentColumn.width
                text: root.description
                color: root.dark ? "#aeb6c2" : "#4c545e"
                font.pixelSize: 12
                lineHeight: 1.2
                wrapMode: Text.Wrap
            }

            Text {
                width: contentColumn.width
                text: "密码"
                color: root.dark ? "#8d94a2" : "#657080"
                font.pixelSize: 11
            }

            Rectangle {
                width: contentColumn.width
                height: 30
                radius: 6
                color: root.dark ? "#16181c" : "#f4f6f9"
                border.width: 1
                border.color: passwordInput.activeFocus
                    ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf")
                    : (root.dark ? "#26ffffff" : "#18000000")

                TextInput {
                    id: passwordInput
                    anchors.fill: parent
                    anchors.leftMargin: 9
                    anchors.rightMargin: 9
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.dark ? "#e7ebf2" : "#20252b"
                    selectionColor: "#2f6fdf"
                    selectedTextColor: "#ffffff"
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
                text: "确认密码"
                color: root.dark ? "#8d94a2" : "#657080"
                font.pixelSize: 11
            }

            Rectangle {
                visible: root.compressMode
                width: contentColumn.width
                height: 30
                radius: 6
                color: root.dark ? "#16181c" : "#f4f6f9"
                border.width: 1
                border.color: confirmInput.activeFocus
                    ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf")
                    : (root.dark ? "#26ffffff" : "#18000000")

                TextInput {
                    id: confirmInput
                    anchors.fill: parent
                    anchors.leftMargin: 9
                    anchors.rightMargin: 9
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.dark ? "#e7ebf2" : "#20252b"
                    selectionColor: "#2f6fdf"
                    selectedTextColor: "#ffffff"
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
                color: root.dark ? "#f08a8a" : "#b42323"
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
                ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"
            border.width: 1
            border.color: root.dark ? "#26ffffff" : "#18000000"

            Accessible.role: Accessible.Button
            Accessible.name: "取消"
            Accessible.onPressAction: {
                root.close()
                root.canceled()
            }

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
            color: submitMouse.containsMouse ? "#3f7ce0" : "#2f6fdf"

            Accessible.role: Accessible.Button
            Accessible.name: submitLabel.text
            Accessible.onPressAction: root.submit()

            Text {
                id: submitLabel
                anchors.centerIn: parent
                text: root.compressMode ? "压缩" : "解压"
                color: "#ffffff"
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
