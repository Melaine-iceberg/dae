pragma ComponentBehavior: Bound
import QtQuick

// The one-line label shown when the pointer rests on a control.
//
// §7.5: 12 px, `raised`, `md` corners, the one shadow, 500 ms delay, text
// only. Tooltips explain, they never carry actions — anything that needs a
// click is a menu item or a button with a label.
//
// Declare it *inside* the item it describes; the HoverHandler attaches to
// that item through `target`, which defaults to `parent`. The plate itself
// is reparented to the window's content item on reveal: a tooltip inside a
// toolbar would otherwise be clipped by the toolbar's own bounds.
Item {
    id: root

    /// The item this label describes. A declaration inside that item is the
    /// normal way to set it, so the default is its own parent.
    property Item target: parent
    property string text: ""
    /// §7.5: half a second — long enough that crossing a toolbar does not
    /// spray labels, short enough that resting on a glyph still explains it.
    property int delay: 500

    width: 0
    height: 0
    z: 1100

    /// The plate lives outside this item while it is visible (see `reveal`),
    /// so it would otherwise outlive a toolbar slot that is rebuilt.
    Component.onDestruction: {
        if (plate.parent !== null && plate.parent !== root)
            plate.destroy()
    }

    HoverHandler {
        id: hover
        parent: root.target
        onHoveredChanged: {
            if (hovered)
                wait.restart()
            else
                root.hide()
        }
    }

    Timer {
        id: wait
        interval: root.delay
        onTriggered: root.reveal()
    }

    function hide() {
        wait.stop()
        plate.visible = false
    }

    function reveal() {
        if (root.text === "" || root.target === null || !hover.hovered)
            return
        const window = root.Window.window
        plate.parent = window !== null ? window.contentItem : root
        label.text = root.text
        plate.width = Math.min(360, label.implicitWidth + 16)
        plate.height = 24
        plate.place()
        plate.visible = true
    }

    /// Shadow and plate move together, so they share one wrapper. It starts
    /// under this item only so the binding has somewhere to live before the
    /// first reveal.
    Item {
        id: plate

        visible: false
        width: 0
        height: 0

        Shadow {
            target: bubble
        }

        Rectangle {
            id: bubble

            width: plate ? plate.width : 0
            height: plate ? plate.height : 0
            radius: Theme.radiusMd
            color: "transparent"
            border.width: 0

            /// The frost is the expensive half of a tooltip: two scene grabs
            /// and two blur passes, resident even while the plate is hidden.
            /// A detail row declares one of these per time column and almost
            /// none is ever revealed, so the material is built only while the
            /// plate shows and released with it.
            Loader {
                anchors.fill: parent
                active: plate.visible
                sourceComponent: Glass { plate: bubble; radius: Theme.radiusMd }
            }
        }

        Text {
            id: label

            anchors.centerIn: parent
            text: ""
            color: Theme.ink
            font.pixelSize: Theme.typeCaption
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }

        /// Above the target by default; below it when the window's top edge
        /// would cut the label off. Both axes are clamped, so a control at the
        /// window's corner still gets a readable tooltip.
        function place() {
            if (root.target === null || plate.parent === null)
                return
            const point = root.target.mapToItem(plate.parent, 0, 0)
            const gap = 6
            let x = point.x + (root.target.width - plate.width) / 2
            let y = point.y - plate.height - gap
            if (y < 8)
                y = point.y + root.target.height + gap
            x = Math.max(8, Math.min(x, plate.parent.width - plate.width - 8))
            y = Math.max(8, Math.min(y, plate.parent.height - plate.height - 8))
            plate.x = Math.round(x)
            plate.y = Math.round(y)
        }
    }
}
