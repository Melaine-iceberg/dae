import QtQuick
import dae 1.0

// Update status and the single "download and install" action. The startup
// check is silent; this dialog is the place where a failure becomes visible
// and where the user can check again.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline
    readonly property color dangerColor: Theme.danger
    readonly property bool busy: Updater.checking || Updater.downloading
        || Updater.restarting

    visible: false
    z: 1005
    focus: visible

    signal canceled()

    function open() {
        root.visible = true
        if (!Updater.checking && !Updater.downloading && !Updater.available
                && !Updater.checked)
            Updater.check(true)
        else
            forceActiveFocus()
    }

    function close() {
        root.visible = false
    }

    function dismiss() {
        if (Updater.restarting || !root.visible)
            return
        root.close()
        root.canceled()
    }

    function checkNow() {
        if (!root.busy)
            Updater.check(true)
    }

    Keys.onEscapePressed: root.dismiss()

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.dismiss()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(470, root.width - 40)
        height: Math.min(410, root.height - 48)
        radius: 16
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Item {
            id: inner
            anchors.fill: parent
            anchors.margins: 18

            Column {
                id: body
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                spacing: 0

                Text {
                    width: parent.width
                    text: qsTr("软件更新")
                    color: root.textColor
                    font.pixelSize: 14
                    font.weight: Font.DemiBold
                }

                Item { width: 1; height: 4 }

                Text {
                    width: parent.width
                    text: qsTr("当前版本 v") + Updater.currentVersion
                    color: root.mutedColor
                    font.pixelSize: 11
                }

                Item { width: 1; height: 12 }

                Text {
                    width: parent.width
                    visible: Updater.checking
                    text: qsTr("正在检查更新…")
                    color: root.textColor
                    font.pixelSize: 12
                }

                Text {
                    width: parent.width
                    visible: !Updater.checking && Updater.restarting
                    text: qsTr("正在安装更新，应用即将重启…")
                    color: root.textColor
                    font.pixelSize: 12
                }

                Text {
                    width: parent.width
                    visible: !Updater.checking && !Updater.restarting && Updater.available
                        && !Updater.downloading
                    text: qsTr("发现新版本 v") + Updater.latestVersion
                    color: root.textColor
                    font.pixelSize: 13
                    font.weight: Font.DemiBold
                }

                Item {
                    width: 1
                    height: !Updater.checking && !Updater.restarting && Updater.available
                        && !Updater.downloading ? 8 : 0
                }

                Flickable {
                    id: notesArea
                    width: parent.width
                    height: visible ? Math.min(contentHeight, 168) : 0
                    visible: !Updater.checking && !Updater.restarting && Updater.available
                        && !Updater.downloading
                    clip: true
                    contentWidth: width
                    contentHeight: notesText.height
                    boundsBehavior: Flickable.StopAtBounds

                    Text {
                        id: notesText
                        width: notesArea.width
                        text: Updater.notes !== ""
                            ? Updater.notes
                            : qsTr("新版本已经准备好。下载完成后会自动校验更新包，确认无误后安装并重启。")
                        color: root.mutedColor
                        font.pixelSize: 11
                        lineHeight: 1.35
                        wrapMode: Text.Wrap
                        textFormat: Text.PlainText
                    }
                }

                Item {
                    width: 1
                    height: Updater.downloading ? 14 : 0
                }

                Text {
                    width: parent.width
                    visible: Updater.downloading
                    text: qsTr("正在下载并校验更新… ") + Updater.installProgress + "%"
                    color: root.textColor
                    font.pixelSize: 12
                }

                Item {
                    width: 1
                    height: Updater.downloading ? 8 : 0
                }

                Rectangle {
                    width: parent.width
                    height: 4
                    radius: 4
                    visible: Updater.downloading
                    color: Theme.fill

                    Rectangle {
                        width: parent.width * Math.max(0, Math.min(100, Updater.installProgress)) / 100
                        height: parent.height
                        radius: 4
                        color: root.accent
                    }
                }

                Item {
                    width: 1
                    height: !Updater.checking && !Updater.available && !Updater.downloading
                        && !Updater.restarting && Updater.error === "" && Updater.checked ? 8 : 0
                }

                Text {
                    width: parent.width
                    visible: !Updater.checking && !Updater.available && !Updater.downloading
                        && !Updater.restarting && Updater.error === "" && Updater.checked
                    text: qsTr("已是最新版本")
                    color: root.mutedColor
                    font.pixelSize: 12
                }

                Item {
                    width: 1
                    height: Updater.error !== "" ? 10 : 0
                }

                Text {
                    width: parent.width
                    visible: Updater.error !== ""
                    text: Updater.error
                    color: root.dangerColor
                    font.pixelSize: 11
                    wrapMode: Text.Wrap
                }
            }

            Row {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                spacing: 8

                Rectangle {
                    width: 72
                    height: 26
                    radius: 6
                    color: closeMouse.containsMouse
                        ? Theme.fill
                        : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("关闭")
                        color: root.mutedColor
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !Updater.restarting
                        onClicked: root.dismiss()
                    }
                }

                Rectangle {
                    width: visible ? 76 : 0
                    height: 26
                    radius: 6
                    visible: !Updater.downloading && !Updater.restarting
                    color: checkMouse.containsMouse
                        ? Theme.fill
                        : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("重新检查")
                        color: root.mutedColor
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: checkMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !root.busy
                        onClicked: root.checkNow()
                    }
                }

                Rectangle {
                    width: visible ? 94 : 0
                    height: 26
                    radius: 6
                    visible: Updater.available && !Updater.restarting
                    opacity: Updater.downloading ? 0.55 : 1
                    color: root.accent

                    Text {
                        anchors.centerIn: parent
                        text: Updater.downloading ? qsTr("下载中…") : qsTr("下载并安装")
                        color: Theme.onInteractive
                        font.pixelSize: 12
                        font.weight: Font.DemiBold
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !Updater.downloading && !Updater.restarting
                        cursorShape: Qt.PointingHandCursor
                        onClicked: Updater.downloadAndInstall()
                    }
                }
            }
        }
    }
}
