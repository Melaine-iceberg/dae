pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The docked preview pane.
//
// The controller answers metadata and a bounded text peek asynchronously; the
// image itself comes from the `thumbnail://` provider, which caches by file
// revision and decodes off the GUI thread. Selecting several entries collapses
// the pane to a count rather than describing only the first one.
Item {
    id: root

    property var paths: []
    property bool dark: true
    property color accent: "#2f6fdf"

    readonly property color panelColor: dark ? "#17191e" : "#fbfcfd"
    readonly property color sectionColor: dark ? "#1f2229" : "#f1f4f7"
    readonly property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8f98aa" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"
    readonly property bool single: paths.length === 1
    /// The entry was read through a network backend. Text previews work, but
    /// thumbnails, media tags, and OS open/reveal do not.
    readonly property bool remote: controller.remote
    readonly property string thumbnailSource: root.single && controller.hasThumbnail
        && controller.thumbnailRevision !== ""
        ? "image://thumbnail/" + encodeURIComponent(controller.path)
          + "?size=384&v=" + encodeURIComponent(controller.thumbnailRevision)
        : ""

    implicitWidth: 300

    signal closeRequested()
    signal openRequested(string path, bool isDir)

    onDarkChanged: controller.setDark(root.dark)
    Component.onCompleted: controller.setDark(root.dark)

    function showPaths(next) {
        paths = (next || []).slice()
        if (paths.length === 1)
            controller.open(paths[0])
        else
            controller.clear()
    }

    PreviewController {
        id: controller
    }

    Rectangle {
        anchors.fill: parent
        color: root.panelColor

        Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 1
            color: root.hairline
        }
    }

    Item {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 36

        Text {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("预览")
            color: root.mutedColor
            font.pixelSize: 11
            font.bold: true
        }

        Item {
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 24
            height: 24

            Text {
                anchors.centerIn: parent
                text: "×"
                color: root.mutedColor
                font.pixelSize: 16
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.closeRequested()
            }
        }
    }

    Flickable {
        id: scroller
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        contentWidth: width
        contentHeight: content.height
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: content
            width: scroller.width
            spacing: 10

            Text {
                x: 14
                visible: !root.single
                width: content.width - 28
                text: root.paths.length === 0 ? qsTr("未选择文件")
                    : root.paths.length + qsTr(" 项已选择")
                color: root.textColor
                font.pixelSize: 14
                font.bold: true
                wrapMode: Text.Wrap
            }

            Text {
                x: 14
                visible: !root.single && root.paths.length > 1
                width: content.width - 28
                text: qsTr("多选时不展开内容预览")
                color: root.mutedColor
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Text {
                x: 14
                visible: root.single
                width: content.width - 28
                text: controller.name !== "" ? controller.name
                    : (root.paths.length === 1 ? root.paths[0].split(/[\\/]/).pop() : "")
                color: root.textColor
                font.pixelSize: 14
                font.bold: true
                wrapMode: Text.Wrap
            }

            Text {
                x: 14
                visible: root.single && !controller.loading && controller.error === ""
                width: content.width - 28
                text: controller.kindText + (controller.sizeText !== "" ? " · " + controller.sizeText : "")
                color: root.mutedColor
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Rectangle {
                visible: root.single && controller.hasThumbnail && controller.mediaKind === ""
                width: content.width - 24
                x: 12
                height: 200
                radius: 9
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Image {
                    id: thumbnail
                    anchors.fill: parent
                    anchors.margins: 10
                    source: root.thumbnailSource
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    cache: false
                    visible: status === Image.Ready
                }

                Text {
                    anchors.centerIn: parent
                    visible: thumbnail.status !== Image.Ready
                    text: thumbnail.status === Image.Error ? qsTr("无法生成缩略图") : qsTr("正在加载…")
                    color: root.mutedColor
                    font.pixelSize: 11
                }
            }

            Loader {
                id: mediaLoader
                x: 12
                width: content.width - 24
                height: active && item ? item.implicitHeight : 0
                active: root.single && controller.mediaKind !== ""
                    && controller.mediaUrl !== "" && root.visible
                source: active ? "MediaPreview.qml" : ""

                onLoaded: {
                    item.mediaUrl = Qt.binding(function () { return controller.mediaUrl })
                    item.mediaKind = Qt.binding(function () { return controller.mediaKind })
                    item.posterSource = Qt.binding(function () { return root.thumbnailSource })
                    item.dark = Qt.binding(function () { return root.dark })
                    item.accent = Qt.binding(function () { return root.accent })
                    item.textColor = Qt.binding(function () { return root.textColor })
                    item.mutedColor = Qt.binding(function () { return root.mutedColor })
                    item.sectionColor = Qt.binding(function () { return root.sectionColor })
                    item.hairline = Qt.binding(function () { return root.hairline })
                }
            }

            Text {
                x: 14
                visible: mediaLoader.active && mediaLoader.status === Loader.Error
                width: content.width - 28
                text: qsTr("当前构建不支持媒体播放")
                color: root.mutedColor
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Rectangle {
                visible: root.single && controller.hasText
                width: content.width - 24
                x: 12
                height: Math.min(controller.isMarkdown ? 360 : 300,
                                 Math.max(120, root.height * (controller.isMarkdown ? 0.46 : 0.34)))
                radius: 9
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    anchors.fill: parent
                    anchors.margins: 1
                    contentWidth: width
                    contentHeight: textPreview.contentHeight + 16
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    TextEdit {
                        id: textPreview
                        x: 10
                        y: 8
                        width: parent.width - 20
                        height: Math.max(contentHeight, parent.height - 16)
                        readOnly: true
                        selectByMouse: true
                        text: controller.contentHtml !== "" ? controller.contentHtml
                            : (controller.content !== "" ? controller.content : qsTr("（空文件）"))
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: "#ffffff"
                        font.pixelSize: 11
                        font.family: controller.isMarkdown ? "" : "Cascadia Mono"
                        textFormat: controller.contentHtml !== "" ? TextEdit.RichText
                            : TextEdit.PlainText
                        wrapMode: TextEdit.Wrap

                        onLinkActivated: function (link) {
                            if (!link || link.charAt(0) === "#")
                                return
                            if (/^[a-zA-Z][a-zA-Z0-9+.-]+:/.test(link))
                                Qt.openUrlExternally(link)
                            else
                                Workspace.openPath(link)
                        }
                    }
                }

                Text {
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 6
                    visible: controller.truncated
                    text: qsTr("仅显示前 64 KB")
                    color: root.mutedColor
                    font.pixelSize: 10
                }
            }

            Text {
                x: 14
                visible: root.single && controller.mediaText !== ""
                width: content.width - 28
                text: controller.mediaText
                color: root.mutedColor
                font.pixelSize: 11
                lineHeight: 1.35
                wrapMode: Text.Wrap
            }

            Text {
                x: 14
                visible: root.single && controller.remote && !controller.loading
                       && controller.error === ""
                width: content.width - 28
                text: qsTr("远程项目：可预览文本；缩略图和媒体信息仅支持本地文件。")
                color: root.mutedColor
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Text {
                x: 14
                visible: root.single && controller.loading
                width: content.width - 28
                text: qsTr("读取中…")
                color: root.mutedColor
                font.pixelSize: 11
            }

            Text {
                x: 14
                visible: root.single && controller.error !== ""
                width: content.width - 28
                text: controller.error
                color: "#e06c75"
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Column {
                width: content.width
                visible: root.single && controller.error === "" && !controller.loading

                Repeater {
                    model: [
                        [qsTr("类型"), controller.kindText],
                        [qsTr("大小"), controller.sizeText],
                        [qsTr("修改时间"), controller.modifiedText]
                    ]

                    Item {
                        id: metaRow
                        required property var modelData
                        width: content.width
                        height: 28

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.leftMargin: 14
                            anchors.rightMargin: 14
                            height: 1
                            color: root.hairline
                        }

                        Text {
                            x: 14
                            anchors.verticalCenter: parent.verticalCenter
                            width: 66
                            text: metaRow.modelData[0]
                            color: root.mutedColor
                            font.pixelSize: 11
                        }

                        Text {
                            anchors.left: parent.left
                            anchors.leftMargin: 86
                            anchors.right: parent.right
                            anchors.rightMargin: 14
                            anchors.verticalCenter: parent.verticalCenter
                            text: metaRow.modelData[1]
                            color: root.textColor
                            font.pixelSize: 11
                            textFormat: Text.PlainText
                            elide: Text.ElideMiddle
                        }
                    }
                }
            }

            Item {
                width: 1
                height: 4
            }

            Row {
                x: 12
                width: content.width - 24
                height: visible ? 30 : 0
                spacing: 8
                visible: root.single && controller.path !== "" && !controller.loading

                Rectangle {
                    width: (parent.width - parent.spacing) * 0.58
                    height: parent.height
                    radius: 6
                    color: root.accent
                    opacity: (!root.remote || controller.isDirectory) ? 1 : 0.4

                    Text {
                        anchors.centerIn: parent
                        text: controller.isDirectory ? qsTr("打开文件夹")
                            : (root.remote ? qsTr("远程文件") : qsTr("打开"))
                        color: "#ffffff"
                        font.pixelSize: 11
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !root.remote || controller.isDirectory
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: root.openRequested(controller.path, controller.isDirectory)
                    }
                }

                Rectangle {
                    width: parent.width - parent.spacing
                           - (parent.width - parent.spacing) * 0.58
                    height: parent.height
                    radius: 6
                    color: "transparent"
                    border.width: 1
                    border.color: root.hairline
                    opacity: root.remote ? 0.4 : 1

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("定位")
                        color: root.textColor
                        font.pixelSize: 11
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !root.remote
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: Workspace.reveal(controller.path)
                    }
                }
            }

            Item {
                width: 1
                height: 12
            }
        }
    }

    Connections {
        target: controller

        function onPathChanged() {
            scroller.contentY = 0
        }
    }
}
