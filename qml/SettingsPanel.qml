import QtQuick
import dae 1.0

// The terminal typography section of the settings document.
//
// Theme and locale deliberately stay out: they are view preferences the shells
// keep for themselves. Font size, line height and family are persisted in
// `settings.toml` and shared with the Tauri shell, so they are edited here
// through `Workspace.saveTerminalSettings`.
Item {
    id: root

    property bool dark: true

    signal closed()

    visible: false
    z: 1001
    focus: visible

    function open() {
        sizeInput.text = Workspace.terminalFontSize
        heightInput.text = Workspace.terminalLineHeight.toFixed(2)
        familyInput.text = Workspace.terminalFontFamily
        errorText.text = ""
        root.visible = true
        sizeInput.forceActiveFocus()
        sizeInput.selectAll()
    }

    function close() {
        root.visible = false
        root.closed()
    }

    function save() {
        const size = parseInt(sizeInput.text, 10)
        const lineHeight = parseFloat(heightInput.text)
        if (!(size >= 8 && size <= 24)) {
            errorText.text = "字号需要在 8 到 24 之间"
            return
        }
        if (!(lineHeight >= 0.8 && lineHeight <= 3.0)) {
            errorText.text = "行高需要在 0.80 到 3.00 之间"
            return
        }
        Workspace.saveTerminalSettings(size, lineHeight, familyInput.text)
        root.close()
    }

    Keys.onEscapePressed: root.close()

    Rectangle {
        anchors.fill: parent
        color: "#73000000"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.close()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: 430
        height: 268
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
        }

        Text {
            x: 20
            y: 16
            text: "终端设置"
            color: root.dark ? "#e7ebf2" : "#20252b"
            font.pixelSize: 14
            font.weight: Font.DemiBold
        }

        Text {
            x: 20
            y: 40
            text: "写入 settings.toml，两个壳共用同一份文档"
            color: root.dark ? "#7f8797" : "#8a929d"
            font.pixelSize: 11
        }

        Text {
            x: 20
            y: 74
            text: "字号"
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        FieldFrame {
            id: sizeFrame
            x: 110
            y: 68
            width: 80
            height: 28
        }

        TextInput {
            id: sizeInput
            anchors.fill: sizeFrame
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            inputMethodHints: Qt.ImhDigitsOnly
            color: root.dark ? "#e7ebf2" : "#20252b"
            selectionColor: "#2f6fdf"
            selectedTextColor: "#ffffff"
            font.pixelSize: 12
            clip: true
            onAccepted: root.save()
        }

        Text {
            x: 200
            anchors.verticalCenter: sizeFrame.verticalCenter
            text: "8 – 24"
            color: root.dark ? "#6e7684" : "#98a0ab"
            font.pixelSize: 11
        }

        Text {
            x: 20
            y: 116
            text: "行高"
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        FieldFrame {
            id: heightFrame
            x: 110
            y: 110
            width: 80
            height: 28
        }

        TextInput {
            id: heightInput
            anchors.fill: heightFrame
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            color: root.dark ? "#e7ebf2" : "#20252b"
            selectionColor: "#2f6fdf"
            selectedTextColor: "#ffffff"
            font.pixelSize: 12
            clip: true
            onAccepted: root.save()
        }

        Text {
            x: 200
            anchors.verticalCenter: heightFrame.verticalCenter
            text: "0.80 – 3.00"
            color: root.dark ? "#6e7684" : "#98a0ab"
            font.pixelSize: 11
        }

        Text {
            x: 20
            y: 158
            text: "字体"
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        FieldFrame {
            id: familyFrame
            x: 110
            y: 152
            width: 292
            height: 28
        }

        TextInput {
            id: familyInput
            anchors.fill: familyFrame
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            color: root.dark ? "#e7ebf2" : "#20252b"
            selectionColor: "#2f6fdf"
            selectedTextColor: "#ffffff"
            font.pixelSize: 12
            clip: true
            onAccepted: root.save()
        }

        Text {
            x: 110
            y: 186
            text: "留空使用 Cascadia Mono / Consolas"
            color: root.dark ? "#6e7684" : "#98a0ab"
            font.pixelSize: 10
        }

        Text {
            id: errorText
            x: 20
            y: 208
            text: ""
            color: root.dark ? "#f08a8a" : "#b42323"
            font.pixelSize: 11
        }

        Rectangle {
            anchors.right: saveButton.left
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
                onClicked: root.close()
            }
        }

        Rectangle {
            id: saveButton
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 78
            height: 26
            radius: 6
            color: saveMouse.containsMouse ? "#3f7ce0" : "#2f6fdf"

            Text {
                anchors.centerIn: parent
                text: "保存"
                color: "#ffffff"
                font.pixelSize: 12
            }

            MouseArea {
                id: saveMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.save()
            }
        }
    }

    component FieldFrame: Rectangle {
        radius: 6
        color: root.dark ? "#16181c" : "#f4f6f9"
        border.width: 1
        border.color: root.dark ? "#26ffffff" : "#18000000"
    }
}
