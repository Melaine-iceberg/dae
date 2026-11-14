pragma ComponentBehavior: Bound
import QtQuick

// The navigation row's one input control: a breadcrumb trail, a path editor
// and a search field sharing a single 32 dp frame (proposal §7.2).
//
// The modes are states of this bar, not three widgets:
//
//   breadcrumb  every ancestor is a destination; the current folder is the one
//               name the bar must keep. Clicking the empty part types a path.
//   address     the whole frame becomes a mono text field seeded with the
//               current path, all selected, Enter navigates, Escape returns.
//   search      the frame holds the query. The filter row below it carries the
//               mode, scope and filters; this control only types and clears.
//
// `ShellWindow` owns the mode (`pathEditing` / `searchOpen`) and the query, so
// a tab switch resets both without this component keeping stale state. The
// split pane instantiates it with `framed: false` and `interactive: false`,
// which is the old read-only trail.
Item {
    id: bar

    /// "breadcrumb", "address" or "search". The shell derives it from
    /// `pathEditing` and `searchOpen` so only one mode can be active.
    property string mode: "breadcrumb"
    /// False for a surface label (overview, spaces, recycle bin) and for the
    /// split pane's trail.
    property bool interactive: true
    /// Draws the shared field well. The split pane keeps the bare trail.
    property bool framed: true

    /// The listed directory's ancestors, outermost first, exactly as the
    /// backend spelled them. The two are parallel lists, so the shorter wins.
    property var names: []
    property var paths: []

    property bool dark: true
    property color accent: Theme.accent
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkSecondary

    /// Seeds the address mode. Read when the mode becomes "address" so typing
    /// over the path never drops a binding.
    property string addressPath: ""
    /// "name" or "content". Drives the search placeholder only.
    property string searchMode: "name"
    property bool searchBusy: false
    /// "name" only, but kept as a property so the escape rule does not have to
    /// know about the query state.
    property string searchText: ""

    /// The path of the ancestor that was clicked.
    signal crumbActivated(string path)
    /// The ancestors hidden behind the `…`, with the position to open a menu
    /// at — in window coordinates, because the menu belongs to the window.
    signal ancestorsRequested(real x, real y, var names, var paths)
    /// A right-click on the bar itself: copy path, copy name, terminal, edit
    /// path. Window coordinates for the same reason as above.
    signal contextMenuRequested(real x, real y)
    /// A click on the empty part of the trail: the shell switches to address
    /// mode. (Typing itself is handled by the address field.)
    signal addressRequested()
    signal addressAccepted(string path)
    /// The address field lost focus without Enter.
    signal addressCanceled()
    signal searchEdited(string value)
    signal searchCleared()
    /// Escape on an empty search field; the shell collapses search mode.
    signal searchDismissed()

    implicitHeight: Theme.controlLg
    height: Theme.controlLg

    readonly property bool editing: mode === "address"
    readonly property bool searching: mode === "search"
    readonly property bool breadcrumb: mode === "breadcrumb"
    readonly property bool inputFocused: addressInput.activeFocus
        || searchInput.activeFocus

    function focusAddress() {
        if (mode !== "address")
            return
        addressInput.forceActiveFocus()
        addressInput.selectAll()
    }

    function focusSearch() {
        if (mode !== "search")
            return
        searchInput.forceActiveFocus()
        searchInput.selectAll()
    }

    function setSearchText(value) {
        if (searchInput.text !== value)
            searchInput.text = value
    }

    onModeChanged: {
        if (mode === "address") {
            addressInput.text = bar.addressPath
            // The field is only visible once the mode has been laid out; a
            // hidden item cannot take focus.
            Qt.callLater(bar.focusAddress)
        } else if (mode === "search") {
            bar.setSearchText(bar.searchText)
            Qt.callLater(bar.focusSearch)
        }
    }

    onSearchTextChanged: bar.setSearchText(searchText)

    Rectangle {
        id: well
        anchors.fill: parent
        visible: bar.framed
        radius: Theme.radiusSm
        color: Theme.field
        border.width: bar.inputFocused ? 1 : 0
        border.color: bar.accent
    }

    // ── Breadcrumb ──────────────────────────────────────────────────────────
    // The trail is laid out here rather than by a `Row`: the fold decision is a
    // measured one, so the same numbers have to position the crumbs that
    // decided whether they fit.
    readonly property real fontSize: Theme.typeBody
    readonly property real crumbGap: 3
    readonly property real crumbPad: 5
    readonly property real plateHeight: Theme.controlSm
    readonly property real ellipsisWidth: 22
    readonly property real maxAncestorWidth: 160
    readonly property real maxCurrentWidth: 256
    readonly property real minCurrentWidth: bar.crumbPad * 2 + bar.fontSize * 2

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
    readonly property real unit: separatorMeasure.advanceWidth
        + bar.crumbGap * 2

    /// What the bar paints: `{ name, path, current, x, width }` entries plus
    /// one `{ ellipsis: true, x, width }` when ancestors were swallowed by the
    /// menu.
    property var crumbs: []
    property var hiddenNames: []
    property var hiddenPaths: []

    function crumbWidth(name, current) {
        nameMeasure.text = name
        const nameWidth = Math.min(nameMeasure.advanceWidth,
                                   current ? bar.maxCurrentWidth
                                           : bar.maxAncestorWidth)
        return nameWidth + bar.crumbPad * 2
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
        if (total <= trail.width + 1)
            return { "visible": all, "hidden": [] }

        // Seeded with the current crumb, not just the first one: growing
        // towards a crumb that was never counted is what made the Tauri shell
        // report a row as fitting while it painted 70px too wide.
        let used = widths[0] + bar.unit + bar.ellipsisWidth + bar.unit
            + widths[count - 1]
        let start = count - 1
        while (start > 1 && used + widths[start - 1] + bar.unit <= trail.width) {
            used += widths[start - 1] + bar.unit
            start -= 1
        }

        // Even `root › … › current` is too wide. Painting everything and
        // letting the clip cut it is still better than folding to a bar that
        // names nothing: the current crumb truncates inside its own box.
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
        // folder. When even `root › … › current` is wider than the bar, the
        // last crumb is trimmed to what is left rather than cut off by the
        // clip; below the two-character floor the clip is the better answer.
        const last = entries[entries.length - 1]
        if (entries.length > 1) {
            const room = trail.width - last.x
            if (last.width > room)
                last.width = Math.min(last.width,
                                      Math.max(room, bar.minCurrentWidth))
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

    onNamesChanged: Qt.callLater(bar.rebuild)
    onPathsChanged: Qt.callLater(bar.rebuild)
    onWidthChanged: bar.rebuild()
    Component.onCompleted: bar.rebuild()

    Item {
        id: trail
        x: 8
        y: 0
        width: Math.max(0, bar.width - 16)
        height: bar.height
        visible: bar.breadcrumb
        clip: true

        // The empty stretches between crumbs belong to the bar too: a
        // left-click there starts typing a path, a right-click opens the bar
        // menu. Declared under the repeater, so a crumb's own press wins on its
        // plate. In the split pane it only takes the right button, leaving the
        // left one to the pane's focus target.
        MouseArea {
            Accessible.ignored: true
            anchors.fill: parent
            acceptedButtons: bar.interactive
                ? (Qt.LeftButton | Qt.RightButton) : Qt.RightButton
            cursorShape: bar.interactive ? Qt.IBeamCursor : Qt.ArrowCursor

            onClicked: function (mouse) {
                if (mouse.button === Qt.RightButton) {
                    const point = mapToItem(null, mouse.x, mouse.y)
                    bar.contextMenuRequested(point.x, point.y)
                    return
                }
                if (bar.interactive)
                    bar.addressRequested()
            }
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
                    ? qsTr("显示完整路径") : slot.modelData.name

                // The separator belongs to the crumb on its right: that is the
                // only crumb whose plate width the fit added a `unit` for.
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    x: -bar.crumbGap - width
                    visible: slot.index > 0
                    text: "\u203A"
                    color: Theme.inkTertiary
                    font.pixelSize: bar.fontSize
                    Accessible.ignored: true
                }

                Rectangle {
                    id: plate
                    anchors.fill: parent
                    radius: Theme.radiusXs
                    // The trail is structure, not a signal (§7.2): hover is the
                    // only state, and the current segment is distinguished by
                    // ink alone — no weight change, no icon.
                    color: slot.modelData.current !== true
                        && plateMouse.containsMouse ? Theme.hover : "transparent"

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: bar.crumbPad
                        anchors.right: parent.right
                        anchors.rightMargin: bar.crumbPad
                        anchors.verticalCenter: parent.verticalCenter
                        text: slot.modelData.ellipsis === true
                            ? "\u2026" : slot.modelData.name
                        color: slot.modelData.current === true
                            ? bar.textColor
                            : (plateMouse.containsMouse
                                ? bar.textColor : bar.mutedColor)
                        font.pixelSize: bar.fontSize
                        elide: Text.ElideMiddle
                    }

                    MouseArea {
                        id: plateMouse
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        hoverEnabled: true
                        cursorShape: slot.modelData.current === true
                            ? Qt.ArrowCursor : Qt.PointingHandCursor

                        onClicked: function (mouse) {
                            if (mouse.button === Qt.RightButton) {
                                const point = plateMouse.mapToItem(null,
                                                                   mouse.x, mouse.y)
                                bar.contextMenuRequested(point.x, point.y)
                                return
                            }
                            if (slot.modelData.ellipsis === true) {
                                const anchor = plate.mapToItem(null, 0,
                                                               plate.height)
                                bar.ancestorsRequested(anchor.x, anchor.y,
                                                       bar.hiddenNames,
                                                       bar.hiddenPaths)
                            } else if (slot.modelData.current !== true) {
                                bar.crumbActivated(slot.modelData.path)
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Address ─────────────────────────────────────────────────────────────
    TextInput {
        id: addressInput
        Accessible.role: Accessible.EditableText
        Accessible.name: qsTr("路径")

        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        visible: bar.editing
        enabled: bar.interactive
        verticalAlignment: TextInput.AlignVCenter
        text: ""
        color: bar.textColor
        selectionColor: bar.accent
        selectedTextColor: Theme.onAccent
        font.family: Theme.monoFamily
        font.pixelSize: Theme.typeCaption
        selectByMouse: true
        clip: true

        // TextInput assigns to `text` while the user edits, which permanently
        // drops any binding on it. The mode change seeds it instead, and losing
        // focus — Enter, Escape, a click elsewhere — leaves address mode.
        onActiveFocusChanged: {
            if (!activeFocus && bar.editing)
                bar.addressCanceled()
        }

        onAccepted: {
            let target = text.trim()
            if (target.length >= 2
                    && target.charAt(0) === '"'
                    && target.charAt(target.length - 1) === '"')
                target = target.substring(1, target.length - 1)
            focus = false
            if (target.length > 0)
                bar.addressAccepted(target)
        }

        Keys.onEscapePressed: focus = false
    }

    // ── Search ──────────────────────────────────────────────────────────────
    Glyph {
        id: searchLeading
        x: 8
        width: 14
        height: 14
        anchors.verticalCenter: parent.verticalCenter
        visible: bar.searching
        kind: "search"
        tint: Theme.inkSecondary
    }

    Text {
        anchors.left: searchLeading.right
        anchors.leftMargin: 6
        anchors.right: searchTrailing.left
        anchors.rightMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        visible: bar.searching && searchInput.text.length === 0
        text: bar.searchMode === "content" ? qsTr("搜索文件内容") : qsTr("搜索名称")
        color: Theme.inkTertiary
        font.pixelSize: Theme.typeBody
        elide: Text.ElideRight
    }

    TextInput {
        id: searchInput
        anchors.left: searchLeading.right
        anchors.leftMargin: 6
        anchors.right: searchTrailing.left
        anchors.rightMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        visible: bar.searching
        enabled: bar.interactive
        color: bar.textColor
        selectionColor: bar.accent
        selectedTextColor: Theme.onAccent
        font.pixelSize: Theme.typeBody
        selectByMouse: true
        clip: true
        activeFocusOnTab: true

        /// The placeholder above is drawn, not declared: without these the
        /// field is announced as an unnamed text box.
        Accessible.role: Accessible.EditableText
        Accessible.name: qsTr("搜索")
        Accessible.description: bar.searchMode === "content"
            ? qsTr("搜索文件内容") : qsTr("搜索名称")

        onTextEdited: bar.searchEdited(text)
        onAccepted: bar.searchEdited(text)

        Keys.onEscapePressed: {
            if (text.length > 0) {
                text = ""
                bar.searchEdited("")
                return
            }
            focus = false
            bar.searchDismissed()
        }
    }

    Item {
        id: searchTrailing
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 24
        visible: bar.searching

        /// The cross is a picture; this strip is the button, and it only exists
        /// as one when there is something to clear.
        enabled: !bar.searchBusy && searchInput.text.length > 0
        Accessible.role: Accessible.Button
        Accessible.name: qsTr("清除搜索")
        Accessible.onPressAction: {
            searchInput.text = ""
            bar.searchCleared()
        }

        Text {
            anchors.centerIn: parent
            visible: bar.searchBusy
            text: "\u2026"
            color: bar.mutedColor
            font.pixelSize: Theme.typeBody
        }

        Glyph {
            anchors.centerIn: parent
            visible: !bar.searchBusy && searchInput.text.length > 0
            width: 10
            height: 10
            kind: "close"
            tint: bar.mutedColor
        }

        MouseArea {
            anchors.fill: parent
            enabled: !bar.searchBusy && searchInput.text.length > 0
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                searchInput.text = ""
                bar.searchCleared()
            }
        }
    }
}
