pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window

// The status strip's second state (proposal §7.3.3): as soon as the listing has
// a selection the 28 dp status bar becomes a 36 dp row of named actions **in
// place**. Nothing floats: the eight extra dp come out of the listing above, so
// the first selected row never jumps behind a bar and nothing covers the last
// rows. An empty action bar does not exist — the shell hides this component
// when nothing is selected.
//
// The buttons pair a mark with the verb, because a footer of ten words reads
// as a paragraph and a footer of ten glyphs makes the user learn an alphabet.
// Order follows how often the action is used. On a narrow window the tail
// folds away, but 更多 always opens the complete row menu, so no command has a
// width-dependent dead end.
Item {
    id: bar

    /// Rows selected in the listing this bar acts on.
    property int count: 0
    /// The selected files' total, already formatted by the model. Empty when
    /// nothing measurable is selected (a folders-only selection), in which
    /// case the summary is the count alone rather than a made-up `0 B`.
    property string sizeText: ""
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

    /// Responsive command sets. The summary and transfer queue keep their own
    /// space; only the commands that fit are painted. Anything folded away is
    /// still in 更多, which opens the complete row menu rather than a dead end.
    readonly property int layoutTier: {
        const queue = transferChip.count > 0 ? transferChip.width + 20 : 0
        const selection = bar.width < 720 ? 104 : 164
        const room = bar.width - queue - selection - 32
        if (room >= 780)
            return 3
        if (room >= 580)
            return 2
        if (room >= 450)
            return 1
        return 0
    }

    readonly property var items: [
        { id: "open", label: qsTr("打开"), glyph: "external", tier: 2,
          enabled: bar.count > 0 },
        { id: "cut", label: qsTr("剪切"), glyph: "cut", tier: 0,
          enabled: bar.count > 0 },
        { id: "copy", label: qsTr("复制"), glyph: "copy", tier: 0,
          enabled: bar.count > 0 },
        { id: "paste", label: qsTr("粘贴"), glyph: "paste", tier: 0,
          enabled: bar.clipboardReady && !bar.remote },
        { id: "rename", glyph: "pencil", tier: 1,
          label: bar.count > 1 ? qsTr("批量重命名…") : qsTr("重命名"),
          enabled: bar.count > 0 && !bar.remote },
        { id: "trash", label: qsTr("删除"), glyph: "trash", tier: 0,
          enabled: bar.count > 0 },
        { id: "moveTo", label: qsTr("移动到…"), glyph: "folder", tier: 3,
          enabled: bar.count > 0 && !bar.remote },
        { id: "compress", label: qsTr("压缩"), glyph: "archive", tier: 3,
          enabled: bar.count > 0 && !bar.remote },
        { id: "tags", label: qsTr("标签"), glyph: "star", tier: 2,
          enabled: bar.count > 0, menu: true },
        { id: "more", label: qsTr("更多"), glyph: "more", tier: 0,
          enabled: bar.count > 0, menu: true }
    ]

    readonly property var visibleItems: {
        const visible = []
        for (let index = 0; index < bar.items.length; index += 1) {
            const item = bar.items[index]
            if (item.tier <= bar.layoutTier)
                visible.push(item)
        }
        return visible
    }

    Rectangle {
        anchors.fill: parent
        color: bar.background
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: Theme.ruleWidth(Screen.devicePixelRatio)
        color: Theme.hairline
    }

    // Selection context stays on the left, as in Explorer and Finder. It does
    // not jump to the far edge when the command set changes with the width.
    Item {
        id: summary
        anchors.left: parent.left
        anchors.leftMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        width: Math.min(bar.width < 720 ? 92 : 152, summaryText.implicitWidth + 8)
        height: Theme.controlMd

        Text {
            id: summaryText
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("已选 %1 项").arg(bar.count)
                + (bar.sizeText !== "" && bar.width >= 720 ? " · " + bar.sizeText : "")
            color: Theme.inkSecondary
            font.family: Theme.monoFamily
            font.pixelSize: Theme.typeCaption
            font.features: { "tnum": 1 }
            textFormat: Text.PlainText
            elide: Text.ElideRight

            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
    }

    Rectangle {
        anchors.left: summary.right
        anchors.leftMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        width: Theme.ruleWidth(Screen.devicePixelRatio)
        height: 16
        color: Theme.hairline
    }

    Item {
        id: actionViewport
        anchors.left: summary.right
        anchors.leftMargin: 16
        anchors.verticalCenter: parent.verticalCenter
        width: actions.implicitWidth
        height: Theme.controlMd
        clip: true

        Row {
            id: actions
            height: Theme.controlMd
            spacing: 3

            Repeater {
                model: bar.visibleItems
                delegate: Rectangle {
                    id: button
                    required property var modelData

                    width: buttonRow.implicitWidth + 18
                    height: Theme.controlMd
                    radius: Theme.radiusSm
                    color: buttonMouse.pressed && modelData.enabled ? Theme.selection
                        : (buttonMouse.containsMouse && modelData.enabled
                            ? Theme.hover : "transparent")
                    opacity: modelData.enabled ? 1 : 0.38
                    border.width: button.activeFocus ? 1 : 0
                    border.color: Theme.accent

                    Accessible.role: Accessible.Button
                    Accessible.name: modelData.label
                    Accessible.onPressAction: button.trigger()

                    activeFocusOnTab: modelData.enabled
                    Keys.onReturnPressed: button.trigger()
                    Keys.onEnterPressed: button.trigger()
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

                    Row {
                        id: buttonRow
                        anchors.centerIn: parent
                        spacing: 6

                        Glyph {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 14
                            height: 14
                            kind: button.modelData.glyph
                            tint: button.modelData.id === "trash" ? Theme.danger : Theme.inkSecondary
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: button.modelData.label
                            color: button.modelData.id === "trash" ? Theme.danger : Theme.ink
                            font.pixelSize: Theme.typeBody
                        }
                    }

                    ToolTip { text: button.modelData.label }

                    MouseArea {
                        id: buttonMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: button.trigger()
                    }
                }
            }
        }
    }

    // The queue stays pinned to the edge. On exceptionally narrow windows the
    // full queue remains available after clearing the selection; commands take
    // precedence only when the two surfaces physically cannot coexist.
    TransferChip {
        id: transferChip
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        visible: transferChip.count > 0
            && actionViewport.x + actionViewport.width + 12 < transferChip.x
        onOpenRequested: function (x, y) { bar.transferRequested(x, y) }
    }
}
