import QtQuick

// The Chrome GM3 desktop dialog button: 32 high full pill (`radius: height / 2`),
// outlined secondary / filled primary / filled danger with pointer-originated
// RHI ink ripple.
Rectangle {
    id: root

    property string label: ""
    /// "quiet" (cancel/outlined), "primary" (filled), "danger" (filled error).
    property string tone: "quiet"
    property bool focused: false

    signal clicked()

    activeFocusOnTab: true
    Keys.onReturnPressed: if (root.enabled) { buttonRipple.pulse(); root.clicked() }
    Keys.onEnterPressed: if (root.enabled) { buttonRipple.pulse(); root.clicked() }
    Keys.onSpacePressed: if (root.enabled) { buttonRipple.pulse(); root.clicked() }

    implicitWidth: Math.max(72, labelText.implicitWidth + 36)
    implicitHeight: Theme.controlLg
    opacity: enabled ? 1 : 0.38
    radius: height / 2
    color: {
        if (root.tone === "danger")
            return buttonMouse.containsMouse ? Theme.dangerSolidHover : Theme.dangerSolid
        if (root.tone === "primary")
            return buttonMouse.containsMouse ? Theme.accentHover : Theme.accent
        return buttonMouse.containsMouse || root.focused || root.activeFocus
            ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
            : "transparent"
    }
    border.width: root.focused || root.activeFocus ? 2 : (root.tone === "quiet" ? 1 : 0)
    border.color: {
        if (root.focused || root.activeFocus) {
            if (root.tone === "quiet")
                return Theme.accent
            return root.tone === "danger" ? Theme.inkOnDanger : Theme.inkOnAccent
        }
        return Theme.outlineVariant
    }

    Behavior on color {
        enabled: Theme.motion
        ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
    }

    InkRipple {
        id: buttonRipple
        radius: root.height / 2
        color: root.tone === "quiet" ? Theme.ripple : Theme.accentRipple
    }

    Accessible.role: Accessible.Button
    Accessible.name: root.label
    Accessible.onPressAction: if (root.enabled) root.clicked()

    Text {
        id: labelText
        anchors.centerIn: parent
        text: root.label
        color: root.tone === "quiet" ? Theme.accent
            : (root.tone === "danger" ? Theme.inkOnDanger : Theme.inkOnAccent)
        font.pixelSize: Theme.typeBody
        font.weight: Theme.weightStrong
    }

    MouseArea {
        id: buttonMouse
        anchors.fill: parent
        enabled: root.enabled
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onPressed: function (mouse) { buttonRipple.burst(mouse.x, mouse.y) }
        onReleased: buttonRipple.release()
        onCanceled: buttonRipple.release()
        onClicked: root.clicked()
    }
}
