import QtQuick

// The path bar's search field.
//
// It owns only the input and its two glyph buttons; the query, debounce and
// result model live in ShellWindow because they have to be reset when the tab
// or directory changes. `text` is assigned imperatively for the same reason:
// letting `text` be bound would break the moment the user types.
//
// The shell shows it only while searching. A 224 px field parked in the
// navigation bar for the whole session costs the breadcrumb a quarter of the
// width it needs, to serve an action taken a few times an hour.

Item {
    id: root

    property string text: ""
    property string mode: "name"
    property bool busy: false
    /// Content search is a local ripgrep walk; network listings only get
    /// filename search, so the mode toggle is disabled there.
    property bool contentEnabled: true
    property bool dark: true
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline

    signal queryEdited(string value)
    signal modeToggled()
    signal clearRequested()
    /// Escape on an empty field. The shell answers by collapsing the field back
    /// into its icon; the field itself does not own that state.
    signal dismissRequested()

    implicitWidth: 224
    implicitHeight: 26

    function setText(value) {
        input.text = value
    }

    function focusInput() {
        input.forceActiveFocus()
        input.selectAll()
    }

    Rectangle {
        anchors.fill: parent
        radius: 6
        color: Theme.hover
        border.width: 1
        border.color: input.activeFocus
            ? Theme.focusRing : root.hairline

        Item {
            id: modeButton
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 26

            Text {
                anchors.centerIn: parent
                text: root.mode === "content" ? qsTr("文") : qsTr("名")
                color: !root.contentEnabled ? root.mutedColor
                    : (root.mode === "content" ? root.accent : root.mutedColor)
                opacity: root.contentEnabled ? 1 : 0.55
                font.pixelSize: 12
                font.bold: root.contentEnabled && root.mode === "content"
            }

            MouseArea {
                anchors.fill: parent
                enabled: root.contentEnabled
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: root.modeToggled()
            }
        }

        Text {
            anchors.left: modeButton.right
            anchors.right: trailing.left
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            visible: input.text.length === 0
            text: root.mode === "content" ? qsTr("搜索文件内容") : qsTr("搜索名称")
            color: root.mutedColor
            font.pixelSize: 12
            elide: Text.ElideRight
        }

        TextInput {
            id: input
            anchors.left: modeButton.right
            anchors.right: trailing.left
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            color: root.textColor
            selectionColor: root.accent
            selectedTextColor: Theme.onInteractive
            font.pixelSize: 13
            selectByMouse: true
            clip: true

            onTextEdited: root.queryEdited(text)
            onAccepted: root.queryEdited(text)

            Keys.onEscapePressed: {
                if (text.length > 0) {
                    text = ""
                    root.queryEdited("")
                    return
                }
                focus = false
                root.dismissRequested()
            }
        }

        Item {
            id: trailing
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 24

            Text {
                anchors.centerIn: parent
                visible: root.busy
                text: "…"
                color: root.mutedColor
                font.pixelSize: 13
            }

            Text {
                anchors.centerIn: parent
                visible: !root.busy && input.text.length > 0
                text: "×"
                color: root.mutedColor
                font.pixelSize: 15
            }

            MouseArea {
                anchors.fill: parent
                enabled: !root.busy && input.text.length > 0
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    input.text = ""
                    root.clearRequested()
                }
            }
        }
    }
}
