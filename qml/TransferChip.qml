pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The running transfer, as a capsule. It used to be a "queue" chip bolted
// after the status text. The centre of the strip is where the eye looks for
// progress, and this chip is the queue's only door: without it a second paste
// looks like it did nothing until the first copy finished.
//
// Both footer states (the 28 dp status strip and the 36 dp action bar) show
// the same capsule, so it lives here rather than being duplicated: the two
// bars differ in what surrounds it, never in whether a transfer is visible.
Rectangle {
    id: chip

    /// Where the queue panel should open, in window coordinates.
    signal openRequested(real x, real y)

    width: transferRow.implicitWidth + 14
    height: Theme.controlSm
    radius: Theme.radiusXs
    visible: chip.count > 0
    color: transferMouse.containsMouse ? Theme.raised : Theme.canvas
    border.width: 1

    // QStringList exposes `.length` to QML; qmllint's type model
    // does not (same caveat in SpaceView/TransferQueue).
    // qmllint disable missing-property
    readonly property int count: Workspace.queuedTransfers.length
        + (Workspace.busy ? 1 : 0)
    readonly property real percent: Workspace.progressTotal > 0
        ? Math.round(Workspace.progressDone * 100 / Workspace.progressTotal)
        : (Workspace.busy ? 0 : 100)

    function openPanel() {
        const point = chip.mapToItem(null, 0, 0)
        chip.openRequested(point.x, point.y)
    }

    Accessible.role: Accessible.Button
    Accessible.name: qsTr("%1 项传输，%2%").arg(count).arg(percent)
        + qsTr("，查看传输队列")
    Accessible.onPressAction: chip.openPanel()

    /// The chip is a tab stop only while it exists: an empty queue has nothing
    /// to show in its panel. The border it already carries turns into the
    /// focus ring rather than gaining a second one.
    activeFocusOnTab: chip.visible
    border.color: chip.activeFocus ? Theme.accent : Theme.hairline
    Keys.onReturnPressed: chip.openPanel()
    Keys.onSpacePressed: chip.openPanel()

    Row {
        id: transferRow
        anchors.centerIn: parent
        spacing: 6

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("%1 项传输").arg(chip.count)
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeMicro
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 32
            height: 2
            color: Theme.line

            Rectangle {
                width: parent.width * chip.percent / 100
                height: parent.height
                color: Theme.accent
            }
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: chip.percent + "%"
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeMicro
            font.family: Theme.monoFamily
        }
    }

    MouseArea {
        id: transferMouse
        anchors.fill: parent
        hoverEnabled: true
        onClicked: chip.openPanel()
    }
}
