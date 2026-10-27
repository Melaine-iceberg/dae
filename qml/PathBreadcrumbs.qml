pragma ComponentBehavior: Bound
import QtQuick

// The path above the listing, one clickable crumb per ancestor.
//
// This is the shape the Tauri shell's `ExplorerBreadcrumbs` had: the bar names
// where the user is and every ancestor is a destination, so the text field is
// the exception — typing a path — rather than the default. `ShellWindow` owns
// that editor; this component draws the trail and asks to navigate.
//
// The trail is laid out here rather than by a `Row`: the fold decision is a
// measured one, so the same numbers have to position the crumbs that decided
// whether they fit. A positioner would add its own spacing on top, and the bar
// would either overflow the clip or leave room it claimed not to have.
Item {
    id: bar

    /// The listed directory's ancestors, outermost first, exactly as the
    /// backend spelled them (`DirectoryListModel.breadcrumbNames` and
    /// `breadcrumbPaths`). The two are parallel lists, so the shorter wins.
    property var names: []
    property var paths: []

    property bool dark: true
    property color textColor: "#e7e9ee"
    property color mutedColor: "#8d94a2"
    property color accent: "#2f6fdf"

    /// The path of the ancestor that was clicked.
    signal crumbActivated(string path)
    /// The ancestors hidden behind the `…`, with the position to open a menu
    /// at — in window coordinates, because the menu belongs to the window.
    signal ancestorsRequested(real x, real y, var names, var paths)

    property int fontSize: 13
    readonly property int iconSize: 13
    /// Space between a crumb's glyph and its name.
    readonly property real iconGap: 4
    /// Space between a separator and the crumbs on either side of it.
    readonly property real crumbGap: 3
    /// A crumb's own padding; the plate is what hover paints.
    readonly property real crumbPad: 4
    readonly property real plateHeight: 22
    readonly property real ellipsisWidth: 22

    TextMetrics {
        id: nameMeasure
        font.pixelSize: bar.fontSize
    }

    TextMetrics {
        id: separatorMeasure
        text: "\u203A"
        font.pixelSize: bar.fontSize
    }

    /// One separator plus the gaps around it: what every crumb but the first
    /// costs beyond its own plate.
    readonly property real unit: separatorMeasure.advanceWidth + bar.crumbGap * 2

    /// An ancestor reads as context and is rarely needed in full: 160px is a
    /// long folder name, and eliding past it would only take room from the
    /// crumb that matters.
    readonly property real maxAncestorWidth: 160
    readonly property real maxCurrentWidth: 256
    /// The narrowest the current crumb may become before it is left to the
    /// clip: the glyph, its padding and two characters. Below that a name
    /// elides to nothing, which loses the one crumb the bar exists to show.
    readonly property real minCurrentWidth: bar.iconSize + bar.iconGap
        + bar.crumbPad * 2 + bar.fontSize * 2

    /// What the bar paints: `{ name, path, current, x, width }` entries plus one
    /// `{ ellipsis: true, x, width }` when ancestors were swallowed by the menu.
    property var crumbs: []
    property var hiddenNames: []
    property var hiddenPaths: []

    clip: true

    onNamesChanged: Qt.callLater(bar.rebuild)
    onPathsChanged: Qt.callLater(bar.rebuild)
    onFontSizeChanged: bar.rebuild()
    onWidthChanged: bar.rebuild()
    Component.onCompleted: bar.rebuild()

    /// The plate width a crumb needs, capped: the glyph, the gap, the padding,
    /// and the name as far as it is allowed to grow.
    function crumbWidth(name, current) {
        nameMeasure.text = name
        const nameWidth = Math.min(nameMeasure.advanceWidth,
                                   current ? bar.maxCurrentWidth : bar.maxAncestorWidth)
        return nameWidth + bar.iconSize + bar.iconGap + bar.crumbPad * 2
    }

    /// Mirrors the Tauri shell's `computeHiddenRange`: the first crumb and the
    /// current one always stay, the ellipsis claims one slot, and the ancestors
    /// between them are added back from the current folder outwards for as long
    /// as they fit.
    function fit(all) {
        const count = all.length
        if (count <= 2)
            return { "visible": all, "hidden": [] }

        const widths = []
        let total = 0
        for (let index = 0; index < count; index += 1) {
            const width = bar.crumbWidth(all[index].name, all[index].current)
            widths.push(width)
            total += width
        }
        // 1px tolerance absorbs sub-pixel rounding, so a bar that sits exactly
        // on the edge does not flip between folded and whole on every resize.
        total += bar.unit * (count - 1)
        if (total <= bar.width + 1)
            return { "visible": all, "hidden": [] }

        // Seeded with the current crumb, not just the first one: growing
        // towards a crumb that was never counted is what made the Tauri shell
        // report a row as fitting while it painted 70px too wide, squeezing
        // every crumb to nothing.
        let used = widths[0] + bar.unit + bar.ellipsisWidth + bar.unit + widths[count - 1]
        let start = count - 1
        while (start > 1 && used + widths[start - 1] + bar.unit <= bar.width) {
            used += widths[start - 1] + bar.unit
            start -= 1
        }

        // Even `root › … › current` is too wide. Painting everything and letting
        // the clip cut it is still better than folding to a bar that names
        // nothing: the current crumb truncates inside its own box.
        if (start <= 1)
            return { "visible": all, "hidden": [] }

        const visible = [all[0], { "ellipsis": true }]
        for (let index = start; index < count; index += 1)
            visible.push(all[index])
        return { "visible": visible, "hidden": all.slice(1, start) }
    }

    /// Stamps the horizontal position into every entry, so the delegates paint
    /// exactly the geometry the fold decision was made with.
    function place(entries) {
        let x = 0
        for (let index = 0; index < entries.length; index += 1) {
            const entry = entries[index]
            entry.width = entry.ellipsis === true
                ? bar.ellipsisWidth : bar.crumbWidth(entry.name, entry.current)
            entry.x = index === 0 ? 0 : x + bar.unit
            x = entry.x + entry.width
        }

        // The fold can only give up what lies between the root and the current
        // folder. When even `root › … › current` is wider than the bar, the last
        // crumb is trimmed to what is left rather than cut off by the clip: a
        // name that elides reads as "too long", a name clipped mid glyph reads
        // as broken. Below the two-character floor the clip is the better
        // answer again, so the trim stops there.
        const last = entries[entries.length - 1]
        if (entries.length > 1) {
            const room = bar.width - last.x
            if (last.width > room)
                last.width = Math.min(last.width, Math.max(room, bar.minCurrentWidth))
        }
        return entries
    }

    function sameCrumbs(left, right) {
        if (left.length !== right.length)
            return false
        for (let index = 0; index < left.length; index += 1) {
            const a = left[index]
            const b = right[index]
            if (a.ellipsis !== b.ellipsis)
                return false
            if (a.path !== b.path || a.current !== b.current)
                return false
            // The trim above can change a plate's width without changing which
            // crumbs are shown, and the delegates read that width.
            if (Math.round(a.width) !== Math.round(b.width))
                return false
        }
        return true
    }

    function rebuild() {
        const count = Math.min(bar.names.length, bar.paths.length)
        const all = []
        for (let index = 0; index < count; index += 1) {
            all.push({
                "name": bar.names[index],
                "path": bar.paths[index],
                "current": index === count - 1
            })
        }

        const fitted = bar.fit(all)
        const placed = bar.place(fitted.visible)
        // A resize usually crosses no fold threshold, and reassigning the array
        // would rebuild every delegate for a change of nothing but the bar's
        // own width. Only a change of shape is published.
        if (bar.sameCrumbs(bar.crumbs, placed))
            return

        bar.hiddenNames = fitted.hidden.map(function (crumb) {
            return crumb.name
        })
        bar.hiddenPaths = fitted.hidden.map(function (crumb) {
            return crumb.path
        })
        bar.crumbs = placed
    }

    Repeater {
        model: bar.crumbs

        Item {
            id: slot
            required property int index
            required property var modelData

            x: slot.modelData.x
            width: slot.modelData.width
            height: bar.plateHeight
            anchors.verticalCenter: parent.verticalCenter
            Accessible.role: Accessible.Link
            Accessible.name: slot.modelData.ellipsis === true
                ? "显示完整路径" : slot.modelData.name

            // The separator belongs to the crumb on its right: that is the only
            // crumb whose plate width the fit added a `unit` for.
            Text {
                anchors.verticalCenter: parent.verticalCenter
                x: -bar.crumbGap - width
                visible: slot.index > 0
                text: "\u203A"
                color: bar.mutedColor
                font.pixelSize: bar.fontSize
                Accessible.ignored: true
            }

            Rectangle {
                id: plate
                anchors.fill: parent
                radius: 5
                color: slot.modelData.current !== true && plateMouse.containsMouse
                    ? (bar.dark ? "#1fffffff" : "#0f000000") : "transparent"

                ThemedIcon {
                    id: glyph
                    anchors.left: parent.left
                    anchors.leftMargin: bar.crumbPad
                    anchors.verticalCenter: parent.verticalCenter
                    visible: slot.modelData.ellipsis !== true
                    isDir: true
                    category: "folder"
                    size: bar.iconSize
                    dark: bar.dark
                    accent: bar.accent
                    opacity: slot.modelData.current === true ? 1.0 : 0.55
                    Accessible.ignored: true
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: slot.modelData.ellipsis === true
                        ? bar.crumbPad : bar.crumbPad + bar.iconSize + bar.iconGap
                    anchors.right: parent.right
                    anchors.rightMargin: bar.crumbPad
                    anchors.verticalCenter: parent.verticalCenter
                    text: slot.modelData.ellipsis === true ? "\u2026" : slot.modelData.name
                    color: slot.modelData.current === true
                        ? bar.textColor
                        : (plateMouse.containsMouse ? bar.textColor : bar.mutedColor)
                    font.pixelSize: bar.fontSize
                    elide: Text.ElideMiddle
                }

                MouseArea {
                    id: plateMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: slot.modelData.current === true
                        ? Qt.ArrowCursor : Qt.PointingHandCursor

                    onClicked: {
                        if (slot.modelData.ellipsis === true) {
                            const anchor = plate.mapToItem(null, 0, plate.height)
                            bar.ancestorsRequested(anchor.x, anchor.y,
                                                   bar.hiddenNames, bar.hiddenPaths)
                        } else if (slot.modelData.current !== true) {
                            bar.crumbActivated(slot.modelData.path)
                        }
                    }
                }
            }
        }
    }
}
