pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Localize.js" as Localize

// Every transfer the scheduler has accepted, running or waiting on a conflict
// answer, one row each; behind them the jobs still in the waiting queue.
//
// Running rows carry a transport — pause, resume and cancel — because the core
// holds a `TransferControl` per operation; cancelling removes the file it was
// writing and keeps the entries that already landed, which is what lets the
// status center offer "continue" instead of "start over". A row still in its
// pre-run phase — checking or awaiting answers — carries a ✕ that dismisses
// that job alone: other jobs' questions and every running row are untouched.
// Every queued row carries a ✕ as well, and that one simply means "drop it
// from the queue".
//
// The header's rate menu is the only place the transfer limit is set: one
// process-wide value, obeyed by every running transfer on its next chunk.
//
// A 326 px plate over the bottom of the window, the way the prototype stacks
// it: it is a read-out on the status bar, not a dialog.
// See docs/ui-instrument-migration.md §7.3.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.accent

    visible: false
    /// Arrives rather than appears. Hiding stays instant: a menu that fades
    /// out is a menu that ignores the next click.
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        OpacityAnimator { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1002

    readonly property color panelColor: Theme.raised
    readonly property color borderColor: Theme.line
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color faintColor: Theme.inkTertiary
    // qmllint disable missing-property
    readonly property int queuedCount: Workspace.queuedTransfers.length

    /// The accepted jobs, mirrored into a ListModel so a progress tick updates
    /// the row in place instead of rebuilding it (a rebuilt row would drop the
    /// hover state of the very button the pointer is resting on). Rows keep
    /// their arrival order: the shell only ever appends and removes.
    ListModel { id: transferModel }

    readonly property string transfersJson: Workspace.transfersJson
    onTransfersJsonChanged: root.syncTransfers()
    Component.onCompleted: root.syncTransfers()

    function syncTransfers() {
        const rows = JSON.parse(root.transfersJson)
        for (let i = transferModel.count - 1; i >= 0; i -= 1) {
            const id = transferModel.get(i).jobId
            if (!rows.some(row => row.jobId === id))
                transferModel.remove(i)
        }
        for (const row of rows) {
            let index = -1
            for (let i = 0; i < transferModel.count; i += 1) {
                if (transferModel.get(i).jobId === row.jobId) {
                    index = i
                    break
                }
            }
            if (index >= 0)
                transferModel.set(index, row)
            else
                transferModel.append(row)
        }
    }

    /// The rate menu's presets, in KiB/s; `0` is "no limit". The value the core
    /// holds is what the button prints, so a limit set in `settings.toml` shows
    /// up here even if it is not one of these.
    readonly property var speedPresets: [
        { kib: 0, label: qsTr("不限速") },
        { kib: 1024, label: "1 MB/s" },
        { kib: 5120, label: "5 MB/s" },
        { kib: 10240, label: "10 MB/s" },
        { kib: 20480, label: "20 MB/s" },
        { kib: 51200, label: "50 MB/s" }
    ]

    property bool speedMenuOpen: false

    readonly property string speedLimitLabel: {
        // qmllint disable missing-property
        const kib = Workspace.transferSpeedLimit
        if (kib <= 0)
            return qsTr("限速")
        return (kib / 1024).toFixed(kib % 1024 === 0 ? 0 : 1) + " MB/s"
    }

    /// Kept for the shell's call site. The plate is anchored to the bottom of
    /// the window rather than to the chip: at 326 px it would otherwise jump
    /// sideways every time the progress digits changed width.
    function openAbove(x, y) {
        root.visible = true
        root.forceActiveFocus()
    }

    function close() {
        root.speedMenuOpen = false
        root.visible = false
    }

    Keys.onEscapePressed: root.close()

    MouseArea {
        Accessible.ignored: true
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    /// The plate's hard shadow, declared before the plate so it paints
    /// behind it (see `Shadow.qml`).
    Shadow {
        target: panel
    }

    Rectangle {
        id: panel

        width: 326
        height: Math.min(320, body.height + header.height + 8)
        x: Math.round((root.width - width) / 2)
        y: root.height - height - 32
        radius: Theme.radiusLg
        color: root.panelColor
        border.width: 1
        border.color: root.borderColor

        Item {
            id: header
            width: parent.width
            height: Theme.controlMd

            Text {
                x: 10
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("传输队列")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                Item {
                    id: speedButton
                    width: 56
                    height: Theme.controlSm

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("传输限速")
                    Accessible.onPressAction: root.speedMenuOpen = !root.speedMenuOpen

                    Text {
                        id: speedText
                        anchors.centerIn: parent
                        text: root.speedLimitLabel
                        color: root.speedMenuOpen
                            ? Theme.accent
                            : (speedMouse.containsMouse ? root.textColor : root.faintColor)
                        font.pixelSize: Theme.typeMicro
                        font.features: { "tnum": 1 }
                        textFormat: Text.PlainText
                    }

                    MouseArea {
                        id: speedMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.speedMenuOpen = !root.speedMenuOpen
                    }
                }

                Item {
                    id: cancelAll
                    width: 52
                    height: Theme.controlSm
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
                    height: Theme.controlXs

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

        // The rate menu. Above the header, so it never covers the rows the
        // user is watching; it closes on selection, on Esc and with the plate.
        Rectangle {
            id: speedMenu

            visible: root.speedMenuOpen
            width: 132
            height: speedColumn.height + 8
            x: panel.width - width - 8
            y: -height - 6
            z: 2
            radius: Theme.radiusMd
            color: Theme.raised
            border.width: 1
            border.color: root.borderColor

            Column {
                id: speedColumn
                y: 4
                width: parent.width

                Repeater {
                    model: root.speedPresets

                    Rectangle {
                        id: preset
                        required property var modelData

                        width: speedColumn.width
                        height: Theme.controlMd
                        color: presetMouse.containsMouse ? Theme.hover : "transparent"

                        readonly property bool active: Workspace.transferSpeedLimit === modelData.kib

                        Text {
                            x: 10
                            anchors.verticalCenter: parent.verticalCenter
                            text: preset.modelData.label
                            color: preset.active
                                ? Theme.accent
                                : (presetMouse.containsMouse ? root.textColor : root.mutedColor)
                            font.pixelSize: Theme.typeCaption
                            textFormat: Text.PlainText
                        }

                        Glyph {
                            anchors.right: parent.right
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            width: 12
                            height: 12
                            visible: preset.active
                            kind: "check"
                            tint: Theme.accent
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: preset.modelData.label
                        Accessible.onPressAction: {
                            Workspace.applyTransferSpeedLimit(preset.modelData.kib)
                            root.speedMenuOpen = false
                        }

                        MouseArea {
                            id: presetMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: {
                                Workspace.applyTransferSpeedLimit(preset.modelData.kib)
                                root.speedMenuOpen = false
                            }
                        }
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

                // The accepted jobs, in arrival order. Each running row is a
                // real transport: the core holds a control per operation, so
                // pause and cancel address this row's job and no other.
                Repeater {
                    model: transferModel

                    TransferRow {
                        required property var model

                        width: body.width
                        name: Localize.message(model.name)
                        target: model.target
                        status: {
                            if (model.paused)
                                return qsTr("已暂停")
                            if (!model.running)
                                return qsTr("准备中")
                            if (model.total > 0) {
                                let text = model.percent + "%"
                                if (model.speed !== "")
                                    text += " · " + model.speed
                                if (model.eta !== "" && model.eta !== undefined)
                                    text += " · " + qsTr("剩余 %1").arg(model.eta)
                                return text
                            }
                            return model.speed
                        }
                        progress: model.percent / 100
                        cancellable: true
                        pausable: model.running
                        paused: model.paused
                        onCancelRequested: {
                            if (model.running)
                                Workspace.cancelTransfer(model.jobId)
                            else
                                Workspace.cancelConflict(model.jobId)
                        }
                        onPauseToggled: {
                            if (model.paused)
                                Workspace.resumeTransfer(model.jobId)
                            else
                                Workspace.pauseTransfer(model.jobId)
                        }
                    }
                }

                // The jobs behind it, in the order the core will start them.
                Repeater {
                    model: Workspace.queuedTransfers

                    TransferRow {
                        required property int index
                        required property string modelData
                        width: body.width
                        name: Localize.message(modelData)
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
                    height: Theme.controlXl
                    visible: transferModel.count === 0 && root.queuedCount === 0

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
        /// The running row only: the core can hold a transfer, so the row
        /// offers Pause (or Resume) as well as Cancel.
        property bool pausable: false
        property bool paused: false

        signal cancelRequested()
        signal pauseToggled()

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
                height: Theme.controlXs

                Text {
                    anchors.left: parent.left
                    anchors.right: actions.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.name
                    color: root.textColor
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                }

                Row {
                    id: actions
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.status
                        color: root.faintColor
                        font.pixelSize: Theme.typeMicro
                        font.features: { "tnum": 1 }
                        textFormat: Text.PlainText
                    }

                    Item {
                        id: pauseButton
                        width: row.pausable ? 18 : 0
                        height: Theme.controlXs
                        visible: row.pausable

                        Accessible.role: Accessible.Button
                        Accessible.name: row.paused ? qsTr("继续 ") + row.name : qsTr("暂停 ") + row.name
                        Accessible.onPressAction: row.pauseToggled()

                        Glyph {
                            anchors.centerIn: parent
                            width: 10
                            height: 10
                            kind: row.paused ? "play" : "pause"
                            tint: pauseMouse.containsMouse ? root.textColor : root.faintColor
                        }

                        MouseArea {
                            id: pauseMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: row.pauseToggled()
                        }
                    }

                    Item {
                        id: cancelButton
                        anchors.verticalCenter: parent.verticalCenter
                        width: row.cancellable ? 18 : 0
                        height: Theme.controlXs
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
                color: Theme.line

                Rectangle {
                    width: parent.width * Math.max(0, Math.min(1, row.progress))
                    height: parent.height
                    color: Theme.accent
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
