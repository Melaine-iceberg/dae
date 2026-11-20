pragma ComponentBehavior: Bound
import QtQuick

// The one dialog plate in the shell.
//
// The M3 Expressive dialog: a `surfaceContainerHigh` plate on the 32 % scrim,
// a 28 dp extra-large corner, no stroke (tone and elevation separate it), a
// 17 px emphasized heading and a pill button row at the bottom right. Dialogs
// used to lay this out by hand with drift in radius and foot height; this file
// owns the one definition, and a dialog supplies a body and a footer.
//
//     Dialog {
//         heading: qsTr("新建文件夹")
//         bodyHeight: 28
//         TextInput { width: parent.width; height: 28 }
//         footer: Component { Row { ... } }
//     }
//
// `content` lands in a body band inset 16 dp from the sides, 12 dp under the
// head and 12 dp above the footer; the caller sets `bodyHeight` because only
// it knows how tall its content is, and a measured height here would chase
// the panel it is measuring.
FocusScope {
    id: root

    property bool dark: true
    property string heading: ""
    property string subtitle: ""
    /// Height of whatever the caller puts in `content`.
    property int bodyHeight: 0
    /// §7.5: 440 for a simple dialog. A prompt narrows it, a properties sheet
    /// widens it.
    property int panelWidth: 440
    /// A destructive question does not dismiss on a stray click: "empty the
    /// bin" has to be answered, not clicked away.
    property bool closeOnScrim: true
    property Component footer: null

    default property alias content: bodyArea.data

    signal cancelRequested()

    visible: false
    z: 1002
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: root.heading

    readonly property int footerHeight: footerLoader.implicitHeight

    /// Deliberately not named `open`/`close`: a dialog that extends this file
    /// declares its own `open(...)` with its own arguments, and `root.open()`
    /// would then call that override instead of this one.
    function show() {
        root.visible = true
        root.forceActiveFocus()
        // A modal opened from a menu is shown while the menu is still
        // unwinding; asking for focus again on the next turn is what actually
        // lands it in the dialog rather than back on the path field.
        Qt.callLater(function () {
            if (root.visible)
                root.forceActiveFocus()
        })
    }

    function dismiss() {
        root.visible = false
    }

    Keys.onEscapePressed: root.cancelRequested()

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: {
                if (root.closeOnScrim)
                    root.cancelRequested()
            }
        }
    }

    /// The plate's soft shadow, declared before the plate so it paints behind
    /// it (see `Shadow.qml`).
    Shadow {
        target: panel
    }

    Rectangle {
        id: panel

        anchors.horizontalCenter: parent.horizontalCenter
        y: 64
        width: Math.min(root.panelWidth, root.width - 32)
        height: head.height + 12 + root.bodyHeight + 12
            + (root.footerHeight > 0 ? root.footerHeight + 16 : 0)
        radius: Theme.radiusXl
        color: "transparent"
        border.width: 0

        /// M3 Expressive confirmation is springy: the plate scales up a hair
        /// past its resting size while the scrim fades. Hiding stays instant so
        /// the next dialog never opens on top of a departing one.
        opacity: root.visible ? 1 : 0
        Behavior on opacity {
            enabled: Theme.motion && root.visible
            OpacityAnimator {
                duration: Theme.motionOverlay
                easing.type: Easing.OutCubic
            }
        }
        scale: root.visible ? 1 : 0.94
        Behavior on scale {
            enabled: Theme.motion && root.visible
            SpringAnimation {
                spring: Theme.springSpatial
                damping: Theme.dampSpatial
                epsilon: 0.005
            }
        }

        Glass { radius: Theme.radiusXl }

        // The plate is not an input item, so a press on its padding would
        // otherwise reach the scrim behind it and close the dialog. This is
        // declared before the head and body, so the controls above it still
        // receive their presses; only the plate's empty space is swallowed.
        MouseArea {
            anchors.fill: parent
            onClicked: {}
            onWheel: function (wheel) { wheel.accepted = true }
        }

        DialogHeader {
            id: head
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            title: root.heading
            subtitle: root.subtitle
            inset: 16
            onCloseRequested: root.cancelRequested()
        }

        Item {
            id: bodyArea
            x: 16
            y: head.height + 12
            width: parent.width - 32
            height: root.bodyHeight
        }

        Loader {
            id: footerLoader
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 16
            sourceComponent: root.footer
        }
    }
}
