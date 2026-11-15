pragma ComponentBehavior: Bound
import QtQuick

// One modal question with a cancel and a confirm button. Permanent deletion
// in the recycle bin is the only flow that needs it, but the dialog is plain
// on purpose: the caller supplies the sentence and the confirm label.
//
// The plate, the scrim, the head and the foot are `Dialog`'s; this file owns
// the sentence and the two-button tab order.
Dialog {
    id: root

    property string message: ""
    property string confirmLabel: qsTr("确定")
    property bool danger: false

    /// 0 = cancel, 1 = the primary action. Tab cycles between the two, which
    /// is the whole tab order this dialog has.
    property int focusIndex: 1

    signal accepted()
    signal canceled()

    /// The sentence is the whole body, so the panel grows with it.
    bodyHeight: messageText.implicitHeight

    function open(title, text, label, isDanger) {
        root.heading = title
        root.message = text
        root.confirmLabel = label || qsTr("确定")
        root.danger = isDanger === true
        // A destructive question has to be answered, not clicked away.
        root.closeOnScrim = !root.danger
        root.focusIndex = 1
        root.show()
    }

    function close() {
        root.visible = false
    }

    function confirm() {
        root.close()
        root.accepted()
    }

    function cancel() {
        root.close()
        root.canceled()
    }

    /// Two buttons, so "cycle" is a toggle. Tab and Shift+Tab both land on the
    /// other one, and neither can escape into the page behind the scrim.
    function cycleFocus() {
        root.focusIndex = root.focusIndex === 1 ? 0 : 1
    }

    onCancelRequested: root.cancel()

    Keys.onReturnPressed: root.confirm()
    Keys.onEnterPressed: root.confirm()
    Keys.onTabPressed: function (event) {
        root.cycleFocus()
        event.accepted = true
    }
    Keys.onBacktabPressed: function (event) {
        root.cycleFocus()
        event.accepted = true
    }

    Text {
        id: messageText

        width: parent.width
        text: root.message
        color: Theme.inkSecondary
        font.pixelSize: Theme.typeBody
        lineHeight: Theme.leadingBody
        wrapMode: Text.Wrap
    }

    footer: Component {
        Item {
            implicitHeight: Theme.controlMd

            DialogButton {
                id: submitButton
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                label: root.confirmLabel
                tone: root.danger ? "danger" : "primary"
                focused: root.focusIndex === 1
                onClicked: root.confirm()
            }

            DialogButton {
                anchors.right: submitButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                label: qsTr("取消")
                tone: "quiet"
                focused: root.focusIndex === 0
                onClicked: root.cancel()
            }
        }
    }
}
