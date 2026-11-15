pragma ComponentBehavior: Bound
import QtQuick
import "DialogFocus.js" as DialogFocus

// The one-line name dialog used by "new folder", "new file" and rename.
// A real modal is overkill for a single input, and the frameless window has no
// native dialog to hang off; this overlay keeps focus inside the QML scene.
//
// The plate, the scrim, the head and the foot are `Dialog`'s; this file owns
// the one field and the two things a name needs — a hint and an inline error.
Dialog {
    id: root

    property string hint: ""
    property string submitLabel: qsTr("确定")

    signal accepted(string text)
    signal canceled()

    /// The field (28) plus the hint line (6 + 14).
    bodyHeight: 48

    onCancelRequested: root.cancel()

    function open(title, initial, placeholder) {
        root.heading = title
        root.hint = placeholder || ""
        input.text = initial || ""
        errorText.text = ""
        root.show()
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

    Keys.onTabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, true)
    }
    Keys.onBacktabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, false)
    }

    Rectangle {
        id: field

        width: parent.width
        height: Theme.controlMd
        radius: Theme.radiusSm
        color: Theme.field
        border.width: 1
        border.color: input.activeFocus ? Theme.accent : Theme.line

        TextInput {
            id: input
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            color: Theme.ink
            selectionColor: Theme.accent
            selectedTextColor: Theme.onAccent
            font.pixelSize: Theme.typeBody
            selectByMouse: true
            clip: true

            Accessible.role: Accessible.EditableText
            Accessible.name: root.heading

            onAccepted: root.submit()
        }
    }

    Text {
        id: errorText
        y: field.height + 6
        text: ""
        color: Theme.danger
        font.pixelSize: Theme.typeMicro
    }

    Text {
        anchors.baseline: errorText.baseline
        x: errorText.implicitWidth > 0 ? errorText.implicitWidth + 8 : 0
        text: errorText.text === "" ? root.hint : ""
        color: Theme.inkTertiary
        font.pixelSize: Theme.typeMicro
    }

    footer: Component {
        Item {
            implicitHeight: Theme.controlMd

            DialogButton {
                id: submitButton
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                label: root.submitLabel
                tone: "primary"
                onClicked: root.submit()
            }

            DialogButton {
                anchors.right: submitButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("取消")
                tone: "quiet"
                onClicked: root.cancel()
            }
        }
    }
}
