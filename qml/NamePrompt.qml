import QtQuick

// The one-line name dialog used by "new folder", "new file" and rename.
// A real modal is overkill for a single input, and the frameless window has no
// native dialog to hang off; this overlay keeps focus inside the QML scene.
Item {
    id: root

    property bool dark: true
    property string heading: ""
    property string hint: ""
    property string submitLabel: "确定"

    signal accepted(string text)
    signal canceled()

    visible: false
    z: 1001
    focus: visible

    function open(title, initial, placeholder) {
        root.heading = title
        root.hint = placeholder || ""
        input.text = initial || ""
        errorText.text = ""
        root.visible = true
        input.forceActiveFocus()
        input.selectAll()
    }

    function close() {
        root.visible = false
    }

    function submit() {
        const value = input.text.trim()
        if (value.length === 0) {
            errorText.text = "名称不能为空"
            return
        }
        const valueToUse = value
        root.close()
        root.accepted(valueToUse)
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
        width: 360
        height: 154
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Text {
            x: 18
            y: 16
            text: root.heading
            color: root.dark ? "#e7ebf2" : "#20252b"
            font.pixelSize: 14
            font.weight: Font.DemiBold
        }

        Rectangle {
            id: field
            x: 18
            y: 46
            width: parent.width - 36
            height: 30
            radius: 6
            color: root.dark ? "#16181c" : "#f4f6f9"
            border.width: 1
            border.color: input.activeFocus
                ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf")
                : (root.dark ? "#26ffffff" : "#18000000")

            TextInput {
                id: input
                anchors.fill: parent
                anchors.leftMargin: 9
                anchors.rightMargin: 9
                verticalAlignment: TextInput.AlignVCenter
                color: root.dark ? "#e7ebf2" : "#20252b"
                selectionColor: "#2f6fdf"
                selectedTextColor: "#ffffff"
                font.pixelSize: 13
                selectByMouse: true
                clip: true

                onAccepted: root.submit()
            }
        }

        Text {
            id: errorText
            x: 18
            y: 80
            text: ""
            color: root.dark ? "#f08a8a" : "#b42323"
            font.pixelSize: 11
        }

        Text {
            x: 18
            anchors.baseline: errorText.baseline
            text: errorText.text === "" ? root.hint : ""
            color: root.dark ? "#7f8797" : "#8a929d"
            font.pixelSize: 11
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

            Text {
                anchors.centerIn: parent
                text: root.submitLabel
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
