import QtQuick

// One line of feedback, used for operation results and errors. Long operations
// already have the status bar's progress row; this is only for the outcome.
//
// The plate is the one raised surface in the window's bottom margin: 34 high,
// 38 up from the edge, 4 px corners, no drop shadow. It used to be a dark pill
// in the middle of the window with 2.6 seconds of life, which made an error as
// easy to miss as a success. A toast now sits where the eye already goes for
// transient state, stays for 4.8 seconds, and offers the undo when the outcome
// can be taken back instead of making the user hunt for Mod+Z.
Item {
    id: root

    property bool dark: true
    property int duration: 4800
    /// True when the outcome can be reversed; the row then offers the verb.
    /// The shell owns the undo stack — this only asks for it.
    property bool undoable: false

    signal undoRequested()

    visible: opacity > 0.01
    opacity: 0
    z: 900

    // Deliberately unanimated: the plate is gone the moment it is dismissed, so
    // it can never swallow a click aimed at the window behind it.
    implicitWidth: bubble.width
    implicitHeight: bubble.height
    width: implicitWidth
    height: implicitHeight

    function show(message, isError, canUndo) {
        label.text = message
        root.undoable = canUndo === true
        icon.kind = isError === true ? "close" : "check"
        icon.tint = isError === true ? Theme.danger : Theme.inkMuted
        root.opacity = 1
        timer.restart()
    }

    function dismiss() {
        timer.stop()
        root.opacity = 0
        root.undoable = false
    }

    Timer {
        id: timer
        interval: root.duration
        onTriggered: root.dismiss()
    }

    Rectangle {
        id: bubble

        anchors.centerIn: parent
        width: row.implicitWidth + 20
        height: 34
        radius: Theme.radiusSm
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        Row {
            id: row

            anchors.centerIn: parent
            spacing: 9

            Glyph {
                id: icon
                width: 14
                height: 14
                anchors.verticalCenter: parent.verticalCenter
                kind: "check"
                tint: Theme.inkMuted
            }

            Text {
                id: label
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(320, implicitWidth)
                text: ""
                color: Theme.ink
                font.pixelSize: Theme.typeSmall
                elide: Text.ElideRight
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }

            Rectangle {
                visible: root.undoable
                width: visible ? undoLabel.implicitWidth + 12 : 0
                height: 24
                radius: Theme.radiusXs
                color: undoMouse.containsMouse ? Theme.signalSoft : "transparent"

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("撤销")
                Accessible.onPressAction: {
                    root.dismiss()
                    root.undoRequested()
                }

                Text {
                    id: undoLabel
                    anchors.centerIn: parent
                    text: qsTr("撤销")
                    color: Theme.signal
                    font.pixelSize: Theme.typeSmall
                }

                MouseArea {
                    id: undoMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        // Undo first, then close: the toast must not outlive the
                        // action it is offering to reverse.
                        root.dismiss()
                        root.undoRequested()
                    }
                }
            }

            Item {
                width: 20
                height: 20
                anchors.verticalCenter: parent.verticalCenter

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("关闭提示")
                Accessible.onPressAction: root.dismiss()

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusXs
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
                    onClicked: root.dismiss()
                }
            }
        }
    }
}
