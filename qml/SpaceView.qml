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
    property color accent: "#2f6fdf"
    property color headerColor: dark ? "#14161a" : "#f4f6f8"
    property color textColor: dark ? "#e6e9ef" : "#1c1f24"
    property color mutedColor: dark ? "#8f98aa" : "#657080"
    property color hairline: dark ? "#26ffffff" : "#16000000"

    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color hoverColor: dark ? "#14ffffff" : "#0b000000"
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
            root.errorReported(Spaces.error !== "" ? Spaces.error : "无法从空间移除")
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
                text: root.spaceName !== "" ? root.spaceName : "空间"
                color: root.textColor
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }

            Text {
                text: root.itemCount === 0
                    ? "还没有固定项目"
                    : root.itemCount + " 个固定项目"
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
                label: "重命名"
                enabled: root.spaceId !== ""
                onClicked: root.renameRequested(root.spaceId, root.spaceName)
            }

            ActionButton {
                visible: !root.preset
                label: "删除空间"
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

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                radius: 6
                color: rowMouse.containsMouse ? root.hoverColor : "transparent"
            }

            Rectangle {
                x: 22
                anchors.verticalCenter: parent.verticalCenter
                width: 26
                height: 26
                radius: 7
                color: root.dark ? "#1b1f26" : "#eef1f5"

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
                    font.pixelSize: 10
                    elide: Text.ElideMiddle
                }
            }

            ActionButton {
                id: removeButton

                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                label: "移除"
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

    Column {
        anchors.centerIn: parent
        width: parent.width - 80
        spacing: 6
        visible: items.count === 0
        opacity: root.spaceId === "" ? 0 : 1

        Text {
            width: parent.width
            text: "空间还是空的"
            color: root.textColor
            font.pixelSize: 14
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            width: parent.width
            text: "在文件上右键，选择“添加到空间”，把常用位置固定到这里。"
            color: root.mutedColor
            font.pixelSize: 11
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
        }
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
        radius: 5
        opacity: enabled ? 1 : 0.45
        color: danger
            ? (buttonMouse.containsMouse && enabled ? "#a33232" : "#8f2d2d")
            : (buttonMouse.containsMouse && enabled
                ? (root.dark ? "#2b3038" : "#e9edf2") : "transparent")
        border.width: danger ? 0 : 1
        border.color: root.hairline

        Text {
            id: buttonLabel
            anchors.centerIn: parent
            text: button.label
            color: button.danger ? "#ffffff"
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
