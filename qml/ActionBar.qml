pragma ComponentBehavior: Bound
import QtQuick

// The status strip's second state (proposal §7.3.3): as soon as the listing has
// a selection the 28 dp status bar becomes a 36 dp row of named actions **in
// place**. Nothing floats: the eight extra dp come out of the listing above, so
// the first selected row never jumps behind a bar and nothing covers the last
// rows. An empty action bar does not exist — the shell hides this component
// when nothing is selected.
//
// The buttons are words, not glyphs: "移动到…" has no good icon, and the names
// are what a mouse user reads before the second click. Order follows how often
// the action is used, not how the context menu is grouped; everything rarer
// sits behind 更多, which opens the row menu's second page.
Item {
    id: bar

    /// Rows selected in the listing this bar acts on.
    property int count: 0
    /// The listing is on a network backend: local-only verbs are disabled and
    /// 删除 becomes the permanent-delete confirmation.
    property bool remote: false
    /// Something is on the in-app clipboard, so a paste is possible.
    property bool clipboardReady: false
    /// `Theme.chrome` or the material tint, passed by the shell so this bar and
    /// the status strip it replaces are the same surface.
    property color background: Theme.chrome

    implicitHeight: Theme.actionBar
    height: Theme.actionBar

    signal actionRequested(string id)
    /// `kind` is `tags` or `more`. The point is in window coordinates: the menu
    /// clamps itself into the window, so a footer button opens it upward.
    signal menuRequested(string kind, real x, real y)
    signal transferRequested(real x, real y)

    readonly property var items: [
        { id: "open", label: qsTr("打开"), enabled: bar.count > 0 },
        { id: "cut", label: qsTr("剪切"), enabled: bar.count > 0 },
        { id: "copy", label: qsTr("复制"), enabled: bar.count > 0 },
        { id: "paste", label: qsTr("粘贴"),
          enabled: bar.clipboardReady && !bar.remote },
        { id: "rename",
          label: bar.count > 1 ? qsTr("批量重命名…") : qsTr("重命名"),
          enabled: bar.count > 0 && !bar.remote },
        { id: "trash", label: qsTr("删除"), enabled: bar.count > 0 },
        { id: "moveTo", label: qsTr("移动到…"),
          enabled: bar.count > 0 && !bar.remote },
        { id: "compress", label: qsTr("压缩"),
          enabled: bar.count > 0 && !bar.remote },
        { id: "tags", label: qsTr("标签"), enabled: bar.count > 0, menu: true },
        { id: "more", label: qsTr("更多"), enabled: bar.count > 0, menu: true }
    ]

    Rectangle {
        anchors.fill: parent
        color: bar.background
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 1
        color: Theme.hairline
    }

    Row {
        id: actions
        anchors.left: parent.left
        anchors.leftMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Repeater {
            model: bar.items
            delegate: Rectangle {
                id: button
                required property var modelData

                width: buttonLabel.implicitWidth + (modelData.menu === true ? 26 : 16)
                height: Theme.controlMd
                radius: Theme.radiusSm
                color: buttonMouse.containsMouse && modelData.enabled
                    ? Theme.hover : "transparent"
                opacity: modelData.enabled ? 1 : 0.4
                border.width: button.activeFocus ? 1 : 0
                border.color: Theme.accent

                Accessible.role: Accessible.Button
                Accessible.name: modelData.label
                Accessible.onPressAction: button.trigger()

                /// The buttons are the footer's keyboard stops, in the same
                /// order they are painted; Enter and Space fire them like a
                /// click does.
                activeFocusOnTab: modelData.enabled
                Keys.onReturnPressed: button.trigger()
                Keys.onSpacePressed: button.trigger()

                function trigger() {
                    if (!modelData.enabled)
                        return
                    if (modelData.menu === true) {
                        const point = button.mapToItem(null, 0, 0)
                        bar.menuRequested(modelData.id, point.x, point.y)
                    } else {
                        bar.actionRequested(modelData.id)
                    }
                }

                Text {
                    id: buttonLabel
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: button.modelData.label
                    color: button.modelData.id === "trash" ? Theme.danger : Theme.ink
                    font.pixelSize: Theme.typeBody
                }

                Glyph {
                    anchors.left: buttonLabel.right
                    anchors.leftMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    visible: button.modelData.menu === true
                    kind: "chevronDown"
                    implicitWidth: 10
                    implicitHeight: 10
                    tint: Theme.inkTertiary
                }

                MouseArea {
                    id: buttonMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: button.trigger()
                }
            }
        }
    }

    Text {
        id: summary
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("已选 %1 项").arg(bar.count)
        color: Theme.inkSecondary
        font.family: Theme.monoFamily
        font.pixelSize: Theme.typeCaption
        font.features: { "tnum": 1 }
        textFormat: Text.PlainText

        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }

    // The queue's only door stays open while a selection exists — a paste that
    // starts a transfer must never be invisible. It holds the centre unless the
    // action row reaches it, in which case it moves right rather than covering
    // a button.
    TransferChip {
        id: transferChip
        anchors.verticalCenter: parent.verticalCenter
        x: Math.max(actions.x + actions.width + 12, (bar.width - width) / 2)
        visible: transferChip.count > 0
            && transferChip.x + transferChip.width < summary.x - 8
        onOpenRequested: function (x, y) { bar.transferRequested(x, y) }
    }
}
