import QtQuick
import dae.m3 1.0

// The M3 Expressive loading indicator: a blob morphing through seven Material
// shapes while it rotates, not a spinner (see cpp/m3_items.cpp). Themed to
// `primary`; reduced motion holds one static shape. The caller sets the edge
// (24 dp inline, 48 dp centred in a loading region) and `running`.
LoadingIndicator {
    color: Theme.accent
    reduceMotion: !Theme.motion
    implicitWidth: 24
    implicitHeight: 24

    Accessible.role: Accessible.ProgressBar
    Accessible.name: qsTr("加载中")
}