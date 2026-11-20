import QtQuick
import QtQuick.Window
import QtQuick.Effects

// The frosted backdrop under a floating plate (dialogs, menus, tooltips,
// toasts, the palette and the side sheets), in the manner of macOS vibrancy.
//
// Qt Quick has no window-server blur behind an item, so the material is
// composed here on the RHI, in two passes: a `ShaderEffectSource` grabs the
// window's scene in the plate's footprint, one `MultiEffect` blurs it hard and
// lifts its saturation (the vibrancy "thickness"), and a second `MultiEffect`
// clips that result to the plate's corner. The clip is a separate pass on
// purpose: masking inside the blur pass would let the blur bleed back out past
// the rounded edge and read as a square patch behind a round outline. A
// translucent veil sits on top so text keeps its contrast, with a hairline edge
// and a one-pixel top highlight the way a macOS material catches the light.
//
// The component is declared as the plate's first child, so it inherits the
// plate's position, scale and opacity — the frost arrives and leaves with the
// plate rather than lagging behind it.
//
// The scene is the `daeGlassScene` item ShellWindow wraps its content in; it is
// found through the window so a plate declared in any file (a tooltip deep in a
// toolbar, a dialog built by a Loader) resolves the same source. A plate that
// lives *inside* the scene (the path bar's completion list) cannot grab its own
// ancestor, so it falls back to the solid `raised` plate it had before — no
// recursion, no regression.
Item {
    id: root

    /// The plate this frost sits behind. Defaults to the parent it is declared
    /// in; its geometry is read live so a menu that repositions on every open
    /// needs no extra wiring.
    property Item plate: parent
    /// The plate's corner radius. Passed by the caller so the mask and the veil
    /// match the plate exactly.
    property real radius: Theme.radiusMd
    /// The blur kernel ceiling in device pixels. macOS materials blur hard
    /// enough that the backdrop reads as colour, not content; 64 is that.
    property int blurMax: 64
    /// The translucent veil over the frost. Defaults to the `raised` glass tint;
    /// an inverse-surface snackbar passes its own so it keeps its polarity.
    property color veil: Theme.glassTint
    /// The plate's original solid paint, used when the frost cannot run.
    property color solidVeil: Theme.raised

    anchors.fill: parent
    clip: true
    visible: plate !== null

    readonly property real dpr: Screen.devicePixelRatio
    readonly property real margin: Math.ceil(blurMax / Math.max(1, dpr)) + 2

    /// The window's scene content, found by object name among the window's
    /// top-level items. Re-resolved when the window or its children change.
    readonly property Item scene: {
        const window = root.Window.window
        if (window === null || window.contentItem === null)
            return null
        const kids = window.contentItem.children
        for (let index = 0; index < kids.length; index += 1) {
            if (kids[index].objectName === "daeGlassScene")
                return kids[index]
        }
        return null
    }

    /// True when the scene is an ancestor of this plate: grabbing it would be
    /// recursive, so the frost stands down and the veil stays solid.
    readonly property bool recursive: {
        if (scene === null)
            return false
        let ancestor = root.parent
        while (ancestor !== null) {
            if (ancestor === scene)
                return true
            ancestor = ancestor.parent
        }
        return false
    }

    readonly property bool blurActive: Theme.glassEnabled && scene !== null && !recursive

    /// The plate's footprint in scene coordinates, expanded by the blur margin.
    /// Mapped from the plate's parent so the plate's own arrival scale does not
    /// churn the grab region every frame of the spring.
    readonly property rect sceneRect: {
        if (plate === null || plate.parent === null || scene === null)
            return Qt.rect(0, 0, 0, 0)
        const point = plate.parent.mapToItem(scene, plate.x, plate.y)
        return Qt.rect(point.x - margin, point.y - margin,
                       plate.width + margin * 2, plate.height + margin * 2)
    }

    // Every intermediate texture lives offscreen: a `ShaderEffectSource` or
    // `MultiEffect` paints its own output wherever it sits, so leaving them in
    // the plate would leak a square patch of grab/blur/mask past the rounded
    // edge. Only the final masked pass below is allowed to paint in place.
    Item {
        id: offscreen

        x: -100000
        y: -100000
        width: root.width + root.margin * 2
        height: root.height + root.margin * 2

        ShaderEffectSource {
            id: grab

            anchors.fill: parent
            sourceItem: root.scene
            sourceRect: root.sceneRect
            textureSize: Qt.size(Math.max(1, Math.ceil(root.sceneRect.width * root.dpr)),
                                 Math.max(1, Math.ceil(root.sceneRect.height * root.dpr)))
            live: root.blurActive
            smooth: true
            Accessible.ignored: true
        }

        // Pass one: the vibrancy blur. A modest saturation lift keeps colourful
        // backdrops glowing like a macOS material; a strong boost would amplify
        // the shell's own near-neutral surfaces into a colour cast.
        MultiEffect {
            id: blurred

            source: grab
            anchors.fill: parent
            blurEnabled: true
            blur: 1.0
            blurMax: root.blurMax
            saturation: 1.3
            autoPaddingEnabled: false
            Accessible.ignored: true
        }

        ShaderEffectSource {
            id: maskGrab

            anchors.fill: parent
            sourceItem: maskShape
            textureSize: Qt.size(Math.max(1, Math.ceil(offscreen.width * root.dpr)),
                                 Math.max(1, Math.ceil(offscreen.height * root.dpr)))
            live: root.blurActive
            smooth: true
            Accessible.ignored: true
        }

        /// The mask shape: a white rounded rectangle the size of the plate,
        /// centred in the expanded backdrop so its texture aligns with the
        /// blur's. Only ever sampled as a texture, never painted.
        Item {
            id: maskShape

            width: offscreen.width
            height: offscreen.height

            Rectangle {
                x: root.margin
                y: root.margin
                width: root.width
                height: root.height
                radius: root.radius
                color: "#ffffff"
            }
        }
    }

    Item {
        id: backdrop

        visible: root.blurActive
        x: -root.margin
        y: -root.margin
        width: root.width + root.margin * 2
        height: root.height + root.margin * 2

        // Pass two: clip the blurred result to the plate's corner. Keeping the
        // mask in its own pass guarantees it lands after the blur, so the frost
        // ends exactly at the rounded edge.
        MultiEffect {
            source: blurred
            anchors.fill: parent
            maskEnabled: true
            maskSource: maskGrab
            maskThresholdMin: 0.5
            maskSpreadAtMin: 0.02
            autoPaddingEnabled: false
            Accessible.ignored: true
        }
    }

    /// The veil. Translucent over the frost so the blur reads through; solid
    /// `raised` when the frost cannot run, which is the plate's original look.
    Rectangle {
        anchors.fill: parent
        radius: root.radius
        color: root.blurActive ? root.veil : root.solidVeil
        border.width: root.blurActive ? 1 : 0
        border.color: Theme.glassEdge
        Accessible.ignored: true

        // The top highlight: one pixel of caught light inside the corner, the
        // way a macOS material's upper edge reads slightly brighter.
        Rectangle {
            visible: root.blurActive
            anchors.top: parent.top
            anchors.topMargin: 1
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: root.radius
            anchors.rightMargin: root.radius
            height: 1
            color: Theme.glassHighlight
            Accessible.ignored: true
        }
    }
}