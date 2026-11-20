import QtQuick

// The M3 Expressive switch: a wide tonal track and a handle that grows when
// selected and grows again under the press, riding the fast spatial spring.
// The selected handle carries a check, so the state reads at a glance without
// relying on colour alone. Desktop density — 40×22 rather than the phone's
// 52×32 — because it sits in a settings row, not a touch list.
Item {
    id: root

    property bool checked: false
    signal clicked()

    implicitWidth: 40
    implicitHeight: 22
    width: implicitWidth
    height: implicitHeight

    Accessible.role: Accessible.CheckBox
    Accessible.checkable: true
    Accessible.checked: root.checked
    Accessible.name: qsTr("开关")
    Accessible.onPressAction: root.clicked()

    activeFocusOnTab: true
    Keys.onSpacePressed: root.clicked()
    Keys.onReturnPressed: root.clicked()

    readonly property real handleRest: root.checked ? 18 : 14
    readonly property real handleSize: mouse.pressed ? 20 : handleRest
    readonly property real handleX: root.checked
        ? root.width - handleSize - 3 : 3

    Rectangle {
        id: track
        anchors.fill: parent
        radius: height / 2
        color: root.checked ? Theme.accent : Theme.surfaceContainerHighest
        border.width: root.checked ? 0 : 2
        border.color: Theme.outline
        opacity: root.enabled ? 1 : 0.38

        Behavior on color {
            enabled: Theme.motion
            ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
        }
    }

    Rectangle {
        id: handle
        x: root.handleX
        anchors.verticalCenter: parent.verticalCenter
        width: root.handleSize
        height: root.handleSize
        radius: height / 2
        color: root.checked ? Theme.inkOnAccent : Theme.outline

        Behavior on x {
            enabled: Theme.motion
            SpringAnimation {
                spring: Theme.springSpatialFast
                damping: Theme.dampSpatialFast
                epsilon: 0.25
            }
        }
        Behavior on width {
            enabled: Theme.motion
            SpringAnimation {
                spring: Theme.springSpatialFast
                damping: Theme.dampSpatialFast
                epsilon: 0.25
            }
        }
        Behavior on height {
            enabled: Theme.motion
            SpringAnimation {
                spring: Theme.springSpatialFast
                damping: Theme.dampSpatialFast
                epsilon: 0.25
            }
        }
        Behavior on color {
            enabled: Theme.motion
            ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
        }

        Glyph {
            anchors.centerIn: parent
            width: 10
            height: 10
            visible: root.checked
            opacity: root.checked ? 1 : 0
            kind: "check"
            tint: Theme.accent
            Behavior on opacity {
                enabled: Theme.motion
                OpacityAnimator { duration: Theme.motionFast }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: "transparent"
        border.width: root.activeFocus ? 2 : 0
        border.color: Theme.accent
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }
}