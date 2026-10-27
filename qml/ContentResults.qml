pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// Grouped content-search results: one header row per file, then one row per
// match. The backend has already trimmed each line and turned its character
// ranges into spans, so a delegate only paints `lineHtml`.
Item {
    id: root

    property var model: null
    property bool dark: true
    property color accent: "#2f6fdf"
    property color headerColor: dark ? "#14161a" : "#f4f6f8"
    property int currentIndex: -1

    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color alternateColor: dark ? "#15181e" : "#f7f8fa"
    readonly property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8f98aa" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"
    readonly property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b,
                                                   dark ? 0.30 : 0.18)

    signal previewRequested(string path)

    function openCurrent() {
        if (model === null || currentIndex < 0)
            return
        const path = model.pathAt(currentIndex)
        if (path !== "")
            Workspace.openPath(path)
    }

    function takeFocus() {
        list.forceActiveFocus()
    }

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
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("内容匹配")
            color: root.mutedColor
            font.pixelSize: 12
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            text: root.model !== null ? root.model.status : ""
            color: root.mutedColor
            font.pixelSize: 11
        }
    }

    Text {
        anchors.centerIn: parent
        visible: root.model !== null && root.model.finished && root.model.loaded === 0
        text: root.model !== null && root.model.status !== ""
            ? root.model.status : qsTr("没有匹配内容")
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
        cacheBuffer: 1200
        clip: true
        focus: true
        boundsBehavior: Flickable.StopAtBounds
        currentIndex: -1

        Keys.onPressed: function (event) {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                root.openCurrent()
                event.accepted = true
            }
        }

        delegate: Item {
            id: row

            required property int index
            required property bool isHeader
            required property string fileName
            required property string filePath
            required property string relativePath
            required property int lineNumber
            required property string lineText
            required property string lineHtml
            required property int matchCount

            width: ListView.view.width
            height: isHeader ? 30 : 24

            Rectangle {
                anchors.fill: parent
                color: row.index === list.currentIndex ? root.selectionColor
                    : (row.isHeader
                        ? (root.dark ? "#1d2633" : "#eef3fb")
                        : (row.index % 2 === 0 ? root.contentColor : root.alternateColor))
            }

            Text {
                visible: row.isHeader
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, row.width - 230)
                text: row.isHeader ? row.relativePath || row.fileName : ""
                color: root.textColor
                font.pixelSize: 12
                font.bold: true
                textFormat: Text.PlainText
                elide: Text.ElideMiddle
            }

            Text {
                visible: row.isHeader
                anchors.right: revealButton.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: row.matchCount + qsTr(" 处")
                color: root.mutedColor
                font.pixelSize: 11
            }

            Item {
                id: revealButton
                visible: row.isHeader
                z: 2
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 34
                height: 20

                Text {
                    anchors.centerIn: parent
                    text: qsTr("定位")
                    color: root.mutedColor
                    font.pixelSize: 10
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Workspace.reveal(row.filePath)
                }
            }

            Text {
                visible: !row.isHeader
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 42
                horizontalAlignment: Text.AlignRight
                text: row.isHeader ? "" : row.lineNumber
                color: root.mutedColor
                font.pixelSize: 11
                font.family: "Cascadia Mono"
            }

            Text {
                visible: !row.isHeader
                anchors.left: parent.left
                anchors.leftMargin: 66
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                text: row.isHeader ? "" : row.lineHtml
                color: root.textColor
                font.pixelSize: 12
                font.family: "Cascadia Mono"
                textFormat: Text.StyledText
                elide: Text.ElideRight
                clip: true
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton
                onClicked: {
                    list.forceActiveFocus()
                    list.currentIndex = row.index
                    root.currentIndex = row.index
                    root.previewRequested(row.filePath)
                }
                onDoubleClicked: Workspace.openPath(row.filePath)
            }
        }

        onCurrentIndexChanged: root.currentIndex = currentIndex
    }
}
