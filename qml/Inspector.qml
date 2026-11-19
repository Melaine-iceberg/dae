pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0
import "Localize.js" as Localize

// The docked inspector (proposal §11.5).
//
// It is off by default and opens on Ctrl+I or the status bar button. With a
// selection it describes that selection; with none it describes the directory
// on screen, because an empty panel wastes the 300 dp it occupies. The three
// states are therefore directory, single and multiple — plus a recycle-bin row
// whose file no longer exists on disk and cannot be described by the
// controller.
//
// The pane sits on `chrome`, not on a raised plate: it is the fourth column of
// the window, not a card over it. Key/value rows are 12 px type separated by
// solid hairlines, and the preview well is the only inset inside the column.
Item {
    id: root

    property var paths: []
    /// The rows around the selection, in the order the shell expects to reach
    /// them. `showPaths` receives them from the listing model; the Space
    /// overlay has no model to ask and leaves them empty.
    property var neighbors: []
    /// One recycle-bin row, or null. The trash lists entries that no longer
    /// exist on disk, so the controller cannot describe them; the shell hands
    /// the row's strings over instead.
    property var trashItem: null
    property bool inTrash: false
    /// The listing on screen, or null outside a folder. The no-selection state
    /// reads the directory's own numbers from it rather than restating them in
    /// the shell.
    property var listing: null
    /// `selectionBreakdown()` of the list that produced `paths`: the folder and
    /// family counts for the multi-selection state. Empty when the shell has no
    /// model to ask (a history restore, a search result).
    property string breakdown: ""
    /// Recycle-bin item count, for the no-selection state in trash.
    property int trashCount: 0
    property bool dark: true
    property color accent: Theme.accent
    /// True when drawn on a plate of its own — the Space overlay — rather than
    /// as the window's fourth column. The column paints `chrome` and a left
    /// hairline; the plate is already `canvas` with a border and rounded
    /// corners, so the pane leaves both to it.
    property bool plate: false
    /// The preview well is 128 dp in the column (§6.3.1). The overlay has the
    /// whole window and gives its well more room.
    property int wellHeight: 128
    /// Bumped by the tag store so `currentTags`/`allTags` re-read after a
    /// write or a definition change.
    property int tagRevision: 0

    readonly property color panelColor: root.plate ? "transparent" : Theme.chrome
    readonly property color wellColor: Theme.inset
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color faintColor: Theme.inkTertiary
    readonly property color hairline: Theme.hairline

    readonly property bool single: paths.length === 1 && trashItem === null
    readonly property bool multi: paths.length > 1 && trashItem === null
    readonly property bool empty: paths.length === 0 && trashItem === null
    readonly property bool directoryState: root.empty && !root.inTrash
    readonly property bool trashState: root.empty && root.inTrash

    /// The directory's numbers. `listing` is a `DirectoryListModel`; the
    /// properties are read through bindings, so a finish after the rows stream
    /// in updates the pane without the shell forwarding each count.
    readonly property int directoryItems: root.listing !== null ? root.listing.loaded : 0
    readonly property int directoryFolders: root.listing !== null ? root.listing.folders : 0
    readonly property int directoryFiles: root.listing !== null ? root.listing.files : 0
    readonly property string directoryPath: root.listing !== null ? root.listing.path : ""
    readonly property bool directoryLoading: root.listing !== null && !root.listing.finished
    readonly property string directoryGitBranch: root.listing !== null ? root.listing.gitBranch : ""
    readonly property string directoryGitRoot: root.listing !== null ? root.listing.gitRoot : ""
    readonly property bool directoryRemote: root.listing !== null && root.listing.remote
    readonly property string directoryName: {
        if (root.listing === null)
            return ""
        // qmllint disable missing-property
        const names = root.listing.breadcrumbNames
        if (names !== undefined && names.length > 0)
            return names[names.length - 1]
        // qmllint enable missing-property
        const parts = root.directoryPath.split(/[\\/]/).filter(function (part) {
            return part !== ""
        })
        return parts.length > 0 ? parts[parts.length - 1] : root.directoryPath
    }
    /// File families of the finished listing: `[{ id, count }, …]`.
    readonly property var directoryFamilies: {
        if (root.listing === null)
            return []
        // qmllint disable missing-property
        const raw = root.listing.familyCounts
        // qmllint enable missing-property
        if (!raw)
            return []
        try {
            return JSON.parse(raw)
        } catch (error) {
            return []
        }
    }

    /// The multi-selection distribution, as the list model reported it.
    readonly property var selectionFamilies: {
        if (root.breakdown === "")
            return []
        try {
            const parsed = JSON.parse(root.breakdown)
            return parsed.families !== undefined ? parsed.families : []
        } catch (error) {
            return []
        }
    }
    readonly property int selectionFolders: {
        if (root.breakdown === "")
            return 0
        try {
            const parsed = JSON.parse(root.breakdown)
            return parsed.folders !== undefined ? parsed.folders : 0
        } catch (error) {
            return 0
        }
    }

    /// The entry was read through a network backend (or an archive view).
    /// Text and image previews work — images are pulled bounded through the
    /// vfs layer — but media tags and OS open/reveal do not.
    readonly property bool remote: controller.remote
    readonly property string thumbnailSource: root.single && controller.hasThumbnail
        && controller.thumbnailRevision !== ""
        ? "image://thumbnail/" + encodeURIComponent(controller.path)
          + "?size=384&v=" + encodeURIComponent(controller.thumbnailRevision)
        : ""

    /// Plain text keeps its line numbers; rendered Markdown and highlighted
    /// code arrive as HTML and are shown as they are. Five hundred lines is
    /// far past what the well shows at a time, and it keeps a 64 KB peek from
    /// becoming a text-shaping job on a 300 px pane. `plainCapped` lets the
    /// well say so instead of ending mid-file without a word.
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

    /// Which kind of well the single state draws, in priority order. The
    /// thumbnail is skipped for clips the media player takes over, and the
    /// fallback marks a type that has no inner preview at all.
    readonly property bool showsMedia: root.single && controller.mediaKind !== ""
        && controller.mediaUrl !== ""
    readonly property bool showsThumbnail: root.single && controller.hasThumbnail
        && controller.mediaKind === ""
    readonly property bool showsOffice: root.single && (controller.hasOffice
        || controller.officeError !== "")
    readonly property bool showsText: root.single && controller.hasText
        && controller.contentHtml === ""
    readonly property bool showsRich: root.single && controller.hasText
        && controller.contentHtml !== ""
    readonly property bool showsFolder: root.single && controller.isDirectory
    readonly property bool showsFallback: root.single && !root.showsMedia
        && !root.showsThumbnail && !root.showsOffice && !root.showsText
        && !root.showsRich && !root.showsFolder && controller.path !== ""

    /// The directory total is only measured when asked: a deep tree is a
    /// minute of I/O, so the row starts as an action instead of a number.
    property real directorySizeBytes: -1
    property bool measuring: false

    implicitWidth: 300

    signal closeRequested()
    signal openRequested(string path, bool isDir)
    signal propertiesRequested(string path)

    onDarkChanged: controller.setDark(root.dark)
    onDirectoryPathChanged: {
        root.directorySizeBytes = -1
        root.measuring = false
    }
    /// A new single selection opens the preview and its details — the
    /// controller drops the previous hashes when it loads, so no digest can
    /// be attributed wrongly — and queues the neighbours for preload. The
    /// trigger lives here rather than in `showPaths` because the Space
    /// overlay binds `paths` directly instead of calling the function.
    onPathsChanged: {
        if (root.paths.length === 1) {
            detailsController.open(root.paths[0])
            controller.open(root.paths[0])
            controller.preload(root.neighbors)
        } else {
            controller.clear()
        }
    }
    Component.onCompleted: controller.setDark(root.dark)

    /// §8.3: where F6 puts focus when the ring reaches the inspector. The
    /// pane itself is a column of text, so the scroll surface is the stop.
    function takeFocus() {
        scroller.forceActiveFocus()
    }

    function showPaths(next, breakdownJson, neighborPaths) {
        trashItem = null
        breakdown = breakdownJson !== undefined ? breakdownJson : ""
        neighbors = (neighborPaths || []).slice()
        paths = (next || []).slice()
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

    /// The same ladder `dae-qt::format::size_with()` draws, in the units the
    /// user chose. Only the directory summary needs it in QML: every row size
    /// was already formatted by the backend.
    function sizeText(bytes) {
        if (bytes === undefined || bytes === null || bytes < 0)
            return ""
        const decimal = ExplorerPrefs.sizeUnits !== "binary"
        const base = decimal ? 1000 : 1024
        const labels = decimal
            ? ["B", "KB", "MB", "GB", "TB"] : ["B", "KiB", "MiB", "GiB", "TiB"]
        if (bytes < base)
            return bytes + " B"
        let value = bytes
        let unit = 0
        while (value >= base && unit + 1 < labels.length) {
            value /= base
            unit += 1
        }
        return value.toFixed(1) + " " + labels[unit]
    }

    function measureDirectorySize() {
        if (root.directoryPath === "" || root.directoryRemote || root.measuring)
            return
        root.measuring = true
        Workspace.measureFolders([root.directoryPath])
    }

    function showTrash(item) {
        paths = []
        breakdown = ""
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
    /// controller rather than a copy: the created time, the mode and the owner
    /// are the answers the properties dialog already knows how to get.
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
            width: Theme.ruleWidth(Screen.devicePixelRatio)
            visible: !root.plate
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
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: root.hairline
        }

        Text {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("检查器")
            color: root.faintColor
            font.family: Theme.uiFamily
            font.pixelSize: Theme.typeCaption
            font.weight: Font.Medium
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
            width: Theme.controlXs
            height: Theme.controlXs

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("关闭检查器")

            activeFocusOnTab: true
            Keys.onReturnPressed: root.closeRequested()
            Keys.onSpacePressed: root.closeRequested()

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
        /// §8.3: the inspector is one stop in the F6 ring, so it has to be
        /// able to hold focus; its own rows are text, so the scroller is the
        /// stop and the arrow keys scroll it.
        activeFocusOnTab: true
        Keys.onUpPressed: function (event) {
            scroller.contentY = Math.max(0, scroller.contentY - 40)
            event.accepted = true
        }
        Keys.onDownPressed: function (event) {
            scroller.contentY = Math.min(
                Math.max(0, scroller.contentHeight - scroller.height),
                scroller.contentY + 40)
            event.accepted = true
        }
        Keys.onPressed: function (event) {
            if (event.key === Qt.Key_PageUp) {
                scroller.contentY = Math.max(0, scroller.contentY - scroller.height)
                event.accepted = true
            } else if (event.key === Qt.Key_PageDown) {
                scroller.contentY = Math.min(
                    Math.max(0, scroller.contentHeight - scroller.height),
                    scroller.contentY + scroller.height)
                event.accepted = true
            }
        }

        Column {
            id: content
            x: 14
            y: 14
            width: scroller.width - 28
            spacing: 12

            // ── No selection: the directory on screen ─────────────────────
            Column {
                width: parent.width
                spacing: 12
                visible: root.directoryState

                Text {
                    width: parent.width
                    text: root.directoryName
                    color: root.textColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                }

                Text {
                    width: parent.width
                    text: root.directoryPath
                    color: root.faintColor
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                }

                Column {
                    width: parent.width
                    visible: root.listing !== null

                    MetaRow {
                        width: content.width
                        key: qsTr("项数")
                        value: root.directoryLoading
                            ? qsTr("读取中…") : qsTr("%1 项").arg(root.directoryItems)
                    }
                }

                Text {
                    width: parent.width
                    visible: root.listing !== null && !root.directoryLoading
                    text: qsTr("类型分布")
                    color: root.faintColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeMicro
                }

                Column {
                    width: parent.width
                    visible: root.listing !== null && !root.directoryLoading
                        && (root.directoryFolders > 0 || root.directoryFamilies.length > 0)

                    MetaRow {
                        width: content.width
                        visible: root.directoryFolders > 0
                        key: Localize.familyName("folder")
                        value: root.directoryFolders
                    }

                    Repeater {
                        model: root.directoryFamilies

                        MetaRow {
                            width: content.width
                            required property var modelData
                            key: Localize.familyName(modelData.id)
                            value: modelData.count
                        }
                    }
                }

                Text {
                    width: parent.width
                    visible: root.listing !== null && !root.directoryLoading
                        && root.directoryFolders === 0 && root.directoryFamilies.length === 0
                    text: qsTr("空文件夹")
                    color: root.faintColor
                    font.pixelSize: Theme.typeCaption
                }

                Column {
                    width: parent.width
                    visible: root.listing !== null

                    MetaRow {
                        width: content.width
                        key: qsTr("总大小")
                        value: root.measuring
                            ? qsTr("正在计算…")
                            : root.sizeText(root.directorySizeBytes)
                        actionLabel: !root.measuring && root.directorySizeBytes < 0
                            && !root.directoryRemote ? qsTr("计算") : ""
                        onActionClicked: root.measureDirectorySize()
                    }

                    MetaRow {
                        width: content.width
                        key: "Git"
                        value: root.directoryGitRoot !== ""
                            ? root.directoryGitBranch : ""
                    }
                }

                Text {
                    width: parent.width
                    visible: root.directoryRemote
                    text: qsTr("远程位置：缩略图与文件夹大小仅支持本地路径。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                    lineHeight: Theme.leadingBody
                }
            }

            // ── No selection in the recycle bin ───────────────────────────
            Column {
                width: parent.width
                spacing: 12
                visible: root.trashState

                Text {
                    width: parent.width
                    text: qsTr("回收站")
                    color: root.textColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                }

                Column {
                    width: parent.width

                    MetaRow {
                        width: content.width
                        key: qsTr("项数")
                        value: qsTr("%1 项").arg(root.trashCount)
                    }
                }

                Text {
                    width: parent.width
                    text: qsTr("选中项目查看原位置")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                }
            }

            // ── A recycle-bin row ─────────────────────────────────────────
            Column {
                width: parent.width
                spacing: 12
                visible: root.trashItem !== null

                Column {
                    width: parent.width
                    spacing: 3

                    Text {
                        width: parent.width
                        text: root.trashItem !== null ? root.trashItem.name : ""
                        color: root.textColor
                        font.pixelSize: Theme.typeHeading
                        font.weight: Font.Medium
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
            }

            // ── Several selected: 3 items · 56.6 KB ───────────────────────
            Column {
                width: parent.width
                spacing: 12
                visible: root.multi

                Text {
                    width: parent.width
                    text: root.multiSize !== ""
                        ? qsTr("%1 项").arg(root.paths.length) + " · " + root.multiSize
                        : qsTr("%1 项").arg(root.paths.length)
                    color: root.textColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                    font.features: { "tnum": 1 }
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.StaticText
                    Accessible.name: qsTr("已选 %1 项").arg(root.paths.length)
                }

                Text {
                    width: parent.width
                    visible: root.selectionFamilies.length > 0
                        || root.selectionFolders > 0
                    text: qsTr("类型分布")
                    color: root.faintColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeMicro
                }

                Column {
                    width: parent.width
                    visible: root.selectionFamilies.length > 0
                        || root.selectionFolders > 0

                    MetaRow {
                        width: content.width
                        visible: root.selectionFolders > 0
                        key: Localize.familyName("folder")
                        value: root.selectionFolders
                    }

                    Repeater {
                        model: root.selectionFamilies

                        MetaRow {
                            width: content.width
                            required property var modelData
                            key: Localize.familyName(modelData.id)
                            value: modelData.count
                        }
                    }
                }

                TagFlow {
                }

                Text {
                    width: parent.width
                    visible: root.currentTags.length === 0
                    text: qsTr("没有共同标签")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                }
            }

            // ── One entry ─────────────────────────────────────────────────
            Column {
                width: parent.width
                spacing: 12
                visible: root.single

                // Image, when the provider can make one and the file is not a
                // clip that the media player will take over.
                Rectangle {
                    id: thumbnailCard
                    visible: root.showsThumbnail
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
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
                        anchors.margins: 8
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

                // Office documents: the extracted body of a .docx/.xlsx/.pptx,
                // or the one line that explains why there is none. The
                // container is read in the bridge; this well only draws what
                // came back.
                Rectangle {
                    id: officeCard

                    visible: root.showsOffice
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
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
                    visible: root.showsText
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
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
                                width: 24
                                text: root.lineNumbers
                                color: root.faintColor
                                horizontalAlignment: Text.AlignRight
                                font.family: Theme.monoFamily
                                font.pixelSize: Theme.typeMicro
                                lineHeight: Theme.leadingProse
                                font.features: { "tnum": 1 }
                            }

                            // Not elided: the end of a stack trace or a long
                            // path is the part being looked for, so the well
                            // scrolls sideways instead of cutting the line.
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
                // colours, so they keep the text-edit path instead of the
                // gutter.
                Rectangle {
                    visible: root.showsRich
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
                    border.width: 1
                    border.color: root.hairline
                    clip: true

                    Flickable {
                        id: richScroller
                        anchors.fill: parent
                        anchors.margins: 1
                        // Code blocks are laid out `white-space:pre`, so their
                        // width is the longest line's and this is what lets it
                        // be read; Markdown paragraphs still wrap to the pane.
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

                // A folder: a large mark and how much is in it. The names
                // themselves belong to the listing, not to a 128 dp well.
                Rectangle {
                    visible: root.showsFolder
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
                    border.width: 1
                    border.color: root.hairline
                    clip: true

                    Column {
                        anchors.centerIn: parent
                        spacing: 6

                        FileIcon {
                            anchors.horizontalCenter: parent.horizontalCenter
                            /// §6.3.2: the inspector's well uses the family
                            /// glyph at 64–96 dp, so the preview and the row
                            /// show the same shape at two scales.
                            edge: 72
                            category: "folder"
                            path: root.directoryPath
                            outline: Theme.inkSecondary
                        }

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: controller.loading
                                ? qsTr("读取中…")
                                : (controller.childCount === 0
                                    ? qsTr("空文件夹")
                                    : qsTr("%1 项").arg(controller.childCount))
                            color: root.faintColor
                            font.pixelSize: Theme.typeCaption
                            font.features: { "tnum": 1 }
                        }
                    }
                }

                // A type with no inner preview: the family mark and the type
                // name. The well stays in the layout so the rows below do not
                // jump between selections.
                Rectangle {
                    visible: root.showsFallback
                    width: parent.width
                    height: root.wellHeight
                    radius: Theme.radiusSm
                    color: root.wellColor
                    border.width: 1
                    border.color: root.hairline
                    clip: true

                    Column {
                        anchors.centerIn: parent
                        spacing: 6

                        FileIcon {
                            anchors.horizontalCenter: parent.horizontalCenter
                            /// §6.3.2: a file with no inner preview still gets
                            /// its own outline and family mark, at the
                            /// inspector's 64–96 dp size. The family comes from
                            /// the properties controller, which is what knows
                            /// the entry's category; the preview controller
                            /// only knows how to draw one.
                            edge: 72
                            category: detailsController.category !== ""
                                ? detailsController.category : "file"
                            path: detailsController.path
                            outline: Theme.inkTertiary
                        }

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: Math.min(implicitWidth, root.width - 48)
                            horizontalAlignment: Text.AlignHCenter
                            text: Localize.message(controller.kindText) !== ""
                                ? Localize.message(controller.kindText) : qsTr("文件")
                            color: root.faintColor
                            font.pixelSize: Theme.typeMicro
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }
                    }
                }

                Loader {
                    id: mediaLoader
                    width: content.width
                    height: active && item ? (item as MediaPreview).implicitHeight : 0
                    active: root.showsMedia && root.visible
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
                        preview.sectionColor = Qt.binding(function () { return root.wellColor })
                        preview.hairline = Qt.binding(function () { return root.hairline })
                        preview.contentHeight = Qt.binding(function () { return root.wellHeight })
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
                    visible: root.showsMedia
                    text: Localize.message(controller.mediaText)
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    lineHeight: Theme.leadingBody
                    wrapMode: Text.Wrap
                }

                Text {
                    width: parent.width
                    text: controller.name !== "" ? controller.name
                        : (root.paths.length === 1 ? root.paths[0].split(/[\\/]/).pop() : "")
                    color: root.textColor
                    font.family: Theme.uiFamily
                    font.pixelSize: Theme.typeHeading
                    font.weight: Font.Medium
                    lineHeight: Theme.leadingTight
                    wrapMode: Text.Wrap
                }

                Text {
                    width: parent.width
                    text: controller.path
                    color: root.faintColor
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                }

                Column {
                    width: parent.width
                    visible: controller.error === "" && controller.path !== ""

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
                            width: content.width
                            required property var modelData
                            key: modelData[0]
                            value: modelData[1]
                        }
                    }
                }

                TagFlow {
                }

                Text {
                    width: parent.width
                    visible: controller.error !== ""
                    text: Localize.message(controller.error)
                    color: Theme.danger
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                }

                Text {
                    width: parent.width
                    visible: controller.remote && controller.error === ""
                    text: qsTr("远程项目：可预览文本；缩略图和媒体信息仅支持本地文件。")
                    color: root.faintColor
                    font.pixelSize: Theme.typeMicro
                    wrapMode: Text.Wrap
                    lineHeight: Theme.leadingBody
                }

                // Quiet text, not buttons: the pane is a read-out, and the one
                // saturated colour in it belongs to `全部属性`.
                Row {
                    id: actionRow
                    width: parent.width
                    height: Theme.controlMd
                    spacing: 16
                    visible: controller.path !== "" && !root.plate
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

    /// The tags of the selection, as pills (§6.1.7): the eight tag colours are
    /// shared by the list's 6 dp dots and these. Only the tags they all carry
    /// are lit; a click toggles that tag on every selected entry at once, which
    /// is why the multi state can write tags at all. Placement is the design's:
    /// after the facts, before the actions, in both one and several states.
    component TagFlow: Flow {
        width: parent ? parent.width : 0
        spacing: 6
        visible: root.paths.length > 0 && root.allTags.length > 0

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

                width: chipRow.implicitWidth + 18
                height: Theme.controlXs + 2
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
                        color: Theme.tagColorFor(tagChip.modelData.color)
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

    /// One metadata line: 12 px quiet key on the left, 12 px reading value on
    /// the right, separated by a solid hairline (§11.5). `actionLabel` turns
    /// the value side into a quiet text action when the row is on demand —
    /// the directory total is the one such row.
    component MetaRow: Item {
        id: metaRow

        property string key: ""
        property var value: ""
        property string actionLabel: ""

        signal actionClicked()

        width: parent ? parent.width : 0
        height: Theme.controlSm

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            /// §12.5: the rule is one device pixel tall, and it has to *sit*
            /// on one too. The row's bottom is not reliably on the grid — the
            /// pane's content starts at 14 dp and text heights are fractional
            /// — so the top is snapped instead of anchored: at 125 % a
            /// fractional bottom smears a 0.8 dp rule across two rows.
            y: Math.round((parent.height - height) * Screen.devicePixelRatio)
               / Screen.devicePixelRatio
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: Theme.hairline
        }

        Text {
            id: keyLabel
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, metaRow.width * 0.42)
            text: metaRow.key
            color: Theme.inkTertiary
            font.family: Theme.uiFamily
            font.pixelSize: Theme.typeCaption
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
            visible: metaRow.actionLabel === ""
            text: metaRow.value
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeCaption
            font.features: { "tnum": 1 }
            elide: Text.ElideMiddle
            textFormat: Text.PlainText
        }

        Text {
            id: actionLabelText
            anchors.left: keyLabel.right
            anchors.leftMargin: 12
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            horizontalAlignment: Text.AlignRight
            visible: metaRow.actionLabel !== ""
            text: metaRow.actionLabel
            color: actionMouse.containsMouse ? Theme.accentHover : Theme.accent
            font.pixelSize: Theme.typeCaption
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        MouseArea {
            id: actionMouse
            anchors.fill: parent
            enabled: metaRow.actionLabel !== ""
            hoverEnabled: enabled
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor

            // Only the on-demand rows are controls; a plain key/value line is
            // announced by the two Texts it is made of. The name still follows
            // the action so the button does not read as its key.
            Accessible.role: Accessible.Button
            Accessible.name: metaRow.actionLabel !== ""
                ? metaRow.actionLabel : metaRow.key

            onClicked: metaRow.actionClicked()
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

        /// The inspector's links are the only things in the panel that can be
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

    /// The inspector is one column of content under its header, so it takes
    /// one bar over the same range as the scroller.
    ScrollBar {
        target: scroller
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
    }

    Connections {
        target: controller

        function onPathChanged() {
            scroller.contentY = 0
        }
    }

    /// The directory total arrives on the same signal the listing's folder
    /// sizes use; only the path this pane asked about is adopted.
    Connections {
        target: Workspace

        function onFolderSizesMeasured(sizes) {
            if (!root.measuring)
                return
            root.measuring = false
            let entries = []
            try {
                entries = JSON.parse(sizes)
            } catch (error) {
                return
            }
            for (let index = 0; index < entries.length; ++index) {
                if (entries[index].path === root.directoryPath) {
                    root.directorySizeBytes = entries[index].size
                    return
                }
            }
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
