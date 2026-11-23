pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The navigation row's one input control: an M3 search-bar pill that becomes a
// breadcrumb trail, a path editor or a search field.
//
// The modes are states of this bar, not three widgets:
//
//   breadcrumb  every ancestor is a destination; the current folder is a
//               `secondaryContainer` pill, exactly like a selected list row.
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
    /// False for a surface label (overview, recycle bin) and for the
    /// split pane's trail.
    property bool interactive: true
    /// Draws the shared field well. The split pane keeps the bare trail.
    property bool framed: true
    /// §11.4: this pane is the active one. In split view the active pane's
    /// bar is outlined in the accent and the inactive one in `line`; with a
    /// single pane the property stays false and the bar keeps the hairline.
    property bool paneFocused: false

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
    /// A segment's `⌄`: open the menu of directories beside `path`, anchored
    /// below the chevron in window coordinates; `path` is the segment itself.
    signal siblingsRequested(real x, real y, string path)
    /// Files dropped on a segment: move them into that ancestor directory.
    signal filesDropped(var paths, string path)
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

    implicitHeight: Theme.controlXl
    height: Theme.controlXl

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
            // The recent directories back the completion until the typed text
            // grows a separator; they arrive once per edit, not per keystroke.
            bar.recentPaths = Workspace.recentDirectories()
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
        /// M3 filled fields carry no stroke at rest; the 2 px primary ring
        /// appears with the keyboard or the active split pane.
        border.width: bar.inputFocused || bar.paneFocused ? 2 : 0
        border.color: bar.accent
    }

    // ── Breadcrumb ──────────────────────────────────────────────────────────
    // The trail is laid out here rather than by a `Row`: the fold decision is a
    // measured one, so the same numbers have to position the crumbs that
    // decided whether they fit.
    readonly property real fontSize: Theme.typeBody
    readonly property real crumbGap: 3
    readonly property real crumbPad: 8
    /// The `⌄` between a segment's name and the next separator (§7.2). It is
    /// part of the measured width, so the fold decision accounts for it.
    readonly property real chevronWidth: 12
    readonly property real plateHeight: Theme.controlMd
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
        return nameWidth + bar.crumbPad * 2 + (bar.interactive ? bar.chevronWidth : 0)
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

                /// §10.3: a dragged file dropped on a segment moves into that
                /// ancestor. The ellipsis stands for several ancestors at
                /// once, so it is not a target.
                property bool dropActive: false

                DropArea {
                    anchors.fill: parent
                    keys: ["text/uri-list"]
                    enabled: slot.modelData.ellipsis !== true

                    onEntered: function (drag) {
                        slot.dropActive = true
                        drag.accept(Qt.CopyAction)
                    }
                    onPositionChanged: function (drag) {
                        slot.dropActive = true
                        drag.accept(Qt.CopyAction)
                    }
                    onExited: slot.dropActive = false
                    onDropped: function (drop) {
                        slot.dropActive = false
                        const urls = []
                        for (let index = 0; index < drop.urls.length; index += 1)
                            urls.push(String(drop.urls[index]))
                        // qmllint disable missing-property
                        const paths = Workspace.localDropPaths(urls)
                        if (paths.length === 0) {
                            drop.accepted = false
                            return
                        }
                        // qmllint enable missing-property
                        drop.accept(Qt.CopyAction)
                        bar.filesDropped(paths, slot.modelData.path)
                    }
                }

                /// Same feedback as a row: a primary wash and a 2 dp accent
                /// frame. It sits above the plate and below the mouse areas,
                /// so a drop never turns into a click.
                Rectangle {
                    anchors.fill: parent
                    visible: slot.dropActive
                    radius: Theme.radiusSm
                    color: Theme.accentSoft
                    border.width: 2
                    border.color: Theme.accent
                    z: 5
                    Accessible.ignored: true
                }

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
                    radius: Theme.radiusSm
                    // The current folder is a selected destination, so it takes
                    // the same `secondaryContainer` plate as the sidebar; the
                    // ancestors only gain the hover state layer.
                    color: slot.modelData.current === true
                        ? Theme.selection
                        : ((plateMouse.containsMouse || siblingMouse.containsMouse)
                            ? Theme.hover : "transparent")

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: bar.crumbPad
                        anchors.right: siblingButton.left
                        anchors.rightMargin: 0
                        anchors.verticalCenter: parent.verticalCenter
                        text: slot.modelData.ellipsis === true
                            ? "\u2026" : slot.modelData.name
                        color: slot.modelData.current === true
                            ? Theme.selectionInk
                            : ((plateMouse.containsMouse || siblingMouse.containsMouse)
                                ? bar.textColor : bar.mutedColor)
                        font.pixelSize: bar.fontSize
                        font.weight: slot.modelData.current === true
                            ? Theme.weightEmphasis : Font.Normal
                        elide: Text.ElideMiddle
                    }

                    /// §7.2: the segment's own `⌄` opens that level's sibling
                    /// directories. The segment body still navigates, so the
                    /// two gestures never fight: the chevron owns only its
                    /// 12 dp box.
                    Item {
                        id: siblingButton

                        anchors.right: parent.right
                        anchors.rightMargin: 1
                        anchors.verticalCenter: parent.verticalCenter
                        width: bar.chevronWidth
                        height: parent.height
                        visible: bar.interactive && slot.modelData.ellipsis !== true

                        Glyph {
                            anchors.centerIn: parent
                            width: 10
                            height: 10
                            kind: "chevronDown"
                            tint: siblingMouse.containsMouse
                                ? bar.textColor
                                : (slot.modelData.current === true
                                    ? Theme.selectionInk : Theme.inkTertiary)
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("打开同级目录")
                        Accessible.onPressAction: {
                            const anchor = siblingButton.mapToItem(null, 0,
                                                                  siblingButton.height)
                            bar.siblingsRequested(anchor.x, anchor.y,
                                                  slot.modelData.path)
                        }

                        MouseArea {
                            id: siblingMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                const anchor = siblingButton.mapToItem(null, 0,
                                                                      siblingButton.height)
                                bar.siblingsRequested(anchor.x, anchor.y,
                                                      slot.modelData.path)
                            }
                        }
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
        anchors.leftMargin: 16
        anchors.rightMargin: 16
        visible: bar.editing
        enabled: bar.interactive
        verticalAlignment: TextInput.AlignVCenter
        text: ""
        color: bar.textColor
        selectionColor: bar.accent
        selectedTextColor: Theme.inkOnAccent
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
            if (!activeFocus)
                bar.closeCompletions()
        }

        onTextEdited: bar.updateCompletions()

        Keys.onEscapePressed: {
            if (bar.completions.length > 0) {
                bar.closeCompletions()
                return
            }
            focus = false
        }
        Keys.onDownPressed: function (event) {
            event.accepted = bar.completions.length > 0
            if (event.accepted)
                bar.moveCompletion(1)
        }
        Keys.onUpPressed: function (event) {
            event.accepted = bar.completions.length > 0
            if (event.accepted)
                bar.moveCompletion(-1)
        }
        /// Tab takes the highlighted directory and keeps the field open; with
        /// no list it stays an ordinary focus move, so the field can still be
        /// left from the keyboard.
        Keys.onTabPressed: function (event) {
            event.accepted = bar.acceptCompletion(true)
        }

        onAccepted: {
            // Enter takes the highlighted completion when there is one; the
            // raw text is the fallback, which is what a fully typed path is.
            if (bar.acceptCompletion(false))
                return
            let target = text.trim()
            if (target.length >= 2
                    && target.charAt(0) === '"'
                    && target.charAt(target.length - 1) === '"')
                target = target.substring(1, target.length - 1)
            // `~` and environment shorthands expand here, after the quote
            // strip and before the navigation; anything unresolvable stays
            // as typed and the navigation says so.
            target = Workspace.expandTypedPath(target)
            focus = false
            if (target.length > 0)
                bar.addressAccepted(target)
        }
    }

    /// ── Address completion (§7.2) ──────────────────────────────────────────
    /// The typed directory's subdirectories, filtered by the segment being
    /// typed. The directory is read once per parent change, not once per
    /// keystroke: a folder with 120,000 entries would otherwise pay a
    /// `readdir` for every letter.
    property var completions: []
    property int completionIndex: -1
    property string completionDir: ""
    property var completionNames: []
    /// The recently visited directories, the completion's source until the
    /// typed text has a separator.
    property var recentPaths: []

    function splitTypedPath(text) {
        let cut = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"))
        if (cut < 0)
            return { "parent": "", "prefix": "" }
        // A drive root keeps its separator (`C:\`), which is what makes the
        // first typed segment complete at all.
        return {
            "parent": text.substring(0, cut + 1),
            "prefix": text.substring(cut + 1)
        }
    }

    function closeCompletions() {
        bar.completions = []
        bar.completionIndex = -1
    }

    function updateCompletions() {
        if (!bar.editing || !bar.interactive) {
            bar.closeCompletions()
            return
        }
        const split = bar.splitTypedPath(addressInput.text)
        if (split.parent === "") {
            // No separator yet: the history is the source, newest first.
            // A prefix that matches nothing closes the list, exactly the
            // way the directory completion does.
            const prefix = split.prefix.toLowerCase()
            const matches = []
            const names = bar.recentPaths
            for (let index = 0; index < names.length && matches.length < 8; index += 1) {
                const path = String(names[index])
                if (prefix === "" || path.toLowerCase().indexOf(prefix) === 0)
                    matches.push(path)
            }
            bar.completions = matches
            bar.completionIndex = matches.length > 0 ? 0 : -1
            return
        }
        if (split.parent !== bar.completionDir) {
            bar.completionDir = split.parent
            bar.completionNames = Workspace.subdirectories(split.parent)
        }
        const prefix = split.prefix.toLowerCase()
        const matches = []
        const names = bar.completionNames
        for (let index = 0; index < names.length && matches.length < 8; index += 1) {
            const name = String(names[index])
            if (prefix === "" || name.toLowerCase().indexOf(prefix) === 0)
                matches.push(split.parent + name)
        }
        bar.completions = matches
        bar.completionIndex = matches.length > 0 ? 0 : -1
    }

    function moveCompletion(step) {
        if (bar.completions.length === 0)
            return
        const count = bar.completions.length
        bar.completionIndex = (bar.completionIndex + step + count) % count
    }

    /// `continueEditing` is the Tab case: it fills the field and stays in it,
    /// because a completed directory is usually the first half of a path.
    /// Enter and a click navigate.
    function acceptCompletion(continueEditing) {
        if (bar.completions.length === 0 || bar.completionIndex < 0)
            return false
        const target = bar.completions[bar.completionIndex]
        bar.closeCompletions()
        if (continueEditing) {
            const separator = target.indexOf("\\") >= 0 ? "\\" : "/"
            addressInput.text = /[\\/]$/.test(target) ? target : target + separator
            addressInput.cursorPosition = addressInput.text.length
            bar.updateCompletions()
            return true
        }
        addressInput.focus = false
        bar.addressAccepted(target)
        return true
    }

    /// The list itself: 28 dp rows on a `surfaceContainerHigh` plate, drawn
    /// above the well and outside the bar's box, so it reads as an attachment
    /// to the field rather than a second control. The shadow is declared before
    /// the plate and in the same coordinate space, as `Shadow` requires.
    Shadow {
        target: completionPopup
        z: 59
    }

    Rectangle {
        id: completionPopup

        visible: bar.editing && bar.completions.length > 0
        x: 0
        y: bar.height + 4
        width: bar.width
        height: completionColumn.implicitHeight + 8
        radius: Theme.radiusMd
        color: "transparent"
        border.width: 0
        z: 60

        Glass { radius: Theme.radiusMd }
        Accessible.role: Accessible.List

        Column {
            id: completionColumn

            x: 4
            y: 4
            width: parent.width - 8

            Repeater {
                model: bar.completions

                Rectangle {
                    id: completionRow

                    required property int index
                    required property string modelData

                    width: completionColumn.width
                    height: 28
                    radius: Theme.radiusSm
                    color: bar.completionIndex === completionRow.index ? Theme.selection
                        : (itemMouse.containsMouse ? Theme.hover : "transparent")

                    Accessible.role: Accessible.ListItem
                    Accessible.name: completionRow.modelData
                    Accessible.selectable: true
                    Accessible.selected: bar.completionIndex === completionRow.index
                    Accessible.onPressAction: {
                        bar.completionIndex = completionRow.index
                        bar.acceptCompletion(false)
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 8
                        anchors.right: parent.right
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        text: completionRow.modelData
                        color: bar.completionIndex === completionRow.index
                            ? Theme.selectionInk : bar.textColor
                        font.family: Theme.monoFamily
                        font.pixelSize: Theme.typeCaption
                        font.weight: bar.completionIndex === completionRow.index
                            ? Theme.weightEmphasis : Font.Normal
                        elide: Text.ElideMiddle
                    }

                    MouseArea {
                        id: itemMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            bar.completionIndex = completionRow.index
                            bar.acceptCompletion(false)
                        }
                    }
                }
            }
        }
    }

    // ── Search ──────────────────────────────────────────────────────────────
    Glyph {
        id: searchLeading
        x: 14
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
        selectedTextColor: Theme.inkOnAccent
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

        M3Loader {
            anchors.centerIn: searchTrailing
            visible: bar.searchBusy
            width: 14
            height: 14
            running: bar.searchBusy
        }

        Glyph {
            anchors.centerIn: searchTrailing
            visible: !bar.searchBusy && searchInput.text.length > 0
            width: 10
            height: 10
            kind: "close"
            tint: bar.mutedColor
        }

        MouseArea {
            anchors.fill: searchTrailing
            enabled: !bar.searchBusy && searchInput.text.length > 0
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                searchInput.text = ""
                bar.searchCleared()
            }
        }
    }
}
