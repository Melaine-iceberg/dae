import QtQuick

// The one empty state: an emphasized sentence, an optional quiet reason, and
// one or two actions.
//
// M3 Expressive gives the empty moment typographic weight instead of an
// illustration: the sentence is `titleLarge`, the reason is body-small, and
// the primary action is a tonal pill. The block stays at the content's
// top-left so the blank area below it does not become the subject.
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
        width: Math.max(0, Math.min(parent.width - 48, 440))
        spacing: 10

        Text {
            width: parent.width
            visible: root.title !== ""
            text: root.title
            color: Theme.ink
            font.pixelSize: Theme.typeTitleLarge
            font.weight: Theme.weightEmphasis
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            lineHeight: Theme.leadingTight
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

            spacing: 8
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

    component TextButton: Rectangle {
        id: button

        property string label: ""
        property bool primary: false

        signal clicked()

        width: labelText.implicitWidth + 28
        height: Theme.controlLg
        radius: Theme.radiusFull
        color: {
            if (button.primary)
                return buttonMouse.containsMouse
                    ? Qt.rgba(Theme.secondaryContainer.r,
                              Theme.secondaryContainer.g,
                              Theme.secondaryContainer.b, 0.82)
                    : Theme.secondaryContainer
            return buttonMouse.containsMouse
                ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
                : "transparent"
        }

        Accessible.role: Accessible.Button
        Accessible.name: button.label

        Text {
            id: labelText
            anchors.centerIn: parent
            text: button.label
            color: button.primary ? Theme.inkOnSecondaryContainer : Theme.accent
            font.pixelSize: Theme.typeBody
            font.weight: Theme.weightStrong
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
