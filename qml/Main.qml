import QtQuick
import QtQuick.Window
import dae 1.0

// The first shipping window. Everything the spike measured is wired here:
// the frameless native frame and backdrop (gates 3/4), the streaming model
// (gate 1) and a real TextInput for the IME focus guard (gate 2).
Window {
    id: root

    objectName: "dae-main"
    width: 1180
    height: 780
    visible: false
    color: "transparent"
    title: listing.path !== "" ? listing.path : "dae"
    flags: Qt.Window | Qt.FramelessWindowHint
        | Qt.WindowMinimizeButtonHint | Qt.WindowMaximizeButtonHint
        | Qt.WindowCloseButtonHint

    property bool dark: config.systemDark
    property string material: "none"
    property int windowSerial: 0
    property var tearOffs: []

    readonly property string sourceLabel: config.startupSource === "recent"
        ? "最近访问"
        : config.startupSource === "commandLine" ? "命令行" : "主目录"
    readonly property string profileText: "启动: " + sourceLabel
        + " · " + (config.settingsFound ? "settings.toml" : "默认设置")
        + " · " + (config.configDir !== "" ? config.configDir : "配置目录不可用")

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"

    AppConfig {
        id: config
    }

    WindowController {
        id: chrome
        token: root.objectName
    }

    DirectoryListModel {
        id: listing
    }

    Shortcut {
        sequence: config.pathJump
        enabled: config.pathJump !== ""
        onActivated: {
            pathInput.forceActiveFocus()
            pathInput.selectAll()
        }
    }

    Component {
        id: tearOffComponent
        TearOffWindow {}
    }

    function openWindow() {
        windowSerial += 1
        const window = tearOffComponent.createObject(null, {
            "model": listing,
            "token": "dae-window-" + windowSerial,
            "dark": dark,
            "material": material,
            "ownerList": tearOffs
        })
        if (window)
            tearOffs.push(window)
    }

    Component.onCompleted: {
        if (!chrome.attach())
            console.warn("dae: native window attach failed")
        material = chrome.startupMaterial()
        chrome.applyMaterial(material, dark)
        listing.load(config.startupPath)
        visible = true
    }

    Column {
        anchors.fill: parent

        TitleBar {
            width: parent.width
            window: root
            dark: root.dark
            material: root.material
            title: root.title
        }

        Rectangle {
            width: parent.width
            height: 40
            color: root.chromeColor

            Rectangle {
                id: pathField
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.right: newWindowButton.left
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                height: 26
                radius: 7
                color: root.dark ? "#1affffff" : "#0d000000"
                border.width: 1
                border.color: pathInput.activeFocus ? (root.dark ? "#4d6ea8fe" : "#4d2f6fdf")
                                                    : root.hairline

                TextInput {
                    id: pathInput
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    text: listing.path
                    color: root.textColor
                    selectionColor: "#2f6fdf"
                    selectedTextColor: "#ffffff"
                    font.pixelSize: 13
                    selectByMouse: true
                    clip: true

                    onAccepted: {
                        let target = text.trim()
                        if (target.length >= 2
                                && target.charAt(0) === '"'
                                && target.charAt(target.length - 1) === '"')
                            target = target.substring(1, target.length - 1)
                        if (target.length > 0)
                            listing.load(target)
                    }

                    Keys.onEscapePressed: {
                        text = listing.path
                        focus = false
                    }
                }
            }

            Rectangle {
                id: newWindowButton
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 96
                height: 26
                radius: 7
                color: newWindowMouse.containsMouse
                    ? (root.dark ? "#26ffffff" : "#12000000")
                    : (root.dark ? "#1affffff" : "#0d000000")

                Text {
                    anchors.centerIn: parent
                    text: "新建窗口"
                    color: root.textColor
                    font.pixelSize: 12
                }

                MouseArea {
                    id: newWindowMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.openWindow()
                }
            }
        }

        FileList {
            width: parent.width
            height: parent.height - 40 - 40 - 22
            model: listing
            dark: root.dark
            headerColor: root.chromeColor
        }

        Rectangle {
            width: parent.width
            height: 22
            color: root.chromeColor

            Text {
                id: statusText
                x: 12
                width: parent.width * 0.42
                anchors.verticalCenter: parent.verticalCenter
                text: listing.finished
                    ? listing.status
                    : (listing.loaded + " 行 · 读取中…")
                color: root.mutedColor
                font.family: "Cascadia Mono"
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            Text {
                anchors.left: statusText.right
                anchors.leftMargin: 16
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                horizontalAlignment: Text.AlignRight
                text: root.profileText
                color: root.mutedColor
                font.family: "Cascadia Mono"
                font.pixelSize: 11
                elide: Text.ElideMiddle
            }
        }
    }

    Connections {
        target: listing

        function onPathChanged() {
            if (!pathInput.activeFocus)
                pathInput.text = listing.path
        }
    }
}
