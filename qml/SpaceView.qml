pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// A space surface: the folders and files the user pinned, in insertion order.
//
// Spaces themselves live in the shared `Spaces` singleton; this view keeps only
// the flat item list because that list is what a `ListView` needs. A rename or
// a pin from another window bumps `Spaces.changed`, and the list reloads from
// the same source instead of trying to patch a row locally.
Item {
    id: root

    property string spaceId: ""
    property bool dark: true
    property color accent: Theme.interactive
    property color headerColor: Theme.chrome
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline

    readonly property color contentColor: Theme.canvas
    readonly property color hoverColor: Theme.hover
    readonly property string spaceName: {
        if (spaceId === "" || Spaces.revision < 0)
            return ""
        return Spaces.nameOfId(spaceId)
    }
    readonly property bool preset: Spaces.revision >= 0
        && Spaces.isPresetOfId(spaceId)
    readonly property int itemCount: items.count

    signal openRequested(string path)
    signal renameRequested(string id, string name)
    signal deleteRequested(string id, string name)
    signal errorReported(string message)

    function takeFocus() {
        list.forceActiveFocus()
    }

    function reload() {
        items.clear()
        if (root.spaceId === "")
            return
        const paths = Spaces.itemPaths(root.spaceId)
        const names = Spaces.itemNames(root.spaceId)
        for (let index = 0; index < paths.length; index += 1)
            items.append({ path: paths[index], name: names[index] })
    }

    function remove(path) {
        if (!Spaces.removeItem(root.spaceId, path))
            root.errorReported(Spaces.error !== "" ? Spaces.error : qsTr("无法从空间移除"))
    }

    onSpaceIdChanged: root.reload()
    onVisibleChanged: {
        if (visible)
            root.reload()
    }
    Component.onCompleted: root.reload()

    Connections {
        target: Spaces

        function onChanged() {
            root.reload()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    ListModel {
        id: items
    }

    // The header stays put while the item list scrolls: the space's identity
    // and its two destructive actions should not scroll out of reach.
    Item {
        id: header

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 76

        Column {
            anchors.left: parent.left
            anchors.leftMargin: 24
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4

            Text {
                text: root.spaceName !== "" ? root.spaceName : qsTr("空间")
                color: root.textColor
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }

            Text {
                text: root.itemCount === 0
                    ? qsTr("还没有固定项目")
                    : root.itemCount + qsTr(" 个固定项目")
                color: root.mutedColor
                font.pixelSize: 11
            }
        }

        Row {
            anchors.right: parent.right
            anchors.rightMargin: 18
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6

            ActionButton {
                label: qsTr("重命名")
                enabled: root.spaceId !== ""
                onClicked: root.renameRequested(root.spaceId, root.spaceName)
            }

            ActionButton {
                visible: !root.preset
                label: qsTr("删除空间")
                danger: true
                enabled: root.spaceId !== ""
                onClicked: root.deleteRequested(root.spaceId, root.spaceName)
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: root.hairline
        }
    }

    ListView {
        id: list

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        clip: true
        focus: true
        activeFocusOnTab: true
        model: items
        boundsBehavior: Flickable.StopAtBounds
        cacheBuffer: 300

        delegate: Item {
            id: row

            required property int index
            required property string path
            required property string name

            width: list.width
            height: 48

            /// A file inside the space. The remove cross is a separate button,
            /// so pressing the row opens rather than deletes.
            Accessible.role: Accessible.ListItem
            Accessible.name: row.name
            Accessible.description: row.path
            Accessible.selectable: true
            Accessible.selected: list.currentIndex === row.index
            Accessible.onPressAction: root.openRequested(row.path)

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                radius: Theme.radiusSm
                color: rowMouse.containsMouse ? root.hoverColor : "transparent"
            }

            Rectangle {
                x: 22
                anchors.verticalCenter: parent.verticalCenter
                width: 26
                height: 26
                radius: Theme.radiusSm
                color: Theme.canvasAlt

                Text {
                    anchors.centerIn: parent
                    text: "\u25b1"
                    color: root.accent
                    font.pixelSize: 12
                }
            }

            Column {
                anchors.left: parent.left
                anchors.leftMargin: 58
                anchors.right: removeButton.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                Text {
                    width: parent.width
                    text: row.name
                    color: root.textColor
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }

                Text {
                    width: parent.width
                    text: row.path
                    color: root.mutedColor
                    font.pixelSize: 11
                    elide: Text.ElideMiddle
                }
            }

            ActionButton {
                id: removeButton

                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("移除")
                onClicked: root.remove(row.path)
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                anchors.rightMargin: 74
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openRequested(row.path)
            }
        }
    }

    EmptyState {
        anchors.fill: parent
        visible: items.count === 0
        opacity: root.spaceId === "" ? 0 : 1
        glyph: "diamond"
        title: qsTr("这个空间是空的")
        hint: qsTr("在文件上右键，选择“添加到空间”，把常用位置固定到这里。")
    }

    // The same small action button the recycle bin uses. It is local because
    // this view is the only other place that needs it; a shared file would be
    // a type every QML module would have to import for two call sites.
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

        /// The label is the button's name; a destructive button says so in its
        /// description instead of relying on the colour.
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
