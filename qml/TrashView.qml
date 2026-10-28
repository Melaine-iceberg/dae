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

    readonly property int rowHeight: 30
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

        Text {
            id: heading
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("回收站")
            color: root.textColor
            font.pixelSize: 13
            font.weight: Font.DemiBold
        }

        Text {
            anchors.left: heading.right
            anchors.leftMargin: 10
            anchors.right: actionRow.left
            anchors.rightMargin: 10
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
            radius: 3
            border.width: 1
            border.color: root.allSelected ? root.accent
                : Theme.hairlineStrong
            color: root.allSelected ? root.accent : "transparent"

            Text {
                anchors.centerIn: parent
                visible: root.allSelected
                text: "\u2713"
                color: Theme.onInteractive
                font.pixelSize: 9
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
            width: Math.round(parent.width * 0.32)
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("名称")
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Text {
            anchors.left: nameHeader.right
            anchors.leftMargin: 14
            anchors.right: deletedHeader.left
            anchors.rightMargin: 14
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

    Column {
        anchors.centerIn: parent
        spacing: 7
        visible: root.model !== null && root.model.loaded
                 && root.model.error === "" && root.model.count === 0

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("回收站是空的")
            color: root.mutedColor
            font.pixelSize: 13
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("删除的文件会先出现在这里")
            color: root.mutedColor
            font.pixelSize: 11
        }
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
                radius: 3
                border.width: 1
                border.color: row.selected ? root.accent
                    : Theme.hairlineStrong
                color: row.selected ? root.accent : "transparent"

                Text {
                    anchors.centerIn: parent
                    visible: row.selected
                    text: "\u2713"
                    color: Theme.onInteractive
                    font.pixelSize: 9
                }
            }

            Rectangle {
                x: 34
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                height: 14
                radius: 3
                color: row.isDir ? root.accent : (root.dark ? "#3b4250" : "#c6ccd5")
            }

            Text {
                id: nameText
                x: 54
                width: Math.round(row.width * 0.32)
                anchors.verticalCenter: parent.verticalCenter
                text: row.fileName
                color: root.textColor
                font.pixelSize: 12
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            Text {
                anchors.left: nameText.right
                anchors.leftMargin: 14
                anchors.right: deletedTextItem.left
                anchors.rightMargin: 14
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
        radius: 5
        opacity: enabled ? 1 : 0.45
        color: danger
            ? (buttonMouse.containsMouse && enabled ? Theme.dangerSolidHover : Theme.dangerSolid)
            : (buttonMouse.containsMouse && enabled
                ? Theme.canvasAlt : "transparent")
        border.width: danger ? 0 : 1
        border.color: root.hairline

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
