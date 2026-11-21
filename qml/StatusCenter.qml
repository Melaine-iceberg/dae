pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The status center: what this shell did lately, and what failed.
//
// The transfer queue above the status bar answers "what is moving now". This
// panel answers the other question — "what happened, and can I try the failed
// one again". The rows come from `Workspace.operationsJson`, which the Rust
// side records around every command and persists 50 of across launches; the
// panel itself keeps no state, so reopening it always shows the same list.
//
// Retry is offered only for the kinds that can mean the same thing twice: a
// copy, a move and an extraction. A delete that removed a file has nothing to
// retry against, and the Rust side leaves its retry payload empty.
Item {
    id: root

    property bool dark: true
    property color accent: Theme.accent

    visible: false
    opacity: visible ? 1 : 0
    Behavior on opacity {
        enabled: Theme.motion
        OpacityAnimator { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }
    focus: visible
    z: 1003

    readonly property color panelColor: Theme.raised
    readonly property color borderColor: Theme.overlayLine
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color faintColor: Theme.inkTertiary

    readonly property var records: {
        let parsed = []
        try {
            parsed = JSON.parse(Workspace.operationsJson)
        } catch (error) {
            parsed = []
        }
        return parsed
    }
    readonly property int failureCount: Workspace.failureCount

    function openAbove(x, y) {
        root.visible = true
        // Opening the panel is what clears the badge; the rows stay until the
        // user clears them.
        Workspace.markOperationsRead()
        root.forceActiveFocus()
    }

    function close() {
        root.visible = false
    }

    function statusText(record) {
        if (record.status === "failed")
            return record.detail !== "" ? record.detail : qsTr("失败")
        return record.detail
    }

    function timeText(record) {
        const stamp = new Date(record.finished)
        return Qt.formatDateTime(stamp, "MM-dd HH:mm")
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

        width: 380
        height: Math.min(420, headerRow.height + Math.max(96, list.contentHeight) + 20)
        x: Math.round((root.width - width) / 2)
        y: root.height - height - 32
        radius: Theme.radiusXl
        color: "transparent"
        border.width: 0

        Glass { radius: Theme.radiusXl }

        Item {
            id: headerRow
            width: parent.width
            height: 44

            Text {
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("状态中心")
                color: root.mutedColor
                font.pixelSize: Theme.typeCaption
                font.weight: Theme.weightEmphasis
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                spacing: 4

                Item {
                    width: 64
                    height: Theme.controlMd

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("清除记录")
                    Accessible.onPressAction: Workspace.clearOperationHistory()

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusSm
                        color: clearMouse.containsMouse
                            ? Qt.rgba(Theme.accent.r, Theme.accent.g,
                                      Theme.accent.b, 0.08) : "transparent"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("清除记录")
                        color: clearMouse.containsMouse ? Theme.accent : root.mutedColor
                        font.pixelSize: Theme.typeMicro
                        textFormat: Text.PlainText
                    }

                    MouseArea {
                        id: clearMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: Workspace.clearOperationHistory()
                    }
                }

                Item {
                    width: Theme.controlMd
                    height: Theme.controlMd

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("关闭")
                    Accessible.onPressAction: root.close()

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radiusFull
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

        ListView {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: headerRow.bottom
            anchors.bottom: parent.bottom
            anchors.margins: 8
            anchors.topMargin: 0
            clip: true
            model: root.records
            spacing: 1

            Text {
                anchors.centerIn: parent
                visible: list.count === 0
                text: qsTr("还没有操作记录")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
            }

            delegate: Rectangle {
                id: recordRow
                required property var modelData
                required property int index

                width: ListView.view ? ListView.view.width : 0
                height: content.height + 14
                radius: Theme.radiusMd
                color: rowHover.hovered ? Theme.hover : "transparent"

                HoverHandler { id: rowHover }

                Rectangle {
                    x: 6
                    y: 10
                    width: 6
                    height: 6
                    radius: 3
                    // A cancelled transfer is neither: it stopped on purpose,
                    // and its row offers "continue" rather than a diagnosis.
                    color: recordRow.modelData.status === "failed"
                        ? Theme.danger
                        : (recordRow.modelData.status === "cancelled" ? Theme.warning : Theme.success)
                }

                Column {
                    id: content
                    x: 18
                    y: 7
                    width: parent.width - 150

                    Text {
                        width: parent.width
                        text: recordRow.modelData.label
                        color: root.textColor
                        font.pixelSize: Theme.typeCaption
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                    }

                    Text {
                        width: parent.width
                        visible: text !== ""
                        text: recordRow.modelData.status === "failed"
                            ? root.statusText(recordRow.modelData)
                            : (recordRow.modelData.status === "cancelled"
                                ? qsTr("已取消") + " · " + root.timeText(recordRow.modelData)
                                : root.timeText(recordRow.modelData))
                        color: recordRow.modelData.status === "failed"
                            ? Theme.danger
                            : (recordRow.modelData.status === "cancelled"
                                ? root.mutedColor : root.faintColor)
                        font.pixelSize: Theme.typeMicro
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                    }
                }

                Row {
                    anchors.right: parent.right
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    Item {
                        id: elevate
                        width: 56
                        height: Theme.controlSm
                        // Only a failed copy or move on a platform that can
                        // raise a separate process; the row's retry payload is
                        // what the elevated job is built from.
                        visible: recordRow.modelData.status === "failed"
                            && recordRow.modelData.elevatable === true

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("以管理员身份重试")
                        Accessible.onPressAction: Workspace.elevateOperation(recordRow.modelData.id)

                        Text {
                            anchors.centerIn: parent
                            text: qsTr("提权重试")
                            color: elevateMouse.containsMouse ? Theme.accent : root.mutedColor
                            font.pixelSize: Theme.typeMicro
                            textFormat: Text.PlainText
                        }

                        MouseArea {
                            id: elevateMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: Workspace.elevateOperation(recordRow.modelData.id)
                        }
                    }

                    Item {
                        id: retry
                        width: 44
                        height: Theme.controlSm
                        // A cancelled copy is continued, not retried: the
                        // record carries `resume`, so the action merges into
                        // what already landed instead of asking about it.
                        visible: recordRow.modelData.retryable === true
                            && (recordRow.modelData.status === "failed"
                                || recordRow.modelData.status === "cancelled")

                        Accessible.role: Accessible.Button
                        Accessible.name: recordRow.modelData.status === "cancelled" ? qsTr("继续") : qsTr("重试")
                        Accessible.onPressAction: Workspace.retryOperation(recordRow.modelData.id)

                        Text {
                            anchors.centerIn: parent
                            text: recordRow.modelData.status === "cancelled" ? qsTr("继续") : qsTr("重试")
                            color: retryMouse.containsMouse ? Theme.accent : root.mutedColor
                            font.pixelSize: Theme.typeMicro
                            textFormat: Text.PlainText
                        }

                        MouseArea {
                            id: retryMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: Workspace.retryOperation(recordRow.modelData.id)
                        }
                    }
                }
            }
        }
    }
}
