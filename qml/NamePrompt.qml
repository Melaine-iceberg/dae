import QtQuick

// The one-line name dialog used by "new folder", "new file" and rename.
// A real modal is overkill for a single input, and the frameless window has no
// native dialog to hang off; this overlay keeps focus inside the QML scene.
//
// It matches `ConfirmDialog`: 440 wide, 64 from the top, 12 px corners, a 44 px
// title band and a 44 px foot. The field is 28 high on the `field` plate with a
// 1 px signal edge while focused, which is the same treatment the path editor
// and the search field use.
Item {
    id: root

    property bool dark: true
    property string heading: ""
    property string hint: ""
    property string submitLabel: qsTr("确定")

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
        // The release that opened the prompt runs after this call; without
        // the second attempt the field is focused but the dialog is not, and
        // Escape goes to the page behind it.
        Qt.callLater(function () {
            if (root.visible) {
                root.forceActiveFocus()
                input.forceActiveFocus()
            }
        })
        input.selectAll()
    }

    function close() {
        root.visible = false
    }

    function submit() {
        const value = input.text.trim()
        if (value.length === 0) {
            errorText.text = qsTr("名称不能为空")
            return
        }
        const valueToUse = value
        root.close()
        root.accepted(valueToUse)
    }

    function cancel() {
        root.close()
        root.canceled()
    }

    Keys.onEscapePressed: root.cancel()

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.cancel()
        }
    }

    Rectangle {
        id: panel

        anchors.horizontalCenter: parent.horizontalCenter
        y: 64
        width: Math.min(440, root.width - 32)
        height: 160
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Item {
            id: head

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 44

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.right: closeButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                text: root.heading
                color: Theme.ink
                font.pixelSize: Theme.typeTitle
                font.weight: Font.Medium
                elide: Text.ElideRight
            }

            Item {
                id: closeButton

                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 20
                height: 20

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("关闭")
                Accessible.onPressAction: root.cancel()

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: closeMouse.containsMouse ? Theme.hover : "transparent"
                }

                Glyph {
                    anchors.centerIn: parent
                    width: 10
                    height: 10
                    kind: "close"
                    tint: closeMouse.containsMouse ? Theme.ink : Theme.inkFaint
                }

                MouseArea {
                    id: closeMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.cancel()
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Theme.hairline
            }
        }

        Rectangle {
            id: field

            x: 16
            y: head.height + 12
            width: parent.width - 32
            height: 28
            radius: Theme.radiusSm
            color: Theme.field
            border.width: 1
            border.color: input.activeFocus ? Theme.signal : Theme.hairlineStrong

            TextInput {
                id: input
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                verticalAlignment: TextInput.AlignVCenter
                color: Theme.ink
                selectionColor: Theme.interactive
                selectedTextColor: Theme.onInteractive
                font.pixelSize: Theme.typeBody
                selectByMouse: true
                clip: true

                onAccepted: root.submit()

                // The dialog has one field and no focusable button, so Tab
                // stays here rather than walking into the list behind the
                // scrim.
                Keys.onTabPressed: function (event) { event.accepted = true }
                Keys.onBacktabPressed: function (event) { event.accepted = true }
            }
        }

        Text {
            id: errorText
            x: 16
            y: field.y + field.height + 6
            text: ""
            color: Theme.danger
            font.pixelSize: Theme.typeMicro
        }

        Text {
            x: 16
            anchors.baseline: errorText.baseline
            text: errorText.text === "" ? root.hint : ""
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
        }

        Item {
            id: foot

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 44

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: Theme.hairline
            }

            DialogButton {
                id: submitButton
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 8
                label: root.submitLabel
                tone: "primary"
                onClicked: root.submit()
            }

            DialogButton {
                anchors.right: submitButton.left
                anchors.rightMargin: 8
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 8
                label: qsTr("取消")
                tone: "quiet"
                onClicked: root.cancel()
            }
        }
    }
}
