pragma ComponentBehavior: Bound
import QtQuick

// A desktop segmented pill control (Chrome / M3 Expressive toolbar switcher).
// Segments sit inside a rounded capsule track with 2 dp inner padding; the
// chosen segment fills with `secondaryContainer` and a soft ink ripple plays
// on pointer press.
FocusScope {
    id: group

    /// [{ id, label, glyph }] — `glyph` is a `Glyph` kind; `label` is optional
    /// (an icon-only segment omits it).
    property var model: []
    property string currentId: ""
    property int segmentHeight: Theme.controlMd
    property int iconEdge: 16
    /// Horizontal padding inside a labelled segment; icon-only segments are
    /// circular/square instead.
    property int labelPad: 14

    signal selected(string id)

    implicitWidth: track.implicitWidth
    implicitHeight: segmentHeight
    height: segmentHeight

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("选项组")

    Rectangle {
        id: track
        implicitWidth: row.implicitWidth + 4
        implicitHeight: group.segmentHeight
        width: implicitWidth
        height: implicitHeight
        radius: height / 2
        color: Theme.field

        Row {
            id: row
            x: 2
            y: 2
            height: group.segmentHeight - 4
            spacing: 2

            Repeater {
                id: segRepeater
                model: group.model

                Rectangle {
                    id: segment
                    required property int index
                    required property var modelData

                    readonly property bool chosen: group.currentId === modelData.id
                    readonly property bool labelled: (modelData.label || "") !== ""
                    readonly property int segH: row.height

                    width: labelled ? label.implicitWidth + group.labelPad * 2
                                    : segH + 4
                    height: segH
                    radius: segH / 2
                    color: chosen ? Theme.selection
                        : (segmentMouse.pressed ? Theme.surfaceContainerHighest
                            : (segmentMouse.containsMouse ? Theme.hover
                                                          : "transparent"))
                    opacity: enabled ? 1 : 0.38
                    border.width: segment.activeFocus ? 2 : 0
                    border.color: Theme.accent

                    Behavior on color {
                        enabled: Theme.motion
                        ColorAnimation {
                            duration: Theme.motionFast
                            easing.type: Theme.motionEaseEffects
                        }
                    }

                    InkRipple {
                        id: segRipple
                        radius: segment.radius
                    }

                    Accessible.role: Accessible.RadioButton
                    Accessible.name: segment.labelled ? segment.modelData.label
                                                      : segment.modelData.id
                    Accessible.checkable: true
                    Accessible.checked: segment.chosen
                    Accessible.onPressAction: group.selected(segment.modelData.id)

                    activeFocusOnTab: true
                    Keys.onReturnPressed: {
                        segRipple.pulse()
                        group.selected(segment.modelData.id)
                    }
                    Keys.onEnterPressed: {
                        segRipple.pulse()
                        group.selected(segment.modelData.id)
                    }
                    Keys.onSpacePressed: {
                        segRipple.pulse()
                        group.selected(segment.modelData.id)
                    }
                    Keys.onLeftPressed: {
                        const prev = segRepeater.itemAt((segment.index - 1 + group.model.length)
                                                        % group.model.length)
                        if (prev)
                            prev.forceActiveFocus()
                    }
                    Keys.onRightPressed: {
                        const next = segRepeater.itemAt((segment.index + 1) % group.model.length)
                        if (next)
                            next.forceActiveFocus()
                    }

                    Glyph {
                        anchors.centerIn: parent
                        visible: (segment.modelData.glyph || "") !== ""
                        width: group.iconEdge
                        height: group.iconEdge
                        kind: segment.modelData.glyph || ""
                        tint: segment.chosen ? Theme.selectionInk : Theme.inkOnSurfaceVariant
                    }

                    Text {
                        id: label
                        anchors.centerIn: parent
                        visible: segment.labelled
                        text: segment.modelData.label || ""
                        color: segment.chosen ? Theme.selectionInk : Theme.inkOnSurfaceVariant
                        font.pixelSize: Theme.typeCaption
                        font.weight: segment.chosen ? Theme.weightStrong : Font.Normal
                    }

                    MouseArea {
                        id: segmentMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPressed: function (mouse) { segRipple.burst(mouse.x, mouse.y) }
                        onReleased: segRipple.release()
                        onCanceled: segRipple.release()
                        onClicked: group.selected(segment.modelData.id)
                    }
                }
            }
        }
    }
}