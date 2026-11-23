pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import dae.terminal 1.0

// The bottom terminal panel. Sessions live here, not on tabs: the panel is
// the terminal's home and a tab only says where a *new* session starts
// (Zed's `current_project_directory` semantics). Hiding the panel keeps
// every shell alive; switching or closing tabs touches none of them.
//
// Embedded in the content column, so the listing above it keeps its rows and
// the terminal never covers them. 240 px by default; the top edge is a drag
// handle whose range reaches the column's top boundary, and the height is
// remembered in explorer-qt.ini. See docs/ui-instrument-migration.md §7.2 and
// docs/terminal-redesign.md §3.
Item {
    id: root

    /// The tab whose folder a *new* session starts in. Existing sessions
    /// are never moved or restarted on a tab switch.
    property var tab: null
    property var sessions: []
    property int activeIndex: 0
    readonly property var session: activeIndex >= 0 && activeIndex < sessions.length
        ? sessions[activeIndex] : null
    readonly property bool running: session !== null && session.running
    property bool dark: true
    property string material: "none"
    property bool active: false
    /// The content column's height: the drag handle's upper boundary.
    property int heightLimit: 0
    /// Live height while the top edge is dragged; -1 when at rest, so the
    /// preference binding below stays the source of truth between gestures.
    property int liveHeight: -1
    readonly property int heightFloor: 120
    readonly property int panelHeight: Math.max(heightFloor,
        Math.min(ExplorerPrefs.terminalHeight, heightLimit))
    /// The shell's OSC 0/2 title when it reports one; the panel names the
    /// configured shell before the first report and after an integration-less
    /// shell stays with the executable's name.
    readonly property string sessionTitle: {
        const reported = session !== null ? String(session.windowTitle) : ""
        // PowerShell 7 reports its own executable path as the window title
        // until a prompt theme replaces it. A path is not a name — and at
        // 12 px mono it ate the working directory beside it.
        if (reported === "" || /^[A-Za-z]:[\\/]/.test(reported)
                || reported.charAt(0) === "/" || /\.exe$/i.test(reported))
            return root.shellName
        return reported
    }
    /// The directory this panel last put the active shell in — the spawn
    /// directory, used as a fallback while the shell has not yet reported
    /// its own working directory through OSC 7/9/1337.
    property string shellDirectory: ""

    /// OSC report when the shell has integration installed; the spawn
    /// directory when it does not. The tooltip says which one it is.
    readonly property string workingDirectory: session !== null && session.cwd !== ""
        ? session.cwd
        : (root.shellDirectory !== ""
            ? root.shellDirectory
            : (root.tab !== null && root.tab.folder ? root.tab.listing.path : ""))
    readonly property bool directoryReported: session !== null && session.cwd !== ""

    signal exited()
    /// The header's close button asks the shell to hide the panel. The panel
    /// must not assign to `active` itself: that property is bound by the shell,
    /// and a local assignment would break the binding, leaving the status bar
    /// and focus ring believing the terminal is still open.
    signal hideRequested()

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

    implicitHeight: active ? (liveHeight >= 0 ? liveHeight : panelHeight) : 0
    height: implicitHeight
    clip: true

    /// The find bar Ctrl+F opens. It searches the active session's grid,
    /// scrollback included; closing it clears the needle.
    property bool searchOpen: false

    function openSearch() {
        root.searchOpen = true
        Qt.callLater(() => searchField.forceActiveFocus())
    }

    function closeSearch() {
        root.searchOpen = false
        if (root.session !== null)
            root.session.setSearch("")
        view.focusTerminal()
    }

    function start() {
        if (!root.active || session === null)
            return
        if (session.running) {
            view.focusTerminal()
            return
        }
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
        const path = root.tab !== null && root.tab.folder ? root.tab.listing.path : ""
        session.start(path, view.columns, view.rows, ExplorerPrefs.terminalShell)
        root.shellDirectory = path
    }

    /// Absolute stays absolute; a relative link resolves against the
    /// session's OSC-reported directory, falling back to where the shell
    /// started.
    function openLink(target) {
        if (/^https?:\/\//i.test(target)) {
            Qt.openUrlExternally(target)
            return
        }
        let path = String(target)
        if (/^[A-Za-z]:[\\/]/.test(path) || path.startsWith("/") || path.startsWith("\\\\"))
            ; // already absolute
        else {
            const reported = root.session !== null && root.session.cwd !== ""
                ? String(root.session.cwd) : root.shellDirectory
            path = reported !== ""
                ? reported.replace(/[\\/]+$/, "") + "/" + target : target
        }
        Workspace.openPath(path)
    }

    /// Opens a new session in the active tab's folder and focuses it.
    function newSession() {
        root.sessions = root.sessions.concat(sessionComponent.createObject(root))
        root.activeIndex = root.sessions.length - 1
        Qt.callLater(root.start)
    }



    /// The shell a session runs, by file name — the pills need it without
    /// touching the panel's "which session is active" state.
    function shellOf(named) {
        const shell = String(named.shell).trim()
        if (shell !== "")
            return shell.split(/[\\/]/).pop().replace(/\.exe$/i, "")
        return Qt.platform.os === "windows" ? "PowerShell"
            : Qt.platform.os === "osx" ? "zsh" : "bash"
    }

    Component {
        id: sessionComponent
        TerminalSession {}
    }

    function closeSession(index) {
        if (index < 0 || index >= root.sessions.length)
            return
        const victim = root.sessions[index]
        root.sessions = root.sessions.filter((named, at) => at !== index)
        victim.kill()
        victim.destroy()
        if (root.activeIndex >= root.sessions.length)
            root.activeIndex = Math.max(0, root.sessions.length - 1)
    }

    /// The panel never *follows* navigation with an injected `cd` — that
    /// pollutes the shell's own history (Zed's rule too). The shell's
    /// directory is *reported* through OSC 7/9/1337 by the integration
    /// snippets in Assets/terminal-integration, and new sessions simply
    /// start in the active tab's folder.

    /// ...but a jump the user *asks* for is theirs: `cd` to the active
    /// tab's folder, sent as keyboard input, quoted the shell in the
    /// session understands. The history entry is the user's own doing.
    function jumpToFolder() {
        if (!root.running || root.session === null || root.tab === null)
            return
        if (!root.tab.folder)
            return
        const path = String(root.tab.listing.path)
        // The PTY is a local shell; a remote tab has no directory to name.
        if (path === "" || !/^([A-Za-z]:[\\/]|\/)/.test(path))
            return
        const shell = root.shellName.toLowerCase()
        let command
        if (shell.indexOf("powershell") >= 0 || shell === "pwsh") {
            // PowerShell: escape what a double-quoted string interpolates.
            const quoted = '"' + path.replace(/`/g, '``').replace(/\$/g, '`$') + '"'
            command = 'cd -- ' + quoted
        } else if (shell.indexOf("cmd") >= 0) {
            command = 'cd /d "' + path.replace(/"/g, '""') + '"'
        } else {
            // POSIX: single quotes are literal; splice in `'\''` for the
            // one character that can appear in them.
            const quoted = "'" + path.replace(/'/g, "'\\''") + "'"
            command = 'cd -- ' + quoted
        }
        root.session.write(command + '\n')
        view.focusTerminal()
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
        if (root.sessions.length === 0) {
            newSession()
            return
        }
        // A session that was created while the panel was hidden (session
        // restore makes `active` true for a moment) has no shell behind it
        // yet; opening the panel starts it in the tab the user is looking
        // at now, not the one from when the ghost object appeared.
        if (!root.running)
            Qt.callLater(root.start)
        view.focusTerminal()
    }

    function stop() {
        if (session !== null)
            session.kill()
    }

    onActiveChanged: {
        if (active) {
            if (root.sessions.length === 0)
                newSession()
            else if (!root.running)
                Qt.callLater(root.start)
            Qt.callLater(root.focusTerminal)
        }
    }

    // The sessions are the panel's, so the directory the panel names changes
    // only when a session starts — never on a tab switch.
    onTabChanged: {
        root.shellDirectory = ""
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
        /// 36 dp: a 28 dp bar could not hold the 12 px session name and the
        /// pills beside it without the two touching the hairlines.
        height: 36
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
            anchors.right: sessionStrip.left
            anchors.rightMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8

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
                    ? qsTr("终端 · %1").arg(root.sessionTitle)
                    : (root.session !== null && root.session.title !== ""
                            && root.session.title !== "Terminal"
                        ? qsTr("终端 · %1").arg(root.session.title)
                        : (root.session !== null
                            ? qsTr("终端 · 已退出 %1").arg(root.session.exitCode)
                            : qsTr("终端")))
                color: Theme.terminalInk
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideRight
                textFormat: Text.PlainText
                width: Math.min(implicitWidth, header.width * 0.4)
            }

            /// §11.6's 工作目录: the shell's OSC report when its integration
            /// is installed, the spawn folder otherwise. The dim mono run says
            /// it without competing with the session name; the tooltip says
            /// which of the two it is.
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, parent.width - x - 24)
                text: root.workingDirectory
                color: Theme.terminalInk
                opacity: root.directoryReported ? 0.85 : 0.55
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideMiddle
                textFormat: Text.PlainText

                ToolTip {
                    text: root.directoryReported
                        ? root.workingDirectory
                        : qsTr("%1（shell 未上报目录；安装 Assets/terminal-integration 后跟随）")
                            .arg(root.workingDirectory)
                }
            }
        }

        /// One pill per session: the running shell's name, or its exit
        /// state. The number keeps pills distinguishable before the shell
        /// has drawn a title.
        Row {
            id: sessionStrip
            anchors.right: actionsRow.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4

            Repeater {
                model: root.sessions.length

                Rectangle {
                    id: pill
                    required property int modelData
                    readonly property int index: pill.modelData
                    readonly property var named: root.sessions[pill.index]
                    readonly property bool current: pill.index === root.activeIndex
                    width: pillRow.implicitWidth + 16
                    height: 22
                    radius: Theme.radiusSm
                    color: pillMouse.containsMouse ? Theme.hover
                        : (pill.current ? Theme.selection : "transparent")

                    Row {
                        id: pillRow
                        anchors.centerIn: parent
                        spacing: 4

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: (pill.index + 1) + " "
                                + (pill.named.running ? root.shellOf(pill.named) : qsTr("已退出"))
                            color: pill.current ? Theme.terminalInkStrong : Theme.terminalInk
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeCaption
                            textFormat: Text.PlainText
                        }
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("终端 %1").arg(pill.index + 1)
                    Accessible.onPressAction: root.activeIndex = pill.index

                    MouseArea {
                        id: pillMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            root.activeIndex = pill.index
                            Qt.callLater(root.focusTerminal)
                        }
                    }
                }
            }

            /// New session. Opens in the active tab's folder.
            Rectangle {
                width: 24
                height: 22
                radius: Theme.radiusFull
                color: newMouse.containsMouse ? Theme.hover : "transparent"

                Glyph {
                    anchors.centerIn: parent
                    width: 12
                    height: 12
                    kind: "plus"
                    tint: newMouse.containsMouse
                        ? Theme.terminalInkStrong : Theme.terminalInk
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("新建终端")
                Accessible.onPressAction: root.newSession()

                MouseArea {
                    id: newMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.newSession()
                }
            }

            /// Closes the active session; the others keep running.
            Rectangle {
                width: 24
                height: 22
                radius: Theme.radiusFull
                visible: root.sessions.length > 0
                color: closeSessionMouse.containsMouse ? Theme.hover : "transparent"

                Glyph {
                    anchors.centerIn: parent
                    width: 12
                    height: 12
                    kind: "close"
                    tint: closeSessionMouse.containsMouse
                        ? Theme.terminalInkStrong : Theme.terminalInk
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("关闭此终端")
                Accessible.onPressAction: root.closeSession(root.activeIndex)

                MouseArea {
                    id: closeSessionMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.closeSession(root.activeIndex)
                }
            }
        }

        Row {
            id: actionsRow
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8

            /// 清屏: one command, spelled for the shell that is running. It is a
            /// shell command rather than a grid wipe, because the scrollback the
            /// user can scroll back into is the shell's, not this panel's.
            Rectangle {
                width: 46
                height: 22
                radius: Theme.radiusSm
                visible: root.running
                color: clearMouse.containsMouse ? Theme.hover : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: qsTr("清屏")
                    color: clearMouse.containsMouse
                        ? Theme.terminalInkStrong : Theme.terminalInk
                    font.pixelSize: Theme.typeCaption
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

            /// 跳转: the deliberate `cd` the user asked for (G-9).
            Rectangle {
                width: 46
                height: 22
                radius: Theme.radiusSm
                visible: root.running
                color: jumpMouse.containsMouse ? Theme.hover : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: qsTr("跳转")
                    color: jumpMouse.containsMouse
                        ? Theme.terminalInkStrong : Theme.terminalInk
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("跳转")
                Accessible.onPressAction: root.jumpToFolder()

                MouseArea {
                    id: jumpMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.jumpToFolder()
                }
            }


            /// 重新启动: the active session died; start it again in the
            /// folder it was born in.
            Rectangle {
                width: 62
                height: 22
                radius: Theme.radiusSm
                visible: root.sessions.length > 0 && !root.running
                color: restartMouse.containsMouse ? Theme.hover : "transparent"

                Text {
                    anchors.centerIn: parent
                    text: qsTr("重新启动")
                    color: restartMouse.containsMouse
                        ? Theme.terminalInkStrong : Theme.terminalInk
                    font.pixelSize: Theme.typeCaption
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

            /// Hides the panel; sessions keep running. A chevron, not a
            /// second ✕: the strip already has one for ending a session, and
            /// two identical crosses two centimetres apart read as a mistake.
            Item {
                width: 24
                height: 22

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("隐藏终端")
                Accessible.onPressAction: root.hideRequested()

                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusFull
                    color: closeMouse.containsMouse ? Theme.hover : "transparent"
                }

                Glyph {
                    anchors.centerIn: parent
                    width: 14
                    height: 14
                    kind: "chevronDown"
                    tint: closeMouse.containsMouse ? Theme.terminalInkStrong : Theme.terminalInk
                }

                MouseArea {
                    id: closeMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.hideRequested()
                }
            }
        }

        /// The Ctrl+F find bar: typing sets the needle, Enter steps to the
        /// next match, Esc closes and clears. It covers the right half of
        /// the header while open; the sessions and actions stay one
        /// key press away.
        Rectangle {
            visible: root.searchOpen
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: barRow.implicitWidth + 18
            height: 24
            radius: Theme.radiusSm
            color: Theme.terminalHeader
            border.width: 2
            border.color: searchField.activeFocus ? Theme.accent : Theme.hairline
            z: 1

            Row {
                id: barRow
                anchors.centerIn: parent
                spacing: 6

                TextInput {
                    id: searchField
                    anchors.verticalCenter: parent.verticalCenter
                    width: 140
                    color: Theme.terminalInk
                    selectionColor: Theme.accent
                    selectedTextColor: Theme.terminalInkStrong
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeCaption
                    focus: root.searchOpen

                    onAccepted: {
                        if (root.session !== null && text !== "") {
                            root.session.setSearch(text)
                            root.session.searchNext()
                        }
                    }
                    onTextChanged: {
                        if (root.session !== null)
                            root.session.setSearch(text)
                    }
                    Keys.onEscapePressed: root.closeSearch()

                    Accessible.role: Accessible.EditableText
                    Accessible.name: qsTr("在终端中搜索")
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: root.session !== null && root.session.searchTotal > 0
                    text: root.session.searchIndex + "/" + root.session.searchTotal
                    color: Theme.terminalInk
                    opacity: 0.7
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: root.session !== null && root.session.searchTotal === 0
                        && searchField.text !== ""
                    text: qsTr("无匹配")
                    color: Theme.terminalInk
                    opacity: 0.7
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeCaption
                    textFormat: Text.PlainText
                }

                Rectangle {
                    width: 22
                    height: Theme.controlXs
                    radius: Theme.radiusFull
                    color: prevMouse.containsMouse ? Theme.hover : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "arrowUp"
                        tint: prevMouse.containsMouse
                            ? Theme.terminalInkStrong : Theme.terminalInk
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("上一个匹配")
                    Accessible.onPressAction: if (root.session !== null) root.session.searchPrev()

                    MouseArea {
                        id: prevMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: if (root.session !== null) root.session.searchPrev()
                    }
                }

                Rectangle {
                    width: 22
                    height: Theme.controlXs
                    radius: Theme.radiusFull
                    color: nextMouse.containsMouse ? Theme.hover : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "arrowDown"
                        tint: nextMouse.containsMouse
                            ? Theme.terminalInkStrong : Theme.terminalInk
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("下一个匹配")
                    Accessible.onPressAction: if (root.session !== null) root.session.searchNext()

                    MouseArea {
                        id: nextMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: if (root.session !== null) root.session.searchNext()
                    }
                }

                Rectangle {
                    width: 22
                    height: Theme.controlXs
                    radius: Theme.radiusFull
                    color: barCloseMouse.containsMouse ? Theme.hover : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "close"
                        tint: barCloseMouse.containsMouse
                            ? Theme.terminalInkStrong : Theme.terminalInk
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("关闭搜索")
                    Accessible.onPressAction: root.closeSearch()

                    MouseArea {
                        id: barCloseMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.closeSearch()
                    }
                }
            }
        }
    }

    /// The top edge is the height grip: a 6 px strip over the header's own
    /// top padding, so no header control loses its hit area. The range runs
    /// from a header plus a few rows up to the column's top boundary; the
    /// preference is written once on release, not per pointer pixel.
    Item {
        id: heightHandle
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 6
        z: 10
        visible: root.active

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            /// A seam at rest, the accent while the edge is grabbed: without
            /// it the panel's top edge was only a jump in tone where the list
            /// happened to be cut.
            color: heightMouse.containsMouse || heightMouse.pressed
                ? Theme.accent : Theme.hairline
        }

        MouseArea {
            id: heightMouse
            Accessible.ignored: true
            anchors.fill: parent
            hoverEnabled: true
            preventStealing: true
            cursorShape: Qt.SizeVerCursor
            property real startY: 0
            property int startHeight: 0
            onPressed: function (mouse) {
                startY = mapToItem(null, mouse.x, mouse.y).y
                startHeight = root.height
            }
            onPositionChanged: function (mouse) {
                if (!pressed)
                    return
                const dy = mapToItem(null, mouse.x, mouse.y).y - startY
                root.liveHeight = Math.max(root.heightFloor,
                    Math.min(root.heightLimit, startHeight - dy))
            }
            onReleased: {
                if (root.liveHeight >= 0) {
                    ExplorerPrefs.setNumber("terminalHeight", root.liveHeight)
                    root.liveHeight = -1
                }
            }
            onCanceled: root.liveHeight = -1
            onDoubleClicked: ExplorerPrefs.setNumber("terminalHeight",
                                                     Theme.terminalHeight)
        }
    }

    TerminalItem {
        id: view
        // TerminalItem is a C++ type; qmllint's model of it stops at the
        // registered metatypes, so its signals read as missing too.
        // qmllint disable unqualified
        onSearchRequested: root.openSearch()
        /// Ctrl+Click on a path or URL. Resolved against the session's
        /// reported directory and handed to the OS-open path the rest of
        /// the shell uses; URLs go straight to the browser. The
        /// `:line:col` suffix is parsed but not used — the OS handler
        /// cannot jump to a line.
        onLinkActivated: function (target) {
            root.openLink(target)
        }

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.topMargin: 10
        anchors.bottomMargin: 8
        /// The native terminal consumes Tab — it has to, Tab is completion
        /// inside a shell — so it may only be part of the focus chain while the
        /// panel is on screen. Hidden, it would be a stop that swallows the key
        /// and leaves focus nowhere.
        activeFocusOnTab: root.active
        session: root.session
        /// Selections copy themselves when the setting says so. A C++
        /// property, so qmllint cannot see it.
        // qmllint disable missing-property
        copyOnSelect: ExplorerPrefs.copyOnSelect
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

}
