pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window

// The custom title bar from gate 3. Its geometry is not free: the native hit
// test in crates/dae-qt/cpp/window.h assumes a 40 dp bar and three 46 dp
// button columns at the right. The left part is HTCAPTION, so drag, Aero Snap
// and double-click maximize are handled by DefWindowProc. Only minimize and
// close are client areas; maximize is HTMAXBUTTON so Snap Layouts opens on
// hover and is never touched by QML.
Item {
    id: root

    property var window: null
    property bool dark: true
    property string material: "none"
    property string title: "dae"

    implicitHeight: 40
    height: 40

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property bool maximized: window !== null
        && window.visibility === Window.Maximized

    Rectangle {
        anchors.fill: parent
        color: root.chromeColor
    }

    Rectangle {
        id: logo
        x: 14
        anchors.verticalCenter: parent.verticalCenter
        width: 20
        height: 20
        radius: 6
        color: root.dark ? "#6ea8fe" : "#2f6fdf"

        Text {
            anchors.centerIn: parent
            text: "D"
            color: "#ffffff"
            font.pixelSize: 12
            font.weight: Font.DemiBold
        }
    }

    Text {
        anchors.left: logo.right
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        text: root.title
        color: root.textColor
        font.pixelSize: 13
        font.weight: Font.DemiBold
    }

    Item {
        id: windowButtons
        anchors.right: parent.right
        height: parent.height
        width: 46 * 3

        Repeater {
            model: 3

            Item {
                id: button
                required property int index

                width: 46
                height: 40
                x: index * 46

                readonly property bool isClose: index === 2
                readonly property bool isMaximize: index === 1
                readonly property bool hovered: mouse.containsMouse

                Rectangle {
                    anchors.fill: parent
                    color: button.isClose
                        ? (button.hovered ? "#c42b1c" : "transparent")
                        : (button.hovered ? (root.dark ? "#1fffffff" : "#12000000")
                                          : "transparent")
                }

                Text {
                    anchors.centerIn: parent
                    text: button.isClose ? "\uE8BB"
                        : button.isMaximize
                            ? (root.maximized ? "\uE923" : "\uE922")
                            : "\uE921"
                    color: button.isClose && button.hovered ? "#ffffff" : root.textColor
                    font.family: "Segoe Fluent Icons"
                    font.pixelSize: 10
                }

                MouseArea {
                    id: mouse
                    anchors.fill: parent
                    enabled: !button.isMaximize
                    hoverEnabled: !button.isMaximize
                    onClicked: {
                        if (button.isClose)
                            root.window.close()
                        else
                            root.window.showMinimized()
                    }
                }
            }
        }
    }
}
