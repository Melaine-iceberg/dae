pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import dae.terminal 1.0

// The bottom terminal panel. The shell is started on first use and stays alive
// while the panel is hidden, which is the behaviour the previous shell already
// had: hiding a terminal is not closing it.
//
// 240 px high and embedded in the content column, so the listing above it keeps
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
    property int panelHeight: 240

    /// §11.6's 跟随当前目录 switch. When it is on, a navigation in the tab's
    /// listing sends the shell a `cd`, so the terminal is always standing in
    /// the folder the user is looking at. Off is the default: a shell the user
    /// has cd'd elsewhere must not be pulled back by a click in the listing.
    property bool followDirectory: false
    /// The directory this panel last put the shell in, as far as the shell can
    /// know it. Empty before the first start; the header then names the tab's
    /// path, which is where the shell will start.
    property string shellDirectory: ""

    readonly property string workingDirectory: root.shellDirectory !== ""
        ? root.shellDirectory
        : (root.tab !== null && root.tab.folder ? root.tab.listing.path : "")

    signal exited()

    /// The shell this session is running, named by its file name. Before the
    /// first start it shows what the setting will launch; afterwards it keeps
    /// naming the process that is actually alive, even if the setting changed.
    readonly property string shellName: {
        const started = session !== null && session.title !== ""
        const shell = (started ? session.shell : ExplorerPrefs.terminalShell).trim()
        if (shell !== "")
            return shell.split(/[\\/]/).pop().replace(/\.exe$/i, "")
        return Qt.platform.os === "windows" ? "PowerShell"
            : Qt.platform.os === "osx" ? "zsh" : "bash"
    }

    implicitHeight: active ? panelHeight : 0
    height: implicitHeight
    clip: true

    function start() {
        if (tab === null || !root.active)
            return
        // Starting the shell before the item has its size would create the PTY
        // at the placeholder size; the resize that follows makes PowerShell
        // redraw its prompt further down and leaves blank rows above it.
        // TerminalItem is a C++ type; qmllint's model of it stops at the
        // registered metatypes, so the grid size reads as a missing property.
        // qmllint disable missing-property
        if (view.columns <= 1 || view.rows <= 1) {
            Qt.callLater(root.start)
            return
        }
        // qmllint disable missing-property
        tab.startTerminal(view.columns, view.rows)
        root.shellDirectory = tab.listing.path
    }

    /// `cd` spelled for the shell that is running. The panel names the shell
    /// by its file name, so the same three branches cover Windows and POSIX.
    function changeDirectoryCommand(path) {
        const shell = root.shellName.toLowerCase()
        if (shell.indexOf("powershell") >= 0 || shell === "pwsh")
            return "Set-Location -LiteralPath '" + String(path).replace(/'/g, "''") + "'"
        if (shell === "cmd")
            return "cd /d \"" + path + "\""
        return "cd '" + String(path).replace(/'/g, "'\\''") + "'"
    }

    /// Moves the live shell into `path`, if the panel is following the tab.
    function syncDirectory() {
        if (!root.followDirectory || !root.running || root.session === null)
            return
        const path = root.tab !== null && root.tab.folder ? root.tab.listing.path : ""
        if (path === "" || path === root.shellDirectory)
            return
        root.shellDirectory = path
        root.session.write(root.changeDirectoryCommand(path) + "\r")
    }

    function toggleFollowDirectory() {
        root.followDirectory = !root.followDirectory
        if (root.followDirectory)
            root.syncDirectory()
    }

    /// §11.6's 清屏: the shell's own clear command, so the scrollback that
    /// goes away is the one the shell owns.
    function clearScreen() {
        if (!root.running || root.session === null)
            return
        const shell = root.shellName.toLowerCase()
        const command = (shell.indexOf("powershell") >= 0 || shell === "pwsh")
            ? "Clear-Host" : (shell === "cmd" ? "cls" : "clear")
        root.session.write(command + "\r")
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
        if (active)
            Qt.callLater(root.focusTerminal)
    }

    // The panel stays open across folder tabs; the new tab's shell starts the
    // first time it becomes visible, and an already running one is only
    // re-bound.
    onTabChanged: {
        // The shell belongs to the tab, so the directory the panel names is
        // the new tab's. It is set again when the shell starts.
        root.shellDirectory = root.tab !== null && root.tab.folder
            ? root.tab.listing.path : ""
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
        height: Theme.controlMd
        color: Theme.terminalHeader

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: Theme.hairline
        }

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.right: clearButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6

            Glyph {
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                height: 14
                kind: "terminal"
                tint: Theme.terminalInk
            }

            // A start failure puts its reason in `title` (a bad configured
            // shell path, for one); name that instead of the exit code -1.
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: root.running
                    ? qsTr("终端 · %1").arg(root.shellName)
                    : (root.session !== null && root.session.title !== ""
                            && root.session.title !== "Terminal"
                        ? qsTr("终端 · %1").arg(root.session.title)
                        : (root.session !== null
                            ? qsTr("终端 · 已退出 %1").arg(root.session.exitCode)
                            : qsTr("终端")))
                color: Theme.terminalInk
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideRight
                textFormat: Text.PlainText
                width: Math.min(implicitWidth, header.width * 0.4)
            }

            /// §11.6's 工作目录: where this shell started, in the tab's own
            /// path. The dim mono run says it without competing with the
            /// session name; the full path is in the tooltip.
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, parent.width - x - 24)
                text: root.workingDirectory
                color: Theme.terminalInk
                opacity: 0.65
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideMiddle
                textFormat: Text.PlainText

                ToolTip { text: root.workingDirectory }
            }
        }

        /// 清屏: one command, spelled for the shell that is running. It is a
        /// shell command rather than a grid wipe, because the scrollback the
        /// user can scroll back into is the shell's, not this panel's.
        Rectangle {
            id: clearButton
            anchors.right: followButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 44
            height: Theme.controlXs
            radius: Theme.radiusXs
            visible: root.running
            color: clearMouse.containsMouse ? Theme.hover : "transparent"

            Text {
                anchors.centerIn: parent
                text: qsTr("清屏")
                color: clearMouse.containsMouse
                    ? Theme.terminalInkStrong : Theme.terminalInk
                font.pixelSize: Theme.typeMicro
                textFormat: Text.PlainText
            }

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("清屏")
            Accessible.onPressAction: root.clearScreen()

            MouseArea {
                id: clearMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.clearScreen()
            }
        }

        /// §11.6's 跟随当前目录 switch. On, a navigation in this tab's listing
        /// sends the shell a `cd`; off, the shell keeps its own directory.
        Rectangle {
            id: followButton
            anchors.right: restartButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: followRow.implicitWidth + 12
            height: Theme.controlXs
            radius: Theme.radiusXs
            color: followMouse.containsMouse ? Theme.hover : "transparent"
            border.width: 1
            border.color: root.followDirectory
                ? Theme.accent : Theme.terminalInk

            Row {
                id: followRow
                anchors.centerIn: parent
                spacing: 4

                Rectangle {
                    width: 6
                    height: 6
                    radius: 3
                    anchors.verticalCenter: parent.verticalCenter
                    color: root.followDirectory ? Theme.accent : "transparent"
                    border.width: 1
                    border.color: Theme.terminalInk
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("跟随目录")
                    color: root.followDirectory
                        ? Theme.terminalInkStrong : Theme.terminalInk
                    font.pixelSize: Theme.typeMicro
                    textFormat: Text.PlainText
                }
            }

            Accessible.role: Accessible.CheckBox
            Accessible.name: qsTr("跟随当前目录")
            Accessible.checked: root.followDirectory
            Accessible.onPressAction: root.toggleFollowDirectory()

            ToolTip {
                text: qsTr("导航后让终端切换到当前目录")
            }

            MouseArea {
                id: followMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.toggleFollowDirectory()
            }
        }

        Rectangle {
            id: restartButton
            anchors.right: closeButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 56
            height: Theme.controlXs
            radius: Theme.radiusXs
            visible: !root.running
            color: restartMouse.containsMouse ? Theme.hover : "transparent"

            Text {
                anchors.centerIn: parent
                text: qsTr("重新启动")
                color: restartMouse.containsMouse ? Theme.terminalInkStrong : Theme.terminalInk
                font.pixelSize: Theme.typeMicro
                textFormat: Text.PlainText
            }

            MouseArea {
                id: restartMouse
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("重新启动终端")
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
            height: Theme.controlXs

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("关闭终端")

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusXs
                color: closeMouse.containsMouse ? Theme.hover : "transparent"
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
        anchors.leftMargin: 12
        anchors.rightMargin: 8
        anchors.topMargin: 8
        anchors.bottomMargin: 8
        /// The native terminal consumes Tab — it has to, Tab is completion
        /// inside a shell — so it may only be part of the focus chain while the
        /// panel is on screen. Hidden, it would be a stop that swallows the key
        /// and leaves focus nowhere.
        activeFocusOnTab: root.active
        session: root.session
        dark: root.dark
        fontSize: Workspace.terminalFontSize
        lineHeight: Workspace.terminalLineHeight
        fontFamily: Workspace.terminalFontFamily !== ""
            ? Workspace.terminalFontFamily : Theme.monoFamily
    }

    Connections {
        target: root.session

        function onExited(code) {
            // The grid keeps the last frame; only the header changes. The
            // window moves focus back to the listing so the next keystroke is
            // not swallowed by a dead shell.
            root.exited()
        }
    }

    Connections {
        target: root.tab !== null && root.tab.folder ? root.tab.listing : null

        /// The tab navigated; with 跟随目录 on, the shell follows it.
        function onPathChanged() {
            root.syncDirectory()
        }
    }
}
