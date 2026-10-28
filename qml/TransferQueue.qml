pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// What is waiting behind the transfer that owns the progress row.
//
// The running job is deliberately not listed: it already names itself on the
// status row, and it cannot be cancelled — the core has no seam for stopping a
// copy mid-flight, and a queue entry that silently did nothing would be worse
// than no entry at all. Every row here therefore means "not started yet", which
// is exactly the set a ✕ can drop.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    property real openX: 0
    property real openY: 0

    visible: false
    focus: visible
    z: 1002

    readonly property color panelColor: Theme.raised
    readonly property color borderColor: Theme.hairlineStrong
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property int rowHeight: 28

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.visible = true
        root.forceActiveFocus()
    }

    function close() {
        root.visible = false
    }

    Keys.onEscapePressed: root.close()

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel

        width: 320
        height: Math.min(280, column.implicitHeight + 12)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, root.openY - height - 6)
        radius: 12
        color: root.panelColor
        border.width: 1
        border.color: root.borderColor
        clip: true

        Column {
            id: column
            width: parent.width

            Item {
                width: parent.width
                height: 26

                Text {
                    x: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("传输队列 · ") + Workspace.queuedTransfers.length + qsTr(" 项等待")
                    color: root.mutedColor
                    font.pixelSize: 11
                }

                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("全部取消")
                    color: cancelAllMouse.containsMouse ? root.textColor : root.mutedColor
                    font.pixelSize: 11
                }

                MouseArea {
                    id: cancelAllMouse
                    anchors.right: parent.right
                    width: 58
                    height: parent.height
                    hoverEnabled: true
                    onClicked: Workspace.cancelQueuedTransfers()
                }
            }

            Repeater {
                model: Workspace.queuedTransfers

                Item {
                    id: row
                    required property int index
                    required property var modelData

                    width: column.width
                    height: root.rowHeight

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        anchors.right: dropHit.left
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.modelData
                        color: root.textColor
                        font.pixelSize: 11
                        elide: Text.ElideMiddle
                    }

                    Text {
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: "\u00d7"
                        color: dropHit.containsMouse ? root.textColor : root.mutedColor
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: dropHit
                        anchors.right: parent.right
                        width: 28
                        height: parent.height
                        hoverEnabled: true
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("取消 ") + row.modelData
                        onClicked: Workspace.cancelQueuedTransfer(row.index)
                    }
                }
            }

            Item {
                width: parent.width
                height: Workspace.queuedTransfers.length === 0 ? 30 : 0
                visible: Workspace.queuedTransfers.length === 0

                Text {
                    anchors.centerIn: parent
                    text: qsTr("没有排队中的传输")
                    color: root.mutedColor
                    font.pixelSize: 11
                }
            }
        }
    }
}
