pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0
import "Localize.js" as Localize

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
    property color accent: Theme.accent
    /// Bumped by the tag store so `currentTags`/`allTags` re-read after a
    /// write or a definition change.
    property int tagRevision: 0

    readonly property color panelColor: Theme.canvas
    readonly property color sectionColor: Theme.inset
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color faintColor: Theme.inkTertiary
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
    /// far past what the card shows at a time, and it keeps a 64 KB peek from
    /// becoming a text-shaping job on a 300 px pane. `plainCapped` lets the
    /// card say so instead of ending mid-file without a word.
    readonly property string plainSource: controller.content !== ""
        ? controller.content : qsTr("（空文件）")
    readonly property var plainLines: plainSource.split("\n")
    readonly property bool plainCapped: plainLines.length > 500
    readonly property string plainText: plainCapped
        ? plainLines.slice(0, 500).join("\n") : plainSource
    readonly property string lineNumbers: {
        const count = Math.min(plainLines.length, 500)
        let out = ""
        for (let index = 1; index <= count; index += 1)
            out += (index === 1 ? "" : "\n") + index
        return out
    }
    /// The content card takes whatever the pane can spare once the details
    /// below it are laid out, so a taller window shows a taller image or text
    /// block instead of stopping at the old fixed 176 px.
    readonly property real previewHeight: Math.max(176,
        scroller.height - 28 - details.height - content.spacing)

    implicitWidth: 300

    signal closeRequested()
    signal openRequested(string path, bool isDir)
    signal propertiesRequested(string path)

    onDarkChanged: controller.setDark(root.dark)
    /// A new single selection opens details for it; the controller drops the
    /// previous hashes when it loads, so no digest can be attributed wrongly.
    onPathsChanged: if (root.paths.length === 1) detailsController.open(root.paths[0])
    Component.onCompleted: controller.setDark(root.dark)

    function showPaths(next) {
        trashItem = null
        paths = (next || []).slice()
        if (paths.length === 1)
            controller.open(paths[0])
        else
            controller.clear()
    }

    /// A one-line permission read-out. Unix keeps the mode in the low bits and
    /// resolves the names in the properties controller; Windows reports the
    /// DOS flags, which is the whole model it has.
    function permissionSummary() {
        if (detailsController.platformKind === "unix") {
            const mode = (detailsController.mode & 0o7777).toString(8)
            return "0" + ("000" + mode).slice(-4)
        }
        if (detailsController.platformKind === "windows") {
            const flags = []
            if (detailsController.readOnly)
                flags.push("R")
            if (detailsController.hidden)
                flags.push("H")
            if (detailsController.archive)
                flags.push("A")
            if (detailsController.system)
                flags.push("S")
            return flags.join(" ")
        }
        return ""
    }

    function ownerSummary() {
        if (detailsController.platformKind !== "unix")
            return ""
        if (detailsController.userName === "" && detailsController.groupName === "")
            return ""
        return detailsController.userName + ":" + detailsController.groupName
    }

    function showTrash(item) {
        paths = []
        trashItem = item && item.name !== undefined && item.name !== "" ? item : null
        controller.clear()
    }

    function clear() {
        showPaths([])
    }

    /// Scrolls the column by `dy` pixels. The quick-preview overlay maps its
    /// arrow and page keys here: in the docked pane the flickable is reachable
    /// with the pointer, but an overlay that traps focus has to move it too.
    function scrollBy(dy) {
        const maximum = Math.max(0, scroller.contentHeight - scroller.height)
        scroller.contentY = Math.max(0, Math.min(maximum, scroller.contentY + dy))
    }

    PreviewController {
        id: controller
    }

    /// Details is a second read of the same entry, and it is the properties
    /// controller rather than a copy: the created time, the mode and the
    /// hashes are the answers the properties dialog already knows how to get.
    PropertiesController {
        id: detailsController
    }

    /// The tags of the selection: for one entry its own, for several the ones
    /// they all share. Re-read whenever a write lands, so a chip toggled here
    /// is reflected without reopening the pane.
    readonly property var currentTags: {
        if (root.tagRevision >= 0 && root.paths.length > 0) {
            try {
                return JSON.parse(TagStore.commonTagsJson(root.paths))
            } catch (error) {
                return []
            }
        }
        return []
    }
    readonly property var allTags: {
        try {
            return JSON.parse(TagStore.tagsJson)
        } catch (error) {
            return []
        }
    }
    readonly property string multiSize: root.paths.length > 1
        ? Workspace.selectionSizeText(root.paths) : ""

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
        height: Theme.controlLg

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
            font.family: Theme.monoFamily
            font.pixelSize: Theme.typeMicro
            font.letterSpacing: 1.6
        }

        /// The pane keeps the previous entry on screen while the next one
        /// loads, so the wait is reported here instead of collapsing the
        /// content and then expanding it again.
        Text {
            anchors.right: closeButton.left
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            visible: controller.loading && !root.empty
            text: qsTr("读取中…")
            color: root.faintColor
            font.pixelSize: Theme.typeMicro
        }

        Item {
            id: closeButton
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: Theme.controlXs

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
            Item {
                width: parent.width
                height: 172
                visible: root.empty

                DriftSheet {
                    width: 82
                    anchors.centerIn: parent
                    anchors.verticalCenterOffset: -6
                    rotation: -3
                    category: "document"
                    shadowed: true
                }
            }

            Text {
                width: parent.width
                visible: root.empty
                text: qsTr("未选择文件")
                color: root.mutedColor
                font.family: Theme.uiFamily
                font.pixelSize: Theme.typeHeading
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                visible: root.empty
                text: root.inTrash ? qsTr("选中项目查看原位置") : qsTr("单击文件查看详情")
                color: root.faintColor
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                wrapMode: Text.Wrap
                lineHeight: Theme.leadingBody
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
                    font.pixelSize: Theme.typeHeading
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
            Item {
                width: parent.width
                height: 172
                visible: !root.empty && !root.single && root.trashItem === null

                /// drift prints the count in the serif, in the signal colour,
                /// at a size no control uses: the pane is reporting a number,
                /// not offering a button.
                /// The count alone: drift prints a number on the plate, and the
                /// line under it ("共 1.2 MiB") already says what was counted.
                /// A separate unit word only competed with the number's tail.
                Text {
                    id: multiCount
                    anchors.centerIn: parent
                    text: root.paths.length
                    color: Theme.accent
                    font.family: Theme.uiFamily
                    font.pixelSize: 96
                    font.features: { "tnum": 1 }
                    Accessible.role: Accessible.StaticText
                    Accessible.name: qsTr("已选 %1 项").arg(root.paths.length)
                }

            }

            Text {
                width: parent.width
                visible: !root.empty && !root.single && root.trashItem === null
                text: root.multiSize !== "" ? qsTr("共 %1").arg(root.multiSize)
                                            : qsTr("多选时不展开内容预览")
                color: root.faintColor
                font.pixelSize: Theme.typeCaption
                wrapMode: Text.Wrap
            }

            // ── One entry ─────────────────────────────────────────────────
            // Image, when the provider can make one and the file is not a
            // clip that the media player will take over.
            Rectangle {
                id: thumbnailCard
                visible: root.single && controller.hasThumbnail && controller.mediaKind === ""
                width: parent.width
                height: root.previewHeight
                radius: Theme.radiusLg
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                /// True while the card has a frame to paint: the current
                /// source as soon as it is ready, or the previous one while
                /// the next is still decoding (`retainWhileLoading`).
                readonly property bool showsImage: thumbnail.status === Image.Ready
                    || (thumbnail.status === Image.Loading && thumbnail.hadImage)

                Image {
                    id: thumbnail
                    anchors.fill: parent
                    anchors.margins: 12
                    source: root.thumbnailSource
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    cache: false
                    retainWhileLoading: true
                    visible: thumbnailCard.showsImage

                    property bool hadImage: false
                    onStatusChanged: if (status === Image.Ready) hadImage = true
                    onSourceChanged: if (source === "") hadImage = false
                }

                Text {
                    anchors.centerIn: parent
                    visible: !thumbnailCard.showsImage
                    text: thumbnail.status === Image.Error ? qsTr("无法生成缩略图") : qsTr("正在加载…")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                }
            }

            // Office documents: the extracted body of a .docx/.xlsx/.pptx, or
            // the one line that explains why there is none. The container is
            // read in the bridge; this card only draws what came back.
            Rectangle {
                id: officeCard

                visible: root.single && (controller.hasOffice || controller.officeError !== "")
                width: parent.width
                height: root.previewHeight
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    anchors.fill: parent
                    anchors.margins: 1
                    contentWidth: width
                    contentHeight: officeText.contentHeight + 16
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    TextEdit {
                        id: officeText

                        x: 10
                        y: 8
                        width: parent.width - 20
                        height: Math.max(contentHeight, parent.height - 16)
                        readOnly: true
                        selectByMouse: true
                        text: controller.hasOffice ? controller.officeHtml
                                                   : Localize.message(controller.officeError)
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: Theme.onAccent
                        font.pixelSize: Theme.typeMicro
                        textFormat: controller.hasOffice ? TextEdit.RichText
                                                         : TextEdit.PlainText
                        wrapMode: TextEdit.Wrap

                        Accessible.role: Accessible.StaticText
                        Accessible.name: qsTr("文档预览")
                        Accessible.description: controller.hasOffice
                            ? qsTr("Office 文档正文") : Localize.message(controller.officeError)
                    }
                }

                Text {
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 6
                    visible: controller.hasOffice && controller.officeTruncated
                    text: qsTr("内容过长，仅显示开头部分")
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
                height: root.previewHeight
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    id: plainScroller
                    anchors.fill: parent
                    anchors.margins: 1
                    contentWidth: Math.max(width, plainRows.implicitWidth)
                    contentHeight: Math.max(height, plainRows.implicitHeight)
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    flickableDirection: Flickable.HorizontalAndVerticalFlick

                    Row {
                        id: plainRows
                        spacing: 8

                        Text {
                            width: 28
                            text: root.lineNumbers
                            color: root.faintColor
                            horizontalAlignment: Text.AlignRight
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            lineHeight: Theme.leadingProse
                            font.features: { "tnum": 1 }
                        }

                        // Not elided: the end of a stack trace or a long path
                        // is the part being looked for, so the card scrolls
                        // sideways instead of cutting the line with "…".
                        Text {
                            text: root.plainText
                            color: root.textColor
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            lineHeight: Theme.leadingProse
                            wrapMode: Text.NoWrap
                            textFormat: Text.PlainText
                        }
                    }
                }

                Text {
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 6
                    visible: controller.truncated || root.plainCapped
                    text: controller.truncated
                        ? qsTr("仅显示前 64 KB")
                        : qsTr("仅显示前 500 行")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                }
            }

            // Rendered Markdown and highlighted code. They carry their own
            // colours, so they keep the text-edit path instead of the gutter.
            Rectangle {
                visible: root.single && controller.hasText && controller.contentHtml !== ""
                width: parent.width
                height: root.previewHeight
                radius: Theme.radiusSm
                color: root.sectionColor
                border.width: 1
                border.color: root.hairline
                clip: true

                Flickable {
                    id: richScroller
                    anchors.fill: parent
                    anchors.margins: 1
                    // Code blocks are laid out `white-space:pre`, so their
                    // width is the longest line's and this is what lets it be
                    // read; Markdown paragraphs still wrap to the pane.
                    contentWidth: Math.max(width, richPreview.contentWidth + 20)
                    contentHeight: Math.max(height, richPreview.contentHeight + 16)
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    flickableDirection: Flickable.HorizontalAndVerticalFlick

                    // Dragging inside a TextEdit selects text, so a mouse
                    // reaches the part of a long line past the edge with
                    // Shift+wheel; touchpads send horizontal deltas and land
                    // here too. Without it the horizontal scroll existed but
                    // no common device could drive it.
                    WheelHandler {
                        acceptedModifiers: Qt.ShiftModifier
                        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad

                        onWheel: function (event) {
                            const step = event.pixelDelta.x !== 0
                                ? event.pixelDelta.x
                                : (event.angleDelta.x !== 0
                                    ? event.angleDelta.x * 0.4
                                    : -event.angleDelta.y * 0.4)
                            const maximum = Math.max(0,
                                richScroller.contentWidth - richScroller.width)
                            richScroller.contentX = Math.max(0,
                                Math.min(maximum, richScroller.contentX + step))
                            event.accepted = true
                        }
                    }

                    TextEdit {
                        id: richPreview
                        Accessible.role: Accessible.StaticText
                        Accessible.name: qsTr("预览内容")
                        x: 10
                        y: 8
                        width: Math.max(richScroller.width - 20, richPreview.contentWidth)
                        height: Math.max(contentHeight, richScroller.height - 16)
                        readOnly: true
                        selectByMouse: true
                        text: controller.contentHtml
                        color: root.textColor
                        selectionColor: root.accent
                        selectedTextColor: Theme.onAccent
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
                        height: Theme.controlSm
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
                            id: childRow

                            required property string modelData
                            width: folderColumn.width
                            height: Theme.controlSm

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
                                    text: childRow.modelData
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
                height: active && item ? (item as MediaPreview).implicitHeight : 0
                active: root.single && controller.mediaKind !== ""
                    && controller.mediaUrl !== "" && root.visible
                source: active ? "MediaPreview.qml" : ""

                onLoaded: {
                    const preview = item as MediaPreview
                    preview.mediaUrl = Qt.binding(function () { return controller.mediaUrl })
                    preview.mediaKind = Qt.binding(function () { return controller.mediaKind })
                    preview.posterSource = Qt.binding(function () { return root.thumbnailSource })
                    preview.dark = Qt.binding(function () { return root.dark })
                    preview.accent = Qt.binding(function () { return root.accent })
                    preview.textColor = Qt.binding(function () { return root.textColor })
                    preview.mutedColor = Qt.binding(function () { return root.mutedColor })
                    preview.sectionColor = Qt.binding(function () { return root.sectionColor })
                    preview.hairline = Qt.binding(function () { return root.hairline })
                    preview.contentHeight = Qt.binding(function () { return root.previewHeight })
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

            /// Everything below the content card. Keeping it in one column is
            /// what lets `previewHeight` measure it: the old snapshot stays
            /// here while the next one loads, so these rows do not collapse
            /// and re-expand between selections.
            Column {
                id: details
                width: parent.width
                spacing: 12

                Text {
                    width: parent.width
                    visible: root.single && controller.mediaText !== ""
                    text: Localize.message(controller.mediaText)
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    lineHeight: Theme.leadingBody
                    wrapMode: Text.Wrap
                }

                Text {
                    width: parent.width
                    visible: root.single && Localize.message(controller.kindText) !== ""
                    text: Localize.message(controller.kindText)
                    color: Theme.accent
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    font.letterSpacing: 1.6
                    font.capitalization: Font.AllUppercase
                    textFormat: Text.PlainText
                }

                Text {
                    width: parent.width
                    visible: root.single
                    text: controller.name !== "" ? controller.name
                        : (root.paths.length === 1 ? root.paths[0].split(/[\\/]/).pop() : "")
                    color: root.textColor
                    font.family: Theme.uiFamily
                    font.pixelSize: 30
                    lineHeight: Theme.leadingTight
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
                    visible: root.single && controller.error === ""
                        && controller.path !== ""

                    Repeater {
                        model: [
                            [qsTr("类型"), Localize.message(controller.kindText)],
                            [qsTr("大小"), controller.sizeText],
                            [qsTr("修改时间"), detailsController.modifiedText],
                            [qsTr("创建时间"), detailsController.createdText],
                            [qsTr("权限"), root.permissionSummary()],
                            [qsTr("所有者"), root.ownerSummary()]
                        ].filter(function (row) { return row[1] !== "" })

                        MetaRow {
                            width: details.width
                            required property var modelData
                            key: modelData[0]
                            value: modelData[1]
                        }
                    }
                }

                /// Hashes are on demand: a 4 GB file is a minute of I/O, so
                /// the pane offers the action and reports progress rather than
                /// starting one on selection.
                Column {
                    width: parent.width
                    visible: root.single && detailsController.path !== ""
                        && !detailsController.isDirectory
                    spacing: 4

                    TextAction {
                        label: detailsController.hashing ? qsTr("正在计算…")
                            : (detailsController.hashDone ? qsTr("重新计算哈希")
                                                          : qsTr("计算哈希"))
                        available: !detailsController.hashing
                        onClicked: detailsController.startHashing()
                    }

                    Repeater {
                        model: [
                            ["MD5", detailsController.hashMd5],
                            ["SHA-1", detailsController.hashSha1],
                            ["SHA-256", detailsController.hashSha256]
                        ].filter(function (row) { return row[1] !== "" })

                        Text {
                            width: parent.width
                            required property var modelData
                            text: modelData[0] + "  " + modelData[1]
                            color: root.faintColor
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.typeMicro
                            elide: Text.ElideMiddle
                            textFormat: Text.PlainText
                        }
                    }

                    Text {
                        width: parent.width
                        visible: detailsController.hashError !== ""
                        text: Localize.message(detailsController.hashError)
                        color: Theme.danger
                        font.pixelSize: Theme.typeMicro
                        wrapMode: Text.Wrap
                    }
                }

                /// Tags: every definition is a chip; the ones this entry (or
                /// this whole selection) carries are filled. A click toggles
                /// the tag over the whole selection.
                Flow {
                    width: parent.width
                    visible: root.paths.length > 0 && root.allTags.length > 0
                    spacing: 6

                    Repeater {
                        model: root.allTags

                        Rectangle {
                            id: tagChip
                            required property var modelData
                            readonly property bool on: {
                                for (let index = 0; index < root.currentTags.length; index += 1) {
                                    if (root.currentTags[index].id === modelData.id)
                                        return true
                                }
                                return false
                            }

                            width: chipRow.implicitWidth + 16
                            height: Theme.controlXs + 4
                            radius: Theme.radiusXs
                            color: on ? Theme.hover : "transparent"
                            border.width: 1
                            border.color: on ? Theme.line : Theme.hairline
                            opacity: tagMouse.containsMouse ? 1 : (on ? 1 : 0.72)

                            Row {
                                id: chipRow
                                anchors.centerIn: parent
                                spacing: 5

                                Rectangle {
                                    width: 6
                                    height: 6
                                    radius: 3
                                    anchors.verticalCenter: parent.verticalCenter
                                    color: tagChip.modelData.color
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: tagChip.modelData.name
                                    color: tagChip.on ? Theme.ink : Theme.inkSecondary
                                    font.pixelSize: Theme.typeMicro
                                    textFormat: Text.PlainText
                                }
                            }

                            MouseArea {
                                id: tagMouse
                                Accessible.role: Accessible.CheckBox
                                Accessible.name: tagChip.modelData.name
                                Accessible.checkable: true
                                Accessible.checked: tagChip.on
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: TagStore.toggleTag(root.paths, tagChip.modelData.id)
                            }
                        }
                    }
                }

                /// The count and the measured size already sit under the big
                /// number above, so only the shared-tag state is left to say
                /// here; repeating the totals in a 300 px pane is the noise
                /// the prototype's single "共 1.2 MB" line avoids.
                Column {
                    width: parent.width
                    visible: root.paths.length > 1
                    spacing: 4

                    Text {
                        width: parent.width
                        visible: root.currentTags.length === 0
                        text: qsTr("没有共同标签")
                        color: root.faintColor
                        font.pixelSize: Theme.typeMicro
                    }
                }

                Text {
                    width: parent.width
                    visible: root.single && controller.error !== ""
                    text: Localize.message(controller.error)
                    color: Theme.danger
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                }

                Text {
                    width: parent.width
                    visible: root.single && controller.remote
                        && controller.error === ""
                    text: qsTr("远程项目：可预览文本；缩略图和媒体信息仅支持本地文件。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                    lineHeight: Theme.leadingBody
                }

                // ── Actions ───────────────────────────────────────────────
                // Quiet text, not buttons: the pane is a read-out, and the one
                // saturated colour in it belongs to `全部属性`.
                Row {
                    id: actionRow
                    width: parent.width
                    height: Theme.controlMd
                    spacing: 16
                    visible: root.single && controller.path !== ""
                    enabled: !controller.loading
                    opacity: controller.loading ? 0.4 : 1

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
    }

    /// One metadata line: quiet key on the left, value in the reading ink on
    /// the right, separated by the lightest rule in the palette.
    component MetaRow: Item {
        id: metaRow

        property string key: ""
        property string value: ""

        width: parent ? parent.width : 0
        height: Theme.controlSm

        /// A dashed rule, the way drift separates a metadata row: a table of
        /// print, not a spreadsheet. Canvas because Rectangle has no dashed
        /// border.
        Canvas {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1

            onPaint: {
                const ctx = getContext("2d")
                ctx.setTransform(1, 0, 0, 1, 0, 0)
                ctx.clearRect(0, 0, width, height)
                ctx.strokeStyle = Theme.hairline
                ctx.lineWidth = 1
                ctx.setLineDash([2, 3])
                ctx.beginPath()
                ctx.moveTo(0, 0.5)
                ctx.lineTo(width, 0.5)
                ctx.stroke()
            }

            onWidthChanged: requestPaint()

            Connections {
                target: Theme
                function onDarkChanged() { requestPaint() }
            }
        }

        Text {
            id: keyLabel
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(keyLabel.implicitWidth, metaRow.width * 0.42)
            text: metaRow.key
            color: Theme.inkTertiary
            font.family: Theme.monoFamily
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
        implicitHeight: Theme.controlMd
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
            border.color: Theme.accent
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
                        ? Theme.accentHover : Theme.accent)
                    : (actionMouse.containsMouse && action.available
                        ? Theme.accent : Theme.ink)
                font.pixelSize: Theme.typeCaption
                textFormat: Text.PlainText
            }

            Glyph {
                anchors.verticalCenter: parent.verticalCenter
                width: 12
                height: 12
                visible: action.chevron
                kind: "chevron"
                tint: actionMouse.containsMouse && action.available
                    ? Theme.accent : root.faintColor
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

    Connections {
        target: TagStore

        function onTagsChanged(paths) {
            root.tagRevision += 1
        }

        function onDefinitionsChanged() {
            root.tagRevision += 1
        }
    }
}
