import QtQuick

/// The 44px title strip that every dialog starts with.
///
/// A head has three jobs: say what the dialog is (`title`), say what it is
/// about (`subtitle` — a path, a file name, an item count), and let the reader
/// out. Putting the dismiss target at the same spot in every dialog is the
/// point: "top right closes" has to hold whether the dialog is a rename prompt
/// or a properties sheet, and a dialog that only dismisses on the scrim is a
/// dead end for anyone who does not guess that.
///
/// Data dialogs keep their own body padding, so `inset` defaults to 0 and the
/// divider is inset with the body. A dialog with an edge-to-edge body leaves it
/// at 0 too — the divider then reaches the panel sides, which is what
/// `ConfirmDialog` wants.
///
/// `implicitHeight` and `Theme.typeHeading` are a pair, and the type rung is
/// deliberately capped at 17 rather than the Web shell's 22: a title and its
/// subtitle have to share 44 dp, and the hosts that lay this head out by hand
/// would overlap before any layout pass ran. `SettingsPanel` stacks them at
/// y 16 / y 42 in its 176 dp nav column and at y 20 / y 50 in its five page
/// heads, against a 44 dp band; a 20 px title clears the subtitle there by
/// roughly nothing, and 22 px runs into it. Raising the rung means converting
/// those six pairs to a Column first — see the note in `Theme.qml`.
Item {
    id: root

    property string title: ""
    property string subtitle: ""
    property bool showClose: true
    property bool divider: true
    /// Distance from the panel edge to the title. Set by hosts whose body is
    /// already inset, so the head lines up with the first row of content.
    property int inset: 0
    /// Gap between the title and its subtitle. Two lines in 44px need a tight
    /// stack; a head with no subtitle ignores this entirely.
    property int lineGap: 1

    signal closeRequested()

    implicitHeight: Theme.shellTitle

    Column {
        anchors.left: parent.left
        anchors.leftMargin: root.inset
        anchors.right: root.showClose ? closeButton.left : parent.right
        anchors.rightMargin: root.showClose ? 8 : root.inset
        anchors.verticalCenter: parent.verticalCenter
        spacing: root.lineGap

        Text {
            width: parent.width
            text: root.title
            color: Theme.ink
            font.pixelSize: Theme.typeHeading
            font.weight: Font.Medium
            elide: Text.ElideRight
        }

        Text {
            width: parent.width
            visible: root.subtitle.length > 0
            text: root.subtitle
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeMicro
            elide: Text.ElideMiddle
        }
    }

    Item {
        id: closeButton

        anchors.right: parent.right
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        width: 20
        height: Theme.controlXs
        visible: root.showClose

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("关闭")
        Accessible.onPressAction: root.closeRequested()

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusSm
            color: closeMouse.containsMouse ? Theme.hover : "transparent"
        }

        Glyph {
            anchors.centerIn: parent
            width: 10
            height: 10
            kind: "close"
            tint: closeMouse.containsMouse ? Theme.ink : Theme.inkTertiary
        }

        MouseArea {
            id: closeMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.closeRequested()
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: root.divider ? 1 : 0
        visible: root.divider
        color: Theme.hairline
    }
}
