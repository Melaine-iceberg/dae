pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import dae.m3 1.0

// The Chrome Desktop pill Omnibox: one capsule control that serves as the
// breadcrumb trail, surface readout, path editor with autocompletion, and
// search input, complete with a leading location chip and trailing bookmark
// star + copy-path actions.
Item {
    id: bar

    /// "breadcrumb", "address", "search", or "surface".
    property string mode: "breadcrumb"
    /// False for a surface label (overview, recycle bin) and read-only trails.
    property bool interactive: true
    /// Draws the shared pill well.
    property bool framed: true
    /// In split view the active pane's bar is outlined in the accent.
    property bool paneFocused: false
    /// Non-folder surface title and glyph (overview, trash).
    property string surfaceTitle: ""
    property string surfaceGlyph: "home"
    property bool isRemote: false
    property bool isFavorite: false

    /// The listed directory's ancestors, outermost first.
    property var names: []
    property var paths: []

    property bool dark: true
    property color accent: Theme.accent
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkSecondary

    property string addressPath: ""
    property string searchMode: "name"
    property bool searchBusy: false
    property string searchText: ""

    signal crumbActivated(string path)
    signal ancestorsRequested(real x, real y, var names, var paths)
    signal siblingsRequested(real x, real y, string path)
    signal filesDropped(var paths, string path)
    signal contextMenuRequested(real x, real y)
    signal addressRequested()
    signal addressAccepted(string path)
    signal addressCanceled()
    signal searchEdited(string value)
    signal searchCleared()
    signal searchDismissed()
    signal favoriteToggled()
    signal copyPathRequested()

    implicitHeight: 34
    height: 34

    readonly property bool editing: mode === "address"
    readonly property bool searching: mode === "search"
    readonly property bool surfaceOnly: mode === "surface"
    readonly property bool breadcrumb: mode === "breadcrumb"
    readonly property bool inputFocused: addressInput.activeFocus
        || searchInput.activeFocus
    readonly property bool showTrailingActions: bar.framed && bar.interactive
        && bar.breadcrumb && bar.width >= 260
    readonly property real leadingOffset: bar.framed ? 36 : 8
    readonly property real trailingReserve: bar.showTrailingActions
        ? (bar.isRemote ? 34 : 62) : (bar.searching ? 34 : 10)

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
            bar.recentPaths = Workspace.recentDirectories()
            Qt.callLater(bar.focusAddress)
        } else if (mode === "search") {
            bar.setSearchText(bar.searchText)
            Qt.callLater(bar.focusSearch)
        }
    }

    onSearchTextChanged: bar.setSearchText(searchText)

    HoverHandler {
        id: wellHover
    }

    // Chrome Omnibox capsule well: softly tinted at rest, lifting to the
    // toolbar surface with a crisp accent ring when focused or editing.
    Rectangle {
        id: well
        anchors.fill: parent
        visible: bar.framed
        radius: height / 2
        color: (bar.inputFocused || bar.editing || bar.searching)
            ? Theme.toolbar
            : (wellHover.hovered ? Theme.fieldHover : Theme.field)
        border.width: (bar.inputFocused || bar.paneFocused)
            ? 2
            : ((bar.editing || bar.searching) ? 1 : 0)
        border.color: (bar.inputFocused || bar.paneFocused)
            ? bar.accent : Theme.hairline

        Behavior on color {
            enabled: Theme.motion
            ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
        }
    }

    // Leading context icon chip (Chrome's site-info / search pill on the left
    // of the Omnibox).
    Item {
        id: leadingChip
        visible: bar.framed
        x: 5
        anchors.verticalCenter: parent.verticalCenter
        width: 24
        height: 24

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: (bar.editing || bar.searching)
                ? Theme.accentSoft
                : (leadingMouse.containsMouse ? Theme.hover
                    : (Theme.dark ? Theme.surfaceContainer : Theme.toolbar))

            Behavior on color {
                enabled: Theme.motion
                ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
            }

            InkRipple { id: leadingRipple; radius: 12 }
        }

        Glyph {
            anchors.centerIn: parent
            width: 13
            height: 13
            kind: bar.searching ? "search"
                : (bar.surfaceOnly ? bar.surfaceGlyph
                    : (bar.isRemote ? "cloud" : "folder"))
            tint: (bar.editing || bar.searching) ? bar.accent : Theme.inkSecondary
        }

        Accessible.role: Accessible.Button
        Accessible.name: bar.searching ? qsTr("搜索")
            : (bar.surfaceOnly ? bar.surfaceTitle : qsTr("编辑路径"))
        Accessible.onPressAction: {
            if (bar.interactive && bar.breadcrumb)
                bar.addressRequested()
        }

        MouseArea {
            id: leadingMouse
            anchors.fill: parent
            enabled: bar.interactive
            hoverEnabled: true
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onPressed: function (mouse) { leadingRipple.burst(mouse.x, mouse.y) }
            onReleased: leadingRipple.release()
            onCanceled: leadingRipple.release()
            onClicked: function (mouse) {
                if (bar.breadcrumb)
                    bar.addressRequested()
                else if (bar.editing)
                    bar.focusAddress()
            }
        }
    }

    // Surface title readout when showing Overview or Recycle Bin inside the
    // unified Omnibox pill.
    Text {
        visible: bar.surfaceOnly
        anchors.left: parent.left
        anchors.leftMargin: bar.leadingOffset
        anchors.right: parent.right
        anchors.rightMargin: 14
        anchors.verticalCenter: parent.verticalCenter
        text: bar.surfaceTitle
        color: bar.textColor
        font.pixelSize: Theme.typeBody
        font.weight: Theme.weightStrong
        elide: Text.ElideRight
    }

    // ── Breadcrumb ──────────────────────────────────────────────────────────
    readonly property real fontSize: Theme.typeRow
    readonly property real crumbGap: 3
    readonly property real crumbPad: 9
    readonly property real chevronWidth: 13
    readonly property real plateHeight: 24
    readonly property real ellipsisWidth: 24
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

    readonly property real unit: separatorMeasure.advanceWidth
        + bar.crumbGap * 2

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
        total += bar.unit * (count - 1)
        if (total <= trail.width + 1)
            return { "visible": all, "hidden": [] }

        let used = widths[0] + bar.unit + bar.ellipsisWidth + bar.unit
            + widths[count - 1]
        let start = count - 1
        while (start > 1 && used + widths[start - 1] + bar.unit <= trail.width) {
            used += widths[start - 1] + bar.unit
            start -= 1
        }

        if (start <= 1)
            return { "visible": all, "hidden": [] }

        const visible = [all[0], { "ellipsis": true }]
        for (let index = start; index < count; index += 1)
            visible.push(all[index])
        return { "visible": visible, "hidden": all.slice(1, start) }
    }

    function place(entries) {
        let x = 0
        for (let index = 0; index < entries.length; index += 1) {
            const entry = entries[index]
            entry.width = entry.ellipsis === true
                ? bar.ellipsisWidth : bar.crumbWidth(entry.name, entry.current)
            entry.x = index === 0 ? 0 : x + bar.unit
            x = entry.x + entry.width
        }

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
    onTrailingReserveChanged: Qt.callLater(bar.rebuild)
    Component.onCompleted: bar.rebuild()

    Item {
        id: trail
        x: bar.leadingOffset
        y: 0
        width: Math.max(0, bar.width - bar.leadingOffset - bar.trailingReserve)
        height: bar.height
        visible: bar.breadcrumb
        clip: true

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

                Rectangle {
                    anchors.fill: parent
                    visible: slot.dropActive
                    radius: height / 2
                    color: Theme.accentSoft
                    border.width: 2
                    border.color: Theme.accent
                    z: 5
                    Accessible.ignored: true
                }

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
                    radius: height / 2
                    color: slot.modelData.current === true
                        ? Theme.selection
                        : ((plateMouse.containsMouse || siblingMouse.containsMouse)
                            ? Theme.hover : "transparent")

                    Behavior on color {
                        enabled: Theme.motion
                        ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
                    }

                    InkRipple {
                        id: plateRipple
                        radius: plate.height / 2
                    }

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

                    Item {
                        id: siblingButton

                        anchors.right: parent.right
                        anchors.rightMargin: 2
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

                        onPressed: function (mouse) {
                            if (mouse.button === Qt.LeftButton && slot.modelData.current !== true)
                                plateRipple.burst(mouse.x, mouse.y)
                        }
                        onReleased: plateRipple.release()
                        onCanceled: plateRipple.release()
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

    // Trailing inline Chrome Omnibox actions: Copy Path & Bookmark Star.
    Row {
        id: trailingActions
        visible: bar.showTrailingActions
        anchors.right: parent.right
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Item {
            width: 24
            height: 24

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: copyPathMouse.containsMouse ? Theme.hover : "transparent"

                InkRipple { id: copyPathRipple; radius: 12 }
            }

            Glyph {
                anchors.centerIn: parent
                width: 13
                height: 13
                kind: "copy"
                tint: copyPathMouse.containsMouse ? bar.textColor : Theme.inkTertiary
            }

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("复制路径")
            Accessible.onPressAction: bar.copyPathRequested()

            MouseArea {
                id: copyPathMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onPressed: function (mouse) { copyPathRipple.burst(mouse.x, mouse.y) }
                onReleased: copyPathRipple.release()
                onCanceled: copyPathRipple.release()
                onClicked: bar.copyPathRequested()
            }

            ToolTip { text: qsTr("复制路径") }
        }

        Item {
            visible: !bar.isRemote
            width: 24
            height: 24

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: bar.isFavorite
                    ? Theme.accentSoft
                    : (starMouse.containsMouse ? Theme.hover : "transparent")

                Behavior on color {
                    enabled: Theme.motion
                    ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
                }

                InkRipple { id: starRipple; radius: 12 }
            }

            Glyph {
                anchors.centerIn: parent
                width: 14
                height: 14
                kind: "star"
                wash: bar.isFavorite ? 0.9 : 0.25
                tint: bar.isFavorite
                    ? bar.accent
                    : (starMouse.containsMouse ? bar.textColor : Theme.inkTertiary)
            }

            Accessible.role: Accessible.Button
            Accessible.name: bar.isFavorite ? qsTr("取消固定") : qsTr("固定到侧边栏")
            Accessible.onPressAction: bar.favoriteToggled()

            MouseArea {
                id: starMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onPressed: function (mouse) { starRipple.burst(mouse.x, mouse.y) }
                onReleased: starRipple.release()
                onCanceled: starRipple.release()
                onClicked: bar.favoriteToggled()
            }

            ToolTip {
                text: bar.isFavorite ? qsTr("取消固定") : qsTr("固定到侧边栏")
            }
        }
    }

    // ── Address ─────────────────────────────────────────────────────────────
    TextInput {
        id: addressInput
        Accessible.role: Accessible.EditableText
        Accessible.name: qsTr("路径")

        anchors.left: parent.left
        anchors.leftMargin: bar.leadingOffset
        anchors.right: parent.right
        anchors.rightMargin: 14
        anchors.top: parent.top
        anchors.bottom: parent.bottom
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
        Keys.onTabPressed: function (event) {
            event.accepted = bar.acceptCompletion(true)
        }

        onAccepted: {
            if (bar.acceptCompletion(false))
                return
            let target = text.trim()
            if (target.length >= 2
                    && target.charAt(0) === '"'
                    && target.charAt(target.length - 1) === '"')
                target = target.substring(1, target.length - 1)
            const expanded = String(Workspace.expandTypedPath(target))
            focus = false
            if (expanded.length > 0)
                bar.addressAccepted(expanded)
        }
    }

    /// ── Address completion ─────────────────────────────────────────────────
    property var completions: []
    property int completionIndex: -1
    property string completionDir: ""
    property var completionNames: []
    property var recentPaths: []

    function splitTypedPath(text) {
        let cut = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"))
        if (cut < 0)
            return { "parent": "", "prefix": "" }
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

    Shadow {
        target: completionPopup
        z: 59
    }

    Rectangle {
        id: completionPopup

        visible: bar.editing && bar.completions.length > 0
        x: 0
        y: bar.height + 6
        width: bar.width
        height: completionColumn.implicitHeight + 10
        radius: Theme.radiusLg
        color: "transparent"
        border.width: 1
        border.color: Theme.overlayLine
        z: 60

        Glass { radius: Theme.radiusLg }
        Accessible.role: Accessible.List

        Column {
            id: completionColumn

            x: 5
            y: 5
            width: parent.width - 10
            spacing: 1

            Repeater {
                model: bar.completions

                Rectangle {
                    id: completionRow

                    required property int index
                    required property string modelData

                    readonly property bool chosen: bar.completionIndex === completionRow.index

                    width: completionColumn.width
                    height: 32
                    radius: Theme.radiusMd
                    color: chosen ? Theme.selection
                        : (itemMouse.containsMouse ? Theme.hover : "transparent")

                    InkRipple {
                        id: completionRipple
                        radius: Theme.radiusMd
                    }

                    Accessible.role: Accessible.ListItem
                    Accessible.name: completionRow.modelData
                    Accessible.selectable: true
                    Accessible.selected: completionRow.chosen
                    Accessible.onPressAction: {
                        bar.completionIndex = completionRow.index
                        bar.acceptCompletion(false)
                    }

                    Glyph {
                        x: 10
                        anchors.verticalCenter: parent.verticalCenter
                        width: 14
                        height: 14
                        kind: bar.splitTypedPath(addressInput.text).parent === ""
                            ? "clock" : "folder"
                        tint: completionRow.chosen ? Theme.selectionInk : Theme.inkTertiary
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 32
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: completionRow.modelData
                        color: completionRow.chosen
                            ? Theme.selectionInk : bar.textColor
                        font.family: Theme.monoFamily
                        font.pixelSize: Theme.typeCaption
                        font.weight: completionRow.chosen
                            ? Theme.weightEmphasis : Font.Normal
                        elide: Text.ElideMiddle
                    }

                    MouseArea {
                        id: itemMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPressed: function (mouse) { completionRipple.burst(mouse.x, mouse.y) }
                        onReleased: completionRipple.release()
                        onCanceled: completionRipple.release()
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
    Text {
        anchors.left: parent.left
        anchors.leftMargin: bar.leadingOffset
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
        anchors.left: parent.left
        anchors.leftMargin: bar.leadingOffset
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
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        width: 24
        height: 24
        visible: bar.searching

        enabled: !bar.searchBusy && searchInput.text.length > 0
        Accessible.role: Accessible.Button
        Accessible.name: qsTr("清除搜索")
        Accessible.onPressAction: {
            searchInput.text = ""
            bar.searchCleared()
        }

        ChromeSpinner {
            anchors.centerIn: searchTrailing
            visible: bar.searchBusy
            width: 14
            height: 14
            running: bar.searchBusy
            color: bar.accent
            strokeWidth: 1.8
            reduceMotion: !Theme.motion
        }

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            visible: !bar.searchBusy && searchInput.text.length > 0
            color: clearSearchMouse.containsMouse ? Theme.hover : "transparent"
            InkRipple { id: clearSearchRipple; radius: 12 }
        }

        Glyph {
            anchors.centerIn: searchTrailing
            visible: !bar.searchBusy && searchInput.text.length > 0
            width: 10
            height: 10
            kind: "close"
            tint: clearSearchMouse.containsMouse ? bar.textColor : bar.mutedColor
        }

        MouseArea {
            id: clearSearchMouse
            anchors.fill: searchTrailing
            hoverEnabled: true
            enabled: !bar.searchBusy && searchInput.text.length > 0
            cursorShape: Qt.PointingHandCursor
            onPressed: function (mouse) { clearSearchRipple.burst(mouse.x, mouse.y) }
            onReleased: clearSearchRipple.release()
            onCanceled: clearSearchRipple.release()
            onClicked: {
                searchInput.text = ""
                bar.searchCleared()
            }
        }
    }
}
