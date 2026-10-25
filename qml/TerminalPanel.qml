import QtQuick
import dae 1.0
import dae.terminal 1.0

// The bottom terminal panel. The shell is started on first use and stays alive
// while the panel is hidden, which is the behaviour the previous shell already
// had: hiding a terminal is not closing it.
Item {
    id: root

    property string cwd: ""
    property bool dark: true
    property string material: "none"
    property bool active: false
    property int panelHeight: 280

    signal exited()

    readonly property color chromeColor: material === "none"
        ? (dark ? "#14161a" : "#f4f6f8")
        : (dark ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
                : Qt.rgba(0.98, 0.985, 0.99, 0.46))
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"

    implicitHeight: active ? panelHeight : 0
    height: implicitHeight
    clip: true

    TerminalSession {
        id: session
    }

    function start() {
        if (session.running)
            return
        session.start(root.cwd, 80, 24)
    }

    function focusTerminal() {
        if (!session.running)
            start()
        view.focusTerminal()
    }

    function stop() {
        session.kill()
    }

    onActiveChanged: {
        if (active) {
            start()
            Qt.callLater(root.focusTerminal)
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "#101216"
    }

    Rectangle {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 26
        color: root.chromeColor

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: root.hairline
        }

        Text {
            x: 10
            anchors.verticalCenter: parent.verticalCenter
            text: session.running
                ? (root.cwd !== "" ? "终端 · " + root.cwd : "终端")
                : ("终端 · 已退出 " + session.exitCode)
            color: session.running ? root.textColor : root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideMiddle
            width: parent.width - 180
        }

        Rectangle {
            id: restartButton
            anchors.right: closeButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 52
            height: 18
            radius: 4
            visible: !session.running
            color: restartMouse.containsMouse ? (root.dark ? "#26ffffff" : "#12000000")
                                              : "transparent"

            Text {
                anchors.centerIn: parent
                text: "重新启动"
                color: root.mutedColor
                font.pixelSize: 10
            }

            MouseArea {
                id: restartMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: {
                    session.start(root.cwd, 80, 24)
                    Qt.callLater(root.focusTerminal)
                }
            }
        }

        Item {
            id: closeButton
            anchors.right: parent.right
            anchors.rightMargin: 5
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20

            Rectangle {
                anchors.fill: parent
                radius: 3
                color: closeMouse.containsMouse ? (root.dark ? "#26ffffff" : "#12000000")
                                                : "transparent"
            }

            Text {
                anchors.centerIn: parent
                text: "\u00d7"
                color: closeMouse.containsMouse ? root.textColor : root.mutedColor
                font.pixelSize: 13
            }

            MouseArea {
                id: closeMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.active = false
            }
        }
    }

    TerminalItem {
        id: view
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        anchors.margins: 6
        session: session
        dark: root.dark
        fontSize: Workspace.terminalFontSize
        lineHeight: Workspace.terminalLineHeight
        fontFamily: Workspace.terminalFontFamily
    }

    Connections {
        target: session

        function onExited(code) {
            // The grid keeps the last frame; only the header changes. The
            // window moves focus back to the listing so the next keystroke is
            // not swallowed by a dead shell.
            root.exited()
        }
    }
}
