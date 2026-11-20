import QtQuick
import dae.m3 1.0

// The M3 Expressive linear progress indicator, themed. The active track is a
// sine squiggle in `primary`, the remaining track a flat line in
// `secondaryContainer`, with a 4 dp gap and a stop dot (see cpp/m3_items.cpp).
// `wavy: false` or reduced motion draws the flat M3 bar instead. The caller
// sets `value` (0–1) or `indeterminate`, and a width; the height defaults to
// the 10 dp the wave needs.
ProgressIndicator {
    activeColor: Theme.accent
    trackColor: Theme.secondaryContainer
    reduceMotion: !Theme.motion
    thickness: 4
    implicitHeight: wavy && !reduceMotion ? 10 : 4

    Accessible.role: Accessible.ProgressBar
    Accessible.name: indeterminate ? qsTr("正在处理") : qsTr("进度 %1%").arg(Math.round(value * 100))
}