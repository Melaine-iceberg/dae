pragma ComponentBehavior: Bound
import QtQuick

// The M3 Expressive connected button group (m3.material.io/components/button-groups),
// the component that replaces the segmented button. Segments sit flush with a
// 2 dp gap; the chosen one takes a strong `secondary` fill and morphs from a
// pill to a 12 dp corner, while the rest stay round on a light container tint.
// Selecting changes only that segment's shape — the connected variant leaves
// its neighbours alone — and the morph rides the fast spatial spring, so the
// choice is felt in the silhouette. Single-select, keyboard-navigable.
FocusScope {
    id: group

    /// [{ id, label, glyph }] — `glyph` is a `Glyph` kind; `label` is optional
    /// (an icon-only segment omits it).
    property var model: []
    property string currentId: ""
    property int segmentHeight: Theme.controlMd
    property int iconEdge: 16
    /// Horizontal padding inside a labelled segment; icon-only segments are
    /// square instead.
    property int labelPad: 14

    signal selected(string id)

    implicitWidth: row.implicitWidth
    implicitHeight: segmentHeight
    height: segmentHeight

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("选项组")

    Row {
        id: row
        height: group.height
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
                /// 0 = pill, 1 = the 12 dp pressed/selected corner. The spring
                /// drives this one scalar; the radius follows it.
                property real morph: (segment.chosen || segmentMouse.pressed) ? 1 : 0
                readonly property real restRadius: height / 2
                readonly property real squareRadius: Theme.radiusMorphPress

                width: labelled ? label.implicitWidth + group.labelPad * 2
                                : group.segmentHeight + 2
                height: group.segmentHeight
                radius: restRadius + (squareRadius - restRadius) * morph
                color: chosen ? Theme.secondary
                    : (segmentMouse.pressed ? Theme.surfaceContainerHighest
                        : (segmentMouse.containsMouse ? Theme.hover
                                                      : Theme.surfaceContainerHigh))
                opacity: enabled ? 1 : 0.38
                border.width: segment.activeFocus ? 2 : 0
                border.color: Theme.accent

                Behavior on morph {
                    enabled: Theme.motion
                    SpringAnimation {
                        spring: Theme.springSpatialFast
                        damping: Theme.dampSpatialFast
                        epsilon: 0.005
                    }
                }
                Behavior on color {
                    enabled: Theme.motion
                    ColorAnimation {
                        duration: Theme.motionFast
                        easing.type: Theme.motionEaseEffects
                    }
                }

                Accessible.role: Accessible.RadioButton
                Accessible.name: segment.labelled ? segment.modelData.label
                                                  : segment.modelData.id
                Accessible.checkable: true
                Accessible.checked: segment.chosen
                Accessible.onPressAction: group.selected(segment.modelData.id)

                activeFocusOnTab: true
                Keys.onReturnPressed: group.selected(segment.modelData.id)
                Keys.onEnterPressed: group.selected(segment.modelData.id)
                Keys.onSpacePressed: group.selected(segment.modelData.id)
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
                    tint: segment.chosen ? Theme.inkOnSecondary : Theme.inkOnSurfaceVariant
                }

                Text {
                    id: label
                    anchors.centerIn: parent
                    visible: segment.labelled
                    text: segment.modelData.label || ""
                    color: segment.chosen ? Theme.inkOnSecondary : Theme.inkOnSurfaceVariant
                    font.pixelSize: Theme.typeCaption
                    font.weight: segment.chosen ? Theme.weightStrong : Font.Normal
                }

                MouseArea {
                    id: segmentMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: group.selected(segment.modelData.id)
                }
            }
        }
    }
}