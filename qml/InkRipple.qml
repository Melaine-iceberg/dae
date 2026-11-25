import QtQuick
import dae.m3 1.0

// Themed Material 3 / Chrome Desktop ink ripple overlay rendered on the RHI.
// Place inside a control plate, bind `radius` to the plate's corner radius,
// and call `burst(mouse.x, mouse.y)` / `release()` from its MouseArea.
InkRippleEffect {
    anchors.fill: parent
    color: Theme.ripple
    radius: Theme.radiusSm
    reduceMotion: !Theme.motion
    Accessible.ignored: true
}
