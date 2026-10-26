import QtQuick

// One modal question with a cancel and a confirm button. Permanent deletion
// in the recycle bin is the only flow that needs it, but the dialog is plain
// on purpose: the caller supplies the sentence and the confirm label.
FocusScope {
    id: root

    property bool dark: true
    property string heading: ""
    property string message: ""
    property string confirmLabel: "确定"
    property bool danger: false

    signal accepted()
    signal canceled()

    visible: false
    z: 1002
    focus: visible

    function open(title, text, label, isDanger) {
        root.heading = title
        root.message = text
        root.confirmLabel = label || "确定"
        root.danger = isDanger === true
        root.visible = true
        root.forceActiveFocus()
        // A modal opened from a menu is shown while the menu is still
        // unwinding; asking for focus again on the next turn is what actually
        // lands it in the dialog rather than back on the path field.
        Qt.callLater(function () {
            if (root.visible)
                root.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
    }

    function confirm() {
        root.close()
        root.accepted()
    }

    Keys.onEscapePressed: {
        root.close()
        root.canceled()
    }

    Keys.onReturnPressed: root.confirm()
    Keys.onEnterPressed: root.confirm()

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
        width: Math.min(440, root.width - 48)
        height: messageText.implicitHeight + 128
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        // Wheel events must not scroll the list behind the dialog; a NoButton
        // guard cannot steal presses from the buttons below it.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Text {
            x: 18
            y: 18
            width: parent.width - 36
            text: root.heading
            color: root.dark ? "#e7ebf2" : "#20252b"
            font.pixelSize: 14
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }

        Text {
            id: messageText
            x: 18
            y: 48
            width: parent.width - 36
            text: root.message
            color: root.dark ? "#aeb6c2" : "#4c545e"
            font.pixelSize: 12
            lineHeight: 1.25
            wrapMode: Text.Wrap
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
            width: Math.max(78, submitLabel.implicitWidth + 24)
            height: 26
            radius: 6
            color: root.danger
                ? (submitMouse.containsMouse ? "#a33232" : "#8f2d2d")
                : (submitMouse.containsMouse ? "#3f7ce0" : "#2f6fdf")

            Accessible.role: Accessible.Button
            Accessible.name: root.confirmLabel
            Accessible.onPressAction: root.confirm()

            Text {
                id: submitLabel
                anchors.centerIn: parent
                text: root.confirmLabel
                color: "#ffffff"
                font.pixelSize: 12
            }

            MouseArea {
                id: submitMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.confirm()
            }
        }
    }
}
