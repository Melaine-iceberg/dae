pragma ComponentBehavior: Bound
import QtQuick

// Space, without giving up the listing.
//
// The docked inspector is a fourth column: opening it reflows the window.
// This overlay is the other gesture — the one Files trains — a plate over the
// content that closes on Escape or an outside click and leaves the selection
// exactly where it was. It reuses `Inspector` rather than a second reader, so
// a file that previews in the pane previews here with the same limits (text
// peeks; thumbnails are local-only).
Item {
    id: root

    property var paths: []
    property bool dark: true
    property color accent: Theme.accent

    /// The plate and the Inspector behind it are built on the first open and
    /// kept afterwards: a preview nobody has asked for costs no resident tree.
    property bool plateLoaded: false

    visible: false
    focus: visible
    z: 1004

    readonly property bool open: visible

    /// The property `open` is the state; this is the verb.
    function show(next) {
        root.paths = next && next.length > 0 ? next : []
        if (root.paths.length === 0)
            return
        // Loaded before `visible` flips, so the plate is created at opacity 0
        // and still animates in.
        root.plateLoaded = true
        root.visible = true
        if (plateLoader.item !== null) {
            // qmllint disable missing-property
            plateLoader.item.takeFocus()
            // qmllint enable missing-property
        }
    }

    function close() {
        root.visible = false
        root.paths = []
    }

    onVisibleChanged: if (visible)
        root.plateLoaded = true

    function scrollPreview(distance) {
        if (plateLoader.item !== null) {
            // qmllint disable missing-property
            plateLoader.item.scrollBy(distance)
            // qmllint enable missing-property
        }
    }

    function pagePreview(direction) {
        if (plateLoader.item !== null) {
            // qmllint disable missing-property
            plateLoader.item.pageBy(direction)
            // qmllint enable missing-property
        }
    }

    /// The overlay owns the arrow keys, not the file list: scrolling the
    /// preview must not move the selection behind it.
    Keys.onEscapePressed: root.close()
    Keys.onUpPressed: root.scrollPreview(-40)
    Keys.onDownPressed: root.scrollPreview(40)
    Keys.onLeftPressed: root.close()
    Keys.onRightPressed: root.close()
    Keys.onPressed: function (event) {
        if (event.key === Qt.Key_PageUp) {
            root.pagePreview(-1)
            event.accepted = true
        } else if (event.key === Qt.Key_PageDown) {
            root.pagePreview(1)
            event.accepted = true
        } else if (event.key === Qt.Key_Space) {
            root.close()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            onClicked: root.close()
        }
    }

    Loader {
        id: plateLoader

        anchors.fill: parent
        active: root.plateLoaded
        sourceComponent: Component {
            Item {
                anchors.fill: parent

                function takeFocus() {
                    panel.forceActiveFocus()
                }

                function scrollBy(distance) {
                    panelContent.scrollBy(distance)
                }

                function pageBy(direction) {
                    panelContent.scrollBy(direction * panelContent.height * 0.8)
                }

                Shadow {
                    target: panel
                }

                Rectangle {
                    id: panel

                    anchors.centerIn: parent
                    width: Math.min(1120, Math.max(360, parent.width - 96))
                    height: Math.min(920, Math.max(320, parent.height - 96))
                    /// The preview is a large floating sheet, so it wears the
                    /// dialog corner and the high container instead of a
                    /// hairline outline.
                    radius: Theme.radiusXl
                    color: "transparent"
                    border.width: 0
                    focus: true

                    opacity: root.visible ? 1 : 0
                    Behavior on opacity {
                        enabled: Theme.motion && root.visible
                        OpacityAnimator {
                            duration: Theme.motionOverlay
                            easing.type: Easing.OutCubic
                        }
                    }
                    Glass { radius: Theme.radiusXl }

                    // A click inside the plate must not close it; the scrim is
                    // behind.
                    MouseArea {
                        Accessible.ignored: true
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                    }

                    Inspector {
                        id: panelContent
                        anchors.fill: parent
                        anchors.margins: 1
                        paths: root.paths
                        dark: root.dark
                        accent: root.accent
                        plate: true
                        wellHeight: 260
                        onCloseRequested: root.close()
                    }
                }
            }
        }
    }
}
