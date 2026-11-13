import QtQuick

// The shadow under a floating surface.
//
// drift has exactly one kind of shadow and it has no blur: the surface's own
// rectangle, displaced a few pixels down and right, painted in solid ink. An
// overlay then reads as a sheet laid on the page — an edge with a second edge
// behind it — instead of as something hovering over it, which is what a blurred
// shadow says.
//
// The shadow is a sibling that follows its target rather than a child of the
// plate: a child paints after its parent, so a negative-z rectangle would still
// cover the surface it is supposed to sit behind. Declare this immediately
// before the plate, in the same coordinate space.
Item {
    id: root

    /// The plate the shadow belongs to. Its x, y, width and height are read
    /// live, so a menu that repositions on every open needs no extra wiring.
    property Item target: null
    /// How far the shadow steps out. Menus and dialogs use the token; the
    /// toast is 4 dp and takes the signal colour instead.
    property int offset: Theme.shadowOffset
    property color color: Theme.ink

    visible: target !== null && target.visible
    x: target === null ? 0 : target.x + offset
    y: target === null ? 0 : target.y + offset
    width: target === null ? 0 : target.width
    height: target === null ? 0 : target.height

    Rectangle {
        anchors.fill: parent
        color: root.color
        Accessible.ignored: true
    }
}
