pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Localize.js" as Localize

// Grouped content-search results: one header row per file, then one row per
// match. The backend has already trimmed each line and turned its character
// ranges into spans, so a delegate only paints `lineHtml`.
Item {
    id: root

    property var model: null
    property color headerColor: Theme.canvas
    property int currentIndex: -1

    readonly property color contentColor: Theme.content
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline

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
        height: Theme.controlXl
        color: root.headerColor

        Text {
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("内容匹配")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
            font.weight: Font.Medium
            font.letterSpacing: 0.55
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            text: root.model !== null ? Localize.message(root.model.status) : ""
            color: root.mutedColor
            font.pixelSize: Theme.typeMicro
        }
    }

    Text {
        anchors.centerIn: parent
        visible: root.model !== null && root.model.finished && root.model.loaded === 0
        text: root.model !== null && root.model.status !== ""
            ? Localize.message(root.model.status) : qsTr("没有匹配内容")
        color: root.mutedColor
        font.pixelSize: Theme.typeBody
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
        activeFocusOnTab: true
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

            /// A hit is read as "file, line, text"; a group header is a label
            /// over the hits, and the only thing to press in it is 定位, which
            /// is its own button below.
            Accessible.role: row.isHeader ? Accessible.StaticText : Accessible.ListItem
            Accessible.name: row.isHeader
                ? (row.relativePath || row.fileName)
                : row.fileName + " · " + qsTr("%1 行").arg(row.lineNumber)
            Accessible.description: row.isHeader ? "" : row.lineText
            Accessible.selectable: !row.isHeader
            Accessible.selected: !row.isHeader && row.index === list.currentIndex
            Accessible.onPressAction: {
                if (row.isHeader)
                    return
                list.forceActiveFocus()
                list.currentIndex = row.index
                root.currentIndex = row.index
                root.previewRequested(row.filePath)
            }

            Rectangle {
                anchors.fill: parent
                // Group headers keep a quiet plate; result rows use only hover
                // and selection, never zebra stripes.
                color: !row.isHeader && row.index === list.currentIndex
                    ? Theme.signalSoft
                    : (rowMouse.containsMouse ? Theme.hover
                        : (row.isHeader ? Theme.canvasAlt : "transparent"))
            }

            Rectangle {
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: 3
                color: Theme.signal
                visible: !row.isHeader && row.index === list.currentIndex
            }

            Text {
                visible: row.isHeader
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, row.width - 230)
                text: row.isHeader ? row.relativePath || row.fileName : ""
                color: root.textColor
                font.pixelSize: Theme.typeCaption
                font.weight: Font.Medium
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
                font.pixelSize: Theme.typeMicro
            }

            Item {
                id: revealButton
                visible: row.isHeader
                z: 2
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 34
                height: Theme.controlXs

                /// "定位" is drawn as text inside a strip; the strip is the
                /// control, and it reveals the file in the listing.
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("定位")
                Accessible.onPressAction: Workspace.reveal(row.filePath)

                Text {
                    anchors.centerIn: parent
                    text: qsTr("定位")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeMicro
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
                font.pixelSize: Theme.typeMicro
                font.family: Theme.monoFamily
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
                font.pixelSize: Theme.typeCaption
                font.family: Theme.monoFamily
                textFormat: Text.StyledText
                elide: Text.ElideRight
                clip: true
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
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
