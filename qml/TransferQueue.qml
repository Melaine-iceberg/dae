pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// What is waiting behind the transfer that owns the status bar's progress row.
//
// The running job leads the list because it is the one the user is watching;
// it cannot be cancelled — the core has no seam for stopping a copy mid-flight,
// and a queue entry that silently did nothing would be worse than no entry at
// all. Every row that carries a ✕ therefore means "not started yet", which is
// exactly the set a cancel can drop.
//
// A 326 px plate over the bottom of the window, the way the prototype stacks
// it: it is a read-out on the status bar, not a dialog.
// See docs/ui-instrument-migration.md §7.3.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    visible: false
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        NumberAnimation { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1002

    readonly property color panelColor: Theme.raised
    readonly property color borderColor: Theme.hairlineStrong
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color faintColor: Theme.inkFaint
    // qmllint disable missing-property
    readonly property int queuedCount: Workspace.queuedTransfers.length
    readonly property int runningCount: Workspace.busy ? 1 : 0
    readonly property real percent: Workspace.progressTotal > 0
        ? Math.max(0, Math.min(100, Math.round(Workspace.progressDone * 100 / Workspace.progressTotal)))
        : (Workspace.busy ? 0 : 100)

    /// Kept for the shell's call site. The plate is anchored to the bottom of
    /// the window rather than to the chip: at 326 px it would otherwise jump
    /// sideways every time the progress digits changed width.
    function openAbove(x, y) {
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

        width: 326
        height: Math.min(320, body.height + header.height + 8)
        x: Math.round((root.width - width) / 2)
        y: root.height - height - 32
        radius: Theme.radiusMd
        color: root.panelColor
        border.width: 1
        border.color: root.borderColor

        Item {
            id: header
            width: parent.width
            height: 26

            Text {
                x: 10
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("传输队列")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                font.letterSpacing: 0.55
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                Item {
                    id: cancelAll
                    width: 52
                    height: 22
                    visible: root.queuedCount > 0

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("全部取消")
                    Accessible.onPressAction: Workspace.cancelQueuedTransfers()

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("全部取消")
                        color: cancelAllMouse.containsMouse ? root.textColor : root.faintColor
                        font.pixelSize: Theme.typeMicro
                        textFormat: Text.PlainText
                    }

                    MouseArea {
                        id: cancelAllMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: Workspace.cancelQueuedTransfers()
                    }
                }

                Item {
                    id: closeButton
                    width: 20
                    height: 20

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("关闭传输队列")
                    Accessible.onPressAction: root.close()

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusXs
                        color: closeMouse.containsMouse ? Theme.hover : "transparent"
                    }

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "close"
                        tint: closeMouse.containsMouse ? root.textColor : root.faintColor
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.close()
                    }
                }
            }
        }

        Flickable {
            id: scroller
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: header.bottom
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            contentWidth: width
            contentHeight: body.height
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            Column {
                id: body
                width: scroller.width

                // The job that is running.
                TransferRow {
                    width: parent.width
                    visible: Workspace.busy
                    name: Workspace.progressLabel
                    target: Workspace.transferDestination
                    status: root.percent + "%"
                    progress: root.percent / 100
                    cancellable: false
                }

                // The jobs behind it, in the order the core will start them.
                Repeater {
                    model: Workspace.queuedTransfers

                    TransferRow {
                        required property int index
                        required property string modelData
                        width: body.width
                        name: modelData
                        // qmllint disable missing-property
                        target: Workspace.queuedTransferPaths.length > index
                            ? Workspace.queuedTransferPaths[index] : ""
                        status: qsTr("等待")
                        progress: 0
                        cancellable: true
                        onCancelRequested: Workspace.cancelQueuedTransfer(index)
                    }
                }

                Item {
                    width: parent.width
                    height: 34
                    visible: !Workspace.busy && root.queuedCount === 0

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("没有排队中的传输")
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                    }
                }
            }
        }
    }

    component TransferRow: Item {
        id: row

        property string name: ""
        /// Where this job is copying to. Printed under the name in the faint
        /// ink: the title says what is moving, this says where to.
        property string target: ""
        property string status: ""
        property real progress: 0
        property bool cancellable: false

        signal cancelRequested()

        width: parent ? parent.width : 0
        height: row.target !== "" ? 52 : 40

        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 4
            anchors.rightMargin: 4
            radius: Theme.radiusSm
            color: rowMouse.containsMouse ? Theme.hover : "transparent"
        }

        Column {
            x: 8
            y: 5
            width: row.width - 16
            spacing: 3

            Item {
                width: parent.width
                height: 20

                Text {
                    anchors.left: parent.left
                    anchors.right: statusText.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.name
                    color: root.textColor
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                }

                Text {
                    id: statusText
                    anchors.right: row.cancellable ? cancelButton.left : parent.right
                    anchors.rightMargin: row.cancellable ? 4 : 0
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.status
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                    textFormat: Text.PlainText
                }

                Item {
                    id: cancelButton
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    width: row.cancellable ? 18 : 0
                    height: 18
                    visible: row.cancellable

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("取消 ") + row.name
                    Accessible.onPressAction: row.cancelRequested()

                    Glyph {
                        anchors.centerIn: parent
                        width: 10
                        height: 10
                        kind: "close"
                        tint: cancelMouse.containsMouse ? root.textColor : root.faintColor
                    }

                    MouseArea {
                        id: cancelMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: row.cancelRequested()
                    }
                }
            }

            Text {
                width: parent.width
                height: row.target !== "" ? implicitHeight : 0
                visible: row.target !== ""
                text: row.target
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideMiddle
                textFormat: Text.PlainText
            }

            Rectangle {
                width: parent.width
                height: 2
                color: Theme.hairlineStrong

                Rectangle {
                    width: parent.width * Math.max(0, Math.min(1, row.progress))
                    height: parent.height
                    color: Theme.signal
                }
            }
        }

        MouseArea {
            id: rowMouse
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
        }
    }
}
