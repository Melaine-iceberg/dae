pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The folder tree under one sidebar volume.
//
// The tree itself is a flat `Repeater`: `SidebarTreeModel` already flattened
// the expanded nodes into depth-tagged rows, so nesting here would only
// duplicate state. Expansion is owned by the model, which is what lets a
// collapsed volume come back the way the user left it after delegate
// recycling. Children are read only when a node is expanded.
Item {
    id: root

    property string rootPath: ""
    property string activePath: ""
    property bool showHidden: true
    property bool dark: true
    property color accent: "#2f6fdf"
    property string material: "none"

    signal navigateRequested(string path)
    signal contextRequested(string path, real x, real y)

    readonly property color panelColor: material === "none"
        ? (dark ? "#121418" : "#eef1f5")
        : (dark ? Qt.rgba(0.05, 0.06, 0.07, 0.30) : Qt.rgba(0.95, 0.96, 0.97, 0.34))
    readonly property color textColor: dark ? "#cfd5df" : "#333940"
    readonly property color mutedColor: dark ? "#7f8797" : "#7a8492"
    readonly property color hoverColor: dark ? "#18ffffff" : "#0d000000"
    readonly property color activeColor: dark ? "#22ffffff" : "#12000000"

    implicitHeight: content.implicitHeight + notice.height

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

    function reload() {
        if (root.rootPath === "")
            return
        treeModel.load(root.rootPath, root.showHidden)
    }

    onRootPathChanged: reload()
    // Re-reading resets the expansion set, because the model filters hidden
    // folders as it reads rather than remembering both views. The preference
    // changes rarely enough that a collapsed tree is the honest trade.
    onShowHiddenChanged: reload()

    SidebarTreeModel {
        id: treeModel
    }

    Column {
        id: content

        width: root.width

        Repeater {
            model: treeModel

            delegate: Item {
                id: node

                required property string treeName
                required property string treePath
                required property int depth
                required property bool expanded
                required property bool mayHaveChildren
                required property bool loaded

                width: content.width
                height: 24

                readonly property bool current: root.pathEquals(node.treePath, root.activePath)
                readonly property int indent: 16 + node.depth * 13

                Rectangle {
                    x: 6
                    width: parent.width - 12
                    height: parent.height
                    radius: 5
                    color: node.current
                        ? root.activeColor
                        : (nodeMouse.containsMouse ? root.hoverColor : "transparent")
                }

                Text {
                    x: node.indent
                    anchors.verticalCenter: parent.verticalCenter
                    width: 12
                    visible: node.mayHaveChildren
                    text: node.expanded ? "▾" : "▸"
                    color: node.expanded ? root.textColor : root.mutedColor
                    font.pixelSize: 8
                    horizontalAlignment: Text.AlignHCenter
                    // The row's click area fills the node, so the chevron hit
                    // target has to sit above it or expanding a folder would
                    // always navigate instead.
                    z: 1

                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton
                        cursorShape: Qt.PointingHandCursor
                        onClicked: treeModel.toggle(node.treePath)
                    }
                }

                Text {
                    x: node.indent + 15
                    anchors.verticalCenter: parent.verticalCenter
                    text: "▱"
                    color: root.dark ? "#8a93a6" : "#6b7482"
                    font.pixelSize: 9
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: node.indent + 29
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: node.treeName
                    color: root.textColor
                    font.pixelSize: 11
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                MouseArea {
                    id: nodeMouse

                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: function (event) {
                        if (event.button === Qt.RightButton) {
                            const point = node.mapToItem(null, event.x, event.y)
                            root.contextRequested(node.treePath, point.x, point.y)
                        } else {
                            root.navigateRequested(node.treePath)
                        }
                    }
                }
            }
        }
    }

    // Empty and error states live under the rows, at the first indent level,
    // so they do not masquerade as a folder. The status string is owned by the
    // model ("No subfolders" / "Could not read folder" / "Loading folder…").
    Text {
        id: notice

        anchors.top: content.bottom
        x: 31
        width: parent.width - 41
        height: visible ? 20 : 0
        visible: treeModel.count === 0 && (treeModel.busy || treeModel.loaded)
        text: treeModel.status
        color: root.mutedColor
        font.pixelSize: 10
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
    }
}
