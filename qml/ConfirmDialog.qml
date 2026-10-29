import QtQuick

// One modal question with a cancel and a confirm button. Permanent deletion
// in the recycle bin is the only flow that needs it, but the dialog is plain
// on purpose: the caller supplies the sentence and the confirm label.
//
// Geometry follows the prototype rather than the centre of the window: 440
// wide, 64 down from the top, 12 px corners, a 44 px title band and a 44 px
// foot. A dialog that centres itself jumps every time its message wraps to a
// second line; anchoring it near the top keeps the eye in one place.
FocusScope {
    id: root

    property bool dark: true
    property string heading: ""
    property string message: ""
    property string confirmLabel: qsTr("确定")
    property bool danger: false

    /// 0 = cancel, 1 = the primary action. Tab cycles between the two, which
    /// is the whole tab order this dialog has.
    property int focusIndex: 1

    signal accepted()
    signal canceled()

    visible: false
    z: 1002
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: root.heading

    readonly property bool closeOnScrim: !root.danger

    function open(title, text, label, isDanger) {
        root.heading = title
        root.message = text
        root.confirmLabel = label || qsTr("确定")
        root.danger = isDanger === true
        root.focusIndex = 1
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

    function cancel() {
        root.close()
        root.canceled()
    }

    /// Two buttons, so "cycle" is a toggle. Tab and Shift+Tab both land on the
    /// other one, and neither can escape into the page behind the scrim.
    function cycleFocus() {
        root.focusIndex = root.focusIndex === 1 ? 0 : 1
    }

    Keys.onEscapePressed: root.cancel()
    Keys.onReturnPressed: root.confirm()
    Keys.onEnterPressed: root.confirm()
    Keys.onTabPressed: function (event) {
        root.cycleFocus()
        event.accepted = true
    }
    Keys.onBacktabPressed: function (event) {
        root.cycleFocus()
        event.accepted = true
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: {
                // A destructive question does not dismiss on a stray click:
                // "empty the bin" has to be answered, not clicked away.
                if (root.closeOnScrim)
                    root.cancel()
            }
        }
    }

    Rectangle {
        id: panel

        anchors.horizontalCenter: parent.horizontalCenter
        y: 64
        width: Math.min(440, root.width - 32)
        height: 44 + messageText.implicitHeight + 24 + 44
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        // Wheel events must not scroll the list behind the dialog; a NoButton
        // guard cannot steal presses from the buttons below it.
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

        Text {
            id: messageText

            x: 16
            y: head.height + 12
            width: parent.width - 32
            text: root.message
            color: Theme.inkMuted
            font.pixelSize: Theme.typeName
            lineHeight: 1.3
            wrapMode: Text.Wrap
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
                label: root.confirmLabel
                tone: root.danger ? "danger" : "primary"
                focused: root.focusIndex === 1
                onClicked: root.confirm()
            }

            DialogButton {
                anchors.right: submitButton.left
                anchors.rightMargin: 8
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 8
                label: qsTr("取消")
                tone: "quiet"
                focused: root.focusIndex === 0
                onClicked: root.cancel()
            }
        }
    }
}
