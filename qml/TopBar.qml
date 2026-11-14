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

    implicitHeight: Theme.shellTitle
    height: Theme.shellTitle

    readonly property bool isMac: Qt.platform.os === "osx"
    /// Where the tab strip ends and the window buttons begin. The shell anchors
    /// its strip to this instead of repeating the button width in QML.
    readonly property real buttonWidth: isMac ? 0
        : (Qt.platform.os === "linux" ? 40 : 46)
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

    // ── Tab metrics (§7.3.1) ────────────────────────────────────────────────
    readonly property int tabWidthValue: 184
    /// 30 dp inside a 38 dp bar: four above, four below.
    readonly property int tabHeightValue: 30
    readonly property int tabTop: 4
    readonly property int tabGap: 2
    /// The strip starts where the mark ends (14 + 17 + 13); on macOS it starts
    /// after the three traffic lights.
    readonly property int tabLeft: isMac ? 78 : 44
    readonly property int plusWidth: 26
    /// Tabs stop before the button columns. The 8 dp gap stays caption, like
    /// the rest of the empty bar.
    readonly property real stripWidth: Math.max(0, width - buttonsWidth - 8)

    /// The horizontal slice of the bar that the tabs occupy, in bar
    /// coordinates. The shell forwards it to the native hit test so a tab
    /// click reaches QML instead of starting a window drag; the empty space to
    /// the right of `+` stays caption and remains the window's grab handle.
    readonly property real clientLeft: tabLeft
    readonly property real clientRight: Math.min(tabX(tabs.length) + plusWidth,
                                                 stripWidth)

    function tabX(index) {
        return tabLeft + index * (tabWidthValue + tabGap)
    }

    function indexAt(x) {
        for (let i = 0; i < tabs.length; i += 1) {
            const middle = tabX(i) + tabWidthValue / 2
            if (x < middle)
                return i
        }
        return tabs.length
    }

    function indicatorX(index) {
        if (index <= 0)
            return root.tabLeft
        const column = Math.min(index, tabs.length) - 1
        return tabX(column) + tabWidthValue + tabGap / 2
    }

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
    /// `DropArea` drives this while the native loop runs.
    function showDrop(x) {
        dropX = x
    }

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
        height: 1
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
        width: 17
        height: 17
        radius: Theme.radiusXs
        // The mark is the brand, not an interactive control: keeping it on
        // `accent` means a system accent cannot turn the window's identity a
        // second colour while every selection track stays on the theme accent.
        color: Theme.accent
        visible: !root.isMac

        Text {
            anchors.centerIn: parent
            text: "D"
            color: Theme.onAccent
            font.pixelSize: Theme.typeMicro
            font.weight: Font.Medium
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

    // The chips, `+` and the drop indicator. Clipped at the button columns so a
    // full strip never paints over them.
    Item {
        id: strip
        x: 0
        y: 0
        width: root.stripWidth
        height: root.height
        clip: true

        Row {
            x: root.tabLeft
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
                    radius: Theme.radiusSm
                    // Active reads as content (§7.3.1): canvas fill, no
                    // underline, no accent bar. It is the surface the tab
                    // reveals, not a highlighted control.
                    color: root.activeIndex === index
                        ? Theme.canvas
                        : (tabHover.hovered ? Theme.hover : "transparent")
                    opacity: root.dragging && root.dragIndex === index ? 0.45 : 1.0

                    /// Hover over the whole chip, close control included.
                    /// `tabMouse` stops short of the close button so a press
                    /// there cannot activate the tab.
                    HoverHandler { id: tabHover }

                    /// A tab is the first thing Tab should reach, and Enter is
                    /// the keyboard's click.
                    activeFocusOnTab: true
                    border.width: tabItem.activeFocus ? 1 : 0
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
                        width: 5
                        height: 5
                        radius: 2.5
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.accent
                    }

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: tabItem.modelData
                            && tabItem.modelData.pinned ? 19 : 10
                        anchors.right: closeButton.left
                        anchors.rightMargin: 4
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.tabLabel(tabItem.index)
                        color: root.activeIndex === tabItem.index
                            ? Theme.ink
                            : (tabHover.hovered ? Theme.ink : Theme.inkSecondary)
                        font.pixelSize: Theme.typeCaption
                        font.weight: root.activeIndex === tabItem.index
                            ? Font.Medium : Font.Normal
                        elide: Text.ElideMiddle
                    }

                    Item {
                        id: closeButton
                        anchors.right: parent.right
                        anchors.rightMargin: 5
                        anchors.verticalCenter: parent.verticalCenter
                        width: 18
                        height: Theme.controlXs
                        visible: root.activeIndex === tabItem.index
                            || tabHover.hovered

                        Rectangle {
                            anchors.fill: parent
                            radius: Theme.radiusXs
                            color: closeMouse.containsMouse
                                ? Theme.hover : "transparent"
                        }

                        Glyph {
                            anchors.centerIn: parent
                            width: 12
                            height: 12
                            kind: "close"
                            tint: closeMouse.containsMouse
                                ? Theme.ink : Theme.inkSecondary
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

                        onPressed: function (mouse) {
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

                        onReleased: {
                            root.dragArmed = false
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

            Item {
                width: root.plusWidth
                height: root.plusWidth
                // Centred against the 38 dp bar, not the 30 dp chip.
                y: (root.height - root.plusWidth) / 2 - root.tabTop

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
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
                readonly property bool hovered: mouse.containsMouse
                /// Native only on Windows: there DefWindowProc owns the
                /// maximise button so Snap Layouts opens on hover, and the NC
                /// hit test means a QML click would never arrive anyway.
                readonly property bool nativeMaximize: button.isMaximize
                    && Qt.platform.os === "windows"

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
                        ? (button.hovered ? "#c42b1c" : "transparent")
                        : (button.hovered ? Theme.hover : "transparent")
                }

                // Windows keeps its own glyph font. The other two platforms
                // have no font that carries these code points — naming
                // "Segoe Fluent Icons" there is how the buttons used to render
                // as empty boxes — so they get drawn shapes instead.
                Text {
                    anchors.centerIn: parent
                    visible: Qt.platform.os === "windows"
                    text: button.isClose ? "\uE8BB"
                        : button.isMaximize
                            ? (root.maximized ? "\uE923" : "\uE922")
                            : "\uE921"
                    color: button.isClose && button.hovered ? Theme.onSolid : root.textColor
                    font.family: "Segoe Fluent Icons"
                    font.pixelSize: Theme.typeMicro
                }

                Item {
                    id: drawnGlyph
                    anchors.centerIn: parent
                    width: 14
                    height: 14
                    visible: Qt.platform.os !== "windows"

                    readonly property color ink: button.isClose && button.hovered
                        ? Theme.onSolid : root.textColor

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
                    enabled: !button.nativeMaximize
                    hoverEnabled: !button.nativeMaximize
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
