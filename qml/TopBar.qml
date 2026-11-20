pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import "Shell.js" as ShellState

// The chrome bar: the mark, the tabs and the three window buttons, all in
// 38 dp (proposal §7.3.1 and §8.1). It replaces the old 44 dp title row plus
// 44 dp tab strip; the tabs themselves are 30 dp, with 4 dp above and below.
//
// The bar is the window's drag handle, and that is a native decision on
// Windows, not a QML one: `crates/dae-qt/cpp/window.h` returns HTCAPTION for
// the bar minus the three button columns minus the band the tabs occupy.
// DefWindowProc then runs the move loop, which is the only way drag, Aero Snap,
// the Snap Layouts hover on maximise and double-click maximise all stay
// native. `implicitHeight` here and `kTitleBarDp` there are the same number;
// changing one without the other puts the hit zones off by the difference.
//
// A tab drag is handed to the platform's own drag loop (`Shell.startTabDrag`).
// That loop draws the chip with the grab point under the cursor on every
// platform (OLE, NSDraggingSession, X11/Wayland DnD) and keeps reporting after
// the pointer leaves the window, which a MouseArea cannot. Each window carries
// a `DropArea` matching the tab MIME type; a drop only records where it landed
// in `ShellState`, and this bar applies the reorder or merge once the loop
// returns. A release that no drop target took — the desktop or another app —
// tears the tab off at the final cursor position.
//
// ## The macOS three (`docs/qt-platform-parity.md` §2.8)
//
// The window is frameless everywhere, so on macOS the traffic lights are drawn
// here and the platform never sees them. Three decisions follow from the
// native behaviour rather than from taste:
//
//  * **Green toggles full screen**, not a maximised frame. Since 10.10 that is
//    what the button means; a file manager filling the visible frame and
//    leaving the menu bar and Dock in place is the old "zoom" behaviour, and
//    users reach it by double-clicking the bar instead.
//  * **The glyphs appear on hover of the group**, with the trio's own marks
//    (`×`, `−`, and the two opposing arrows): drawn, not typed, because no
//    macOS system font carries the Windows glyph code points.
//  * **Double-click obeys the system preference.** AppKit would read
//    `AppleActionOnDoubleClick` itself, but the click never reaches AppKit —
//    the bar is content. `WindowController.titlebarDoubleAction()` reads the
//    same default per call, so the choice applies without a restart.
Item {
    id: root

    property var window: null
    /// The window's `WindowController`. macOS reads the double-click
    /// preference through it; the other platforms never call it.
    property var controller: null
    property bool dark: true
    property string material: "none"
    /// For the OS: `Window.title` comes from the same property. Nothing on the
    /// bar renders it.
    property string title: "dae"

    property var tabs: []
    property int activeIndex: 0
    property var hostWindow: null
    property color accent: Theme.accent

    /// The accepted transfers, parsed once per change so every tab chip can
    /// ask whether one of them lands in its directory.
    readonly property var transferRows: JSON.parse(Workspace.transfersJson)

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
    readonly property color solidChrome: Theme.chrome
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property bool maximized: window !== null
        && window.visibility === Window.Maximized
    /// The green button's own state on macOS. Kept apart from `maximized`,
    /// which the Windows-drawn buttons still use.
    readonly property bool fullScreen: window !== null
        && window.visibility === Window.FullScreen

    /// On Windows the three window buttons are native non-client zones:
    /// DefWindowProc owns their clicks and the Snap Layouts hover, and the
    /// native filter mirrors the NC hover/press state onto the controller. The
    /// bar paints the system glyphs and wash from that state rather than taking
    /// mouse events it would never receive.
    readonly property bool nativeButtons: Qt.platform.os === "windows"
    readonly property int ncHover: nativeButtons && controller !== null
        ? controller.ncButtonHover : -1
    readonly property int ncPressed: nativeButtons && controller !== null
        ? controller.ncButtonPressed : -1
    readonly property string nativeIconFamily: controller !== null
        ? controller.captionIconFamily : ""
    /// The system caption hover/press wash: a translucent on-surface layer over
    /// the chrome, matching the native buttons in both themes. Close keeps its
    /// own red below.
    readonly property color ncHoverFill: dark ? Qt.rgba(1, 1, 1, 0.06)
                                              : Qt.rgba(0, 0, 0, 0.04)
    readonly property color ncPressFill: dark ? Qt.rgba(1, 1, 1, 0.09)
                                              : Qt.rgba(0, 0, 0, 0.06)

    // ── Tab metrics (§7.3.1) ────────────────────────────────────────────────
    readonly property int tabWidthValue: 184
    /// 30 dp inside a 38 dp bar: four above, four below.
    readonly property int tabHeightValue: 30
    readonly property int tabTop: 4
    readonly property int tabGap: 2
    /// The strip starts where the mark ends (14 + 19 + 13); on macOS it starts
    /// after the three traffic lights.
    readonly property int tabLeft: isMac ? 78 : 46
    /// The `+` column. It is pinned to the right of the strip rather than
    /// following the last chip, so opening a tab never depends on scrolling
    /// the strip all the way to its end.
    readonly property int plusWidth: 26
    /// Each overflow arrow claims this much of the strip.
    readonly property int arrowWidth: 26
    /// Tabs stop before the button columns. The 8 dp gap stays caption, like
    /// the rest of the empty bar.
    readonly property real stripWidth: Math.max(0, width - buttonsWidth - 8)

    /// The chips' own width, before any scrolling arrows are inserted.
    readonly property real tabsWidth: tabs.length === 0 ? 0
        : tabs.length * tabWidthValue + (tabs.length - 1) * tabGap
    /// True when the chips outgrow the space the strip leaves them. The arrows
    /// then appear at both ends; `+` stays pinned at the right.
    readonly property bool tabsOverflow: tabs.length > 0
        && tabsWidth > Math.max(0, stripWidth - plusWidth - tabLeft)
    /// The visible chip viewport, in bar coordinates. Its left edge moves
    /// right by the left arrow only while the strip overflows, so the chips do
    /// not jump while the window is resized across the threshold.
    readonly property real tabViewportLeft: tabLeft
        + (tabsOverflow ? arrowWidth : 0)
    readonly property real tabViewportWidth: Math.max(0,
        stripWidth - plusWidth - tabViewportLeft
        - (tabsOverflow ? arrowWidth : 0))
    /// The pinned `+` column, immediately left of the button columns.
    readonly property real plusLeft: Math.max(0, stripWidth - plusWidth)
    readonly property real plusRight: plusLeft + plusWidth

    /// The horizontal slices of the bar that must stay client areas, in bar
    /// coordinates. The shell forwards them to the native hit test so a click
    /// on a chip or on `+` reaches QML instead of starting a window drag. Two
    /// bands, because the chips and the pinned `+` are not adjacent: the
    /// empty strip between them stays caption and remains the window's grab
    /// handle. A zero-width band means "no client band".
    readonly property real clientLeft: tabLeft
    readonly property real clientRight: tabsOverflow
        ? Math.max(tabLeft, plusLeft)
        : Math.min(tabLeft + tabsWidth, plusLeft)

    /// The URLs a drop carries, as strings. `Workspace.localDropPaths` turns
    /// the `file:` half into paths; anything else (a browser URL) is dropped
    /// there, so only the real files reach the pipeline.
    function urlsOf(drag) {
        const urls = []
        for (let index = 0; index < drag.urls.length; index += 1)
            urls.push(String(drag.urls[index]))
        return urls
    }

    /// A chip's left edge in the scroll content, which starts at the
    /// viewport's left edge rather than at the bar's left edge.
    function tabX(index) {
        return index * (tabWidthValue + tabGap)
    }

    /// Which chip sits at a bar coordinate. The viewport's left edge and the
    /// current scroll offset turn it into a content coordinate first.
    function indexAt(x) {
        const content = x - tabViewportLeft + tabFlick.contentX
        for (let i = 0; i < tabs.length; i += 1) {
            const middle = tabX(i) + tabWidthValue / 2
            if (content < middle)
                return i
        }
        return tabs.length
    }

    /// Where the drop indicator belongs, in scroll-content coordinates.
    function indicatorX(index) {
        if (index <= 0)
            return 0
        const column = Math.min(index, tabs.length) - 1
        return tabX(column) + tabWidthValue + tabGap / 2
    }

    /// Scrolls by one chip. The arrows are the pointer's way along a strip
    /// that overflows; the wheel still works as before.
    function scrollStrip(delta) {
        const max = Math.max(0, tabFlick.contentWidth - tabFlick.width)
        const step = delta * (tabWidthValue + tabGap)
        tabFlick.contentX = Math.max(0, Math.min(max,
                                                 tabFlick.contentX + step))
    }

    readonly property bool canScrollLeft: tabFlick.contentX > 0.5
    readonly property bool canScrollRight: tabFlick.contentX + tabFlick.width
        < tabFlick.contentWidth - 0.5

    /// The chip names only its directory (§7.3.1). Two tabs showing the same
    /// name get the parent on the left of a middle dot; the path itself stays
    /// available to assistive clients.
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

    /// Whether a logical screen point is inside the window this bar belongs
    /// to. `hostWindow` is frameless, so its position/size are the window rect.
    function insideWindow(x, y) {
        return hostWindow !== null
            && x >= hostWindow.x && x < hostWindow.x + hostWindow.width
            && y >= hostWindow.y && y < hostWindow.y + hostWindow.height
    }

    /// Marks where a tab drag hovering this bar would insert. The window's
    /// `DropArea` drives this while the native loop runs. `x` arrives in bar
    /// coordinates; `indexAt` and the indicator convert it as needed.
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
        dragArmed = false
        dragging = true
        dragIndex = index
        dragSuppressClick = true
        dropX = -1
        ShellState.clearTabDrop()

        // Blocks in the platform's modal loop. The drag image is the tab chip
        // with `grabX/grabY` as its hot spot, so the preview keeps the cursor
        // on the point the user grabbed. Drops land in the window DropAreas
        // while this runs, so the record is complete when it returns.
        const action = Shell.startTabDrag(token, title, index,
                                          Math.round(grabX), Math.round(grabY),
                                          tabWidthValue, tabHeightValue,
                                          Theme.canvas, accent, textColor)

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
                    // The dragged tab is still in the list; the insertion index
                    // is measured among the tabs it would land between.
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

        // Nothing took the drop. Outside this window that is the platform
        // reporting a release over the desktop or another app; inside it, the
        // loop was cancelled (Escape) and the tab stays where it was.
        if (action === Qt.IgnoreAction && !insideWindow(globalX, globalY))
            tabTornOff(index, globalX, globalY)
    }

    /// macOS' green light: full screen, or back to the previous frame.
    function toggleMacFullScreen() {
        if (window === null)
            return
        if (fullScreen)
            window.showNormal()
        else
            window.showFullScreen()
    }

    /// The zoom half of a double-click: the visible frame, not full screen.
    function toggleMacZoom() {
        if (window === null || fullScreen)
            return
        maximized ? window.showNormal() : window.showMaximized()
    }

    Rectangle {
        anchors.fill: parent
        color: root.chromeColor
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Theme.ruleWidth(Screen.devicePixelRatio)
        color: Theme.hairline
    }

    // Windows drags the bar through the native hit test described above. The
    // other platforms have no WM_NCHITTEST, so the empty parts of the bar ask
    // the window manager for its own move loop instead. It is declared before
    // the chips and the buttons, so their clicks keep precedence.
    Item {
        id: dragSurface
        anchors.fill: parent
        // The bar itself is not a control: it exists to start the platform's
        // move loop. A name here would announce a second, invisible button
        // around the real ones.
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
            /// Double-click-to-maximize is a pointer gesture on the bar; the
            /// window is maximized from the system menu, so this handler is
            /// not a control.
            enabled: Qt.platform.os !== "windows"
            onDoubleTapped: {
                if (root.window === null)
                    return
                if (!root.isMac) {
                    root.maximized ? root.window.showNormal()
                                   : root.window.showMaximized()
                    return
                }
                // macOS: the action belongs to the user's preference, the same
                // one AppKit would have applied had the click reached the
                // caption.
                const action = root.controller !== null
                    ? root.controller.titlebarDoubleAction() : "zoom"
                if (action === "minimize")
                    root.window.showMinimized()
                else if (action === "zoom")
                    root.toggleMacZoom()
            }
        }
    }

    Rectangle {
        id: logo
        x: 14
        anchors.verticalCenter: parent.verticalCenter
        width: 19
        height: 19
        radius: Theme.radiusSm
        // The mark is the brand, not an interactive control. M3 Expressive
        // keeps the product colour at the top of the hierarchy, so it stays on
        // `primary` even when a system accent moves every selection role.
        color: Theme.accent
        visible: !root.isMac

        Text {
            anchors.centerIn: parent
            text: "D"
            color: Theme.inkOnAccent
            font.pixelSize: Theme.typeCaption
            font.weight: Theme.weightEmphasis
        }
    }

    // A frameless window draws its own traffic lights. They sit on the left,
    // where macOS puts them, and the strip starts after the 78 dp they claim.
    // The trio behaves like the group AppKit draws: the marks stay hidden until
    // the pointer is over any of the three, then all three show at once.
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

                readonly property color light: index === 0 ? "#ff5f57"
                    : index === 1 ? "#febc2e" : "#28c840"
                readonly property bool isZoom: index === 2
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

                // Drawn marks, not text: the Windows glyph code points have no
                // macOS font, and SF Symbols is not reachable from QML.
                Glyph {
                    anchors.centerIn: parent
                    width: 8
                    height: 8
                    visible: lightsHover.hovered
                    kind: lightItem.index === 0 ? "windowClose"
                        : lightItem.index === 1 ? "windowMin"
                        : (root.fullScreen ? "windowExitFullScreen" : "windowFullScreen")
                    // AppKit tints its own marks dark on all three lights; the
                    // yellow one is bright enough that white would disappear.
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

    // The chips and the drop indicator. The overflow arrows and `+` are pinned
    // outside this viewport, so they never scroll away; the chips are clipped
    // at the button columns and scroll horizontally (§7.3.1: one row, never
    // wrapped) once they outgrow the width.
    Flickable {
        id: tabFlick
        x: root.tabViewportLeft
        y: 0
        width: root.tabViewportWidth
        height: root.height
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.HorizontalFlick
        contentWidth: Math.max(tabRow.width, width)
        contentHeight: height
        /// Not `interactive`: the chips drag to reorder and tear off, and a
        /// Flickable that also took horizontal drags would steal those
        /// gestures after six pixels. The arrows, the wheel and the
        /// reveal-on-switch rule below are how a strip that does not fit is
        /// traversed.
        interactive: false

        /// The wheel is the pointer's way along the strip; a horizontal wheel
        /// delta (a trackpad's two-finger swipe) is honoured too. The arrows
        /// are the same traversal for a pointer that has no wheel.
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

        /// Keeps the active chip inside the strip after a switch, a close or a
        /// new tab. `ViewTransition` is not involved: the strip is not a view.
        function revealTab(index) {
            if (index < 0 || contentWidth <= width)
                return
            const left = root.tabX(index) - root.tabGap
            const right = root.tabX(index) + root.tabWidthValue + root.tabGap
            if (left < contentX)
                contentX = Math.max(0, left)
            else if (right > contentX + width)
                contentX = Math.min(contentWidth - width, right - width)
        }

        onWidthChanged: {
            // Closing tabs can make the strip stop overflowing; clamp before
            // revealing so a stale offset cannot hide the active chip.
            const max = Math.max(0, contentWidth - width)
            if (contentX > max)
                contentX = max
            revealTab(root.activeIndex)
        }
        /// The row's width lands one layout pass after `tabs` changes, so a
        /// reveal that ran on `tabsChanged` alone would clamp against the old
        /// content width and leave the new chip half under the window buttons.
        onContentWidthChanged: Qt.callLater(function () {
            revealTab(root.activeIndex)
        })
        Component.onCompleted: revealTab(root.activeIndex)

        Row {
            id: tabRow
            x: 0
            y: root.tabTop
            height: root.tabHeightValue
            spacing: root.tabGap

            Repeater {
                model: root.tabs

                Rectangle {
                    id: tabItem
                    required property int index
                    required property var modelData

                    width: root.tabWidthValue
                    height: root.tabHeightValue
                    radius: Theme.radiusFull
                    // M3 Expressive tabs are tonal pills: the active tab is a
                    // `secondaryContainer` block, not the same colour as the
                    // content below it.
                    color: root.activeIndex === index
                        ? Theme.selection
                        : (tabHover.hovered ? Theme.hover : "transparent")
                    opacity: root.dragging && root.dragIndex === index ? 0.45 : 1.0

                    /// Hover over the whole chip, close control included.
                    /// `tabMouse` stops short of the close button so a press
                    /// there cannot activate the tab.
                    HoverHandler { id: tabHover }

                    /// §7.3.1: a tab names its directory; the full path is one
                    /// hover away, which is the only place the truncated
                    /// middle of a deep path can be read.
                    ToolTip {
                        text: tabItem.modelData ? String(tabItem.modelData.title) : ""
                    }

                    /// §7.3.1: a transfer belongs to the tab it lands in, so
                    /// this chip carries the queue's progress as a 2 dp bar
                    /// along its bottom edge. Several transfers may land in
                    /// the same directory; the bar is their byte-weighted sum.
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
                        anchors.leftMargin: 6
                        anchors.rightMargin: 6
                        height: 2
                        radius: 1
                        color: Theme.line
                        z: 2

                        Rectangle {
                            width: parent.width * tabItem.progress
                            height: parent.height
                            radius: 1
                            color: Theme.accent
                        }
                    }

                    /// Whether a file drag from the OS is over this chip. A
                    /// drop here moves the files into the tab's directory,
                    /// which is the same act as dropping on a breadcrumb.
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

                    /// The drop feedback the whole window uses: a soft primary
                    /// wash and a 2 dp accent frame, carried on the pill.
                    Rectangle {
                        anchors.fill: parent
                        visible: tabItem.fileDropTarget
                        radius: Theme.radiusFull
                        color: Theme.accentSoftStrong
                        border.width: 2
                        border.color: Theme.accent
                        z: 3
                    }

                    /// A tab is the first thing Tab should reach, and Enter is
                    /// the keyboard's click.
                    activeFocusOnTab: true
                    border.width: tabItem.activeFocus ? 2 : 0
                    border.color: Theme.accent
                    Accessible.role: Accessible.PageTab
                    Accessible.name: tabItem.modelData
                        ? tabItem.modelData.title : ""
                    Accessible.selectable: true
                    Accessible.selected: root.activeIndex === tabItem.index
                    Keys.onReturnPressed: root.tabActivated(tabItem.index)
                    Keys.onEnterPressed: root.tabActivated(tabItem.index)
                    Keys.onSpacePressed: root.tabActivated(tabItem.index)

                    Rectangle {
                        readonly property bool pinned: tabItem.modelData
                            ? tabItem.modelData.pinned === true : false
                        visible: pinned
                        x: 10
                        width: 6
                        height: 6
                        radius: 3
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.tertiary
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: tabItem.modelData
                            && tabItem.modelData.pinned ? 20 : 12
                        anchors.right: closeButton.left
                        anchors.rightMargin: 4
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.tabLabel(tabItem.index)
                        color: root.activeIndex === tabItem.index
                            ? Theme.selectionInk
                            : (tabHover.hovered ? Theme.ink : Theme.inkSecondary)
                        font.pixelSize: Theme.typeCaption
                        font.weight: root.activeIndex === tabItem.index
                            ? Theme.weightEmphasis : Font.Normal
                        elide: Text.ElideMiddle
                    }

                    Item {
                        id: closeButton
                        anchors.right: parent.right
                        anchors.rightMargin: 6
                        anchors.verticalCenter: parent.verticalCenter
                        width: 20
                        height: 20
                        visible: root.activeIndex === tabItem.index
                            || tabHover.hovered

                        Rectangle {
                            anchors.fill: parent
                            radius: Theme.radiusFull
                            color: closeMouse.containsMouse
                                ? (root.activeIndex === tabItem.index
                                    ? Qt.rgba(Theme.selectionInk.r,
                                              Theme.selectionInk.g,
                                              Theme.selectionInk.b, 0.10)
                                    : Theme.hover)
                                : "transparent"
                        }

                        Glyph {
                            anchors.centerIn: parent
                            width: 12
                            height: 12
                            kind: "close"
                            tint: closeMouse.containsMouse
                                ? (root.activeIndex === tabItem.index
                                    ? Theme.selectionInk : Theme.ink)
                                : (root.activeIndex === tabItem.index
                                    ? Theme.selectionInk : Theme.inkSecondary)
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("关闭标签页")
                        Accessible.onPressAction: root.tabClosed(tabItem.index)

                        MouseArea {
                            id: closeMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: root.tabClosed(tabItem.index)
                        }
                    }

                    MouseArea {
                        id: tabMouse
                        anchors.fill: parent
                        anchors.rightMargin: 24
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
                            if (Math.sqrt(dx * dx + dy * dy) > 6)
                                root.beginDrag(tabItem.index, root.dragStartX,
                                               root.dragStartY)
                        }

                        onReleased: function (mouse) {
                            root.dragArmed = false
                            if (mouse.button !== Qt.LeftButton)
                                return
                            // The release that ended a native drag may arrive
                            // as a click after the loop; it must not activate
                            // the tab.
                            if (root.dragSuppressClick) {
                                root.dragSuppressClick = false
                                return
                            }
                            if (!root.dragging)
                                root.tabActivated(tabItem.index)
                        }

                        onCanceled: {
                            root.dragArmed = false
                        }
                    }
                }
            }
        }

        // Where the tab would land. The platform draws the dragged chip
        // itself, so this is the only in-window feedback while a drag is over
        // the strip; it also shows for a tab dragged in from another window.
        Rectangle {
            visible: root.dropX >= 0
            x: root.indicatorX(root.indexAt(root.dropX))
            y: root.tabTop
            width: 2
            height: root.tabHeightValue
            radius: 1
            color: root.accent
        }
    }

    // The pair the strip grows when it overflows. Each arrow scrolls one chip
    // and dims at its own end; `+` sits between the right arrow and the window
    // buttons, so it is always reachable.
    Item {
        id: scrollLeftButton
        visible: root.tabsOverflow
        x: root.tabLeft
        y: (root.height - root.arrowWidth) / 2
        width: root.arrowWidth
        height: root.arrowWidth
        readonly property bool canScroll: root.canScrollLeft
        opacity: canScroll ? 1.0 : 0.4

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusFull
            color: scrollLeftMouse.containsMouse && scrollLeftButton.canScroll
                ? Theme.hover : "transparent"
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
        y: (root.height - root.arrowWidth) / 2
        width: root.arrowWidth
        height: root.arrowWidth
        readonly property bool canScroll: root.canScrollRight
        opacity: canScroll ? 1.0 : 0.4

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusFull
            color: scrollRightMouse.containsMouse && scrollRightButton.canScroll
                ? Theme.hover : "transparent"
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
        y: (root.height - root.plusWidth) / 2
        width: root.plusWidth
        height: root.plusWidth

        Rectangle {
            anchors.fill: parent
            property real morph: plusMouse.pressed ? 1 : 0
            radius: height / 2 + (Theme.radiusMorphPress - height / 2) * morph
            Behavior on morph {
                enabled: Theme.motion
                SpringAnimation {
                    spring: Theme.springSpatialFast
                    damping: Theme.dampSpatialFast
                    epsilon: 0.005
                }
            }
            color: plusMouse.containsMouse ? Theme.hover : "transparent"
        }

        Glyph {
            anchors.centerIn: parent
            width: 14
            height: 14
            kind: "plus"
            tint: plusMouse.containsMouse
                ? Theme.ink : Theme.inkSecondary
        }

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("新建标签页")
        Accessible.onPressAction: root.newTabRequested()

        MouseArea {
            id: plusMouse
            anchors.fill: parent
            hoverEnabled: true
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
                /// Windows reads hover/press from the native NC state; the other
                /// platforms keep the QML MouseArea below.
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
                                : Theme.hover)
                            : "transparent")
                }

                // The system's own caption glyphs (Segoe Fluent Icons on Windows
                // 11, Segoe MDL2 Assets on 10): the exact code points DWM draws,
                // at the native 10 px, so the buttons read as the system's own.
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

                // Drawn fallback for the platforms without the system icon face
                // (§5.4): a minimise line, a maximise square, a restore pair and
                // a close cross. Windows uses the system glyphs above instead.
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

                    // Restore: the back plate is offset up and right, the front
                    // plate is filled with the bar so the overlap reads as two.
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
