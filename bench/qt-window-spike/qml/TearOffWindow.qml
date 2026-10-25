import QtQuick
import QtQuick.Window

// The window a tab becomes when it is torn out. Same frameless native path as
// the main window (the C++ probe styles it by title before it is shown) and the
// same QAbstractListModel instance — the tab's data is not copied.
Window {
    id: root

    property var model: null
    property string token: ""
    property bool dark: true
    property string material: "mica"
    readonly property int rowHeight: 26

    width: 900
    height: 620
    title: token
    flags: Qt.Window | Qt.FramelessWindowHint
        | Qt.WindowMinimizeButtonHint | Qt.WindowMaximizeButtonHint
        | Qt.WindowCloseButtonHint
    color: "transparent"
    visible: false

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"

    Rectangle {
        id: titleBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 40
        color: root.chromeColor

        Rectangle {
            id: logo
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20
            radius: 6
            color: root.dark ? "#6ea8fe" : "#2f6fdf"

            Text {
                anchors.centerIn: parent
                text: "D"
                color: "#ffffff"
                font.pixelSize: 12
                font.weight: Font.DemiBold
            }
        }

        Text {
            anchors.left: logo.right
            anchors.leftMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: "dae · 撕出"
            color: root.textColor
            font.pixelSize: 13
            font.weight: Font.DemiBold
        }

        Text {
            anchors.left: logo.right
            anchors.leftMargin: 82
            anchors.verticalCenter: parent.verticalCenter
            text: root.token
            color: root.mutedColor
            font.family: "Cascadia Mono"
            font.pixelSize: 11
        }

        Item {
            id: windowButtons
            anchors.right: parent.right
            height: parent.height
            width: 46 * 3

            Repeater {
                model: 3

                Item {
                    required property int index
                    width: 46
                    height: 40
                    x: index * 46

                    readonly property bool isClose: index === 2
                    readonly property bool isMaximize: index === 1

                    Rectangle {
                        anchors.fill: parent
                        color: isClose ? "#c42b1c" : "transparent"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: isClose ? "\uE8BB"
                              : isMaximize ? "\uE922"
                              : "\uE921"
                        color: isClose ? "#ffffff" : root.textColor
                        font.family: "Segoe Fluent Icons"
                        font.pixelSize: 10
                    }

                    // Close is a client area (HTCLIENT), so the click reaches
                    // QML; minimize/maximize stay native zones for gate 3's
                    // reasons and are drawn only.
                    MouseArea {
                        anchors.fill: parent
                        enabled: parent.isClose
                        onClicked: root.close()
                    }
                }
            }
        }
    }

    Rectangle {
        id: columnHeader
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: titleBar.bottom
        height: 34
        color: root.chromeColor

        Text {
            x: 36
            anchors.verticalCenter: parent.verticalCenter
            text: "名称"
            color: root.mutedColor
            font.pixelSize: 12
        }

        Text {
            anchors.right: modifiedLeft.left
            anchors.rightMargin: 24
            anchors.verticalCenter: parent.verticalCenter
            width: 90
            horizontalAlignment: Text.AlignRight
            text: "大小"
            color: root.mutedColor
            font.pixelSize: 12
        }

        Item {
            id: modifiedLeft
            anchors.right: parent.right
            anchors.rightMargin: 156
            width: 1
            height: 1
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            width: 140
            horizontalAlignment: Text.AlignRight
            text: "修改时间"
            color: root.mutedColor
            font.pixelSize: 12
        }
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 22
        model: root.model
        reuseItems: true
        cacheBuffer: 2600
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0

        delegate: Item {
            id: delegateRoot
            required property int index
            required property string fileName
            required property string sizeText
            required property string modifiedText
            required property bool isDir
            required property bool hidden

            width: ListView.view.width
            height: root.rowHeight

            Rectangle {
                anchors.fill: parent
                color: (delegateRoot.index % 2) === 0 ? root.contentColor
                                                      : (root.dark ? "#15181e" : "#f7f8fa")
            }

            Rectangle {
                x: 12
                y: 6
                width: 14
                height: 14
                radius: 3
                color: delegateRoot.isDir ? "#5b8def" : "#3b4250"
            }

            Text {
                x: 36
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, delegateRoot.width - 36 - 340)
                text: delegateRoot.fileName
                color: delegateRoot.hidden ? "#6b7280" : root.textColor
                font.pixelSize: 13
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }

            Text {
                anchors.right: modifiedTextItem.left
                anchors.rightMargin: 24
                anchors.verticalCenter: parent.verticalCenter
                width: 90
                horizontalAlignment: Text.AlignRight
                text: delegateRoot.sizeText
                color: root.mutedColor
                font.pixelSize: 12
                textFormat: Text.PlainText
            }

            Text {
                id: modifiedTextItem
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 140
                horizontalAlignment: Text.AlignRight
                text: delegateRoot.modifiedText
                color: root.mutedColor
                font.pixelSize: 12
                textFormat: Text.PlainText
            }
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 22
        color: root.chromeColor

        Text {
            x: 12
            anchors.verticalCenter: parent.verticalCenter
            text: root.model !== null
                  ? (root.model.loaded + " 行 · 与源窗口共用同一个模型")
                  : "no model"
            color: root.mutedColor
            font.family: "Cascadia Mono"
            font.pixelSize: 11
        }
    }
}
