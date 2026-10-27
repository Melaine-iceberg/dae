pragma ComponentBehavior: Bound
import QtQuick

// The branch switch and remote actions for the repository shown in the
// active pane.
//
// The status chip is the only door: it opens above itself, orders local
// branches with the current one from the core, and keeps the create form in
// place so a rejected name can be corrected without losing the panel. Network
// actions close the panel first; their spinner lives on the status chip.
Item {
    id: root

    property var repo: null
    property bool dark: true
    property color accent: "#2f6fdf"
    property real openX: 0
    property real openY: 0

    property bool creating: false
    property bool createPending: false
    property bool createCheckout: true
    property string createError: ""

    visible: false
    focus: visible
    z: 1002

    readonly property color panelColor: dark ? "#202329" : "#ffffff"
    readonly property color borderColor: dark ? "#33ffffff" : "#18000000"
    readonly property color textColor: dark ? "#e3e7ee" : "#24292f"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color fieldColor: dark ? "#171a1f" : "#f3f5f8"
    readonly property color hoverColor: dark ? "#1affffff" : "#0d000000"
    readonly property bool hasRepo: repo !== null && repo.gitRoot !== ""
    readonly property bool canNetwork: hasRepo && repo.gitRemotes.length > 0
        && !repo.gitBusy
    readonly property bool busy: hasRepo && repo.gitBusy
    readonly property real listHeight: Math.min(260, rowsHeight())

    function currentBranch() {
        return hasRepo ? repo.gitBranch : ""
    }

    function branchHint() {
        if (!hasRepo)
            return ""
        if (repo.gitDetached)
            return qsTr("分离 HEAD")
        if (repo.gitRemotes.length === 0)
            return qsTr("无远程")
        if (!repo.gitHasUpstream)
            return qsTr("未配置上游")
        const parts = []
        if (repo.gitAhead > 0)
            parts.push("↑" + repo.gitAhead)
        if (repo.gitBehind > 0)
            parts.push("↓" + repo.gitBehind)
        return parts.join(" ")
    }

    /// One flat list keeps section headers, branches and the empty state in a
    /// single ListView, so a repository with a thousand branches still creates
    /// delegates only for the visible window.
    function branchRows() {
        const rows = []
        if (!hasRepo)
            return rows
        rows.push({"kind": "section", "key": "local-section", "text": qsTr("本地分支")})
        const local = repo.gitLocalBranches
        if (local.length === 0) {
            rows.push({"kind": "empty", "key": "local-empty", "text": qsTr("没有本地分支")})
        } else {
            for (let index = 0; index < local.length; index += 1) {
                const name = local[index]
                rows.push({
                    "kind": "branch",
                    "key": "local-" + name,
                    "name": name,
                    "remote": false,
                    "current": name === repo.gitCurrentBranch
                })
            }
        }
        const remote = repo.gitRemoteBranches
        if (remote.length > 0) {
            rows.push({"kind": "section", "key": "remote-section", "text": qsTr("远程分支")})
            for (let index = 0; index < remote.length; index += 1) {
                const name = remote[index]
                rows.push({
                    "kind": "branch",
                    "key": "remote-" + name,
                    "name": name,
                    "remote": true,
                    "current": false
                })
            }
        }
        return rows
    }

    function rowsHeight() {
        const rows = branchRows()
        let height = 0
        for (let index = 0; index < rows.length; index += 1)
            height += rows[index].kind === "branch" ? 26 : 22
        return height
    }

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.creating = false
        root.createPending = false
        root.createCheckout = true
        root.createError = ""
        branchInput.text = ""
        root.visible = true
        root.forceActiveFocus()
        if (root.repo !== null)
            root.repo.refreshGit()
    }

    function close() {
        root.visible = false
        root.creating = false
        root.createPending = false
        root.createError = ""
    }

    function beginCreate() {
        root.creating = true
        root.createPending = false
        root.createError = ""
        branchInput.text = ""
        branchInput.forceActiveFocus()
    }

    function submitCreate() {
        if (root.repo === null || root.createPending)
            return
        const name = branchInput.text.trim()
        if (name === "") {
            root.createError = qsTr("请输入分支名")
            return
        }
        root.createError = ""
        root.createPending = true
        root.repo.gitCreateBranch(name, root.createCheckout)
    }

    function runAction(action) {
        if (root.repo === null || root.repo.gitBusy)
            return
        if (action === "create") {
            root.beginCreate()
            return
        }
        const target = root.repo
        root.close()
        if (action === "fetch")
            target.gitFetch()
        else if (action === "pull")
            target.gitPull()
        else if (action === "push")
            target.gitPush()
        else if (action === "sync")
            target.gitSync()
    }

    function checkout(name, remote) {
        if (root.repo === null || root.repo.gitBusy)
            return
        const target = root.repo
        root.close()
        target.gitCheckoutBranch(name, remote)
    }

    onRepoChanged: {
        if (root.visible)
            root.close()
    }

    Keys.onEscapePressed: {
        if (root.creating) {
            root.creating = false
            root.createPending = false
            root.createError = ""
        } else {
            root.close()
        }
    }

    Connections {
        target: root.repo

        function onGitOperationFinished(operation, message) {
            if (operation !== "create" || !root.createPending)
                return
            root.createPending = false
            root.creating = false
            branchInput.text = ""
        }

        function onGitOperationFailed(operation, message) {
            if (operation !== "create")
                return
            root.createPending = false
            root.createError = message
        }
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onClicked: root.close()
        onWheel: root.close()
    }

    Rectangle {
        id: panel

        width: 300
        height: Math.min(root.height - 16, content.implicitHeight + 12)
        x: Math.max(8, Math.min(root.openX, root.width - width - 8))
        y: Math.max(8, root.openY - height - 6)
        radius: 9
        color: root.panelColor
        border.width: 1
        border.color: root.borderColor
        clip: true

        // A click on blank panel space must not reach the backdrop. Children
        // declared later sit above this and keep their own mouse handling.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
            onWheel: wheel.accepted = true
        }

        Column {
            id: content
            width: parent.width

            Item {
                width: parent.width
                height: 44

                Text {
                    x: 10
                    y: 5
                    width: Math.max(0, parent.width - hint.width - 30)
                    text: "Git · " + root.currentBranch()
                    color: root.textColor
                    font.pixelSize: 11
                    font.weight: Font.DemiBold
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                }

                Text {
                    id: hint
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    y: 6
                    text: root.branchHint()
                    color: root.mutedColor
                    font.pixelSize: 9
                    textFormat: Text.PlainText
                }

                Text {
                    x: 10
                    y: 25
                    width: parent.width - 20
                    text: root.hasRepo ? root.repo.gitRoot : ""
                    color: root.mutedColor
                    font.family: "Cascadia Mono"
                    font.pixelSize: 9
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: root.borderColor
            }

            Item {
                width: parent.width
                height: 34

                Repeater {
                    model: [
                        {"label": qsTr("新建"), "action": "create"},
                        {"label": qsTr("获取"), "action": "fetch"},
                        {"label": qsTr("拉取"), "action": "pull"},
                        {"label": qsTr("推送"), "action": "push"},
                        {"label": qsTr("同步"), "action": "sync"}
                    ]

                    delegate: Rectangle {
                        id: actionButton
                        required property var modelData
                        required property int index

                        x: 8 + index * 57
                        y: 5
                        width: 53
                        height: 24
                        radius: 4
                        readonly property bool actionEnabled: !root.busy
                            && (modelData.action === "create" || root.canNetwork)
                        color: actionMouse.containsMouse && actionEnabled
                            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.18)
                            : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.08)
                        border.width: 1
                        border.color: Qt.rgba(root.accent.r, root.accent.g,
                            root.accent.b, actionEnabled ? 0.35 : 0.12)

                        Accessible.role: Accessible.Button
                        Accessible.name: modelData.label
                        Accessible.onPressAction: root.runAction(modelData.action)

                        Text {
                            anchors.centerIn: parent
                            text: modelData.label
                            color: root.textColor
                            font.pixelSize: 10
                            textFormat: Text.PlainText
                            opacity: actionButton.actionEnabled ? 1 : 0.35
                        }

                        MouseArea {
                            id: actionMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            enabled: actionButton.actionEnabled
                            onClicked: root.runAction(modelData.action)
                        }
                    }
                }
            }

            Item {
                id: createPanel
                width: parent.width
                height: root.creating ? (root.createError !== "" ? 82 : 64) : 0
                visible: root.creating
                clip: true

                Rectangle {
                    x: 8
                    y: 5
                    width: parent.width - 16 - 60
                    height: 24
                    radius: 4
                    color: root.fieldColor
                    border.width: 1
                    border.color: branchInput.activeFocus
                        ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.65)
                        : root.borderColor

                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 7
                        anchors.verticalCenter: parent.verticalCenter
                        visible: branchInput.text === ""
                        text: qsTr("分支名称")
                        color: root.mutedColor
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }

                    TextInput {
                        id: branchInput
                        anchors.fill: parent
                        anchors.leftMargin: 7
                        anchors.rightMargin: 7
                        verticalAlignment: TextInput.AlignVCenter
                        color: root.textColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 11
                        selectByMouse: true
                        focus: root.creating
                        onTextChanged: {
                            if (root.createError !== "")
                                root.createError = ""
                        }
                        Keys.onReturnPressed: root.submitCreate()
                        Keys.onEscapePressed: {
                            root.creating = false
                            root.createPending = false
                            root.createError = ""
                        }
                    }
                }

                Rectangle {
                    x: parent.width - 8 - 52
                    y: 5
                    width: 52
                    height: 24
                    radius: 4
                    color: root.createPending || branchInput.text.trim() === ""
                        ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.10)
                        : Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.22)
                    border.width: 1
                    border.color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.42)

                    Text {
                        anchors.centerIn: parent
                        text: root.createPending ? qsTr("创建中…") : qsTr("创建")
                        color: root.textColor
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !root.createPending && branchInput.text.trim() !== ""
                        onClicked: root.submitCreate()
                    }
                }

                Item {
                    x: 8
                    y: 34
                    width: 110
                    height: 14

                    Rectangle {
                        x: 0
                        width: 12
                        height: 12
                        radius: 3
                        anchors.verticalCenter: parent.verticalCenter
                        color: root.createCheckout
                            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.18)
                            : "transparent"
                        border.width: 1
                        border.color: root.createCheckout ? root.accent : root.borderColor

                        Rectangle {
                            anchors.centerIn: parent
                            width: 6
                            height: 6
                            radius: 1
                            visible: root.createCheckout
                            color: root.accent
                        }
                    }

                    Text {
                        x: 18
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("创建后切换")
                        color: root.mutedColor
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !root.createPending
                        onClicked: root.createCheckout = !root.createCheckout
                    }
                }

                Text {
                    x: 8
                    y: 55
                    width: parent.width - 16
                    visible: root.createError !== ""
                    text: root.createError
                    color: root.dark ? "#f07f78" : "#c0392b"
                    font.pixelSize: 10
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }
            }

            ListView {
                id: branchList
                width: parent.width
                height: root.listHeight
                visible: height > 0
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                model: root.branchRows()

                delegate: Item {
                    id: branchRow
                    required property var modelData
                    width: branchList.width
                    height: modelData.kind === "branch" ? 26 : 22

                    Rectangle {
                        anchors.fill: parent
                        visible: branchRow.modelData.kind === "branch"
                        color: branchMouse.containsMouse ? root.hoverColor : "transparent"
                    }

                    Text {
                        x: 10
                        anchors.verticalCenter: parent.verticalCenter
                        visible: branchRow.modelData.kind === "section"
                        text: branchRow.modelData.kind === "section"
                            ? branchRow.modelData.text : ""
                        color: root.mutedColor
                        font.pixelSize: 9
                        font.weight: Font.DemiBold
                        textFormat: Text.PlainText
                    }

                    Text {
                        x: 10
                        anchors.verticalCenter: parent.verticalCenter
                        visible: branchRow.modelData.kind === "empty"
                        text: branchRow.modelData.kind === "empty"
                            ? branchRow.modelData.text : ""
                        color: root.mutedColor
                        font.pixelSize: 10
                        textFormat: Text.PlainText
                    }

                    Text {
                        x: 9
                        anchors.verticalCenter: parent.verticalCenter
                        visible: branchRow.modelData.kind === "branch"
                            && branchRow.modelData.current
                        text: "✓"
                        color: root.accent
                        font.pixelSize: 11
                        textFormat: Text.PlainText
                    }

                    Text {
                        id: branchName
                        x: 25
                        width: Math.max(0, parent.width - 34)
                        anchors.verticalCenter: parent.verticalCenter
                        visible: branchRow.modelData.kind === "branch"
                        text: branchRow.modelData.kind === "branch"
                            ? branchRow.modelData.name : ""
                        color: branchRow.modelData.current ? root.accent : root.textColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 10
                        font.weight: branchRow.modelData.current ? Font.DemiBold : Font.Normal
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                    }

                    MouseArea {
                        id: branchMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: branchRow.modelData.kind === "branch" && !root.busy
                        onClicked: root.checkout(branchRow.modelData.name,
                            branchRow.modelData.remote)
                    }
                }
            }

            Item {
                width: parent.width
                height: 7
            }
        }
    }
}
