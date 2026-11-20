import QtQuick

/// The shell's scrollbar: a 6 dp capsule that grows to 8 dp under the pointer,
/// no arrows, and no gutter reserved for it. It is an overlay: it covers the
/// last few dp of the content rather than shrinking it, because a file
/// manager's column widths are the user's, not the bar's. The thumb uses the
/// M3 `outline` / `onSurfaceVariant` roles so it belongs to the surface regardless
/// of which tonal step it sits on.
///
/// One bar serves a `Flickable` or any `ListView`/`GridView`: all three expose
/// `contentY`, `contentHeight` and `height`, which is the whole contract. The
/// bar follows the target's position; it never owns it.
Item {
    id: bar

    /// The flickable to observe. Assign `null` while this surface is hidden so
    /// the bar disappears with it.
    property var target: null

    readonly property bool scrollable: target !== null
        && target.contentHeight > target.height + 1
    readonly property real travel: Math.max(0, target !== null ? target.height : 0)
    readonly property real maximum: scrollable ? target.contentHeight - target.height : 0
    /// Never shorter than 24 dp: a six-row menu would otherwise give a thumb
    /// too small to see, let alone grab.
    readonly property real thumbLength: scrollable
        ? Math.max(24, travel * travel / target.contentHeight) : 0

    /// The hit strip is 10 dp wide so the 6 dp capsule is comfortable to catch;
    /// the capsule itself stays 6 / 8 dp as the specification asks.
    width: 10
    height: travel
    visible: scrollable
    /// Above the views and their marquee overlays: a bar that a rubber-band
    /// area could cover would be visible but not grabbable.
    z: 10
    Accessible.ignored: true

    function scrollTo(pointerY, grabOffset) {
        if (!scrollable)
            return
        const runway = travel - thumbLength
        if (runway <= 0)
            return
        const offset = Math.max(0, Math.min(runway, pointerY - grabOffset))
        target.contentY = maximum * offset / runway
    }

    Item {
        id: thumb

        readonly property bool lit: pointer.containsMouse || pointer.pressed

        /// Hover is a 6 → 8 dp growth in `onSurfaceVariant`. Qt Quick has no
        /// width animator and no `ColorAnimator`, so both are two capsules
        /// crossfading — the narrow one in the rest colour, the wide one in the
        /// lit colour — and each fade is an `OpacityAnimator` inside the
        /// `Behavior`, so the animation runs on the render thread instead of
        /// repainting a colour on the GUI thread.
        width: 8
        height: bar.thumbLength
        x: (bar.width - width) / 2
        y: bar.maximum > 0
            ? (bar.travel - bar.thumbLength) * bar.target.contentY / bar.maximum
            : 0

        Rectangle {
            anchors.centerIn: parent
            width: 6
            height: parent.height
            radius: width / 2
            color: Theme.outline
            opacity: thumb.lit ? 0 : 1
            Accessible.ignored: true

            Behavior on opacity {
                enabled: Theme.motion
                OpacityAnimator {
                    duration: Theme.motionFast
                    easing.type: Theme.motionEaseEffects
                }
            }
        }

        Rectangle {
            anchors.centerIn: parent
            width: 8
            height: parent.height
            radius: width / 2
            color: Theme.inkOnSurfaceVariant
            opacity: thumb.lit ? 1 : 0
            Accessible.ignored: true

            Behavior on opacity {
                enabled: Theme.motion
                OpacityAnimator {
                    duration: Theme.motionFast
                    easing.type: Theme.motionEaseEffects
                }
            }
        }
    }

    MouseArea {
        id: pointer

        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.PointingHandCursor
        /// Distance from the top of the thumb to where the press landed, so a
        /// grab does not jump the thumb to the pointer.
        property real grabOffset: 0

        onPressed: function (mouse) {
            if (mouse.y >= thumb.y && mouse.y <= thumb.y + thumb.height) {
                grabOffset = mouse.y - thumb.y
            } else {
                // A press on the track jumps to centre the thumb there.
                grabOffset = bar.thumbLength / 2
                bar.scrollTo(mouse.y, grabOffset)
            }
        }

        onPositionChanged: function (mouse) {
            if (pressed)
                bar.scrollTo(mouse.y, grabOffset)
        }

        /// The wheel belongs to the listing behind the bar; swallowing it here
        /// would create a dead strip along the right edge.
        onWheel: function (wheel) {
            wheel.accepted = false
        }
    }
}
