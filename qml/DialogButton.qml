import QtQuick

// The button every dialog in the shell uses: 28 high, 4 px corner, three
// tones. The dialogs used to each carry their own 26 px rectangle with a 6 px
// corner and their own idea of what "primary" looked like; a shared component
// is what keeps "取消 → 主操作" and the destructive plate from drifting again.
Rectangle {
    id: root

    property string label: ""
    /// "quiet" (cancel), "primary" (signal plate) or "danger" (solid red).
    property string tone: "quiet"
    /// Drawn as a 1 px ring. Tab moves it between the buttons of a dialog, so
    /// it has to be visible on all three tones — white on the saturated ones,
    /// the signal colour on the quiet one.
    property bool focused: false

    signal clicked()

    implicitWidth: Math.max(64, labelText.implicitWidth + 24)
    implicitHeight: 28
    radius: Theme.radiusSm
    color: {
        if (root.tone === "danger")
            return buttonMouse.containsMouse ? Theme.dangerSolidHover : Theme.dangerSolid
        if (root.tone === "primary")
            return buttonMouse.containsMouse ? Theme.interactiveHover : Theme.interactive
        return buttonMouse.containsMouse ? Theme.hover : Theme.canvas
    }
    border.width: 1
    border.color: {
        if (root.focused)
            return root.tone === "quiet" ? Theme.signal : Theme.onInteractive
        return root.tone === "quiet" ? Theme.hairlineStrong : "transparent"
    }

    Accessible.role: Accessible.Button
    Accessible.name: root.label
    Accessible.onPressAction: root.clicked()

    Text {
        id: labelText
        anchors.centerIn: parent
        text: root.label
        color: root.tone === "quiet" ? Theme.ink : Theme.onInteractive
        font.pixelSize: Theme.typeSmall
    }

    MouseArea {
        id: buttonMouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }
}
