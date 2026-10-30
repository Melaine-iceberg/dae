pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The docked preview pane.
//
// The controller answers metadata and a bounded text peek asynchronously; the
// image itself comes from the `thumbnail://` provider, which caches by file
// revision and decodes off the GUI thread. Selecting several entries collapses
// the pane to a count rather than describing only the first one.
//
// The pane sits on `canvas`, not on a raised plate: it is the fourth column of
// the window, not a dialog over it. See docs/ui-instrument-migration.md §7.1.
Item {
    id: root

    property var paths: []
    /// One recycle-bin row, or null. The trash lists entries that no longer
    /// exist on disk, so the controller cannot describe them; the shell hands
    /// the row's strings over instead.
    property var trashItem: null
    property bool inTrash: false
    property bool dark: true
    property color accent: Theme.interactive

    readonly property color panelColor: Theme.canvas
    readonly property color sectionColor: Theme.canvasAlt
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color faintColor: Theme.inkFaint
    readonly property color hairline: Theme.hairline
    readonly property bool single: paths.length === 1
    readonly property bool empty: paths.length === 0 && trashItem === null
    /// The entry was read through a network backend. Text previews work, but
    /// thumbnails, media tags, and OS open/reveal do not.
    readonly property bool remote: controller.remote
    readonly property string thumbnailSource: root.single && controller.hasThumbnail
        && controller.thumbnailRevision !== ""
        ? "image://thumbnail/" + encodeURIComponent(controller.path)
          + "?size=384&v=" + encodeURIComponent(controller.thumbnailRevision)
        : ""

    /// Plain text keeps its line numbers; rendered Markdown and highlighted
    /// code arrive as HTML and are shown as they are. Five hundred lines is
    /// far past what the 176 px block shows at a time, and it keeps a 64 KB
    /// peek from becoming a text-shaping job on a 300 px pane.
    readonly property string plainText: {
        const source = controller.content !== "" ? controller.content : qsTr("（空文件）")
        const lines = source.split("\n")
        return lines.length > 500 ? lines.slice(0, 500).join("\n") : source
    }
    readonly property string lineNumbers: {
        const count = root.plainText.split("\n").length
        let out = ""
        for (let index = 1; index <= count; index += 1)
            out += (index === 1 ? "" : "\n") + index
        return out
    }

    implicitWidth: 300

    signal closeRequested()
    signal openRequested(string path, bool isDir)
    signal propertiesRequested(string path)

    onDarkChanged: controller.setDark(root.dark)
    Component.onCompleted: controller.setDark(root.dark)

    function showPaths(next) {
        trashItem = null
        paths = (next || []).slice()
        if (paths.length === 1)
            controller.open(paths[0])
        else
            controller.clear()
    }

    function showTrash(item) {
        paths = []
        trashItem = item && item.name !== undefined && item.name !== "" ? item : null
        controller.clear()
    }

    function clear() {
        showPaths([])
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
        height: 32

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.leftMargin: 14
            height: 1
            color: root.hairline
        }

        Text {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("预览")
            color: root.faintColor
            font.pixelSize: Theme.typeMicro
            font.letterSpacing: 0.9
        }

        Item {
            id: closeButton
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("关闭预览")

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusXs
                color: closeMouse.containsMouse ? Theme.hover : "transparent"
            }

            Glyph {
                anchors.centerIn: parent
                width: 14
                height: 14
                kind: "close"
                tint: closeMouse.containsMouse ? root.textColor : root.faintColor
            }

            MouseArea {
                id: closeMouse
                anchors.fill: parent
                hoverEnabled: true
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
        contentHeight: content.height + 28
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: content
            x: 14
            y: 14
            width: scroller.width - 28
            spacing: 12

            // ── Nothing selected ──────────────────────────────────────────
            Text {
                width: parent.width
                visible: root.empty
                text: qsTr("未选择文件")
                color: root.mutedColor
                font.pixelSize: Theme.typeName
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.empty
                text: root.inTrash ? qsTr("选中项目查看原位置") : qsTr("单击文件查看详情")
                color: root.faintColor
                font.pixelSize: Theme.typeCaption
                wrapMode: Text.Wrap
                lineHeight: 1.4
            }

            // ── A recycle-bin row ─────────────────────────────────────────
            Column {
                width: parent.width
                spacing: 3
                visible: root.trashItem !== null

                Text {
                    width: parent.width
                    text: root.trashItem !== null ? root.trashItem.name : ""
                    color: root.textColor
                    font.pixelSize: Theme.typeEmphasis
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                }

                Text {
                    width: parent.width
                    text: root.trashItem !== null ? root.trashItem.originalParent : ""
                    color: root.faintColor
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                }
            }

            Column {
                width: parent.width
                visible: root.trashItem !== null

                Repeater {
                    model: root.trashItem !== null
                        ? [
                            [qsTr("类型"), root.trashItem.isDir ? qsTr("文件夹") : qsTr("文件")],
                            [qsTr("大小"), root.trashItem.sizeText],
                            [qsTr("原位置"), root.trashItem.originalParent],
                            [qsTr("删除时间"), root.trashItem.deletedText]
                        ]
                        : []

                    MetaRow {
                        width: content.width
                        required property var modelData
                        key: modelData[0]
                        value: modelData[1]
                    }
                }
            }

            // ── Several selected ──────────────────────────────────────────
            Text {
                width: parent.width
                visible: !root.empty && !root.single && root.trashItem === null
                text: root.paths.length + qsTr(" 项")
                color: root.textColor
                font.pixelSize: Theme.typeDisplay
                font.features: { "tnum": 1 }
            }

            Text {
                width: parent.width
                visible: !root.empty && !root.single && root.trashItem === null
                text: qsTr("多选时不展开内容预览")
                color: root.faintColor
                font.pixelSize: Theme.typeCaption
                wrapMode: Text.Wrap
            }

            // ── One entry ─────────────────────────────────────────────────
            // Image, when the provider can make one and the file is not a
            // clip that the media player will take over.
            Rectangle {
                visible: root.single && controller.hasThumbnail && controller.mediaKind === ""
                width: parent.width
                height: 176
                radius: Theme.radiusMd
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
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                }
            }

            // Plain text: a line-number gutter and the content, both in the
            // same mono face at the same line height, so the digits line up
            // with the rows they count.
            Rectangle {
                visible: root.single && controller.hasText && controller.contentHtml === ""
                width: parent.width
                height: 176
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    anchors.fill: parent
                    anchors.margins: 1
                    contentWidth: width
                    contentHeight: Math.max(plainRows.height, height)
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    Row {
                        id: plainRows
                        width: parent.width
                        spacing: 8

                        Text {
                            width: 28
                            text: root.lineNumbers
                            color: root.faintColor
                            horizontalAlignment: Text.AlignRight
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            lineHeight: 1.6
                            font.features: { "tnum": 1 }
                        }

                        Text {
                            width: plainRows.width - 36
                            text: root.plainText
                            color: root.textColor
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            lineHeight: 1.6
                            wrapMode: Text.NoWrap
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }
                    }
                }

                Text {
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 6
                    visible: controller.truncated
                    text: qsTr("仅显示前 64 KB")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                }
            }

            // Rendered Markdown and highlighted code. They carry their own
            // colours, so they keep the text-edit path instead of the gutter.
            Rectangle {
                visible: root.single && controller.hasText && controller.contentHtml !== ""
                width: parent.width
                height: 176
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    anchors.fill: parent
                    anchors.margins: 1
                    contentWidth: width
                    contentHeight: richPreview.contentHeight + 16
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    TextEdit {
                        id: richPreview
                        x: 10
                        y: 8
                        width: parent.width - 20
                        height: Math.max(contentHeight, parent.height - 16)
                        readOnly: true
                        selectByMouse: true
                        text: controller.contentHtml
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: Theme.onInteractive
                        font.pixelSize: Theme.typeMicro
                        textFormat: TextEdit.RichText

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
            }

            // A folder: how much is in it, then the first few names.
            Rectangle {
                visible: root.single && controller.isDirectory
                width: parent.width
                height: Math.min(176, folderColumn.height + 20)
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Column {
                    id: folderColumn
                    x: 10
                    y: 10
                    width: parent.width - 20
                    spacing: 0

                    Text {
                        width: parent.width
                        height: 22
                        text: controller.loading
                            ? qsTr("读取中…")
                            : (controller.childCount === 0
                                ? qsTr("空文件夹")
                                : controller.childCount + qsTr(" 项"))
                        color: root.faintColor
                        font.pixelSize: Theme.typeCaption
                        verticalAlignment: Text.AlignVCenter
                        font.features: { "tnum": 1 }
                    }

                    Repeater {
                        model: controller.childNames

                        Item {
                            required property string modelData
                            width: folderColumn.width
                            height: 22

                            Row {
                                anchors.left: parent.left
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 6

                                Glyph {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 14
                                    height: 14
                                    kind: "file"
                                    tint: root.faintColor
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: folderColumn.width - 18
                                    text: modelData
                                    color: root.mutedColor
                                    font.pixelSize: Theme.typeCaption
                                    elide: Text.ElideMiddle
                                    textFormat: Text.PlainText
                                }
                            }
                        }
                    }
                }
            }

            Loader {
                id: mediaLoader
                width: content.width
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
                width: parent.width
                visible: mediaLoader.active && mediaLoader.status === Loader.Error
                text: qsTr("当前构建不支持媒体播放")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.single && controller.mediaText !== ""
                text: controller.mediaText
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                lineHeight: 1.4
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.single
                text: controller.name !== "" ? controller.name
                    : (root.paths.length === 1 ? root.paths[0].split(/[\\/]/).pop() : "")
                color: root.textColor
                font.pixelSize: Theme.typeEmphasis
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.single
                text: controller.path
                color: root.faintColor
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                elide: Text.ElideMiddle
                textFormat: Text.PlainText
            }

            Column {
                width: parent.width
                visible: root.single && controller.error === "" && !controller.loading

                Repeater {
                    model: [
                        [qsTr("类型"), controller.kindText],
                        [qsTr("大小"), controller.sizeText],
                        [qsTr("修改时间"), controller.modifiedText]
                    ]

                    MetaRow {
                        width: content.width
                        required property var modelData
                        key: modelData[0]
                        value: modelData[1]
                    }
                }
            }

            Text {
                width: parent.width
                visible: root.single && controller.loading
                text: qsTr("读取中…")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
            }

            Text {
                width: parent.width
                visible: root.single && controller.error !== ""
                text: controller.error
                color: Theme.danger
                font.pixelSize: Theme.typeMicro
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.single && controller.remote && !controller.loading
                       && controller.error === ""
                text: qsTr("远程项目：可预览文本；缩略图和媒体信息仅支持本地文件。")
                color: root.faintColor
                font.pixelSize: Theme.typeMicro
                wrapMode: Text.Wrap
                lineHeight: 1.4
            }

            // ── Actions ───────────────────────────────────────────────────
            // Quiet text, not buttons: the pane is a read-out, and the one
            // saturated colour in it belongs to `全部属性`.
            Row {
                id: actionRow
                width: parent.width
                height: 26
                spacing: 16
                visible: root.single && controller.path !== "" && !controller.loading

                TextAction {
                    id: openAction
                    label: controller.isDirectory ? qsTr("打开文件夹")
                        : (root.remote ? qsTr("远程文件") : qsTr("打开"))
                    available: !root.remote || controller.isDirectory
                    onClicked: root.openRequested(controller.path, controller.isDirectory)
                }

                TextAction {
                    id: revealAction
                    label: qsTr("定位")
                    available: !root.remote
                    onClicked: Workspace.reveal(controller.path)
                }

                Item {
                    width: Math.max(0, actionRow.width - openAction.width
                        - revealAction.width - propertiesAction.width
                        - actionRow.spacing * 3)
                    height: 1
                }

                TextAction {
                    id: propertiesAction
                    label: qsTr("全部属性")
                    accent: true
                    chevron: true
                    onClicked: root.propertiesRequested(controller.path)
                }
            }
        }
    }

    /// One metadata line: quiet key on the left, value in the reading ink on
    /// the right, separated by the lightest rule in the palette.
    component MetaRow: Item {
        id: metaRow

        property string key: ""
        property string value: ""

        width: parent ? parent.width : 0
        height: 24

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.rowRule
        }

        Text {
            id: keyLabel
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(keyLabel.implicitWidth, metaRow.width * 0.42)
            text: metaRow.key
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        // Anchored between the key and the right edge rather than given a
        // computed width: a subtraction here has to know how wide the key is
        // before it is laid out, and a 300 wide pane leaves too little room to
        // absorb a wrong guess.
        Text {
            id: valueLabel
            anchors.left: keyLabel.right
            anchors.leftMargin: 12
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            horizontalAlignment: Text.AlignRight
            text: metaRow.value
            color: Theme.ink
            font.pixelSize: Theme.typeBody
            font.features: { "tnum": 1 }
            elide: Text.ElideMiddle
            textFormat: Text.PlainText
        }
    }

    component TextAction: Item {
        id: action

        property string label: ""
        property bool accent: false
        property bool chevron: false
        property bool available: true

        signal clicked()

        implicitWidth: actionRowText.implicitWidth + 2
        implicitHeight: 26
        opacity: action.available ? 1 : 0.4

        Accessible.role: Accessible.Button
        Accessible.name: action.label

        /// The preview's links are the only things in the panel that can be
        /// pressed, so they are the panel's tab stop. A transparent ring is
        /// drawn outside the text because the text is only 26 px tall and a
        /// border inside it would touch the glyphs.
        activeFocusOnTab: action.available
        Keys.onReturnPressed: action.clicked()
        Keys.onEnterPressed: action.clicked()
        Keys.onSpacePressed: action.clicked()

        Rectangle {
            anchors.fill: parent
            anchors.margins: -4
            visible: action.activeFocus
            color: "transparent"
            radius: Theme.radiusSm
            border.width: 1
            border.color: Theme.signal
        }

        Row {
            id: actionRowText
            anchors.verticalCenter: parent.verticalCenter
            spacing: 3

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: action.label
                color: action.accent
                    ? (actionMouse.containsMouse && action.available
                        ? Theme.interactiveHover : Theme.signal)
                    : (actionMouse.containsMouse && action.available
                        ? Theme.signal : Theme.ink)
                font.pixelSize: Theme.typeSmall
                textFormat: Text.PlainText
            }

            Glyph {
                anchors.verticalCenter: parent.verticalCenter
                width: 12
                height: 12
                visible: action.chevron
                kind: "chevron"
                tint: actionMouse.containsMouse && action.available
                    ? Theme.signal : root.faintColor
            }
        }

        MouseArea {
            id: actionMouse
            anchors.fill: parent
            hoverEnabled: true
            enabled: action.available
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: action.clicked()
        }
    }

    Connections {
        target: controller

        function onPathChanged() {
            scroller.contentY = 0
        }
    }
}
