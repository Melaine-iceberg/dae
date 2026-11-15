import QtQuick

// The shadow under a floating surface.
//
// Graphite has exactly one elevation and exactly one way to draw it (§6.5):
// a `raised` plate, a 1 px edge, and a soft shadow underneath. The shadow is a
// nine-patch texture rather than a blurred rectangle, because `MultiEffect` is
// the most expensive effect Qt documents and a static texture stays sharp at
// any DPR.
//
// The shadow is a sibling that follows its target rather than a child of the
// plate: a child paints after its parent, so a negative-z rectangle would still
// cover the surface it is supposed to sit behind. Declare this immediately
// before the plate, in the same coordinate space.
//
// Two textures, one per theme, generated at the blur the design asks for
// (light y2/blur12/8 %, dark y2/blur16/45 %). `shadowMargin` is the source's
// own 48 px border used at 1:1, so the falloff is neither stretched nor
// cropped.
Item {
    id: root

    /// The plate the shadow belongs to. Its x, y, width and height are read
    /// live, so a menu that repositions on every open needs no extra wiring.
    property Item target: null
    /// How far the shadow sits below the plate. 2 dp is the design value; a
    /// surface that has to read as closer to the page can pass less.
    property int offset: 2

    visible: target !== null && target.visible
    x: target === null ? 0 : target.x - Theme.shadowMargin
    y: target === null ? 0 : target.y - Theme.shadowMargin + offset
    width: target === null ? 0 : target.width + Theme.shadowMargin * 2
    height: target === null ? 0 : target.height + Theme.shadowMargin * 2

    BorderImage {
        anchors.fill: parent
        source: Theme.dark ? "assets/shadow-dark.png" : "assets/shadow-light.png"
        border {
            left: Theme.shadowMargin
            top: Theme.shadowMargin
            right: Theme.shadowMargin
            bottom: Theme.shadowMargin
        }
        horizontalTileMode: BorderImage.Stretch
        verticalTileMode: BorderImage.Stretch
        smooth: true
        Accessible.ignored: true
    }
}
