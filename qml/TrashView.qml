pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The recycle bin: newest deletion first, with restore, purge and empty.
//
// This view only paints and collects intent. The commands and their
// confirmation live on ShellWindow, next to every other dialog; the rows come
// from `TrashModel`, which does the blocking scan off the Qt thread.
Item {
    id: root

    property var model: null
    property bool dark: true
    property color accent: Theme.interactive
    property color headerColor: Theme.chrome

    /// id -> true. Ids are the system trash ids; a restored entry no longer
    /// exists, so selection is never allowed to outlive a reload.
    property var selected: ({})
    property int selectionVersion: 0
    property int anchorIndex: -1

    readonly property int rowHeight: 32

    /// Below this width the four columns cannot all be read: the name column
    /// would be squeezed under the original location and the two would paint on
    /// top of each other. The pane is narrow whenever the preview is open, so
    /// the location column steps out and the name takes its room.
    readonly property bool compactColumns: width < 560
    /// 54 for the checkbox and the name gutter, 228 for what the three trailing
    /// columns need at their smallest.
    readonly property int nameColumnSpan: 282
    readonly property color contentColor: Theme.canvas
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color hairline: Theme.hairline
    readonly property color selectionColor: Qt.rgba(accent.r, accent.g, accent.b,
                                                   dark ? 0.30 : 0.18)
    readonly property color cursorColor: Theme.canvasAlt

    readonly property int selectionCount: {
        const version = root.selectionVersion
        if (version < 0)
            return 0
        let count = 0
        for (const key in root.selected) {
            if (root.selected[key])
                count += 1
        }
        return count
    }
    readonly property bool allSelected: root.model !== null
        && root.model.count > 0 && root.selectionCount === root.model.count

    signal restoreRequested(var ids)
    signal purgeRequested(var ids)
    signal emptyRequested()
    signal contextRequested(string id, string originalParent, real x, real y)
    /// The preview pane's row, or null when nothing (or several things) is
    /// selected. Trash entries no longer exist on disk, so the pane cannot ask
    /// the filesystem about them; these strings are the whole description.
    signal previewRequested(var item)

    /// Pushes the cursor's row to the preview. The cursor is the row a click or
    /// an arrow key landed on, which is the one the user means when exactly one
    /// entry is selected.
    function pushPreview() {
        if (root.model === null || root.selectionCount !== 1 || list.currentIndex < 0) {
            root.previewRequested(null)
            return
        }
        const index = list.currentIndex
        root.previewRequested({
            "name": root.model.nameAt(index),
            "originalParent": root.model.originalParentAt(index),
            "deletedText": root.model.deletedTextAt(index),
            "sizeText": root.model.sizeTextAt(index),
            "isDir": root.model.isDirAt(index)
        })
    }

    onSelectionVersionChanged: root.pushPreview()
    onModelChanged: root.pushPreview()

    Connections {
        target: list

        function onCurrentIndexChanged() {
            root.pushPreview()
        }
    }

    Connections {
        target: root.model

        function onCountChanged() {
            root.pushPreview()
        }
    }

    function selectedIdList() {
        const ids = []
        for (const key in selected) {
            if (selected[key])
                ids.push(key)
        }
        return ids
    }

    function isSelected(id) {
        return id !== "" && selected[id] === true
    }

    function clearSelection() {
        selected = ({})
        anchorIndex = -1
        selectionVersion += 1
    }

    function selectOnly(id) {
        const next = ({})
        if (id !== "")
            next[id] = true
        selected = next
        selectionVersion += 1
    }

    function selectMany(ids) {
        const next = ({})
        for (let index = 0; index < ids.length; index += 1) {
            if (ids[index] !== "")
                next[ids[index]] = true
        }
        selected = next
        selectionVersion += 1
    }

    function toggle(id) {
        if (id === "")
            return
        const next = ({})
        for (const key in selected) {
            if (selected[key])
                next[key] = true
        }
        if (next[id])
            delete next[id]
        else
            next[id] = true
        selected = next
        selectionVersion += 1
    }

    function idAt(index) {
        return root.model !== null ? root.model.idAt(index) : ""
    }

    function currentId() {
        return list.currentIndex >= 0 ? root.idAt(list.currentIndex) : ""
    }

    /// Delete acts on the whole selection when the cursor is inside it,
    /// otherwise only on the cursor — the same rule the file list uses, so a
    /// Delete press never destroys rows the user can see highlighted.
    function targetIds() {
        const active = root.currentId()
        if (active === "")
            return []
        return root.isSelected(active) ? root.selectedIdList() : [active]
    }

    function takeFocus() {
        if (root.model !== null && list.currentIndex < 0 && root.model.count > 0)
            list.currentIndex = 0
        list.forceActiveFocus()
    }

    function selectAll() {
        if (root.model === null)
            return
        const ids = []
        for (let row = 0; row < root.model.count; row += 1)
            ids.push(root.idAt(row))
        root.selectMany(ids)
    }

    function handleClick(index, shift) {
        if (root.model === null)
            return
        root.takeFocus()
        list.currentIndex = index
        const id = root.idAt(index)
        if (shift && root.anchorIndex >= 0) {
            const first = Math.min(root.anchorIndex, index)
            const last = Math.max(root.anchorIndex, index)
            const ids = []
            for (let row = first; row <= last; row += 1)
                ids.push(root.idAt(row))
            root.selectMany(ids)
            return
        }
        root.toggle(id)
        root.anchorIndex = index
    }

    function handleKey(event) {
        const ctrl = (event.modifiers & Qt.ControlModifier) !== 0
        if (event.key === Qt.Key_Delete) {
            const ids = root.targetIds()
            if (ids.length > 0)
                root.purgeRequested(ids)
            return true
        }
        if (event.key === Qt.Key_F5) {
            if (root.model !== null)
                root.model.refresh()
            return true
        }
        if (event.key === Qt.Key_Escape) {
            root.clearSelection()
            return true
        }
        if (ctrl && event.key === Qt.Key_A) {
            root.selectAll()
            return true
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            const id = root.currentId()
            if (id !== "")
                root.restoreRequested([id])
            return true
        }
        if (event.key === Qt.Key_Space) {
            const id = root.currentId()
            if (id !== "")
                root.toggle(id)
            return true
        }
        return false
    }

    Connections {
        target: root.model

        function onLoadedChanged() {
            if (root.model !== null && root.model.loaded) {
                list.currentIndex = -1
                root.clearSelection()
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    // Actions. The counts are part of the label because the list itself is the
    // only other place a selection is visible.
    Item {
        id: actions
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 40

        /// The buttons keep their size; the heading gives way. With the preview
        /// open the pane is ~440 wide and four buttons already take most of it,
        /// so the title can only have what is left of the row — never an
        /// overlap, which is what anchored-and-unclipped produced.
        readonly property real titleRoom: Math.max(0, actionRow.x - 26)

        Text {
            id: heading
            x: 16
            width: Math.min(heading.implicitWidth, actions.titleRoom)
            visible: width >= 44
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("回收站")
            color: root.textColor
            font.pixelSize: 13
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }

        Text {
            x: heading.visible ? heading.x + heading.width + 10 : 16
            width: Math.max(0, actionRow.x - x - 12)
            visible: width >= 40
            anchors.verticalCenter: parent.verticalCenter
            text: root.model === null ? ""
                : (root.model.loaded ? root.model.status : qsTr("读取回收站…"))
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Row {
            id: actionRow
            anchors.right: parent.right
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6

            ActionButton {
                visible: root.selectionCount > 0
                label: qsTr("恢复 (") + root.selectionCount + ")"
                enabled: !Workspace.busy
                onClicked: root.restoreRequested(root.selectedIdList())
            }

            ActionButton {
                visible: root.selectionCount > 0
                label: qsTr("永久删除 (") + root.selectionCount + ")"
                danger: true
                enabled: !Workspace.busy
                onClicked: root.purgeRequested(root.selectedIdList())
            }

            ActionButton {
                visible: root.model !== null && root.model.count > 0
                label: qsTr("清空回收站")
                enabled: !Workspace.busy
                onClicked: root.emptyRequested()
            }

            ActionButton {
                label: qsTr("刷新")
                enabled: !Workspace.busy
                onClicked: {
                    if (root.model !== null)
                        root.model.refresh()
                }
            }
        }
    }

    // Column labels outside the scroller, like the file list.
    Item {
        id: columnHeader
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: actions.bottom
        height: 30

        Rectangle {
            anchors.fill: parent
            color: root.headerColor

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: root.hairline
            }
        }

        Rectangle {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            width: 14
            height: 14
            radius: Theme.radiusSm
            border.width: 1
            border.color: root.allSelected ? root.accent
                : Theme.hairlineStrong
            color: root.allSelected ? root.accent : "transparent"

            Text {
                anchors.centerIn: parent
                visible: root.allSelected
                text: "\u2713"
                color: Theme.onInteractive
                font.pixelSize: 11
            }

            MouseArea {
                anchors.fill: parent
                onClicked: {
                    if (root.allSelected)
                        root.clearSelection()
                    else
                        root.selectAll()
                }
            }
        }

        Text {
            id: nameHeader
            x: 54
            width: root.compactColumns
                ? Math.max(110, columnHeader.width - root.nameColumnSpan)
                : Math.round(columnHeader.width * 0.32)
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("名称")
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Text {
            visible: !root.compactColumns
            x: nameHeader.x + nameHeader.width + 14
            width: Math.max(0, deletedHeader.x - 14 - nameHeader.x
                            - nameHeader.width - 14)
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("原位置")
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Text {
            id: deletedHeader
            anchors.right: sizeHeader.left
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            width: 118
            horizontalAlignment: Text.AlignRight
            text: qsTr("删除时间")
            color: root.mutedColor
            font.pixelSize: 11
        }

        Text {
            id: sizeHeader
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.verticalCenter: parent.verticalCenter
            width: 68
            horizontalAlignment: Text.AlignRight
            text: qsTr("大小")
            color: root.mutedColor
            font.pixelSize: 11
        }
    }

    EmptyState {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        visible: root.model !== null && root.model.loaded
                 && root.model.error === "" && root.model.count === 0
        glyph: "trash"
        title: qsTr("回收站为空")
        hint: qsTr("删除的文件会先放在这里，可以随时还原。")
    }

    MouseArea {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        acceptedButtons: Qt.RightButton
        enabled: root.model !== null
        onClicked: root.clearSelection()
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: parent.bottom
        model: root.model
        reuseItems: true
        cacheBuffer: 1600
        clip: true
        focus: true
        activeFocusOnTab: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0
        currentIndex: -1

        TapHandler {
            acceptedButtons: Qt.LeftButton
            onTapped: root.takeFocus()
        }

        Keys.onPressed: function (event) {
            if (root.handleKey(event))
                event.accepted = true
        }

        delegate: Item {
            id: row

            required property int index
            required property string entryId
            required property string fileName
            required property string originalParent
            required property string deletedText
            required property string sizeText
            required property bool isDir

            width: ListView.view.width
            height: root.rowHeight

            readonly property bool selected: root.isSelected(row.entryId)

            /// A trashed item is one row, and its "open" gesture is restore —
            /// the same thing a double click does, because the only thing that
            /// can happen to it here is coming back.
            Accessible.role: Accessible.ListItem
            Accessible.name: row.fileName
            Accessible.description: row.isDir ? qsTr("文件夹") : qsTr("文件")
            Accessible.selectable: true
            Accessible.selected: row.selected
            Accessible.onPressAction: root.restoreRequested([row.entryId])

            Rectangle {
                anchors.fill: parent
                color: row.selected
                    ? root.selectionColor
                    : (list.currentIndex === row.index ? root.cursorColor
                        : (rowMouse.containsMouse
                            ? Theme.chrome : "transparent"))
            }

            Rectangle {
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                height: 14
                radius: Theme.radiusSm
                border.width: 1
                border.color: row.selected ? root.accent
                    : Theme.hairlineStrong
                color: row.selected ? root.accent : "transparent"

                Text {
                    anchors.centerIn: parent
                    visible: row.selected
                    text: "\u2713"
                    color: Theme.onInteractive
                    font.pixelSize: 11
                }
            }

            Rectangle {
                x: 34
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                height: 14
                radius: Theme.radiusSm
                color: Theme.inkFaint
            }

            Text {
                id: nameText
                x: 54
                width: root.compactColumns
                    ? Math.max(110, row.width - root.nameColumnSpan)
                    : Math.round(row.width * 0.32)
                anchors.verticalCenter: parent.verticalCenter
                text: row.fileName
                color: root.textColor
                font.pixelSize: 12
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            Text {
                visible: !root.compactColumns
                x: nameText.x + nameText.width + 14
                width: Math.max(0, deletedTextItem.x - 14 - nameText.x
                                - nameText.width - 14)
                anchors.verticalCenter: parent.verticalCenter
                text: row.originalParent !== "" ? row.originalParent : qsTr("原位置未知")
                color: root.mutedColor
                font.pixelSize: 11
                elide: Text.ElideMiddle
                textFormat: Text.PlainText
            }

            Text {
                id: deletedTextItem
                anchors.right: sizeTextItem.left
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 118
                horizontalAlignment: Text.AlignRight
                text: row.deletedText
                color: root.mutedColor
                font.pixelSize: 11
                textFormat: Text.PlainText
            }

            Text {
                id: sizeTextItem
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                width: 68
                horizontalAlignment: Text.AlignRight
                text: row.sizeText
                color: root.mutedColor
                font.pixelSize: 11
                textFormat: Text.PlainText
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                onClicked: function (mouse) {
                    if (mouse.button === Qt.RightButton) {
                        list.forceActiveFocus()
                        list.currentIndex = row.index
                        root.anchorIndex = row.index
                        if (!root.isSelected(row.entryId))
                            root.selectOnly(row.entryId)
                        const point = row.mapToItem(null, mouse.x, mouse.y)
                        root.contextRequested(row.entryId, row.originalParent, point.x, point.y)
                        return
                    }
                    const shift = (mouse.modifiers & Qt.ShiftModifier) !== 0
                    root.handleClick(row.index, shift)
                }
                onDoubleClicked: function (mouse) {
                    if (mouse.button === Qt.LeftButton)
                        root.restoreRequested([row.entryId])
                }
            }
        }
    }

    Column {
        anchors.centerIn: parent
        width: Math.min(420, parent.width - 48)
        spacing: 8
        visible: root.model !== null && root.model.error !== ""
        z: 2

        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: qsTr("无法读取回收站")
            color: Theme.danger
            font.pixelSize: 13
        }

        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: root.model !== null ? root.model.error : ""
            color: root.mutedColor
            font.pixelSize: 11
            wrapMode: Text.Wrap
            elide: Text.ElideRight
            maximumLineCount: 3
        }

        ActionButton {
            anchors.horizontalCenter: parent.horizontalCenter
            label: qsTr("重试")
            onClicked: {
                if (root.model !== null)
                    root.model.refresh()
            }
        }
    }

    component ActionButton: Rectangle {
        id: button

        property string label: ""
        property bool danger: false

        signal clicked()

        width: Math.max(58, buttonLabel.implicitWidth + 22)
        height: 24
        radius: Theme.radiusSm
        opacity: enabled ? 1 : 0.45
        color: danger
            ? (buttonMouse.containsMouse && enabled ? Theme.dangerSolidHover : Theme.dangerSolid)
            : (buttonMouse.containsMouse && enabled
                ? Theme.canvasAlt : "transparent")
        border.width: danger ? 0 : 1
        border.color: root.hairline

        /// The label is the button's name; `danger` is the reason it exists in
        /// red, so it belongs in the description rather than the name.
        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.description: button.danger ? qsTr("不可撤销") : ""
        Accessible.onPressAction: {
            if (button.enabled)
                button.clicked()
        }

        Text {
            id: buttonLabel
            anchors.centerIn: parent
            text: button.label
            color: button.danger ? Theme.onSolid
                : (button.enabled ? root.textColor : root.mutedColor)
            font.pixelSize: 11
        }

        MouseArea {
            id: buttonMouse
            anchors.fill: parent
            hoverEnabled: true
            enabled: button.enabled
            onClicked: button.clicked()
        }
    }
}
