pragma ComponentBehavior: Bound
import QtQuick

// The outcome line: one bubble per finished operation, stacked at the bottom
// centre of the content area.
//
// §7.5 fixes the geometry: 36 dp tall, `raised` under the one shadow,
// `md` corners, above the action bar by 8 dp, at most three at once, 4.5 s
// each (8 s when it offers an undo). The stack is bounded because four lines
// of feedback is a log, and a log belongs in the status centre.
//
// Nothing here is animated. A toast that fades out is a toast that keeps
// swallowing clicks aimed at the window behind it, and the plate has no
// reason to be in the way for longer than it is readable.
Item {
    id: root

    property bool dark: true
    /// §7.5: 4.5 s, and 8 s for an undo offer — that one asks for a decision,
    /// so it gets long enough to read and act.
    readonly property int dwell: 4500
    readonly property int dwellUndoable: 8000
    /// Three is what the bottom band can hold without covering the listing.
    readonly property int maxVisible: 3

    signal undoRequested()

    visible: messages.count > 0
    z: 900
    width: column.width
    height: column.height

    ListModel { id: messages }
    property int nextId: 1

    /// `show(message, isError, canUndo)` is the shell's only call. A fourth
    /// bubble retires the oldest rather than growing the stack.
    function show(message, isError, canUndo) {
        if (messages.count >= root.maxVisible)
            messages.remove(0)
        messages.append({
            uid: root.nextId,
            message: message,
            isError: isError === true,
            undoable: canUndo === true
        })
        root.nextId += 1
    }

    function dismiss(uid) {
        for (let index = 0; index < messages.count; index += 1) {
            if (messages.get(index).uid === uid) {
                messages.remove(index)
                return
            }
        }
    }

    /// Drops every bubble at once. The shell does not use it today; it exists
    /// because a window closing with a toast up should not leave a timer
    /// writing into a dead model.
    function dismissAll() {
        messages.clear()
    }

    Column {
        id: column

        anchors.horizontalCenter: parent.horizontalCenter
        spacing: 6

        Repeater {
            model: messages

            delegate: Item {
                id: slot

                required property int index
                required property int uid
                required property string message
                required property bool isError
                required property bool undoable

                width: column.width
                implicitWidth: bubble.width
                implicitHeight: bubble.height

                Timer {
                    interval: slot.undoable ? root.dwellUndoable : root.dwell
                    running: true
                    repeat: false
                    onTriggered: root.dismiss(slot.uid)
                }

                Shadow {
                    target: bubble
                }

                Rectangle {
                    id: bubble

                    anchors.horizontalCenter: parent.horizontalCenter
                    /// Long enough for a sentence, bounded so an error's stack
                    /// trace does not become the window.
                    width: Math.min(420, row.implicitWidth + 20)
                    height: Theme.controlXl
                    radius: Theme.radiusMd
                    color: Theme.raised
                    border.width: 1
                    border.color: Theme.overlayLine

                    Row {
                        id: row

                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        spacing: 8

                        Glyph {
                            id: icon
                            anchors.verticalCenter: parent.verticalCenter
                            width: 14
                            height: 14
                            kind: slot.isError ? "close" : "check"
                            tint: slot.isError ? Theme.danger : Theme.success
                        }

                        Text {
                            id: label

                            anchors.verticalCenter: parent.verticalCenter
                            width: Math.min(320, implicitWidth)
                            text: slot.message
                            color: Theme.ink
                            font.pixelSize: Theme.typeCaption
                            elide: Text.ElideRight
                            Accessible.role: Accessible.StaticText
                            Accessible.name: text
                        }

                        Rectangle {
                            visible: slot.undoable
                            width: visible ? undoLabel.implicitWidth + 12 : 0
                            height: Theme.controlSm
                            radius: Theme.radiusSm
                            color: undoMouse.containsMouse ? Theme.hover
                                : "transparent"

                            Accessible.role: Accessible.Button
                            Accessible.name: qsTr("撤销")
                            Accessible.onPressAction: {
                                root.dismiss(slot.uid)
                                root.undoRequested()
                            }

                            Text {
                                id: undoLabel
                                anchors.centerIn: parent
                                text: qsTr("撤销")
                                color: Theme.accent
                                font.pixelSize: Theme.typeCaption
                            }

                            MouseArea {
                                id: undoMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    // Undo first, then close: the toast must not
                                    // outlive the action it is offering to
                                    // reverse.
                                    root.dismiss(slot.uid)
                                    root.undoRequested()
                                }
                            }
                        }

                        Item {
                            width: 20
                            height: Theme.controlXs
                            anchors.verticalCenter: parent.verticalCenter

                            Accessible.role: Accessible.Button
                            Accessible.name: qsTr("关闭提示")
                            Accessible.onPressAction: root.dismiss(slot.uid)

                            Rectangle {
                                anchors.fill: parent
                                radius: Theme.radiusSm
                                color: closeMouse.containsMouse ? Theme.hover
                                    : "transparent"
                            }

                            Glyph {
                                anchors.centerIn: parent
                                width: 10
                                height: 10
                                kind: "close"
                                tint: closeMouse.containsMouse ? Theme.inkSecondary
                                    : Theme.inkTertiary
                            }

                            MouseArea {
                                id: closeMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.dismiss(slot.uid)
                            }
                        }
                    }
                }
            }
        }
    }
}
