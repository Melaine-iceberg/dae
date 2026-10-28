import QtQuick

// One line of feedback, used for operation results and errors. Long operations
// already have the status bar's progress row; this is only for the outcome.
Item {
    id: root

    property bool dark: true
    property int duration: 2600

    visible: opacity > 0.01
    opacity: 0
    z: 900

    Behavior on opacity {
        enabled: Theme.motion
        NumberAnimation { duration: Theme.motionOverlay; easing.type: Easing.OutCubic }
    }

    function show(message, isError) {
        label.text = message
        label.color = Theme.onSolid
        background.color = isError
            ? Theme.dangerSolid
            : (root.dark ? Qt.rgba(0.13, 0.15, 0.18, 0.94)
                         : Qt.rgba(0.15, 0.17, 0.20, 0.92))
        root.opacity = 1
        timer.restart()
    }

    Timer {
        id: timer
        interval: root.duration
        onTriggered: root.opacity = 0
    }

    Rectangle {
        id: background
        anchors.centerIn: parent
        width: label.implicitWidth + 28
        height: 30
        radius: 16
        color: Theme.raised

        Text {
            id: label
            anchors.centerIn: parent
            text: ""
            color: Theme.onInteractive
            font.pixelSize: 12
        }
    }
}
