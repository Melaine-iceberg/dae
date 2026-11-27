pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import dae.m3 1.0
import "Shell.js" as ShellState

// The Chrome-style tab strip: brand mark, fluid connected tabs with concave
// bottom fillets (rendered on the RHI via ChromeTabShape), vertical inactive
// dividers, inline new-tab button, and native window caption buttons — all in
// 38 dp (matching `kTitleBarDp` in `crates/dae-qt/cpp/window.h`).
Item {
    id: root

    property var window: null
    /// The window's `WindowController`. macOS reads the double-click
    /// preference through it; the other platforms never call it.
    property var controller: null
    property bool dark: true
    property string material: "none"
    /// Mirrors `Window.active` so the strip softens when unfocused like Chrome.
    property bool windowActive: true
    /// For the OS: `Window.title` comes from the same property.
    property string title: "dae"

    property var tabs: []
    property int activeIndex: 0
    property var hostWindow: null
    property color accent: Theme.accent

    /// The accepted transfers, parsed once per change so every tab chip can
    /// ask whether one of them lands in its directory.
    readonly property var transferRows: JSON.parse(Workspace.transfersJson)

    /// Tracks which inactive tab is currently hovered so adjacent dividers hide.
    property int hoveredIndex: -1

    /// A pressed tab that could still become a drag. Cleared before the native
    /// loop starts, so a stray move after the loop cannot begin a second one.
    property bool dragArmed: false
    property bool dragging: false
    property int dragIndex: -1
    property real dragStartX: 0
    property real dragStartY: 0
    /// A native drag ran and ended somewhere; the release it consumed must not
    /// activate the tab it left behind.
    property bool dragSuppressClick: false
    /// Where a hovering tab drag would insert, in bar coordinates; -1 while no
    /// tab drag is over this bar. Set by the window's `DropArea`.
    property real dropX: -1

    signal tabActivated(int index)
    signal tabClosed(int index)
    signal newTabRequested()
    signal tabMoved(int from, int to)
    signal tabDroppedOnWindow(int index, string token, real localX)
    signal tabTornOff(int index, real globalX, real globalY)
    /// Right-click on a chip, in bar coordinates.
    signal tabContextRequested(int index, real x, real y)
    /// Chrome-style leading tab-switcher chevron button clicked, in bar coordinates.
    signal tabSearchRequested(real x, real y)
    /// Files dropped on a chip: the shell moves them into that tab's folder.
    signal filesDropped(int index, var paths)

    implicitHeight: Theme.shellTitle
    height: Theme.shellTitle

    readonly property bool isMac: Qt.platform.os === "osx"
    /// Where the tab strip ends and the window buttons begin. The shell anchors
    /// its strip to this instead of repeating the button width in QML.
    readonly property real buttonWidth: isMac ? 0 : 46
    readonly property real buttonsWidth: buttonWidth * 3

    readonly property color chromeTint: Theme.backdropTint(material, dark, false)
    readonly property color solidChrome: windowActive ? Theme.tabStrip : Theme.tabStripInactive
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color toolbarColor: Theme.toolbar
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property bool maximized: window !== null
        && window.visibility === Window.Maximized
    /// The green button's own state on macOS.
    readonly property bool fullScreen: window !== null
        && window.visibility === Window.FullScreen

    /// On Windows the three window buttons are native non-client zones.
    readonly property bool nativeButtons: Qt.platform.os === "windows"
    readonly property int ncHover: nativeButtons && controller !== null
        ? controller.ncButtonHover : -1
    readonly property int ncPressed: nativeButtons && controller !== null
        ? controller.ncButtonPressed : -1
    readonly property string nativeIconFamily: controller !== null
        ? controller.captionIconFamily : ""
    readonly property color ncHoverFill: dark ? Qt.rgba(1, 1, 1, 0.08)
                                              : Qt.rgba(0, 0, 0, 0.05)
    readonly property color ncPressFill: dark ? Qt.rgba(1, 1, 1, 0.13)
                                              : Qt.rgba(0, 0, 0, 0.09)

    // ── Chrome tab metrics ──────────────────────────────────────────────────
    readonly property int tabMaxWidth: 216
    readonly property int tabMinWidth: 96
    readonly property int tabPinnedWidth: 44
    /// 34 dp tall starting at y = 6 inside the 40 dp strip so 6 dp of Chrome
    /// frame sits above the active tab and its 10 dp concave flares merge
    /// seamlessly into `pathBar`.
    readonly property int tabHeightValue: 34
    readonly property int tabTop: 6
    readonly property int tabGap: 0
    readonly property int flareWidth: 10
    /// The strip starts after the leading tab-search button (and traffic lights
    /// on macOS), leaving space for tab 0's left concave flare without overlap.
    readonly property int tabLeft: isMac ? 112 : 48
    readonly property int plusWidth: 28
    readonly property int plusGap: 14
    readonly property int arrowWidth: 26
    /// Reserve dragging caption air before the window buttons when possible.
    readonly property real stripWidth: Math.max(0, width - buttonsWidth - 8)

    readonly property int pinnedTabCount: {
        let count = 0
        for (let i = 0; i < tabs.length; i += 1) {
            if (tabs[i] && tabs[i].pinned === true)
                count += 1
        }
        return count
    }
    readonly property int normalTabCount: Math.max(0, tabs.length - pinnedTabCount)
    readonly property real availableStripForTabs: Math.max(0,
        stripWidth - tabLeft - plusWidth - plusGap)
    readonly property real normalTabWidth: {
        if (normalTabCount <= 0)
            return tabMaxWidth
        const remaining = availableStripForTabs - pinnedTabCount * tabPinnedWidth
        const ideal = Math.floor(remaining / normalTabCount)
        return Math.max(tabMinWidth, Math.min(tabMaxWidth, ideal))
    }

    function tabWidthFor(tab) {
        return tab && tab.pinned === true ? tabPinnedWidth : normalTabWidth
    }

    /// The chips' own total width before overflow scrolling.
    readonly property real tabsWidth: {
        let total = 0
        for (let i = 0; i < tabs.length; i += 1) {
            if (i > 0)
                total += tabGap
            total += tabWidthFor(tabs[i])
        }
        return total
    }

    /// True when the chips still exceed the strip even at `tabMinWidth`.
    readonly property bool tabsOverflow: tabs.length > 0
        && tabsWidth > availableStripForTabs

    readonly property real tabViewportLeft: tabLeft
        + (tabsOverflow ? arrowWidth : 0)
    readonly property real tabViewportWidth: Math.max(0,
        stripWidth - plusWidth - tabViewportLeft
        - (tabsOverflow ? arrowWidth : 0))

    /// Chrome places `+` directly beside the last tab while tabs fit, and
    /// pins it to the right of the strip once tabs overflow.
    readonly property real plusLeft: tabsOverflow
        ? Math.max(tabLeft, stripWidth - plusWidth)
        : Math.max(tabLeft, Math.min(stripWidth - plusWidth,
                                     tabLeft + tabsWidth + plusGap))
    readonly property real plusRight: plusLeft + plusWidth

    /// Native hit-test bands in bar coordinates so the leading tab-search pill,
    /// tabs and `+` stay interactive while empty strip space remains HTCAPTION
    /// for native drag & Aero Snap.
    readonly property real clientLeft: isMac ? 74 : 8
    readonly property real clientRight: tabsOverflow
        ? Math.max(tabLeft, plusLeft)
        : Math.min(tabLeft + tabsWidth + flareWidth, plusLeft)

    function urlsOf(drag) {
        const urls = []
        for (let index = 0; index < drag.urls.length; index += 1)
            urls.push(String(drag.urls[index]))
        return urls
    }

    function tabX(index) {
        let x = 0
        const limit = Math.min(index, tabs.length)
        for (let i = 0; i < limit; i += 1) {
            x += tabWidthFor(tabs[i]) + tabGap
        }
        return x
    }

    function indexAt(x) {
        const content = x - tabViewportLeft + tabFlick.contentX
        let cursor = 0
        for (let i = 0; i < tabs.length; i += 1) {
            const w = tabWidthFor(tabs[i])
            if (content < cursor + w / 2)
                return i
            cursor += w + tabGap
        }
        return tabs.length
    }

    function indicatorX(index) {
        if (index <= 0)
            return root.flareWidth
        if (index >= tabs.length)
            return root.flareWidth + tabsWidth
        return root.flareWidth + tabX(index)
    }

    function scrollStrip(delta) {
        const max = Math.max(0, tabFlick.contentWidth - tabFlick.width)
        const step = delta * (normalTabWidth + tabGap)
        tabFlick.contentX = Math.max(0, Math.min(max,
                                                 tabFlick.contentX + step))
    }

    readonly property bool canScrollLeft: tabFlick.contentX > 0.5
    readonly property bool canScrollRight: tabFlick.contentX + tabFlick.width
        < tabFlick.contentWidth - 0.5

    function baseName(path) {
        const text = String(path || "")
        let end = text.length
        while (end > 0 && (text[end - 1] === "/" || text[end - 1] === "\\"))
            end -= 1
        const trimmed = text.substring(0, end)
        if (trimmed === "")
            return text.substring(0, 1)
        const slash = Math.max(trimmed.lastIndexOf("/"),
                               trimmed.lastIndexOf("\\"))
        return slash < 0 ? trimmed : trimmed.substring(slash + 1)
    }

    function parentName(path) {
        const text = String(path || "")
        let end = text.length
        while (end > 0 && (text[end - 1] === "/" || text[end - 1] === "\\"))
            end -= 1
        const trimmed = text.substring(0, end)
        const slash = Math.max(trimmed.lastIndexOf("/"),
                               trimmed.lastIndexOf("\\"))
        if (slash < 0)
            return ""
        return baseName(trimmed.substring(0, slash + 1))
    }

    function tabLabel(index) {
        const tab = root.tabs[index]
        if (!tab)
            return ""
        const name = baseName(tab.title)
        let duplicate = false
        for (let i = 0; i < root.tabs.length; i += 1) {
            if (i === index)
                continue
            const other = root.tabs[i]
            if (other && baseName(other.title) === name) {
                duplicate = true
                break
            }
        }
        if (!duplicate)
            return name
        const parent = parentName(tab.title)
        return parent === "" ? name : parent + " · " + name
    }

    function tabGlyphKind(tab) {
        if (!tab)
            return "folder"
        if (tab.overview === true)
            return "home"
        if (tab.trash === true)
            return "trash"
        const path = tab.listing ? String(tab.listing.path || "") : ""
        if (/^cloud:\/\//i.test(path))
            return "cloud"
        if (/^(smb|sftp|ftp|webdav|webdavs):\/\//i.test(path))
            return "network"
        return "folder"
    }

    function insideWindow(x, y) {
        return hostWindow !== null
            && x >= hostWindow.x && x < hostWindow.x + hostWindow.width
            && y >= hostWindow.y && y < hostWindow.y + hostWindow.height
    }

    function showDrop(x) {
        dropX = x
    }

    onActiveIndexChanged: Qt.callLater(function () { tabFlick.revealTab(root.activeIndex) })
    onTabsChanged: Qt.callLater(function () { tabFlick.revealTab(root.activeIndex) })

    function clearDrop() {
        dropX = -1
    }

    function beginDrag(index, grabX, grabY) {
        if (hostWindow === null || index < 0 || index >= tabs.length)
            return

        const title = tabs[index] ? tabs[index].title : ""
        const token = hostWindow.objectName
        const chipW = Math.round(tabWidthFor(tabs[index]))
        dragArmed = false
        dragging = true
        dragIndex = index
        dragSuppressClick = true
        dropX = -1
        ShellState.clearTabDrop()

        const action = Shell.startTabDrag(token, title, index,
                                          Math.round(grabX), Math.round(grabY),
                                          chipW, tabHeightValue,
                                          String(Theme.toolbar), String(accent), String(textColor))

        const drop = ShellState.takeTabDrop()
        const globalX = Shell.cursorLogicalX()
        const globalY = Shell.cursorLogicalY()

        dragging = false
        dragIndex = -1
        dropX = -1

        if (drop !== null && drop.sourceToken === token
                && drop.sourceIndex === index) {
            if (drop.targetToken === token) {
                if (drop.onStrip) {
                    let to = indexAt(drop.x)
                    if (to > index)
                        to -= 1
                    if (to !== index && to >= 0)
                        tabMoved(index, to)
                }
            } else if (drop.targetToken !== "") {
                tabDroppedOnWindow(index, drop.targetToken, drop.x)
            }
            return
        }

        if (action === Qt.IgnoreAction && !insideWindow(globalX, globalY))
            tabTornOff(index, globalX, globalY)
    }

    function toggleMacFullScreen() {
        if (window === null)
            return
        if (fullScreen)
            window.showNormal()
        else
            window.showFullScreen()
    }

    function toggleMacZoom() {
        if (window === null || fullScreen)
            return
        maximized ? window.showNormal() : window.showMaximized()
    }

    Rectangle {
        anchors.fill: parent
        color: root.chromeColor

        Behavior on color {
            enabled: Theme.motion && root.material === "none"
            ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
        }
    }

    // Subtle bottom seam sits behind `tabFlick` (`z: 0`) only when a translucent
    // native backdrop is active; Chrome solid mode joins `#d3e3fd` to `#ffffff`
    // directly without a dividing rule.
    Rectangle {
        visible: root.material !== "none"
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Theme.ruleWidth(Screen.devicePixelRatio)
        color: Theme.hairline
        z: 0
    }

    Item {
        id: dragSurface
        anchors.fill: parent
        Accessible.ignored: true

        DragHandler {
            id: systemMove
            target: null
            enabled: Qt.platform.os !== "windows"
            onActiveChanged: {
                if (systemMove.active && root.window !== null)
                    root.window.startSystemMove()
            }
        }

        TapHandler {
            enabled: Qt.platform.os !== "windows"
            onDoubleTapped: {
                if (root.window === null)
                    return
                if (!root.isMac) {
                    root.maximized ? root.window.showNormal()
                                   : root.window.showMaximized()
                    return
                }
                const action = root.controller !== null
                    ? root.controller.titlebarDoubleAction() : "zoom"
                if (action === "minimize")
                    root.window.showMinimized()
                else if (action === "zoom")
                    root.toggleMacZoom()
            }
        }
    }

    // Chrome-style leading tab search / switcher pill (`⌄`).
    Item {
        id: tabSearchButton
        x: root.isMac ? 76 : 8
        y: root.tabTop + Math.round((root.tabHeightValue - height) / 2)
        width: 28
        height: 28
        z: 2

        Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: tabSearchMouse.containsMouse ? Theme.tabSearchHover : Theme.tabSearchFill

            Behavior on color {
                enabled: Theme.motion
                ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
            }

            InkRipple {
                id: tabSearchRipple
                radius: Theme.radiusFull
            }
        }

        Glyph {
            anchors.centerIn: parent
            width: 12
            height: 12
            kind: "chevronDown"
            tint: Theme.ink
        }

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("切换标签页")
        Accessible.onPressAction: root.tabSearchRequested(
            tabSearchButton.x,
            tabSearchButton.y + tabSearchButton.height + 4)

        MouseArea {
            id: tabSearchMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onPressed: function (mouse) { tabSearchRipple.burst(mouse.x, mouse.y) }
            onReleased: tabSearchRipple.release()
            onCanceled: tabSearchRipple.release()
            onClicked: root.tabSearchRequested(
                tabSearchButton.x,
                tabSearchButton.y + tabSearchButton.height + 4)
        }

        ToolTip { text: qsTr("搜索与切换标签页") }
    }

    Row {
        id: trafficLights
        visible: root.isMac
        x: 14
        y: (root.height - 12) / 2
        spacing: 8

        HoverHandler {
            id: lightsHover
        }

        Repeater {
            model: 3

            Item {
                id: lightItem
                required property int index
                width: 12
                height: 12

                readonly property color light: !root.windowActive && !lightsHover.hovered
                    ? (root.dark ? "#3a3d45" : "#cfd3da")
                    : (index === 0 ? "#ff5f57"
                        : index === 1 ? "#febc2e" : "#28c840")
                readonly property string lightName: index === 0 ? qsTr("关闭")
                    : index === 1 ? qsTr("最小化")
                    : (root.fullScreen ? qsTr("退出全屏") : qsTr("全屏"))

                function activate() {
                    if (root.window === null)
                        return
                    if (index === 0)
                        root.window.close()
                    else if (index === 1)
                        root.window.showMinimized()
                    else
                        root.toggleMacFullScreen()
                }

                Rectangle {
                    anchors.fill: parent
                    radius: width / 2
                    color: lightItem.light
                    border.width: 1
                    border.color: "#24000000"
                }

                Glyph {
                    anchors.centerIn: parent
                    width: 8
                    height: 8
                    visible: lightsHover.hovered
                    kind: lightItem.index === 0 ? "windowClose"
                        : lightItem.index === 1 ? "windowMin"
                        : (root.fullScreen ? "windowExitFullScreen" : "windowFullScreen")
                    tint: "#73000000"
                }

                Accessible.role: Accessible.Button
                Accessible.name: lightItem.lightName
                Accessible.onPressAction: lightItem.activate()

                MouseArea {
                    anchors.fill: parent
                    onClicked: lightItem.activate()
                }
            }
        }
    }

    Flickable {
        id: tabFlick
        x: root.tabViewportLeft - root.flareWidth
        y: 0
        width: root.tabViewportWidth + root.flareWidth * 2
        height: root.height
        clip: true
        z: 1
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.HorizontalFlick
        contentWidth: Math.max(tabRow.width + root.flareWidth * 2, width)
        contentHeight: height
        interactive: false

        WheelHandler {
            onWheel: function (event) {
                if (tabFlick.contentWidth <= tabFlick.width) {
                    event.accepted = false
                    return
                }
                const step = event.angleDelta.y !== 0
                    ? -event.angleDelta.y / 2 : event.angleDelta.x
                tabFlick.contentX = Math.max(0, Math.min(
                    tabFlick.contentWidth - tabFlick.width,
                    tabFlick.contentX + step))
                event.accepted = true
            }
        }

        function revealTab(index) {
            if (index < 0 || index >= root.tabs.length || contentWidth <= width)
                return
            const left = root.flareWidth + root.tabX(index) - root.flareWidth
            const right = root.flareWidth + root.tabX(index)
                + root.tabWidthFor(root.tabs[index]) + root.flareWidth
            if (left < contentX)
                contentX = Math.max(0, left)
            else if (right > contentX + width)
                contentX = Math.min(contentWidth - width, right - width)
        }

        onWidthChanged: {
            const max = Math.max(0, contentWidth - width)
            if (contentX > max)
                contentX = max
            revealTab(root.activeIndex)
        }
        onContentWidthChanged: Qt.callLater(function () {
            revealTab(root.activeIndex)
        })
        Component.onCompleted: revealTab(root.activeIndex)

        Row {
            id: tabRow
            x: root.flareWidth
            y: root.tabTop
            height: root.tabHeightValue
            spacing: root.tabGap

            Repeater {
                model: root.tabs

                Item {
                    id: tabItem
                    required property int index
                    required property var modelData

                    readonly property bool isActive: root.activeIndex === index
                    readonly property bool pinned: tabItem.modelData
                        ? tabItem.modelData.pinned === true : false
                    readonly property bool listingLoading: tabItem.modelData
                        && tabItem.modelData.folder === true
                        && tabItem.modelData.listing !== undefined
                        && tabItem.modelData.listing !== null
                        && !tabItem.modelData.listing.finished
                    readonly property bool showClose: !tabItem.pinned
                        && (tabItem.isActive || width >= 112 || tabHover.hovered)
                    readonly property bool showDivider: !tabItem.isActive
                        && !tabHover.hovered
                        && (tabItem.index === root.tabs.length - 1
                            || ((tabItem.index + 1) !== root.activeIndex
                                && root.hoveredIndex !== tabItem.index + 1))

                    width: root.tabWidthFor(tabItem.modelData)
                    height: root.tabHeightValue
                    z: tabItem.isActive ? 3 : (tabHover.hovered ? 2 : 1)
                    opacity: root.dragging && root.dragIndex === index ? 0.45 : 1.0

                    HoverHandler {
                        id: tabHover
                        onHoveredChanged: {
                            if (hovered)
                                root.hoveredIndex = tabItem.index
                            else if (root.hoveredIndex === tabItem.index)
                                root.hoveredIndex = -1
                        }
                    }

                    ToolTip {
                        text: tabItem.modelData ? String(tabItem.modelData.title) : ""
                    }

                    // Active Chrome tab shape: RHI-rendered rounded top corners +
                    // concave outward bottom flares seamlessly joining `pathBar`.
                    ChromeTabShape {
                        visible: tabItem.isActive
                        anchors.fill: parent
                        anchors.leftMargin: -root.flareWidth
                        anchors.rightMargin: -root.flareWidth
                        topRadius: 10
                        bottomRadius: 10
                        connected: true
                        color: root.toolbarColor
                        borderColor: Theme.hairline
                        borderWidth: root.material !== "none"
                            ? Theme.ruleWidth(Screen.devicePixelRatio) : 0
                        Accessible.ignored: true
                    }

                    // Inactive tab hover pill (Chrome Refresh style).
                    Rectangle {
                        id: inactivePill
                        visible: !tabItem.isActive
                        x: 4
                        y: 2
                        width: Math.max(0, parent.width - 8)
                        height: parent.height - 4
                        radius: 10
                        color: tabHover.hovered ? Theme.tabHover : "transparent"
                        Accessible.ignored: true

                        Behavior on color {
                            enabled: Theme.motion
                            ColorAnimation {
                                duration: Theme.motionFast
                                easing.type: Theme.motionEaseEffects
                            }
                        }

                        InkRipple {
                            id: tabRipple
                            radius: 10
                        }
                    }

                    // Keyboard focus indicator.
                    Rectangle {
                        visible: tabItem.activeFocus
                        x: 4
                        y: 2
                        width: Math.max(0, parent.width - 8)
                        height: parent.height - 4
                        radius: 10
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.accent
                        z: 4
                        Accessible.ignored: true
                    }

                    // Chrome vertical divider between inactive tabs and before `+`.
                    Rectangle {
                        visible: tabItem.showDivider
                        x: parent.width - 1
                        y: Math.round((parent.height - 16) / 2)
                        width: 2
                        height: 16
                        radius: 1
                        color: Theme.tabDivider
                        Accessible.ignored: true
                    }

                    // Transfer progress indicator along the bottom of the tab.
                    readonly property real incomingTotal: {
                        let total = 0
                        for (const row of root.transferRows)
                            if (row.running && tabItem.modelData !== null
                                    && row.target === tabItem.modelData.title)
                                total += row.total
                        return total
                    }
                    readonly property real incomingDone: {
                        let done = 0
                        for (const row of root.transferRows)
                            if (row.running && tabItem.modelData !== null
                                    && row.target === tabItem.modelData.title)
                                done += row.done
                        return done
                    }
                    readonly property int incomingCount: {
                        let count = 0
                        for (const row of root.transferRows)
                            if (row.running && tabItem.modelData !== null
                                    && row.target === tabItem.modelData.title)
                                count += 1
                        return count
                    }
                    readonly property bool receiving: incomingCount > 0
                    readonly property real progress: incomingTotal > 0
                        ? Math.max(0, Math.min(1, incomingDone / incomingTotal))
                        : 0

                    Rectangle {
                        visible: tabItem.receiving
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        anchors.bottomMargin: tabItem.isActive ? 2 : 5
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        height: 2
                        radius: 1
                        color: Theme.line
                        z: 3

                        Rectangle {
                            width: parent.width * tabItem.progress
                            height: parent.height
                            radius: 1
                            color: Theme.accent
                        }
                    }

                    property bool fileDropTarget: false

                    DropArea {
                        anchors.fill: parent
                        keys: ["text/uri-list"]
                        onEntered: function (drag) {
                            tabItem.fileDropTarget = true
                            drag.accept(Qt.CopyAction)
                        }
                        onPositionChanged: function (drag) {
                            tabItem.fileDropTarget = true
                            drag.accept(Qt.CopyAction)
                        }
                        onExited: tabItem.fileDropTarget = false
                        onDropped: function (drop) {
                            tabItem.fileDropTarget = false
                            // qmllint disable missing-property
                            const paths = Workspace.localDropPaths(root.urlsOf(drop))
                            if (paths.length === 0) {
                                drop.accepted = false
                                return
                            }
                            // qmllint enable missing-property
                            drop.accept(Qt.CopyAction)
                            root.filesDropped(tabItem.index, paths)
                        }
                    }

                    Rectangle {
                        x: 2
                        y: 1
                        width: Math.max(0, parent.width - 4)
                        height: parent.height - 5
                        visible: tabItem.fileDropTarget
                        radius: Theme.radiusMd
                        color: Theme.accentSoftStrong
                        border.width: 2
                        border.color: Theme.accent
                        z: 4
                    }

                    activeFocusOnTab: true
                    Accessible.role: Accessible.PageTab
                    Accessible.name: tabItem.modelData
                        ? tabItem.modelData.title : ""
                    Accessible.selectable: true
                    Accessible.selected: tabItem.isActive
                    Keys.onReturnPressed: root.tabActivated(tabItem.index)
                    Keys.onEnterPressed: root.tabActivated(tabItem.index)
                    Keys.onSpacePressed: root.tabActivated(tabItem.index)

                    // Leading favicon or RHI loading spinner.
                    Item {
                        id: tabFavicon
                        x: tabItem.pinned ? Math.round((parent.width - width) / 2) : 12
                        y: Math.round((parent.height - height) / 2)
                        width: 16
                        height: 16

                        ChromeSpinner {
                            anchors.fill: parent
                            visible: tabItem.listingLoading
                            running: visible
                            color: Theme.accent
                            strokeWidth: 1.6
                            reduceMotion: !Theme.motion
                        }

                        Glyph {
                            anchors.fill: parent
                            visible: !tabItem.listingLoading
                            kind: root.tabGlyphKind(tabItem.modelData)
                            tint: tabItem.isActive
                                ? Theme.ink
                                : (tabHover.hovered ? Theme.ink : Theme.inkSecondary)
                        }

                        // Small pin indicator dot on pinned tabs.
                        Rectangle {
                            visible: tabItem.pinned
                            anchors.right: parent.right
                            anchors.top: parent.top
                            anchors.rightMargin: -3
                            anchors.topMargin: -2
                            width: 5
                            height: 5
                            radius: 2.5
                            color: Theme.tertiary
                        }
                    }

                    Text {
                        visible: !tabItem.pinned
                        anchors.left: tabFavicon.right
                        anchors.leftMargin: 8
                        anchors.right: tabItem.showClose ? closeButton.left : parent.right
                        anchors.rightMargin: tabItem.showClose ? 4 : 12
                        y: 0
                        height: parent.height
                        verticalAlignment: Text.AlignVCenter
                        text: root.tabLabel(tabItem.index)
                        color: tabItem.isActive
                            ? Theme.ink
                            : (tabHover.hovered
                                ? Theme.ink
                                : (root.windowActive ? Theme.inkSecondary : Theme.inkTertiary))
                        font.pixelSize: Theme.typeCaption
                        font.weight: Font.Normal
                        elide: Text.ElideMiddle
                    }

                    Item {
                        id: closeButton
                        anchors.right: parent.right
                        anchors.rightMargin: 8
                        y: Math.round((parent.height - height) / 2)
                        width: 18
                        height: 18
                        visible: tabItem.showClose
                        z: 2

                        Rectangle {
                            anchors.fill: parent
                            radius: Theme.radiusFull
                            color: closeMouse.containsMouse ? Theme.hover : "transparent"

                            InkRipple {
                                id: closeRipple
                                radius: Theme.radiusFull
                            }
                        }

                        Glyph {
                            anchors.centerIn: parent
                            width: 10
                            height: 10
                            kind: "close"
                            tint: closeMouse.containsMouse || tabItem.isActive
                                ? Theme.ink : Theme.inkSecondary
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("关闭标签页")
                        Accessible.onPressAction: root.tabClosed(tabItem.index)

                        MouseArea {
                            id: closeMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onPressed: function (mouse) {
                                closeRipple.burst(mouse.x, mouse.y)
                            }
                            onReleased: closeRipple.release()
                            onCanceled: closeRipple.release()
                            onClicked: root.tabClosed(tabItem.index)
                        }
                    }

                    MouseArea {
                        id: tabMouse
                        anchors.fill: parent
                        anchors.rightMargin: tabItem.showClose ? 26 : 0
                        hoverEnabled: true
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                            | Qt.MiddleButton
                        onPressed: function (mouse) {
                            if (mouse.button === Qt.MiddleButton) {
                                root.tabClosed(tabItem.index)
                                return
                            }
                            if (mouse.button === Qt.RightButton) {
                                const point = root.mapFromItem(tabMouse,
                                                               mouse.x, mouse.y)
                                root.tabContextRequested(tabItem.index,
                                                         point.x, point.y)
                                return
                            }
                            if (!tabItem.isActive)
                                tabRipple.burst(mouse.x, mouse.y)
                            root.dragArmed = true
                            root.dragSuppressClick = false
                            root.dragStartX = mouse.x
                            root.dragStartY = mouse.y
                        }

                        onPositionChanged: function (mouse) {
                            if (!root.dragArmed || root.dragging
                                    || (mouse.buttons & Qt.LeftButton) === 0)
                                return
                            const dx = mouse.x - root.dragStartX
                            const dy = mouse.y - root.dragStartY
                            if (Math.sqrt(dx * dx + dy * dy) > 6) {
                                tabRipple.release()
                                root.beginDrag(tabItem.index, root.dragStartX,
                                               root.dragStartY)
                            }
                        }

                        onReleased: function (mouse) {
                            tabRipple.release()
                            root.dragArmed = false
                            if (mouse.button !== Qt.LeftButton)
                                return
                            if (root.dragSuppressClick) {
                                root.dragSuppressClick = false
                                return
                            }
                            if (!root.dragging)
                                root.tabActivated(tabItem.index)
                        }

                        onCanceled: {
                            tabRipple.release()
                            root.dragArmed = false
                        }
                    }
                }
            }
        }

        Rectangle {
            visible: root.dropX >= 0
            x: root.indicatorX(root.indexAt(root.dropX))
            y: root.tabTop + 2
            width: 2
            height: root.tabHeightValue - 6
            radius: 1
            color: root.accent
            z: 5
        }
    }

    Item {
        id: scrollLeftButton
        visible: root.tabsOverflow
        x: root.tabLeft
        y: root.tabTop + Math.round((root.tabHeightValue - root.arrowWidth) / 2)
        width: root.arrowWidth
        height: root.arrowWidth
        z: 2
        readonly property bool canScroll: root.canScrollLeft
        opacity: canScroll ? 1.0 : 0.4

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusFull
            color: scrollLeftMouse.containsMouse && scrollLeftButton.canScroll
                ? Theme.tabHover : "transparent"
        }

        Glyph {
            anchors.centerIn: parent
            width: 14
            height: 14
            kind: "chevronLeft"
            tint: scrollLeftMouse.containsMouse && scrollLeftButton.canScroll
                ? Theme.ink : Theme.inkSecondary
        }

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("向左滚动标签页")
        Accessible.onPressAction: {
            if (scrollLeftButton.canScroll)
                root.scrollStrip(-1)
        }

        MouseArea {
            id: scrollLeftMouse
            anchors.fill: parent
            hoverEnabled: true
            enabled: scrollLeftButton.canScroll
            onClicked: root.scrollStrip(-1)
        }
    }

    Item {
        id: scrollRightButton
        visible: root.tabsOverflow
        x: Math.max(root.tabViewportLeft, root.plusLeft - root.arrowWidth)
        y: root.tabTop + Math.round((root.tabHeightValue - root.arrowWidth) / 2)
        width: root.arrowWidth
        height: root.arrowWidth
        z: 2
        readonly property bool canScroll: root.canScrollRight
        opacity: canScroll ? 1.0 : 0.4

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusFull
            color: scrollRightMouse.containsMouse && scrollRightButton.canScroll
                ? Theme.tabHover : "transparent"
        }

        Glyph {
            anchors.centerIn: parent
            width: 14
            height: 14
            kind: "chevronRight"
            tint: scrollRightMouse.containsMouse && scrollRightButton.canScroll
                ? Theme.ink : Theme.inkSecondary
        }

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("向右滚动标签页")
        Accessible.onPressAction: {
            if (scrollRightButton.canScroll)
                root.scrollStrip(1)
        }

        MouseArea {
            id: scrollRightMouse
            anchors.fill: parent
            hoverEnabled: true
            enabled: scrollRightButton.canScroll
            onClicked: root.scrollStrip(1)
        }
    }

    Item {
        id: plusButton
        x: root.plusLeft
        y: root.tabTop + Math.round((root.tabHeightValue - root.plusWidth) / 2)
        width: root.plusWidth
        height: root.plusWidth
        z: 2

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: plusMouse.containsMouse ? Theme.tabHover : "transparent"

            Behavior on color {
                enabled: Theme.motion
                ColorAnimation { duration: Theme.motionFast; easing.type: Theme.motionEaseEffects }
            }

            InkRipple {
                id: plusRipple
                radius: Theme.radiusFull
            }
        }

        Glyph {
            anchors.centerIn: parent
            width: 14
            height: 14
            kind: "plus"
            tint: Theme.ink
        }

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("新建标签页")
        Accessible.onPressAction: root.newTabRequested()

        MouseArea {
            id: plusMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onPressed: function (mouse) { plusRipple.burst(mouse.x, mouse.y) }
            onReleased: plusRipple.release()
            onCanceled: plusRipple.release()
            onClicked: root.newTabRequested()
        }

        ToolTip { text: qsTr("新建标签页") }
    }

    Item {
        id: windowButtons
        anchors.right: parent.right
        height: parent.height
        width: root.buttonsWidth
        visible: !root.isMac
        z: 2

        Repeater {
            model: 3

            Item {
                id: button
                required property int index

                width: root.buttonWidth
                height: root.height
                x: index * root.buttonWidth

                readonly property bool isClose: index === 2
                readonly property bool isMaximize: index === 1
                readonly property bool hovered: root.nativeButtons
                    ? root.ncHover === index : mouse.containsMouse
                readonly property bool pressed: root.nativeButtons
                    ? root.ncPressed === index : mouse.pressed

                Accessible.role: Accessible.Button
                Accessible.name: button.isClose ? qsTr("关闭")
                    : button.isMaximize ? (root.maximized ? qsTr("向下还原") : qsTr("最大化"))
                    : qsTr("最小化")
                Accessible.onPressAction: {
                    if (button.isClose)
                        root.window.close()
                    else if (button.isMaximize)
                        root.maximized ? root.window.showNormal()
                                       : root.window.showMaximized()
                    else
                        root.window.showMinimized()
                }

                Rectangle {
                    anchors.fill: parent
                    color: button.isClose
                        ? (button.hovered
                            ? (button.pressed ? "#b22a1c" : "#c42b1c")
                            : "transparent")
                        : (button.hovered
                            ? (root.nativeButtons
                                ? (button.pressed ? root.ncPressFill : root.ncHoverFill)
                                : Theme.tabHover)
                            : "transparent")
                }

                Text {
                    visible: root.nativeButtons && root.nativeIconFamily !== ""
                    anchors.centerIn: parent
                    font.family: root.nativeIconFamily
                    font.pixelSize: 10
                    text: button.isClose ? "\uE8BB"
                        : button.isMaximize ? (root.maximized ? "\uE923" : "\uE922")
                        : "\uE921"
                    color: button.isClose && button.hovered
                        ? Theme.inkOnSolid : root.textColor
                }

                Item {
                    id: drawnGlyph
                    visible: !root.nativeButtons || root.nativeIconFamily === ""
                    anchors.centerIn: parent
                    width: 14
                    height: 14

                    readonly property color ink: button.isClose && button.hovered
                        ? Theme.inkOnSolid : root.textColor

                    Rectangle {
                        visible: button.index === 0
                        anchors.centerIn: parent
                        width: 10
                        height: 1
                        color: drawnGlyph.ink
                    }

                    Rectangle {
                        visible: button.isMaximize && !root.maximized
                        anchors.centerIn: parent
                        width: 9
                        height: 9
                        color: "transparent"
                        border.width: 1
                        border.color: drawnGlyph.ink
                    }

                    Item {
                        visible: button.isMaximize && root.maximized
                        anchors.centerIn: parent
                        width: 10
                        height: 10

                        Rectangle {
                            x: 2
                            y: 0
                            width: 8
                            height: 8
                            color: "transparent"
                            border.width: 1
                            border.color: drawnGlyph.ink
                        }

                        Rectangle {
                            x: 0
                            y: 2
                            width: 8
                            height: 8
                            color: root.chromeColor
                            border.width: 1
                            border.color: drawnGlyph.ink
                        }
                    }

                    Item {
                        visible: button.isClose
                        anchors.centerIn: parent
                        width: 10
                        height: 10

                        Rectangle {
                            anchors.centerIn: parent
                            width: 12
                            height: 1
                            color: drawnGlyph.ink
                            rotation: 45
                        }

                        Rectangle {
                            anchors.centerIn: parent
                            width: 12
                            height: 1
                            color: drawnGlyph.ink
                            rotation: -45
                        }
                    }
                }

                MouseArea {
                    id: mouse
                    anchors.fill: parent
                    enabled: !root.nativeButtons
                    hoverEnabled: !root.nativeButtons
                    onClicked: {
                        if (button.isClose)
                            root.window.close()
                        else if (button.isMaximize)
                            root.maximized ? root.window.showNormal()
                                           : root.window.showMaximized()
                        else
                            root.window.showMinimized()
                    }
                }
            }
        }
    }
}
