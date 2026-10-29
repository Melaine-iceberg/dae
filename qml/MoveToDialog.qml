pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "DialogFocus.js" as DialogFocus

// "移动到": pick a destination folder, then hand the selection to the same
// transfer queue a paste uses.
//
// The folder list is a `DirectoryListModel`, the same model the explorer reads
// a directory with, filtered to directories in the delegate. That keeps one
// reader for local and network paths and one scroll behaviour; the dialog only
// adds the destination field and the confirm step.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    /// The paths being moved.
    property var paths: []
    /// The folder the list is showing.
    property string current: ""
    /// The folder the user picked, empty when they typed nothing else.
    property string picked: ""
    /// The directory the menu was opened in, shown as one-click target.
    property string origin: ""

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color fieldColor: Theme.field
    readonly property color hairline: Theme.hairline

    signal closed()

    visible: false
    z: 1007
    focus: visible

    Accessible.role: Accessible.Dialog
    Accessible.name: qsTr("移动到")

    function open(destination, moving) {
        root.paths = (moving || []).slice()
        root.origin = destination
        root.picked = ""
        root.browse(destination)
        root.visible = true
        root.forceActiveFocus()
        Qt.callLater(function () {
            if (root.visible)
                pathInput.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
        root.closed()
    }

    function browse(path) {
        const clean = root.normalize(path)
        if (clean === "")
            return
        root.current = clean
        root.picked = clean
        pathInput.text = clean
        browser.load(clean)
    }

    function normalize(path) {
        let next = (path || "").trim()
        if (next === "")
            return ""
        // Keep the separator after a drive letter / root: "C:" alone is the
        // drive's working directory, not its root.
        while (next.length > 1 && /[\\/]$/.test(next)
               && !/^[A-Za-z]:[\\/]$/.test(next))
            next = next.slice(0, -1)
        return next
    }

    function parentOf(path) {
        if (/^[A-Za-z]:[\\/]$/.test(path))
            return path
        if (path === "/" || path === "")
            return path
        const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"))
        if (index <= 0)
            return path
        if (index === 2 && /^[A-Za-z]:/.test(path))
            return path.slice(0, 3)
        return path.slice(0, index)
    }

    function accept() {
        const target = root.normalize(root.picked !== "" ? root.picked : pathInput.text)
        if (target === "" || target === root.origin || root.paths.length === 0) {
            root.close()
            return
        }
        Workspace.transferEntries(root.paths, target, true)
        root.close()
    }

    Keys.onEscapePressed: root.close()
    Keys.onTabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, true)
    }
    Keys.onBacktabPressed: function (event) {
        event.accepted = DialogFocus.cycle(root, false)
    }

    DirectoryListModel {
        id: browser
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.close()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(520, root.width - 40)
        height: Math.min(480, root.height - 48)
        radius: Theme.radiusLg
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: layout
            anchors.fill: parent
            anchors.margins: 18
            spacing: 0

            DialogHeader {
                width: parent.width
                title: qsTr("移动到")
                subtitle: root.paths.length + qsTr(" 项将移动到下面的文件夹。")
                onCloseRequested: root.close()
            }

            Item { width: 1; height: 12 }

            Row {
                width: parent.width
                height: 30
                spacing: 8

                Rectangle {
                    width: 26
                    height: 28
                    radius: Theme.radiusSm
                    color: upMouse.containsMouse
                        ? Theme.fill : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: "\u2191"
                        color: root.textColor
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: upMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.browse(root.parentOf(root.current))

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("上一级")
                    }
                }

                Rectangle {
                    width: parent.width - 26 - 8
                    height: 28
                    radius: Theme.radiusSm
                    color: root.fieldColor
                    border.width: 1
                    border.color: pathInput.activeFocus
                        ? Theme.focusRing
                        : root.hairline

                    TextInput {
                        id: pathInput
                        anchors.fill: parent
                        anchors.leftMargin: 9
                        anchors.rightMargin: 9
                        verticalAlignment: TextInput.AlignVCenter
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: Theme.onInteractive
                        font.pixelSize: 12
                        selectByMouse: true
                        clip: true

                        Accessible.role: Accessible.EditableText
                        Accessible.name: qsTr("目标路径")

                        onAccepted: root.browse(text)
                    }
                }
            }

            Item { width: 1; height: 8 }

            Row {
                width: parent.width
                height: 24
                spacing: 8

                QuickButton {
                    label: qsTr("主目录")
                    path: Workspace.homeDirectory
                }

                QuickButton {
                    label: qsTr("当前文件夹")
                    path: root.origin
                }
            }

            Item { width: 1; height: 10 }

            Rectangle {
                width: parent.width
                height: Math.max(120, layout.height - y - 46)
                radius: Theme.radiusSm
                color: root.fieldColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Text {
                    anchors.centerIn: parent
                    visible: !browser.finished
                    text: qsTr("读取中…")
                    color: root.mutedColor
                    font.pixelSize: 12
                }

                ListView {
                    id: folderList
                    anchors.fill: parent
                    anchors.margins: 4
                    model: browser
                    clip: true
                    spacing: 1
                    boundsBehavior: Flickable.StopAtBounds

                    delegate: Item {
                        id: folderRow
                        required property string fileName
                        required property string filePath
                        required property bool isDir

                        // Files are rows in the model but never in this list;
                        // collapsing them keeps the model's own ordering.
                        width: folderList.width
                        height: folderRow.isDir ? 26 : 0
                        visible: folderRow.isDir

                        Rectangle {
                            anchors.fill: parent
                            radius: Theme.radiusSm
                            color: root.picked === folderRow.filePath
                                ? Theme.selection
                                : (folderMouse.containsMouse
                                    ? Theme.hover
                                    : "transparent")
                        }

                        Text {
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            anchors.right: parent.right
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            text: folderRow.fileName
                            color: root.textColor
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }

                        MouseArea {
                            id: folderMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            acceptedButtons: Qt.LeftButton
                            onClicked: {
                                root.picked = folderRow.filePath
                                pathInput.text = folderRow.filePath
                            }
                            onDoubleClicked: root.browse(folderRow.filePath)
                        }
                    }
                }
            }

            Item { width: 1; height: 10 }

            Row {
                width: parent.width
                height: 30
                spacing: 8

                Item {
                    width: Math.max(0, parent.width - 76 - 84 - 16)
                    height: 1
                }

                Rectangle {
                    width: 76
                    height: 28
                    anchors.verticalCenter: parent.verticalCenter
                    radius: Theme.radiusSm
                    color: cancelMouse.containsMouse
                        ? Theme.fill : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("取消")
                        color: Theme.inkMuted
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: cancelMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.close()
                    }
                }

                Rectangle {
                    width: 84
                    height: 28
                    anchors.verticalCenter: parent.verticalCenter
                    radius: Theme.radiusSm
                    color: moveMouse.containsMouse ? Theme.interactiveHover : Theme.interactive

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("移动")
                        color: Theme.onInteractive
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: moveMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.accept()
                    }
                }
            }
        }
    }

    component QuickButton: Rectangle {
        id: quick

        property string label: ""
        property string path: ""

        width: quickLabel.width + 20
        height: 22
        radius: Theme.radiusSm
        color: quickMouse.containsMouse
            ? Theme.hover : "transparent"
        border.width: 1
        border.color: root.hairline
        opacity: quick.path !== "" ? 1 : 0

        Text {
            id: quickLabel
            anchors.centerIn: parent
            text: quick.label
            color: root.textColor
            font.pixelSize: 11
        }

        MouseArea {
            id: quickMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.browse(quick.path)
        }
    }
}
