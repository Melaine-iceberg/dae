import QtQuick
import dae 1.0

// The transfer-conflict dialog: one collision at a time, with the Tauri shell's
// three answers and its "apply to all" escape hatch.
//
// The pending batch, its decisions and the index live in `Workspace` — it owns
// the transfer, so it owns the question — which makes this file a view: every
// label binds to a property and the buttons call back into the command surface.
// Two consequences worth knowing: the dialog appears and disappears on
// `conflictIndex` alone (no `open()`/`close()` pair to keep in sync), and
// answering advances the index, so the same dialog immediately shows the next
// conflict.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    /// The tick belongs to the question on screen, not to the transfer: it is
    /// cleared whenever the dialog goes away, so a later batch starts fresh.
    property bool applyToAll: false

    visible: Workspace.conflictIndex > 0
    z: 1003
    focus: visible

    readonly property string verb: Workspace.conflictCut ? qsTr("移动") : qsTr("复制")
    readonly property int remaining: Workspace.conflictTotal - Workspace.conflictIndex
    readonly property color panelColor: Theme.raised
    readonly property color panelBorder: Theme.hairlineStrong
    readonly property color headingColor: Theme.ink
    readonly property color bodyColor: Theme.inkMuted
    readonly property color mutedColor: Theme.inkMuted

    onVisibleChanged: {
        if (visible) {
            root.forceActiveFocus()
        } else {
            root.applyToAll = false
            root.modalClosed()
        }
    }

    signal modalClosed()

    Keys.onEscapePressed: Workspace.cancelConflict()

    function answer(action) {
        Workspace.resolveConflict(action, root.applyToAll)
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        // Dismissing the scrim cancels the whole batch, which is what the Tauri
        // shell's dialog did when it was closed: nothing is transferred.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: Workspace.cancelConflict()
        }
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: Math.min(520, root.width - 48)
        height: content.implicitHeight + 36
        radius: 10
        color: root.panelColor
        border.width: 1
        border.color: root.panelBorder

        // Wheel events must not scroll the listing behind the dialog.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.NoButton
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: content
            x: 18
            y: 18
            width: parent.width - 36
            spacing: 12

            Text {
                width: parent.width
                text: qsTr("替换或跳过文件")
                color: root.headingColor
                font.pixelSize: 14
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }

            Column {
                width: parent.width
                spacing: 2

                Text {
                    width: parent.width
                    visible: Workspace.conflictTotal > 1
                    text: qsTr("第 %1 / %2 个冲突")
                        .arg(Workspace.conflictIndex).arg(Workspace.conflictTotal)
                    color: root.headingColor
                    font.pixelSize: 11
                    elide: Text.ElideRight
                }

                Text {
                    width: parent.width
                    text: qsTr("“%1”已存在于目标文件夹，请选择%2方式。")
                        .arg(Workspace.conflictName).arg(root.verb)
                    color: root.bodyColor
                    font.pixelSize: 12
                    lineHeight: 1.25
                    wrapMode: Text.WordWrap
                }
            }

            Row {
                width: parent.width
                spacing: 10

                ConflictSide {
                    width: (parent.width - parent.spacing) / 2
                    dark: root.dark
                    accent: root.accent
                    heading: qsTr("准备%1的项目").arg(root.verb)
                    name: Workspace.conflictName
                    category: Workspace.conflictSourceCategory
                    detail: Workspace.conflictSourceDetail
                }

                ConflictSide {
                    width: (parent.width - parent.spacing) / 2
                    dark: root.dark
                    accent: root.accent
                    highlight: true
                    heading: qsTr("目标文件夹中的现有项目")
                    name: Workspace.conflictName
                    category: Workspace.conflictTargetCategory
                    detail: Workspace.conflictTargetDetail
                }
            }

            Item {
                width: parent.width
                height: visible ? 20 : 0
                visible: root.remaining > 1

                Accessible.role: Accessible.CheckBox
                Accessible.name: applyAllLabel.text
                Accessible.onPressAction: root.applyToAll = !root.applyToAll

                Rectangle {
                    id: applyAllBox
                    x: 1
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    radius: 3
                    color: root.applyToAll ? root.accent : "transparent"
                    border.width: 1
                    border.color: root.applyToAll ? root.accent
                        : Theme.scrim

                    Text {
                        anchors.centerIn: parent
                        visible: root.applyToAll
                        text: "\u2713"
                        color: Theme.onInteractive
                        font.pixelSize: 10
                    }
                }

                Text {
                    id: applyAllLabel
                    anchors.left: applyAllBox.right
                    anchors.leftMargin: 7
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("应用到剩余全部（") + root.remaining + qsTr(" 个冲突）")
                    color: root.bodyColor
                    font.pixelSize: 12
                    elide: Text.ElideRight
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: root.applyToAll = !root.applyToAll
                }
            }

            Item {
                width: parent.width
                height: 28

                // "取消复制" cancels the batch rather than the one item: the
                // Tauri shell's wording made the same promise.
                Rectangle {
                    id: cancelButton
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: cancelLabel.implicitWidth + 24
                    height: 26
                    radius: 6
                    color: cancelMouse.containsMouse
                        ? Theme.fill : "transparent"
                    border.width: 1
                    border.color: Theme.hairline

                    Accessible.role: Accessible.Button
                    Accessible.name: cancelLabel.text
                    Accessible.onPressAction: Workspace.cancelConflict()

                    Text {
                        id: cancelLabel
                        anchors.centerIn: parent
                        text: qsTr("取消%1").arg(root.verb)
                        color: Theme.inkMuted
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: cancelMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: Workspace.cancelConflict()
                    }
                }

                // One segmented choice instead of three loose buttons: the three
                // outcomes are mutually exclusive answers to the same question,
                // so they share a frame. Only a Replace that will also hit every
                // remaining conflict wears the destructive red — a single
                // replace is an ordinary action.
                Rectangle {
                    id: choice
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    width: skipSegment.width + keepSegment.width + replaceSegment.width
                    height: 26
                    radius: 6
                    color: "transparent"
                    border.width: 1
                    border.color: Theme.hairline
                    clip: true

                    Row {
                        anchors.fill: parent

                        Rectangle {
                            id: skipSegment
                            width: skipLabel.implicitWidth + 26
                            height: parent.height
                            color: skipMouse.containsMouse
                                ? Theme.hover : "transparent"
                            border.width: 1
                            border.color: Theme.hairline

                            Accessible.role: Accessible.Button
                            Accessible.name: skipLabel.text
                            Accessible.onPressAction: root.answer("skip")

                            Text {
                                id: skipLabel
                                anchors.centerIn: parent
                                text: qsTr("跳过")
                                color: Theme.inkMuted
                                font.pixelSize: 12
                            }

                            MouseArea {
                                id: skipMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: root.answer("skip")
                            }
                        }

                        Rectangle {
                            id: keepSegment
                            width: keepLabel.implicitWidth + 26
                            height: parent.height
                            color: keepMouse.containsMouse
                                ? Theme.hover : "transparent"
                            border.width: 1
                            border.color: Theme.hairline

                            Accessible.role: Accessible.Button
                            Accessible.name: keepLabel.text
                            Accessible.onPressAction: root.answer("keepBoth")

                            Text {
                                id: keepLabel
                                anchors.centerIn: parent
                                text: qsTr("两者保留")
                                color: Theme.inkMuted
                                font.pixelSize: 12
                            }

                            MouseArea {
                                id: keepMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: root.answer("keepBoth")
                            }
                        }

                        Rectangle {
                            id: replaceSegment
                            width: replaceLabel.implicitWidth + 30
                            height: parent.height
                            color: {
                                if (root.applyToAll)
                                    return replaceMouse.containsMouse ? Theme.dangerSolidHover : Theme.dangerSolid
                                return replaceMouse.containsMouse ? Theme.interactiveHover : root.accent
                            }

                            Accessible.role: Accessible.Button
                            Accessible.name: replaceLabel.text
                            Accessible.onPressAction: root.answer("replace")

                            Text {
                                id: replaceLabel
                                anchors.centerIn: parent
                                text: qsTr("替换")
                                color: Theme.onInteractive
                                font.pixelSize: 12
                            }

                            MouseArea {
                                id: replaceMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: root.answer("replace")
                            }
                        }
                    }
                }
            }
        }
    }
}
