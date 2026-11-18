import QtQuick
import dae 1.0
import "DialogFocus.js" as DialogFocus
import "Localize.js" as Localize

// The transfer-conflict dialog: one collision at a time, with the Tauri shell's
// three answers and its "apply to all" escape hatch.
//
// Several jobs can be asking at once — the slots run whole jobs in parallel —
// so `Workspace` queues their prompts and this file renders the front one.
// The job on screen is `Workspace.conflictJobId`; every answer and dismissal
// carries it, and when the front job resolves the next job's question repaints
// this same dialog. `conflictIndex` alone still drives visibility (no
// `open()`/`close()` pair to keep in sync): it advances within one job's
// batch, and drops to 0 only when no prompt is left.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.accent

    /// The tick belongs to the question on screen, not to the transfer: it
    /// is cleared whenever the dialog goes away or the job on screen
    /// changes, so a later job starts fresh.
    property bool applyToAll: false

    visible: Workspace.conflictIndex > 0
    z: 1003
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("文件已存在")

    readonly property string verb: Workspace.conflictCut ? qsTr("移动") : qsTr("复制")
    readonly property int remaining: Workspace.conflictTotal - Workspace.conflictIndex
    readonly property color panelColor: Theme.raised
    readonly property color panelBorder: Theme.overlayLine
    readonly property color headingColor: Theme.ink
    readonly property color bodyColor: Theme.inkSecondary
    readonly property color mutedColor: Theme.inkSecondary

    onVisibleChanged: {
        if (visible) {
            root.forceActiveFocus()
        } else {
            root.applyToAll = false
            root.modalClosed()
        }
    }

    signal modalClosed()

    Keys.onEscapePressed: Workspace.cancelConflict(Workspace.conflictJobId)
    Keys.onTabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, true)
    }
    Keys.onBacktabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, false)
    }

    // A new job's question replacing this one is a new question; the tick
    // never carries across jobs, only within one job's own batch.
    Connections {
        target: Workspace
        function onConflictJobIdChanged() { root.applyToAll = false }
    }

    function answer(action) {
        Workspace.resolveConflict(Workspace.conflictJobId, action, root.applyToAll)
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        // Dismissing the scrim cancels the whole batch, which is what the Tauri
        // shell's dialog did when it was closed: nothing is transferred.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: Workspace.cancelConflict(Workspace.conflictJobId)
        }
    }

    /// The plate's hard shadow, declared before the plate so it paints
    /// behind it (see `Shadow.qml`).
    Shadow {
        target: panel
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: Math.min(520, root.width - 48)
        height: content.implicitHeight + 36
        radius: Theme.radiusLg
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

            DialogHeader {
                width: parent.width
                title: qsTr("替换或跳过文件")
                subtitle: Workspace.conflictTotal > 1
                    ? qsTr("第 %1 / %2 个冲突")
                        .arg(Workspace.conflictIndex).arg(Workspace.conflictTotal)
                    : qsTr("“%1”已存在").arg(Workspace.conflictName)
                // Dismissing cancels the batch — the same answer the scrim
                // gives, because a half-answered transfer is not a state this
                // dialog can leave behind.
                onCloseRequested: Workspace.cancelConflict(Workspace.conflictJobId)
            }

            // Other jobs may be asking behind this one; saying so is the
            // difference between a queue and a surprise.
            Text {
                width: parent.width
                visible: Workspace.conflictWaiting > 0
                height: visible ? implicitHeight : 0
                text: qsTr("回答后还有 %1 个传输等待确认").arg(Workspace.conflictWaiting)
                color: root.mutedColor
                font.pixelSize: Theme.typeCaption
                wrapMode: Text.WordWrap
            }

            Column {
                width: parent.width
                spacing: 2

                Text {
                    width: parent.width
                    text: qsTr("“%1”已存在于目标文件夹，请选择%2方式。")
                        .arg(Workspace.conflictName).arg(root.verb)
                    color: root.bodyColor
                    font.pixelSize: Theme.typeCaption
                    lineHeight: Theme.leadingTight
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
                    detail: Localize.message(Workspace.conflictSourceDetail)
                }

                ConflictSide {
                    width: (parent.width - parent.spacing) / 2
                    dark: root.dark
                    accent: root.accent
                    highlight: true
                    heading: qsTr("目标文件夹中的现有项目")
                    name: Workspace.conflictName
                    category: Workspace.conflictTargetCategory
                    detail: Localize.message(Workspace.conflictTargetDetail)
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
                    radius: Theme.radiusSm
                    color: root.applyToAll ? root.accent : "transparent"
                    border.width: 1
                    border.color: root.applyToAll ? root.accent
                        : Theme.scrim

                    Text {
                        anchors.centerIn: parent
                        visible: root.applyToAll
                        text: "\u2713"
                        color: Theme.onAccent
                        font.pixelSize: Theme.typeMicro
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
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideRight
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: root.applyToAll = !root.applyToAll
                }
            }

            Item {
                width: parent.width
                height: Theme.controlMd

                // "取消复制" cancels the batch rather than the one item: the
                // Tauri shell's wording made the same promise.
                Rectangle {
                    id: cancelButton
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: cancelLabel.implicitWidth + 24
                    height: Theme.controlMd
                    radius: Theme.radiusSm
                    color: cancelMouse.containsMouse
                        ? Theme.hover : "transparent"
                    border.width: 1
                    border.color: Theme.hairline

                    Accessible.role: Accessible.Button
                    Accessible.name: cancelLabel.text
                    Accessible.onPressAction: Workspace.cancelConflict(Workspace.conflictJobId)

                    Text {
                        id: cancelLabel
                        anchors.centerIn: parent
                        text: qsTr("取消%1").arg(root.verb)
                        color: Theme.inkSecondary
                        font.pixelSize: Theme.typeCaption
                    }

                    MouseArea {
                        id: cancelMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: Workspace.cancelConflict(Workspace.conflictJobId)
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
                    height: Theme.controlMd
                    radius: Theme.radiusSm
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
                                color: Theme.inkSecondary
                                font.pixelSize: Theme.typeCaption
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
                                color: Theme.inkSecondary
                                font.pixelSize: Theme.typeCaption
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
                                return replaceMouse.containsMouse ? Theme.accentHover : root.accent
                            }

                            Accessible.role: Accessible.Button
                            Accessible.name: replaceLabel.text
                            Accessible.onPressAction: root.answer("replace")

                            Text {
                                id: replaceLabel
                                anchors.centerIn: parent
                                text: qsTr("替换")
                                color: root.applyToAll ? Theme.onDanger : Theme.onAccent
                                font.pixelSize: Theme.typeCaption
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
