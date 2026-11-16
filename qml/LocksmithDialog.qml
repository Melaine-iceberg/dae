pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// File Locksmith: which processes have the selection open.
//
// The probe lives in `dae_core::fs::occupancy` and answers asynchronously, so
// the dialog opens at once and fills in. `request` is the JSON of the paths it
// asked about; a result for another dialog (or an older query in this one) is
// ignored instead of repainting the list under the user.
Dialog {
    id: root

    /// The paths the dialog is about, in the order they were selected.
    property var paths: []
    /// JSON spelling of `paths`, echoed by `fileHoldersFound`.
    property string request: ""
    property var holders: []
    property int scanned: 0
    property bool truncated: false
    property bool loading: false
    /// A failed query or end-task, shown above the list.
    property string message: ""
    /// Pid whose "结束进程" click is in flight.
    property int busyPid: -1
    /// Pid whose button changed to "确认结束".
    property int confirmingPid: -1

    signal closed()

    heading: qsTr("查看占用进程")
    panelWidth: 560
    bodyHeight: 268

    onCancelRequested: root.close()

    function open(paths) {
        root.paths = (paths || []).slice()
        root.request = JSON.stringify(root.paths)
        root.holders = []
        root.scanned = 0
        root.truncated = false
        root.message = ""
        root.busyPid = -1
        root.confirmingPid = -1
        root.loading = true
        root.show()
        Workspace.fileHolders(root.paths)
    }

    function close() {
        root.visible = false
        root.closed()
    }

    function reload() {
        if (root.loading)
            return
        root.loading = true
        root.message = ""
        root.confirmingPid = -1
        Workspace.fileHolders(root.paths)
    }

    function applyReport(request, report, error) {
        if (request !== root.request)
            return
        root.loading = false
        root.busyPid = -1
        if (error !== "") {
            root.holders = []
            root.message = qsTr("无法检查所选项目：%1").arg(error)
            return
        }
        try {
            const parsed = JSON.parse(report)
            root.holders = parsed.holders || []
            root.scanned = parsed.scanned || 0
            root.truncated = parsed.truncated === true
        } catch (parseError) {
            root.holders = []
            root.message = qsTr("无法读取查询结果")
        }
    }

    /// The first click arms, the second ends. Ending a process is destructive
    /// and unsaved work is the usual casualty; a single mis-aimed click on a
    /// row's right edge should not do it.
    function requestEnd(holder) {
        if (root.busyPid >= 0)
            return
        if (root.confirmingPid !== holder.pid) {
            root.confirmingPid = holder.pid
            root.message = ""
            return
        }
        root.confirmingPid = -1
        root.busyPid = holder.pid
        Workspace.endProcess(holder.pid)
    }

    function copyDetails() {
        const lines = []
        for (let index = 0; index < root.holders.length; index += 1) {
            const holder = root.holders[index]
            lines.push(holder.name + " (" + holder.pid + ")")
        }
        if (lines.length === 0)
            return
        if (Workspace.copyText(lines.join("\n")))
            root.message = qsTr("已复制进程列表")
        else
            root.message = qsTr("无法写入剪贴板")
    }

    Connections {
        target: Workspace

        function onFileHoldersFound(request, report, error) {
            root.applyReport(request, report, error)
        }

        function onProcessEnded(pid, ok, message) {
            if (pid !== root.busyPid)
                return
            root.busyPid = -1
            if (!ok) {
                root.message = qsTr("无法结束进程：%1").arg(message)
                return
            }
            // Re-ask rather than removing the row locally: another process may
            // hold the same file, and the list should be the current answer.
            root.reload()
        }
    }

    readonly property string targetLabel: {
        if (root.paths.length === 0)
            return ""
        const first = String(root.paths[0])
        const parts = first.split(/[\\/]/).filter(part => part.length > 0)
        const name = parts.length > 0 ? parts[parts.length - 1] : first
        return root.paths.length === 1
            ? name
            : qsTr("%1 等 %2 个项目").arg(name).arg(root.paths.length)
    }

    readonly property string statusText: {
        if (root.loading)
            return qsTr("正在检查…")
        if (root.holders.length === 0)
            return qsTr("没有找到占用所选项目的进程。")
        return qsTr("%1 个占用进程").arg(root.holders.length)
    }

    readonly property string scanHint: {
        if (root.loading || root.holders.length === 0)
            return ""
        let text = qsTr("已检查 %1 个文件").arg(root.scanned)
        if (root.truncated)
            text += qsTr("；目录过大，仅检查了前 %1 个文件").arg(root.scanned)
        return text
    }

    Column {
        width: parent.width
        spacing: 6

        Text {
            width: parent.width
            text: root.targetLabel
            color: Theme.ink
            font.pixelSize: Theme.typeBody
            elide: Text.ElideMiddle

            Accessible.role: Accessible.StaticText
            Accessible.name: root.targetLabel
        }

        Text {
            width: parent.width
            text: root.statusText
            color: root.holders.length === 0 && !root.loading
                ? Theme.inkSecondary : Theme.inkTertiary
            font.pixelSize: Theme.typeCaption
            wrapMode: Text.Wrap
        }

        Text {
            width: parent.width
            visible: root.message !== ""
            text: root.message
            color: Theme.danger
            font.pixelSize: Theme.typeMicro
            wrapMode: Text.Wrap
        }
    }

    ListView {
        id: holderList

        anchors.top: parent.top
        anchors.topMargin: 72
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        clip: true
        model: root.holders
        boundsBehavior: Flickable.StopAtBounds
        spacing: 2
        visible: root.holders.length > 0

        delegate: Item {
            id: holderRow

            required property var modelData

            width: holderList.width
            height: 44

            readonly property int pid: Number(modelData.pid)
            readonly property string processName: String(modelData.name)

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusSm
                color: rowHover.containsMouse ? Theme.hover : "transparent"
            }

            Text {
                id: nameText

                anchors.left: parent.left
                anchors.leftMargin: 8
                anchors.right: pidText.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: holderRow.processName
                color: Theme.ink
                font.pixelSize: Theme.typeBody
                elide: Text.ElideRight
            }

            Text {
                id: pidText

                anchors.right: endButton.left
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                text: "PID " + holderRow.pid
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
            }

            DialogButton {
                id: endButton

                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                label: root.confirmingPid === holderRow.pid
                    ? qsTr("确认结束") : qsTr("结束进程")
                tone: root.confirmingPid === holderRow.pid ? "danger" : "quiet"
                enabled: root.busyPid < 0
                onClicked: root.requestEnd(holderRow.modelData)
            }

            MouseArea {
                id: rowHover

                anchors.fill: parent
                anchors.rightMargin: endButton.width + 8
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
            }
        }
    }

    Text {
        anchors.top: parent.top
        anchors.topMargin: 72
        width: parent.width
        visible: !root.loading && root.holders.length === 0
        text: qsTr("其他用户运行的进程可能不在列表中。")
        color: Theme.inkTertiary
        font.pixelSize: Theme.typeMicro
        wrapMode: Text.Wrap
    }

    footer: Component {
        Item {
            implicitHeight: Theme.controlMd

            DialogButton {
                id: closeButton

                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("关闭")
                tone: "primary"
                onClicked: root.close()
            }

            DialogButton {
                id: copyButton

                anchors.right: closeButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("复制信息")
                tone: "quiet"
                enabled: root.holders.length > 0
                onClicked: root.copyDetails()
            }

            DialogButton {
                anchors.right: copyButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("重新检查")
                tone: "quiet"
                enabled: !root.loading
                onClicked: root.reload()
            }
        }
    }
}
