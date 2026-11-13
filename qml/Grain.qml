import QtQuick

// The paper grain over the whole window.
//
// drift prints on stock, not on a screen: a 7 % speckle sits over everything —
// chrome, listing, dialogs — so a flat fill never reads as a plastic panel. The
// prototype gets this from an inline SVG `feTurbulence` at .07 with `multiply`
// (`screen` in dark).
//
// Here it is a 128 dp tile of per-pixel noise, tiled by the renderer with a
// repeat wrap, and tinted by picking one of two textures: ink specks for the
// paper theme, paper specks for the dark one. QML has no blend modes on an
// `Image` (that needs `ShaderEffectSource`, which re-renders what it captures
// every frame), and a two-polarity speckle at low alpha does the same job with
// one textured quad and no capture — the cost this overlay has to justify is
// that it covers every pixel of the window.
//
// It draws over everything, including menus and dialogs, and takes no input.
Item {
    id: root

    /// Held well below the prototype's 7 %. On a 96 dpi desktop panel that much
    /// speckle reads as a texture filter over the whole window rather than as
    /// stock, and nothing in the window is larger or flatter than the paper it
    /// is printed on — so the grain has to be something you notice only when it
    /// is gone. 3 % on paper, 2 % on the dark theme, where light specks read as
    /// a dirty screen sooner.
    readonly property real strength: Theme.dark ? 0.02 : 0.03

    Accessible.ignored: true

    Image {
        anchors.fill: parent
        source: Theme.dark ? "assets/grain-paper.png" : "assets/grain-ink.png"
        fillMode: Image.Tile
        // Nearest neighbour: the speckle is meant to be grain, and a smoothed
        // noise tile is a grey haze instead.
        smooth: false
        mipmap: false
        cache: true
        asynchronous: false
        opacity: root.strength
        Accessible.ignored: true
    }
}
