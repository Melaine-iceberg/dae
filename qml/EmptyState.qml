import QtQuick

// The one empty state: icon, title, hint, and up to two text actions.
//
// Every view used to compose its own version — a muted sentence here, a
// two-line column there — so the same "folder is empty" moment had three
// different weights. The parts below are the whole grammar from
// docs/ui-instrument-migration.md §6.5: an icon, a name, a quiet explanation,
// and actions that are only ever text in the signal colour.
Item {
    id: root

    property string glyph: "folder"
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

        anchors.centerIn: parent
        width: Math.min(parent.width - 48, 380)
        spacing: 2

        Glyph {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 28
            height: 28
            kind: root.glyph
            tint: Theme.inkFaint
        }

        Item {
            width: 1
            height: 8
        }

        Text {
            width: parent.width
            text: root.title
            color: Theme.ink
            font.pixelSize: Theme.typeName
            horizontalAlignment: Text.AlignHCenter
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
        }

        Text {
            width: parent.width
            visible: root.hint !== ""
            text: root.hint
            color: Theme.inkFaint
            font.pixelSize: Theme.typeCaption
            horizontalAlignment: Text.AlignHCenter
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            lineHeight: 1.5
        }

        Item {
            width: 1
            height: 10
            visible: actions.visible
        }

        Row {
            id: actions

            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 18
            visible: root.primaryLabel !== "" || root.secondaryLabel !== ""

            TextButton {
                visible: root.primaryLabel !== ""
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

        signal clicked()

        implicitWidth: labelText.implicitWidth + 4
        implicitHeight: 24

        Accessible.role: Accessible.Button
        Accessible.name: button.label

        Text {
            id: labelText
            anchors.centerIn: parent
            text: button.label
            color: Theme.signal
            font.pixelSize: Theme.typeSmall
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
