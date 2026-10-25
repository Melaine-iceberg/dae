pragma ComponentBehavior: Bound
import QtQuick

// The explorer row from gate 1: 26 dp fixed-height delegates, recycling and a
// cache buffer sized to the visible page. The model supplies finished strings,
// so a delegate never formats a size or a timestamp in JavaScript.
Item {
    id: root

    property var model: null
    property bool dark: true
    property color headerColor: dark ? "#14161a" : "#f4f6f8"

    readonly property int rowHeight: 26
    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color alternateColor: dark ? "#15181e" : "#f7f8fa"
    readonly property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8f98aa" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"
    readonly property color selectionColor: dark ? "#22314a" : "#dbe7fb"

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    Rectangle {
        id: columnHeader
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 34
        color: root.headerColor

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

    Text {
        anchors.centerIn: parent
        visible: root.model !== null && root.model.finished && root.model.loaded === 0
        text: "空目录"
        color: root.mutedColor
        font.pixelSize: 13
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        model: root.model
        reuseItems: true
        cacheBuffer: 2600
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0

        delegate: Item {
            id: row

            required property int index
            required property string fileName
            required property string filePath
            required property string sizeText
            required property string modifiedText
            required property bool isDir
            required property bool hidden
            required property bool readOnly

            width: ListView.view.width
            height: root.rowHeight

            Rectangle {
                anchors.fill: parent
                color: list.currentIndex === row.index
                    ? root.selectionColor
                    : (row.index % 2 === 0 ? root.contentColor : root.alternateColor)
            }

            Rectangle {
                x: 12
                y: 6
                width: 14
                height: 14
                radius: 3
                color: row.isDir ? "#5b8def" : (row.readOnly ? "#59606e" : "#3b4250")
            }

            Text {
                x: 36
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, row.width - 36 - 340)
                text: row.fileName
                color: row.hidden ? (root.dark ? "#6b7280" : "#9aa1ad") : root.textColor
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
                text: row.sizeText
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
                text: row.modifiedText
                color: root.mutedColor
                font.pixelSize: 12
                textFormat: Text.PlainText
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton
                onClicked: {
                    list.forceActiveFocus()
                    list.currentIndex = row.index
                }
                onDoubleClicked: {
                    list.forceActiveFocus()
                    if (row.isDir)
                        root.model.load(row.filePath)
                }
            }
        }
    }
}
