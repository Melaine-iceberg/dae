import QtQuick

// The M3 Expressive dialog button: a pill, 32 high, in three of the Material
// button styles.
//
//   "quiet"    text button — no container, `primary` label, 8 % state layer
//   "primary"  filled button — `primary` container, `inkOnPrimary` label
//   "danger"   filled error button — `error` container, `inkOnError` label
//
// Keyboard focus draws a 2 px ring inside the pill: `primary` on the text
// button, the ink-on colour on the two filled ones. That keeps focus visible
// on every tone without changing the button's footprint.
Rectangle {
    id: root

    property string label: ""
    /// "quiet" (cancel), "primary" (filled), "danger" (filled error).
    property string tone: "quiet"
    property bool focused: false

    signal clicked()

    activeFocusOnTab: true
    Keys.onReturnPressed: if (root.enabled) root.clicked()
    Keys.onEnterPressed: if (root.enabled) root.clicked()
    Keys.onSpacePressed: if (root.enabled) root.clicked()

    implicitWidth: Math.max(64, labelText.implicitWidth + 32)
    implicitHeight: Theme.controlLg
    opacity: enabled ? 1 : 0.38
    /// Shape morph: the pill squishes toward a 12 dp corner while held and
    /// springs back on release (m3.material.io/styles/shape/shape-morph).
    property real morph: buttonMouse.pressed ? 1 : 0
    readonly property real restRadius: height / 2
    radius: restRadius + (Theme.radiusMorphPress - restRadius) * morph
    Behavior on morph {
        enabled: Theme.motion
        SpringAnimation {
            spring: Theme.springSpatialFast
            damping: Theme.dampSpatialFast
            epsilon: 0.005
        }
    }
    color: {
        if (root.tone === "danger")
            return buttonMouse.containsMouse ? Theme.dangerSolidHover : Theme.dangerSolid
        if (root.tone === "primary")
            return buttonMouse.containsMouse ? Theme.accentHover : Theme.accent
        return buttonMouse.containsMouse || root.focused || root.activeFocus
            ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.08)
            : "transparent"
    }
    border.width: root.focused || root.activeFocus ? 2 : 0
    border.color: {
        if (root.tone === "quiet")
            return Theme.accent
        return root.tone === "danger" ? Theme.inkOnDanger : Theme.inkOnAccent
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
        onClicked: root.clicked()
    }
}
