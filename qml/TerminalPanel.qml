import QtQuick
import dae 1.0
import dae.terminal 1.0

// The bottom terminal panel. The shell is started on first use and stays alive
// while the panel is hidden, which is the behaviour the previous shell already
// had: hiding a terminal is not closing it.
Item {
    id: root

    /// The tab whose shell this panel shows. The session belongs to the tab,
    /// so switching tabs swaps the whole shell instead of reusing one process.
    property var tab: null
    readonly property var session: tab !== null ? tab.terminal : null
    readonly property string cwd: tab !== null && tab.listing ? tab.listing.path : ""
    readonly property bool running: session !== null && session.running
    property bool dark: true
    property string material: "none"
    property bool active: false
    property int panelHeight: 280

    signal exited()

    readonly property color chromeColor: material === "none"
        ? Theme.chrome
        : (dark ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
                : Qt.rgba(0.98, 0.985, 0.99, 0.46))
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline

    implicitHeight: active ? panelHeight : 0
    height: implicitHeight
    clip: true

    function start() {
        if (tab !== null)
            tab.startTerminal()
    }

    function focusTerminal() {
        start()
        view.focusTerminal()
    }

    function stop() {
        if (session !== null)
            session.kill()
    }

    onActiveChanged: {
        if (active) {
            start()
            Qt.callLater(root.focusTerminal)
        }
    }

    // The panel stays open across folder tabs; the new tab's shell starts the
    // first time it becomes visible, and an already running one is only
    // re-bound.
    onTabChanged: {
        if (active)
            start()
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
            text: root.running
                ? (root.cwd !== "" ? qsTr("终端 · ") + root.cwd : qsTr("终端"))
                : (session !== null ? qsTr("终端 · 已退出 ") + session.exitCode : qsTr("终端"))
            color: root.running ? root.textColor : root.mutedColor
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
            visible: !root.running
            color: restartMouse.containsMouse ? Theme.fill
                                              : "transparent"

            Text {
                anchors.centerIn: parent
                text: qsTr("重新启动")
                color: root.mutedColor
                font.pixelSize: 11
            }

            MouseArea {
                id: restartMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: {
                    root.start()
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
                radius: 4
                color: closeMouse.containsMouse ? Theme.fill
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
        session: root.session
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
