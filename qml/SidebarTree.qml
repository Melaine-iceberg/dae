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
    property string material: "none"

    signal navigateRequested(string path)
    signal contextRequested(string path, real x, real y)

    readonly property color panelColor: material === "none"
        ? Theme.chrome
        : (dark ? Qt.rgba(0.05, 0.06, 0.07, 0.30) : Qt.rgba(0.95, 0.96, 0.97, 0.34))

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
                height: 28

                readonly property bool current: root.pathEquals(node.treePath, root.activePath)
                readonly property int indent: 16 + node.depth * 13

                /// A row stands for a folder. The tree view's own chevron is a
                /// separate control, so the row says what it is and where it
                /// leads rather than swallowing the expand action with it.
                Accessible.role: Accessible.TreeItem
                Accessible.name: node.treeName
                Accessible.description: node.treePath
                Accessible.selectable: true
                Accessible.selected: node.current
                Accessible.expandable: node.mayHaveChildren
                Accessible.expanded: node.expanded
                Accessible.onPressAction: root.navigateRequested(node.treePath)

                Rectangle {
                    x: 8
                    width: parent.width - 16
                    height: parent.height
                    radius: Theme.radiusSm
                    color: node.current
                        ? Theme.fill
                        : (nodeMouse.containsMouse ? Theme.hover : "transparent")
                }

                Rectangle {
                    visible: node.current
                    x: 0
                    y: 6
                    width: 2
                    height: parent.height - 12
                    radius: 1
                    color: Theme.signal
                }

                Item {
                    x: node.indent
                    width: 14
                    height: parent.height
                    visible: node.mayHaveChildren
                    // The row's click area fills the node, so the chevron hit
                    // target has to sit above it or expanding a folder would
                    // always navigate instead.
                    z: 1

                    /// The disclosure triangle is its own button: left and right
                    /// on a tree row are open and close in every tree control,
                    /// and a pointer that presses the chevron expects only the
                    /// branch to react.
                    Accessible.role: Accessible.Button
                    Accessible.name: node.expanded ? qsTr("折叠") : qsTr("展开")
                    Accessible.onPressAction: treeModel.toggle(node.treePath)

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "chevron"
                        tint: node.expanded || nodeMouse.containsMouse
                            ? Theme.inkMuted : Theme.inkFaint
                        rotation: node.expanded ? 90 : 0
                    }

                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton
                        cursorShape: Qt.PointingHandCursor
                        onClicked: treeModel.toggle(node.treePath)
                    }
                }

                Glyph {
                    x: node.indent + 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    kind: "folder"
                    tint: node.current || nodeMouse.containsMouse
                        ? Theme.ink : Theme.inkMuted
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: node.indent + 36
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: node.treeName
                    color: Theme.inkMuted
                    font.pixelSize: Theme.typeBody
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
        color: Theme.inkFaint
        font.pixelSize: Theme.typeMicro
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
    }
}
