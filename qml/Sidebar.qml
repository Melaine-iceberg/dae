pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The navigation column: system places, favourites, volumes and recents in a
// single sectioned list. `SidebarModel` does the resolving; this file only
// decides what a row looks like and where a click goes.
Item {
    id: root

    property bool dark: true
    property string material: "none"
    property string activePath: ""
    property color accent: "#2f6fdf"
    property bool showHeader: true
    property bool trashActive: false

    signal navigateRequested(string path)
    signal trashRequested()
    signal contextRequested(string kind, string path, real x, real y)

    readonly property color panelColor: material === "none"
        ? (dark ? "#121418" : "#eef1f5")
        : (dark ? Qt.rgba(0.05, 0.06, 0.07, 0.30) : Qt.rgba(0.95, 0.96, 0.97, 0.34))
    readonly property color textColor: dark ? "#dfe3ea" : "#252a31"
    readonly property color mutedColor: dark ? "#7f8797" : "#7a8492"
    readonly property color hoverColor: dark ? "#18ffffff" : "#0d000000"
    readonly property color activeColor: dark ? "#22ffffff" : "#12000000"

    SidebarModel {
        id: model
    }

    function pathEquals(left, right) {
        if (left === "" || right === "")
            return false
        let a = left.replace(/[\\/]+$/, "")
        let b = right.replace(/[\\/]+$/, "")
        if (Qt.platform.os === "windows") {
            a = a.toLowerCase()
            b = b.toLowerCase()
        }
        return a === b
    }

    function glyph(icon) {
        switch (icon) {
        case "home": return "主"
        case "desktop": return "桌"
        case "documents": return "文"
        case "downloads": return "下"
        case "pictures": return "图"
        case "music": return "音"
        case "videos": return "视"
        case "star": return "★"
        case "disk": return "盘"
        case "recent": return "近"
        default: return "•"
        }
    }

    function tint(icon) {
        switch (icon) {
        case "disk": return dark ? "#8a93a6" : "#6b7482"
        case "star": return dark ? "#d8b45a" : "#b8892a"
        case "recent": return dark ? "#6fa0d8" : "#3f78bd"
        default: return root.accent
        }
    }

    Component.onCompleted: model.refresh()

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            model.refresh()
        }

        function onVolumesChanged() {
            model.refresh()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.panelColor

        Rectangle {
            anchors.right: parent.right
            width: 1
            height: parent.height
            color: root.dark ? "#1affffff" : "#12000000"
        }
    }

    Text {
        id: heading
        visible: root.showHeader
        x: 14
        y: 10
        text: "导航"
        color: root.mutedColor
        font.pixelSize: 11
        font.letterSpacing: 1
    }

    /// The bin is a fixed destination above the sectioned list, not a path the
    /// model resolves.
    Rectangle {
        id: trashRow
        x: 6
        y: root.showHeader ? heading.bottom + 6 : 4
        width: parent.width - 12
        height: 27
        radius: 5
        color: root.trashActive
            ? root.activeColor
            : (trashMouse.containsMouse ? root.hoverColor : "transparent")

        Accessible.role: Accessible.Button
        Accessible.name: "回收站"
        Accessible.onPressAction: root.trashRequested()

        Rectangle {
            x: 6
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: 4
            color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.18)

            Text {
                anchors.centerIn: parent
                text: "回"
                color: root.accent
                font.pixelSize: 10
            }
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 30
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            text: "回收站"
            color: root.trashActive ? root.textColor
                : (root.dark ? "#cfd5df" : "#333940")
            font.pixelSize: 12
            elide: Text.ElideRight
        }

        MouseArea {
            id: trashMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.trashRequested()
        }
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: trashRow.bottom
        anchors.bottom: parent.bottom
        anchors.topMargin: 4
        model: model
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 1
        section.property: "section"
        section.criteria: ViewSection.FullString
        section.delegate: Item {
            required property string section

            width: ListView.view.width
            height: 24

            Text {
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                text: section
                color: root.mutedColor
                font.pixelSize: 10
                font.letterSpacing: 0.6
            }
        }

        delegate: Item {
            id: row

            required property string name
            required property string path
            required property string kind
            required property string detail
            required property string icon
            required property bool isDir

            width: ListView.view.width
            height: 27

            readonly property bool current: !root.trashActive
                && root.pathEquals(row.path, root.activePath)

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                radius: 5
                color: row.current
                    ? root.activeColor
                    : (mouse.containsMouse ? root.hoverColor : "transparent")
            }

            Rectangle {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 16
                height: 16
                radius: 4
                color: Qt.rgba(root.tint(row.icon).r, root.tint(row.icon).g,
                               root.tint(row.icon).b, 0.18)

                Text {
                    anchors.centerIn: parent
                    text: root.glyph(row.icon)
                    color: root.tint(row.icon)
                    font.pixelSize: 10
                }
            }

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 36
                anchors.right: detailText.left
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                text: row.name
                color: row.current ? root.textColor : (root.dark ? "#cfd5df" : "#333940")
                font.pixelSize: 12
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            Text {
                id: detailText
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: row.detail
                color: root.mutedColor
                font.pixelSize: 9
                elide: Text.ElideRight
                width: Math.min(74, implicitWidth)
                horizontalAlignment: Text.AlignRight
            }

            MouseArea {
                id: mouse
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                onClicked: function (event) {
                    if (event.button === Qt.RightButton) {
                        const point = row.mapToItem(null, event.x, event.y)
                        root.contextRequested(row.kind, row.path, point.x, point.y)
                        return
                    }
                    if (row.kind === "recent" && !row.isDir)
                        Workspace.openPath(row.path)
                    else
                        root.navigateRequested(row.path)
                }
            }
        }
    }
}
