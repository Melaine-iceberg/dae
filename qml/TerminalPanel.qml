import QtQuick
import dae 1.0
import dae.terminal 1.0

// The bottom terminal panel. The shell is started on first use and stays alive
// while the panel is hidden, which is the behaviour the previous shell already
// had: hiding a terminal is not closing it.
//
// 132 px high and embedded in the content column, so the listing above it keeps
// its rows and the terminal never covers them. See
// docs/ui-instrument-migration.md §7.2.
Item {
    id: root

    /// The tab whose shell this panel shows. The session belongs to the tab,
    /// so switching tabs swaps the whole shell instead of reusing one process.
    property var tab: null
    readonly property var session: tab !== null ? tab.terminal : null
    readonly property bool running: session !== null && session.running
    property bool dark: true
    property string material: "none"
    property bool active: false
    property int panelHeight: 132

    signal exited()

    /// The shell the core starts on this platform. It is a label, not a probe:
    /// the core picks whichever of these is installed, and the panel names the
    /// family rather than the executable.
    readonly property string shellName: Qt.platform.os === "windows" ? "PowerShell"
        : Qt.platform.os === "osx" ? "zsh" : "bash"

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
        color: Theme.terminalCanvas
    }

    Rectangle {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 28
        color: Theme.terminalHeader

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Qt.rgba(1, 1, 1, 0.07)
        }

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6

            Glyph {
                anchors.verticalCenter: parent.verticalCenter
                width: 12
                height: 12
                kind: "terminal"
                tint: Theme.terminalInk
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: root.running
                    ? qsTr("终端 · %1").arg(root.shellName)
                    : (session !== null
                        ? qsTr("终端 · 已退出 %1").arg(session.exitCode)
                        : qsTr("终端"))
                color: Theme.terminalInk
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideRight
                textFormat: Text.PlainText
                width: Math.min(implicitWidth, header.width - 120)
            }
        }

        Rectangle {
            id: restartButton
            anchors.right: closeButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 56
            height: 18
            radius: Theme.radiusXs
            visible: !root.running
            color: restartMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"

            Text {
                anchors.centerIn: parent
                text: qsTr("重新启动")
                color: restartMouse.containsMouse ? Theme.terminalInkStrong : Theme.terminalInk
                font.pixelSize: Theme.typeMicro
                textFormat: Text.PlainText
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
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("关闭终端")

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusXs
                color: closeMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
            }

            Glyph {
                anchors.centerIn: parent
                width: 12
                height: 12
                kind: "close"
                tint: closeMouse.containsMouse ? Theme.terminalInkStrong : Theme.terminalInk
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
        anchors.leftMargin: 10
        anchors.rightMargin: 8
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        /// The native terminal consumes Tab — it has to, Tab is completion
        /// inside a shell — so it may only be part of the focus chain while the
        /// panel is on screen. Hidden, it would be a stop that swallows the key
        /// and leaves focus nowhere.
        activeFocusOnTab: root.active
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
