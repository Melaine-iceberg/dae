pragma ComponentBehavior: Bound
import QtQuick

// A desktop segmented control. Segments sit with a 2 dp gap and every one is
// the same 6 dp rectangle; the chosen segment is distinguished by its fill,
// not by a shape change. This is the view switcher Chrome-style toolbars use:
// three quiet buttons in a row, one of them lit. Single-select,
// keyboard-navigable.
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

                width: labelled ? label.implicitWidth + group.labelPad * 2
                                : group.segmentHeight
                height: group.segmentHeight
                radius: Theme.radiusSm
                /// One selection language across the shell: the chosen segment
                /// wears the same `secondaryContainer` the selected row and
                /// the current sidebar destination do. The
                /// strong `secondary` fill M3 uses for a *pressed* toggle read
                /// as a second accent colour in a toolbar of quiet controls.
                /// The unchosen segments carry no plate at all, so the group
                /// reads as "one of three is on" beside the round icon buttons
                /// rather than as three grey blocks glued together.
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
                    onClicked: group.selected(segment.modelData.id)
                }
            }
        }
    }
}