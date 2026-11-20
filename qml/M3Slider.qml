import QtQuick

// The M3 Expressive slider: a thin vertical handle in `primary` with a gap in
// the track around it, a thick active track and a tonal inactive track. The
// handle widens under the press on the fast spatial spring. `value` is 0–1;
// `moved` fires while dragging and on key steps so the caller can persist it.
Item {
    id: root

    property real value: 0
    property real step: 0.01
    signal moved(real value)

    implicitHeight: 20
    height: implicitHeight

    Accessible.role: Accessible.Slider
    Accessible.name: qsTr("滑杆")
    Accessible.onPressAction: root.moved(root.value)

    activeFocusOnTab: true
    Keys.onLeftPressed: root.moved(Math.max(0, root.value - root.step))
    Keys.onRightPressed: root.moved(Math.min(1, root.value + root.step))

    readonly property real handleX: Math.max(0, Math.min(width, root.value * width))
    readonly property real gap: 3
    readonly property real handleWidth: mouse.pressed ? 6 : 4

    Rectangle {
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        width: Math.max(0, root.handleX - root.gap)
        height: 4
        radius: 2
        color: Theme.accent
    }

    Rectangle {
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: Math.max(0, parent.width - root.handleX - root.gap)
        height: 4
        radius: 2
        color: Theme.secondaryContainer
    }

    Rectangle {
        id: handle
        x: root.handleX - width / 2
        anchors.verticalCenter: parent.verticalCenter
        width: root.handleWidth
        height: parent.height
        radius: width / 2
        color: Theme.accent

        Behavior on width {
            enabled: Theme.motion
            SpringAnimation {
                spring: Theme.springSpatialFast
                damping: Theme.dampSpatialFast
                epsilon: 0.25
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusSm
        color: "transparent"
        border.width: root.activeFocus ? 2 : 0
        border.color: Theme.accent
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onPressed: function (mouse) {
            root.moved(Math.max(0, Math.min(1, mouse.x / root.width)))
        }
        onPositionChanged: function (mouse) {
            if (pressed)
                root.moved(Math.max(0, Math.min(1, mouse.x / root.width)))
        }
    }
}