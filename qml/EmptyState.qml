import QtQuick

// The one empty state (§10.8): a left-aligned sentence, an optional reason,
// and one or two text actions.
//
// The icon, the centred headline and the decorative glyph that used to live
// here are gone. A file manager's empty folder is not a moment to illustrate;
// it is a place to say what happened and offer the next step. The block sits
// at the content's top-left rather than the middle of the viewport so the
// blank area below it does not become the subject.
Item {
    id: root

    property string title: ""
    property string hint: ""
    property string primaryLabel: ""
    property string secondaryLabel: ""

    signal primaryClicked()
    signal secondaryClicked()

    implicitWidth: content.implicitWidth
    implicitHeight: content.implicitHeight

    Column {
        id: content

        x: 24
        y: 24
        width: Math.max(0, Math.min(parent.width - 48, 420))
        spacing: 8

        Text {
            width: parent.width
            visible: root.title !== ""
            text: root.title
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeBody
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            lineHeight: Theme.leadingBody
        }

        Text {
            width: parent.width
            visible: root.hint !== ""
            text: root.hint
            color: Theme.inkTertiary
            font.pixelSize: Theme.typeCaption
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            lineHeight: Theme.leadingBody
        }

        Item {
            width: 1
            height: 2
            visible: actions.visible
        }

        Row {
            id: actions

            spacing: 18
            visible: root.primaryLabel !== "" || root.secondaryLabel !== ""

            TextButton {
                visible: root.primaryLabel !== ""
                primary: true
                label: root.primaryLabel
                onClicked: root.primaryClicked()
            }

            TextButton {
                visible: root.secondaryLabel !== ""
                label: root.secondaryLabel
                onClicked: root.secondaryClicked()
            }
        }
    }

    component TextButton: Item {
        id: button

        property string label: ""
        property bool primary: false

        signal clicked()

        implicitWidth: labelText.implicitWidth + 4
        implicitHeight: Theme.controlSm

        Accessible.role: Accessible.Button
        Accessible.name: button.label

        Text {
            id: labelText
            anchors.centerIn: parent
            text: button.label
            color: button.primary ? Theme.accent : Theme.inkSecondary
            font.pixelSize: Theme.typeCaption
            font.underline: buttonMouse.containsMouse
            textFormat: Text.PlainText
        }

        MouseArea {
            id: buttonMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: button.clicked()
        }
    }
}
