pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import dae 1.0
import "Localize.js" as Localize

// The recycle bin: newest deletion first, with restore, purge and empty.
//
// This view only paints and collects intent. The commands and their
// confirmation live on ShellWindow, next to every other dialog; the rows come
// from `TrashModel`, which does the blocking scan off the Qt thread.
Item {
    id: root

    property var model: null
    property color headerColor: Theme.canvas

    /// id -> true. Ids are the system trash ids; a restored entry no longer
    /// exists, so selection is never allowed to outlive a reload.
    property var selected: ({})
    property int selectionVersion: 0
    property int anchorIndex: -1

    readonly property int rowHeight: Theme.rowSpacious

    /// Below this width the four columns cannot all be read: the name column
    /// would be squeezed under the original location and the two would paint on
    /// top of each other. The pane is narrow whenever the preview is open, so
    /// the location column steps out and the name takes its room.
    readonly property bool compactColumns: width < 560
    readonly property color contentColor: Theme.content
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkSecondary
    readonly property color hairline: Theme.hairline
    /// Set while an arrow key owns the cursor, so the focus ring only shows
    /// for keyboard movement, never for a click — the same rule as the list.
    property bool keyboardCursor: false

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

    function handleClick(index, shift, ctrl) {
        if (root.model === null)
            return
        root.keyboardCursor = false
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
        if (ctrl)
            root.toggle(id)
        else
            root.selectOnly(id)
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

    // A quiet explanation, not a warning banner: nothing here is in danger,
    // and a red strip would make an ordinary page feel like a fault.
    Rectangle {
        id: infoBar

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 36
        color: Theme.chrome

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: root.hairline
        }

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 16
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8

            Glyph {
                width: 14
                height: 14
                anchors.verticalCenter: parent.verticalCenter
                kind: "info"
                tint: Theme.inkTertiary
            }

            Text {
                width: Math.max(0, parent.width - 22)
                text: qsTr("回收站中的项目不会被自动清理；还原会回到原位置，原位置不存在时放回主目录。")
                color: Theme.inkSecondary
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }
        }
    }

    // Actions. They never move: restore, destroy and empty are on the page's
    // floor, with the selection figure on the right of the same line.
    Rectangle {
        id: actionBar

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 44
        color: Theme.chrome

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: Theme.ruleWidth(Screen.devicePixelRatio)
            color: root.hairline
        }

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6

            ActionButton {
                label: qsTr("还原")
                enabled: root.selectionCount > 0 && !Workspace.busy
                onClicked: root.restoreRequested(root.selectedIdList())
            }

            ActionButton {
                label: qsTr("永久删除")
                danger: true
                enabled: root.selectionCount > 0 && !Workspace.busy
                onClicked: root.purgeRequested(root.selectedIdList())
            }

            ActionButton {
                visible: root.model !== null && root.model.count > 0
                label: qsTr("清空回收站")
                danger: true
                enabled: !Workspace.busy
                onClicked: root.emptyRequested()
            }
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            visible: text !== ""
            text: root.selectionCount > 0
                ? qsTr("已选 %1 项").arg(root.selectionCount)
                : (root.model !== null && root.model.error !== ""
                    ? Localize.message(root.model.error) : "")
            color: root.mutedColor
            font.pixelSize: Theme.typeMicro
            font.features: { "tnum": 1 }
            elide: Text.ElideRight
        }
    }

    // Column labels outside the scroller, like the file list.
    Item {
        id: columnHeader
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: infoBar.bottom
        height: Theme.rowCompact

        Rectangle {
            anchors.fill: parent
            color: root.headerColor

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Theme.ruleWidth(Screen.devicePixelRatio)
                color: root.hairline
            }
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 10

            Item {
                Layout.preferredWidth: 42
                Layout.preferredHeight: 14

                Rectangle {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    radius: Theme.radiusXs
                    border.width: 1
                    border.color: root.allSelected ? Theme.accent : Theme.line
                    color: root.allSelected ? Theme.accent : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        visible: root.allSelected
                        width: 9
                        height: 9
                        kind: "check"
                        tint: Theme.onAccent
                    }

                    MouseArea {
                        Accessible.role: Accessible.CheckBox
                        Accessible.name: qsTr("全选")
                        Accessible.checkable: true
                        Accessible.checked: root.allSelected
                        anchors.fill: parent
                        onClicked: {
                            if (root.allSelected)
                                root.clearSelection()
                            else
                                root.selectAll()
                        }
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                text: qsTr("名称")
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.55
                elide: Text.ElideRight
            }

            Text {
                Layout.preferredWidth: 180
                visible: !root.compactColumns
                text: qsTr("原位置")
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.55
                elide: Text.ElideRight
            }

            Text {
                Layout.preferredWidth: 76
                horizontalAlignment: Text.AlignRight
                text: qsTr("大小")
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.55
            }

            Text {
                Layout.preferredWidth: 150
                horizontalAlignment: Text.AlignRight
                text: qsTr("删除时间")
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.55
            }
        }
    }

    EmptyState {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: actionBar.top
        visible: root.model !== null && root.model.loaded
                 && root.model.error === "" && root.model.count === 0
        glyph: "trash"
        title: qsTr("回收站为空")
        hint: qsTr("删除的文件会先放在这里，可以随时还原。")
    }

    MouseArea {
        Accessible.ignored: true
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: actionBar.top
        acceptedButtons: Qt.RightButton
        enabled: root.model !== null
        onClicked: root.clearSelection()
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: columnHeader.bottom
        anchors.bottom: actionBar.top
        model: root.model
        reuseItems: true
        cacheBuffer: 1600
        clip: true
        focus: true
        activeFocusOnTab: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0
        currentIndex: -1

        // The tap only moves focus into the list. Focus is not an action, and
        // the list itself already carries the name.
        // a11y:decorative
        TapHandler {
            acceptedButtons: Qt.LeftButton
            onTapped: root.takeFocus()
        }

        Keys.onPressed: function (event) {
            if (event.key === Qt.Key_Up || event.key === Qt.Key_Down
                    || event.key === Qt.Key_Home || event.key === Qt.Key_End
                    || event.key === Qt.Key_PageUp || event.key === Qt.Key_PageDown)
                root.keyboardCursor = true
            if (root.handleKey(event))
                event.accepted = true
        }

        delegate: ListRow {
            id: row

            required property int index
            required property string entryId
            required property string fileName
            required property string originalParent
            required property string deletedText
            required property string sizeText
            required property bool isDir
            required property string category

            width: list.width
            rowHeight: root.rowHeight
            contentInset: 8
            spacing: 10

            readonly property bool selected: root.isSelected(row.entryId)
            readonly property bool rowAboveSelected: row.index > 0
                && root.isSelected(root.idAt(row.index - 1))
            readonly property bool rowBelowSelected: root.model !== null
                && row.index < root.model.count - 1
                && root.isSelected(root.idAt(row.index + 1))

            isSelected: row.selected
            previousSelected: row.rowAboveSelected
            nextSelected: row.rowBelowSelected
            isCurrent: list.currentIndex === row.index
            keyboardCursor: root.keyboardCursor && list.activeFocus
            title: row.fileName
            accessibleName: row.fileName
            accessibleDescription: row.originalParent !== ""
                ? row.originalParent : qsTr("原位置未知")

            /// A trashed item is one row, and its "open" gesture is restore.
            showIcon: false
            showLeading: true

            leading: Row {
                width: 42
                height: 18
                spacing: 10

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    radius: Theme.radiusXs
                    border.width: 1
                    border.color: row.selected ? Theme.accent : Theme.line
                    color: row.selected ? Theme.accent : "transparent"

                    Glyph {
                        anchors.centerIn: parent
                        visible: row.selected
                        width: 9
                        height: 9
                        kind: "check"
                        tint: Theme.onAccent
                    }

                    MouseArea {
                        Accessible.role: Accessible.CheckBox
                        Accessible.name: qsTr("选择此行")
                        Accessible.checkable: true
                        Accessible.checked: row.selected
                        anchors.fill: parent
                        onClicked: {
                            root.takeFocus()
                            list.currentIndex = row.index
                            root.toggle(row.entryId)
                            root.anchorIndex = row.index
                        }
                    }
                }

                FileIcon {
                    anchors.verticalCenter: parent.verticalCenter
                    category: row.isDir ? "folder" : row.category
                    edge: 18
                }
            }

            /// The original location is the column that gives way first; the
            /// delete time and the size stay.
            Text {
                Layout.preferredWidth: 180
                visible: !root.compactColumns
                text: row.originalParent !== "" ? row.originalParent : qsTr("原位置未知")
                color: Theme.inkSecondary
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideMiddle
                textFormat: Text.PlainText
            }

            meta: row.sizeText
            metaWidth: 76

            trailing: Text {
                width: 150
                horizontalAlignment: Text.AlignRight
                text: row.deletedText
                color: Theme.inkSecondary
                font.pixelSize: Theme.typeCaption
                font.features: { "tnum": 1 }
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }

            onClicked: function (mouse) {
                if (mouse.button === Qt.RightButton) {
                    list.forceActiveFocus()
                    root.keyboardCursor = false
                    list.currentIndex = row.index
                    root.anchorIndex = row.index
                    if (!root.isSelected(row.entryId))
                        root.selectOnly(row.entryId)
                    const point = row.mapToItem(null, mouse.x, mouse.y)
                    root.contextRequested(row.entryId, row.originalParent, point.x, point.y)
                    return
                }
                root.handleClick(row.index,
                                 (mouse.modifiers & Qt.ShiftModifier) !== 0,
                                 (mouse.modifiers & Qt.ControlModifier) !== 0)
            }
            onDoubleClicked: function (mouse) {
                if (mouse.button === Qt.LeftButton)
                    root.restoreRequested([row.entryId])
            }
            onActivated: root.restoreRequested([row.entryId])
        }
    }

    /// The bin is one list between the column header and the fixed action
    /// strip; its bar covers exactly that range.
    ScrollBar {
        target: list
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.top: columnHeader.bottom
        anchors.bottom: actionBar.top
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
            font.pixelSize: Theme.typeBody
        }

        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: root.model !== null ? Localize.message(root.model.error) : ""
            color: root.mutedColor
            font.pixelSize: Theme.typeMicro
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
        height: Theme.controlSm
        radius: Theme.radiusSm
        opacity: enabled ? 1 : 0.45
        color: danger
            ? (buttonMouse.containsMouse && enabled ? Theme.dangerSolidHover : Theme.dangerSolid)
            : (buttonMouse.containsMouse && enabled
                ? Theme.inset : "transparent")
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
            font.pixelSize: Theme.typeMicro
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
