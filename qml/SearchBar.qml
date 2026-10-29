import QtQuick

// The navigation bar's search field.
//
// It owns only the input and its two trailing states; the query, the debounce,
// the mode and the result model live in ShellWindow because they have to be
// reset when the tab or the directory changes. `text` is assigned imperatively
// for the same reason: letting `text` be bound would break the moment the user
// types.
//
// The mode lives in `SearchFilterBar` now. The field used to carry a 26 px
// `名 / 文` switch of its own, which meant the current mode was stated twice —
// once here as a character, once in the options strip as a chip — and the two
// could disagree for a frame. One control, one statement: this field types, the
// filter row says what is being searched.
Item {
    id: root

    property string text: ""
    /// "name" or "content". Drives the placeholder only.
    property string mode: "name"
    property bool busy: false
    property bool dark: true
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline

    signal queryEdited(string value)
    signal clearRequested()
    /// Escape on an empty field. The shell answers by collapsing the field back
    /// into its icon; the field itself does not own that state.
    signal dismissRequested()

    implicitWidth: 210
    implicitHeight: 28

    function setText(value) {
        input.text = value
    }

    function focusInput() {
        input.forceActiveFocus()
        input.selectAll()
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusSm
        color: Theme.field
        border.width: 1
        border.color: input.activeFocus
            ? Theme.signal : Theme.hairlineStrong

        Glyph {
            id: leading

            x: 8
            width: 14
            height: 14
            anchors.verticalCenter: parent.verticalCenter
            kind: "search"
            tint: Theme.inkFaint
        }

        Text {
            anchors.left: leading.right
            anchors.leftMargin: 6
            anchors.right: trailing.left
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            visible: input.text.length === 0
            text: root.mode === "content" ? qsTr("搜索文件内容") : qsTr("搜索名称")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeBody
            elide: Text.ElideRight
        }

        TextInput {
            id: input
            anchors.left: leading.right
            anchors.leftMargin: 6
            anchors.right: trailing.left
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            color: root.textColor
            selectionColor: root.accent
            selectedTextColor: Theme.onInteractive
            font.pixelSize: Theme.typeBody
            selectByMouse: true
            clip: true
            activeFocusOnTab: true

            /// The placeholder above is drawn, not declared: without these the
            /// field is announced as an unnamed text box.
            Accessible.role: Accessible.EditableText
            Accessible.name: qsTr("搜索")
            Accessible.description: root.mode === "content"
                ? qsTr("搜索文件内容") : qsTr("搜索名称")

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

            /// The cross is a picture; this strip is the button, and it only
            /// exists as one when there is something to clear.
            enabled: !root.busy && input.text.length > 0
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("清除搜索")
            Accessible.onPressAction: {
                input.text = ""
                root.clearRequested()
            }

            Text {
                anchors.centerIn: parent
                visible: root.busy
                text: "…"
                color: root.mutedColor
                font.pixelSize: Theme.typeName
            }

            Glyph {
                anchors.centerIn: parent
                visible: !root.busy && input.text.length > 0
                width: 10
                height: 10
                kind: "close"
                tint: root.mutedColor
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
