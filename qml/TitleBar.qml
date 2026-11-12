pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window

// The chrome bar: the window's mark on the left, the tab strip in the middle,
// the three native buttons on the right, all in 44 dp.
//
// The title used to have a row of its own. It was showing the current path,
// which the breadcrumb directly below already shows, in a font that could not
// match it — 40 dp of a name nobody needed to read twice. Merging it into the
// tab strip is what buys the tab chips their 34 dp without making the shell
// taller than it is now.
//
// The bar is also the window's drag handle, and that is a native decision, not
// a QML one: `crates/dae-qt/cpp/window.h` returns HTCAPTION for the bar minus
// the three button columns minus `captionClientRight`, the band the tabs
// occupy. DefWindowProc then runs the move loop, which is the only way drag,
// Aero Snap, the Snap Layouts hover on maximise and double-click maximise all
// stay native. `implicitHeight` here and `kTitleBarDp` there are the same
// number; changing one without the other puts the button hit zones off by the
// difference.
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
    /// bar renders it any more.
    property string title: "dae"

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
    readonly property color mutedColor: Theme.inkMuted
    readonly property bool maximized: window !== null
        && window.visibility === Window.Maximized
    /// The green button's own state on macOS. Kept apart from `maximized`,
    /// which the Windows-drawn buttons still use.
    readonly property bool fullScreen: window !== null
        && window.visibility === Window.FullScreen

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
    // the window manager for its own move loop instead. The tab strip and the
    // buttons are declared later, so they keep their clicks.
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
        /// Double-click-to-maximize is a pointer gesture on the bar; the window
        /// is maximized from the system menu, so this handler is not a control.
        enabled: Qt.platform.os !== "windows"
        onDoubleTapped: {
            if (root.window === null)
                return
            if (!root.isMac) {
                root.maximized ? root.window.showNormal() : root.window.showMaximized()
                return
            }
            // macOS: the action belongs to the user's preference, the same one
            // AppKit would have applied had the click reached the caption.
            const action = root.controller !== null
                ? root.controller.titlebarDoubleAction() : "zoom"
            if (action === "minimize")
                root.window.showMinimized()
            else if (action === "zoom")
                root.toggleMacZoom()
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
        // `signal` means a system accent cannot turn the window's identity a
        // second colour while every selection track stays orange.
        color: Theme.brand
        visible: !root.isMac

        Text {
            anchors.centerIn: parent
            text: "D"
            color: Theme.onInteractive
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
        y: 16
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
                        : (button.hovered ? Theme.fill : "transparent")
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
